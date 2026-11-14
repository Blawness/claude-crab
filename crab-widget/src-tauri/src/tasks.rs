//! Watches the per-session state files written by `--hook` and tells the widget what
//! every Claude Code session is doing: working, waiting on the user, or finished.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::hook::{now_ms, sessions_dir};
use crate::sound::{self, Cue};
use crate::AppState;

const POLL: Duration = Duration::from_secs(1);
/// A "working" file nobody has touched for this long is a session that died without a Stop.
const WORKING_STALE_MS: u64 = 30 * 60 * 1000;
/// How long the crab celebrates a finished turn before going back to idle.
const DONE_PULSE_MS: u64 = 8 * 1000;
/// How long a finished session stays on the board.
const DONE_BOARD_MS: u64 = 30 * 60 * 1000;
const DONE_KEEP_MS: u64 = 24 * 60 * 60 * 1000;
/// How often the "needs you" sound and notification repeat until the user answers.
const NUDGE_EVERY: Duration = Duration::from_secs(30);
const BOARD_MAX: usize = 8;

fn project_of(session: &Value) -> String {
    let cwd = session["cwd"].as_str().unwrap_or("");
    Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Waiting first (the user must act), then working, then finished; newest first within each.
fn rank(session: &Value) -> u8 {
    match session["state"].as_str().unwrap_or("") {
        "waiting" => 0,
        "working" => 1,
        _ => 2,
    }
}

/// The summary sent to the widget, plus every session on record for change tracking.
fn snapshot() -> (Value, Vec<Value>) {
    let now = now_ms();
    let mut sessions: Vec<Value> = Vec::new();
    if let Ok(entries) = fs::read_dir(sessions_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(mut v) = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok())
            else {
                continue;
            };
            let age = now.saturating_sub(v["updated"].as_u64().unwrap_or(0));
            let keep = match v["state"].as_str().unwrap_or("") {
                "done" if age > DONE_KEEP_MS => {
                    let _ = fs::remove_file(&path);
                    false
                }
                "done" => age <= DONE_BOARD_MS,
                "working" | "waiting" => age <= WORKING_STALE_MS,
                _ => false,
            };
            if keep {
                if v["kind"] != "chat" {
                    v["project"] = json!(project_of(&v));
                }
                sessions.push(v);
            }
        }
    }
    sessions.sort_by_key(|s| (rank(s), u64::MAX - s["updated"].as_u64().unwrap_or(0)));

    let count = |state: &str| sessions.iter().filter(|s| s["state"] == state).count();
    let (working, waiting) = (count("working"), count("waiting"));
    // What the crab itself acts out: the top active session, or a brief cheer for a fresh finish.
    let top = sessions.first().filter(|s| {
        s["state"] != "done" || now.saturating_sub(s["updated"].as_u64().unwrap_or(0)) <= DONE_PULSE_MS
    });
    let mut out = match top {
        Some(top) => json!({
            "state": top["state"],
            "tool": top["tool"],
            "coding": top["coding"],
            "kind": top["kind"],
            "project": top["project"],
            "since": top["since"],
            "ended": if top["state"] == "done" { top["updated"].clone() } else { Value::Null },
        }),
        None => json!({ "state": "idle" }),
    };
    out["count"] = json!(working + waiting);
    out["working"] = json!(working);
    out["waiting"] = json!(waiting);
    out["sessions"] = json!(sessions.iter().take(BOARD_MAX).collect::<Vec<_>>());
    (out, sessions)
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// "Fix the export (next-absen)": what the session is about, for a notification body.
fn describe(session: &Value) -> String {
    let title = session["title"].as_str().unwrap_or("");
    let project = session["project"].as_str().unwrap_or("");
    match (title.is_empty(), project.is_empty()) {
        (false, false) => format!("{title} ({project})"),
        (false, true) => title.to_string(),
        (true, false) => project.to_string(),
        (true, true) => "A Claude Code session".to_string(),
    }
}

fn cue(app: &AppHandle, which: Cue) {
    if app.state::<AppState>().sound.load(Ordering::Relaxed) {
        sound::play(which);
    }
}

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut last = String::new();
        // State of every session at the previous poll, to spot the moment one changes.
        let mut known: HashMap<String, String> = HashMap::new();
        let mut first_pass = true;
        let mut last_nudge = Instant::now();
        loop {
            thread::sleep(POLL);
            let (snap, sessions) = snapshot();

            let mut now_known = HashMap::new();
            for s in &sessions {
                let id = s["session"].as_str().unwrap_or("").to_string();
                let state = s["state"].as_str().unwrap_or("").to_string();
                let before = known.get(&id).map(String::as_str);
                // Nothing fires on the first pass: those sessions changed before the widget started.
                if !first_pass && before != Some(state.as_str()) {
                    if state == "waiting" {
                        cue(&app, Cue::NeedsInput);
                        notify(&app, "Claude needs you", &describe(s));
                        last_nudge = Instant::now();
                    } else if state == "done" && (before.is_some() || s["kind"] == "chat") {
                        cue(&app, Cue::Done);
                        let took = s["updated"].as_u64().unwrap_or(0).saturating_sub(s["since"].as_u64().unwrap_or(0));
                        let mins = (took + 30_000) / 60_000;
                        let how_long = if mins == 0 { "under a minute".to_string() } else { format!("{mins} min") };
                        notify(&app, "Claude finished", &format!("{} · {how_long}", describe(s)));
                    }
                }
                now_known.insert(id, state);
            }
            known = now_known;
            first_pass = false;

            // Keep reminding while anything still waits on the user.
            let waiting: Vec<&Value> = sessions.iter().filter(|s| s["state"] == "waiting").collect();
            if !waiting.is_empty()
                && last_nudge.elapsed() >= NUDGE_EVERY
                && app.state::<AppState>().nudge.load(Ordering::Relaxed)
            {
                last_nudge = Instant::now();
                cue(&app, Cue::NeedsInput);
                let body = if waiting.len() == 1 {
                    describe(waiting[0])
                } else {
                    format!("{} sessions are waiting, starting with: {}", waiting.len(), describe(waiting[0]))
                };
                notify(&app, "Claude is still waiting for you", &body);
            }

            let key = snap.to_string();
            if key == last {
                continue;
            }
            last = key;
            *app.state::<AppState>().last_task.lock().unwrap() = snap.clone();
            let _ = app.emit_to("main", "task", snap);
        }
    });
}
