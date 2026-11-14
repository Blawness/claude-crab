//! Claude usage limits, read through a short-lived hidden claude.ai webview that carries
//! the user's own login session. The widget never sees a password or token.
//!
//! The claude.ai page gets no IPC access. Rust runs a small script in it, the script
//! writes its result into the URL fragment, and Rust reads the fragment back.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

use crate::AppState;

pub const WINDOW: &str = "claude";
const HOST: &str = "claude.ai";
/// A JSON endpoint on the same origin: cheap to keep loaded while hidden.
const IDLE_URL: &str = "https://claude.ai/api/organizations";
const LOGIN_URL: &str = "https://claude.ai/login";
const TICK_SECS: u64 = 5;
/// Seconds between usage checks; the user picks this from the menu.
static PROBE_SECS: AtomicU64 = AtomicU64::new(300);

pub fn interval() -> u64 {
    PROBE_SECS.load(Ordering::Relaxed)
}

pub fn set_interval(secs: u64) {
    PROBE_SECS.store(secs, Ordering::Relaxed);
}
const RESULT_PREFIX: &str = "crab=";
/// Seconds between looks at the chat list while chat tracking is on.
const CHAT_SECS: i64 = 10;
/// How long to back off after the chat list could not be read (signed out, offline).
const CHAT_RETRY_SECS: i64 = 60;

static TRACK_CHATS: AtomicBool = AtomicBool::new(true);

pub fn chats_enabled() -> bool {
    TRACK_CHATS.load(Ordering::Relaxed)
}

pub fn set_chats(on: bool) {
    TRACK_CHATS.store(on, Ordering::Relaxed);
}

/// Reads the newest chats from the chat list: id, title and status fields only.
const CHATS_JS: &str = r#"
(async () => {
  const send = (payload) => {
    payload.t = Date.now();
    payload.kind = 'chats';
    const bytes = new TextEncoder().encode(JSON.stringify(payload));
    let hex = '';
    for (const b of bytes) hex += b.toString(16).padStart(2, '0');
    history.replaceState(history.state, '', '#crab=' + hex);
  };
  const get = (path) => fetch(path, { credentials: 'include', headers: { accept: 'application/json' } });
  try {
    if (!window.__crabOrg) {
      const r = await get('/api/organizations');
      if (!r.ok) return send({ ok: false, status: r.status });
      const orgs = await r.json();
      const caps = (o) => o.capabilities || [];
      const org = orgs.find((o) => caps(o).some((c) => /claude_(pro|max)/.test(c)))
        || orgs.find((o) => caps(o).includes('chat'))
        || orgs[0];
      if (!org) return send({ ok: false, status: 0 });
      window.__crabOrg = org.uuid;
    }
    const c = await get('/api/organizations/' + window.__crabOrg + '/chat_conversations?limit=8');
    if (!c.ok) return send({ ok: false, status: c.status });
    const list = await c.json();
    send({
      ok: true,
      chats: list.map((x) => ({
        id: x.uuid,
        name: (x.name || '').slice(0, 90),
        live: x.live_status ?? null,
        needs: !!x.needs_input,
        u: Date.parse(x.updated_at) || null,
        s: x.last_settled_at ? Date.parse(x.last_settled_at) : null,
      })),
    });
  } catch (e) {
    send({ ok: false, status: -1 });
  }
})();
"#;

static FORCE: AtomicBool = AtomicBool::new(false);
static LAST_SEEN: AtomicU64 = AtomicU64::new(0);

/// Runs inside the claude.ai page. Same-origin fetches, so the session cookie applies.
/// The result goes into the URL fragment as hex-encoded JSON.
const PROBE_JS: &str = r#"
(async () => {
  const send = (payload) => {
    payload.t = Date.now();
    const bytes = new TextEncoder().encode(JSON.stringify(payload));
    let hex = '';
    for (const b of bytes) hex += b.toString(16).padStart(2, '0');
    history.replaceState(history.state, '', '#crab=' + hex);
  };
  const get = (path) => fetch(path, { credentials: 'include', headers: { accept: 'application/json' } });
  try {
    const r = await get('/api/organizations');
    if (!r.ok) return send({ ok: false, status: r.status, stage: 'organizations' });
    const orgs = await r.json();
    const caps = (o) => o.capabilities || [];
    const org = orgs.find((o) => caps(o).some((c) => /claude_(pro|max)/.test(c)))
      || orgs.find((o) => caps(o).includes('chat'))
      || orgs[0];
    if (!org) return send({ ok: false, status: 0, stage: 'no-organization' });
    const u = await get('/api/organizations/' + org.uuid + '/usage');
    if (!u.ok) return send({ ok: false, status: u.status, stage: 'usage' });
    send({ ok: true, usage: await u.json() });
  } catch (e) {
    send({ ok: false, status: -1, stage: String(e) });
  }
})();
"#;

/// Appends to %TEMP%\crab-widget.log when CRAB_DEBUG is set. Off by default.
fn debug(msg: &str) {
    if std::env::var_os("CRAB_DEBUG").is_none() {
        return;
    }
    use std::io::Write;
    let path = std::env::temp_dir().join("crab-widget.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {}", now_ms(), msg);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn on_claude(w: &WebviewWindow) -> bool {
    w.url()
        .ok()
        .map(|u| u.scheme() == "https" && u.host_str() == Some(HOST))
        .unwrap_or(false)
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// A result the probe script left in the URL fragment, if there is a new one.
fn take_result(w: &WebviewWindow) -> Option<Value> {
    if !on_claude(w) {
        return None;
    }
    let url = w.url().ok()?;
    let hex = url.fragment()?.strip_prefix(RESULT_PREFIX)?;
    let payload: Value = serde_json::from_slice(&hex_decode(hex)?).ok()?;
    let stamp = payload.get("t").and_then(Value::as_u64)?;
    if LAST_SEEN.swap(stamp, Ordering::Relaxed) == stamp {
        return None;
    }
    Some(payload)
}

/// Opens the claude.ai window, or returns the one that is already open.
fn open_window(app: &AppHandle, url: &str, visible: bool) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(WINDOW) {
        return Some(w);
    }
    WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::External(url.parse().ok()?))
        .title("Sign in to Claude - Crab Widget")
        .inner_size(480.0, 720.0)
        .visible(visible)
        .build()
        .ok()
}

fn pause(secs: u64) {
    thread::sleep(Duration::from_secs(secs));
}

/// Waits for claude.ai to load, runs the probe script, and waits for its answer.
fn probe_once(w: &WebviewWindow) -> Option<Value> {
    for _ in 0..12 {
        if on_claude(w) {
            break;
        }
        pause(1);
    }
    if !on_claude(w) {
        return None;
    }
    pause(1);
    run_js(w, PROBE_JS)
}

/// Runs a script in the claude.ai page and waits for the result it leaves behind.
fn run_js(w: &WebviewWindow, js: &str) -> Option<Value> {
    let _ = w.eval(js);
    for _ in 0..8 {
        pause(1);
        if let Some(payload) = take_result(w) {
            return Some(payload);
        }
    }
    None
}

/// Without chat tracking, the claude.ai window only exists while a usage check runs (or while
/// the user logs in), so its renderer does not sit in memory. With chat tracking it stays
/// open, hidden, because the chat list is polled every few seconds.
pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut since = interval(); // check usage right after start
        let mut chat_since = CHAT_SECS;
        loop {
            pause(TICK_SECS);
            since += TICK_SECS;
            chat_since += TICK_SECS as i64;
            let track = chats_enabled();
            let existing = app.get_webview_window(WINDOW);
            let login_open = existing.as_ref().is_some_and(|w| w.is_visible().unwrap_or(false));
            // While the login window is open, poll fast so it closes itself right after login.
            let usage_due = login_open || since >= interval() || FORCE.swap(false, Ordering::Relaxed);
            let chats_due = track && !login_open && chat_since >= CHAT_SECS;
            if !usage_due && !chats_due {
                // Chat tracking was switched off: let go of the idle hidden window.
                if !track && !login_open {
                    if let Some(w) = existing {
                        let _ = w.destroy();
                    }
                }
                continue;
            }
            let Some(w) = existing.or_else(|| open_window(&app, IDLE_URL, false)) else { continue };

            let mut signed_in = false;
            if usage_due {
                since = 0;
                debug(&format!("probe login_open={login_open} url={:?}", w.url().map(|u| u.to_string())));
                signed_in = match probe_once(&w) {
                    Some(payload) => handle_result(&app, payload),
                    None => {
                        debug("probe: no result");
                        if !login_open {
                            let out = json!({ "state": "error", "status": -2, "at": now_ms() });
                            *app.state::<AppState>().last_usage.lock().unwrap() = out.clone();
                            let _ = app.emit_to("main", "usage", out);
                        }
                        false
                    }
                };
            }
            if track && !w.is_visible().unwrap_or(false) && on_claude(&w) && (chats_due || usage_due) {
                chat_since = 0;
                match run_js(&w, CHATS_JS) {
                    Some(payload) if payload["ok"] == true => {
                        // Status fields only (no titles), to see what the list reports.
                        if let Some(list) = payload["chats"].as_array() {
                            let brief: Vec<String> = list
                                .iter()
                                .map(|c| format!("{}:live={} needs={} u={} s={}", c["id"].as_str().unwrap_or("").chars().take(6).collect::<String>(), c["live"], c["needs"], c["u"], c["s"]))
                                .collect();
                            debug(&format!("chats {}", brief.join(" | ")));
                        }
                        crate::chats::record(&payload)
                    }
                    other => {
                        debug(&format!("chats: no list ({:?})", other.map(|p| p["status"].clone())));
                        chat_since = CHAT_SECS - CHAT_RETRY_SECS;
                    }
                }
            }

            // A login window closes once signed in. A hidden window stays only for chat tracking.
            let visible = w.is_visible().unwrap_or(false);
            if (visible && signed_in) || (!visible && !track) {
                let _ = w.destroy();
            }
        }
    });
}

pub fn force_refresh() {
    FORCE.store(true, Ordering::Relaxed);
}

/// Shows the login page. Runs on its own thread: creating a window from the UI thread
/// can deadlock on Windows.
pub fn open_login(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        if let Some(w) = app.get_webview_window(WINDOW) {
            let _ = w.navigate(LOGIN_URL.parse().unwrap());
            let _ = w.show();
            let _ = w.set_focus();
        } else if let Some(w) = open_window(&app, LOGIN_URL, true) {
            let _ = w.set_focus();
        }
    });
}

/// Publishes a probe result to the widget. Returns whether usage data came back.
fn handle_result(app: &AppHandle, payload: Value) -> bool {
    debug(&format!("result {}", payload.to_string().chars().take(600).collect::<String>()));
    let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let out = if ok {
        let mut data = payload.get("usage").cloned().unwrap_or(Value::Null);
        // Preview a mood without burning real usage: set CRAB_FAKE_PCT=85 before launching.
        if let Some(fake) = std::env::var("CRAB_FAKE_PCT").ok().and_then(|v| v.parse::<f64>().ok()) {
            if let Some(slot) = data.pointer_mut("/five_hour/utilization") {
                *slot = json!(fake);
            }
        }
        notify_thresholds(app, &data);
        json!({ "state": "ok", "data": data, "at": now_ms() })
    } else {
        let status = payload.get("status").and_then(Value::as_i64).unwrap_or(-1);
        let state = if status == 401 || status == 403 { "login" } else { "error" };
        json!({ "state": state, "status": status, "stage": payload.get("stage"), "at": now_ms() })
    };
    *app.state::<AppState>().last_usage.lock().unwrap() = out.clone();
    let _ = app.emit_to("main", "usage", out);
    ok
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

fn notify_thresholds(app: &AppHandle, data: &Value) {
    let Some(pct) = data.pointer("/five_hour/utilization").and_then(Value::as_f64) else { return };
    let state = app.state::<AppState>();
    let mut last = state.last_pct.lock().unwrap();
    if let Some(prev) = *last {
        if prev < 90.0 && pct >= 90.0 {
            notify(app, "Claude limit almost reached", &format!("Your 5-hour session is {pct:.0}% used."));
        } else if prev < 75.0 && pct >= 75.0 {
            notify(app, "Claude limit at 75%", &format!("Your 5-hour session is {pct:.0}% used."));
        } else if prev >= 75.0 && pct < 10.0 {
            notify(app, "Claude limit has reset", "Your 5-hour session starts fresh.");
        }
    }
    *last = Some(pct);
}
