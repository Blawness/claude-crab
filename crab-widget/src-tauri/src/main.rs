#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod focus;
mod agents;
mod chats;
mod hook;
mod sound;
mod system;
mod tasks;
mod usage;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_window_state::StateFlags;

const SMALL: (f64, f64) = (150.0, 150.0);
/// Width with the panel open; its height follows the panel's content.
const PANEL_WIDTH: f64 = 310.0;
const USAGE_PAGE: &str = "https://claude.ai/settings/usage";
/// Choices for how often usage is checked, in seconds.
const INTERVALS: [(u64, &str); 3] = [(120, "2 minutes"), (300, "5 minutes"), (900, "15 minutes")];

pub struct AppState {
    pub last_usage: Mutex<Value>,
    pub last_pct: Mutex<Option<f64>>,
    pub last_task: Mutex<Value>,
    pub mini: AtomicBool,
    pub remaining: AtomicBool,
    pub smooth: AtomicBool,
    pub click_through: AtomicBool,
    pub sound: AtomicBool,
    pub nudge: AtomicBool,
}

struct MenuHolder {
    menu: Menu<Wry>,
    mini: CheckMenuItem<Wry>,
    remaining: CheckMenuItem<Wry>,
    smooth: CheckMenuItem<Wry>,
    click_through: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
    sound: CheckMenuItem<Wry>,
    nudge: CheckMenuItem<Wry>,
    chats: CheckMenuItem<Wry>,
    intervals: Vec<(u64, CheckMenuItem<Wry>)>,
}

/* ---------- settings kept between runs ---------- */

fn settings_path() -> PathBuf {
    hook::sessions_dir().with_file_name("settings.json")
}

fn load_settings() -> Value {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

fn save_settings(state: &AppState) {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let out = json!({
        "mini": state.mini.load(Ordering::Relaxed),
        "remaining": state.remaining.load(Ordering::Relaxed),
        "smooth": state.smooth.load(Ordering::Relaxed),
        "interval": usage::interval(),
        "sound": state.sound.load(Ordering::Relaxed),
        "nudge": state.nudge.load(Ordering::Relaxed),
        "chats": usage::chats_enabled(),
    });
    let _ = fs::write(path, out.to_string());
}

fn mode_payload(state: &AppState) -> Value {
    json!({
        "mini": state.mini.load(Ordering::Relaxed),
        "remaining": state.remaining.load(Ordering::Relaxed),
        "smooth": state.smooth.load(Ordering::Relaxed),
        "interval": usage::interval(),
    })
}

/* ---------- commands ---------- */

#[tauri::command]
fn start_drag(window: WebviewWindow) {
    let _ = window.start_dragging();
}

#[tauri::command]
fn set_panel(window: WebviewWindow, open: bool, height: Option<f64>) {
    let (w, h) = if open { (PANEL_WIDTH, height.unwrap_or(420.0).clamp(SMALL.1, 900.0)) } else { SMALL };
    let _ = window.set_size(LogicalSize::new(w, h));
}

#[tauri::command]
fn show_menu(app: AppHandle, window: WebviewWindow) {
    if let Some(holder) = app.try_state::<MenuHolder>() {
        let _ = window.popup_menu(&holder.menu);
    }
}

#[tauri::command]
fn get_state(state: tauri::State<AppState>) -> Value {
    json!({
        "usage": state.last_usage.lock().unwrap().clone(),
        "task": state.last_task.lock().unwrap().clone(),
        "mode": mode_payload(&state),
        "openPanel": std::env::var_os("CRAB_OPEN_PANEL").is_some(),
    })
}

#[tauri::command]
fn open_login(app: AppHandle) {
    usage::open_login(&app);
}

/// Jumps to a session from the board. Runs off the UI thread: it scans processes and windows.
#[tauri::command]
async fn open_session(app: AppHandle, session: String) -> String {
    // A background agent has no window of its own; its home is the manager.
    if agents::is_agent(&session) {
        show_manager(&app, "swarm");
        return "manager".into();
    }
    focus::open_session(&session).to_string()
}

/// Opens the app window on `page` (home, swarm or settings), or brings it forward there.
/// Built off the UI thread: creating a window from it can deadlock on Windows.
fn show_manager(app: &AppHandle, page: &'static str) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(w) = app.get_webview_window("manager") {
            let _ = w.eval(&format!("location.hash = '{page}'"));
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
            return;
        }
        let _ = WebviewWindowBuilder::new(&app, "manager", WebviewUrl::App(format!("manager.html#{page}").into()))
            .title("Claude Crab")
            .inner_size(760.0, 720.0)
            .min_inner_size(560.0, 480.0)
            .build();
    });
}

#[tauri::command]
fn open_manager(app: AppHandle) {
    show_manager(&app, "swarm");
}

/// Every setting the app window shows, in the same ids the tray menu uses.
#[tauri::command]
fn get_settings(app: AppHandle, state: tauri::State<AppState>) -> Value {
    json!({
        "mini": state.mini.load(Ordering::Relaxed),
        "remaining": state.remaining.load(Ordering::Relaxed),
        "smooth": state.smooth.load(Ordering::Relaxed),
        "click": state.click_through.load(Ordering::Relaxed),
        "auto": app.autolaunch().is_enabled().unwrap_or(false),
        "sound": state.sound.load(Ordering::Relaxed),
        "nudge": state.nudge.load(Ordering::Relaxed),
        "chats": usage::chats_enabled(),
        "interval": usage::interval(),
        "intervals": INTERVALS.iter().map(|(secs, label)| json!([secs, label])).collect::<Vec<_>>(),
    })
}

/// A settings-page click does exactly what the tray menu entry with the same id does.
#[tauri::command]
fn set_setting(app: AppHandle, id: String) {
    on_menu(&app, &id);
}

#[tauri::command]
async fn agent_spawn(folder: String, prompt: String, worktree: bool, board: bool) -> Result<String, String> {
    agents::spawn(&folder, &prompt, worktree, board)
}

#[tauri::command]
async fn board_tasks() -> Result<Value, String> {
    agents::board_tasks()
}

#[tauri::command]
fn agent_list() -> Value {
    agents::list()
}

#[tauri::command]
async fn agent_stop(id: String) -> bool {
    agents::stop(&id)
}

#[tauri::command]
fn agent_remove(id: String) {
    agents::remove(&id);
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog().file().blocking_pick_folder().map(|path| path.to_string())
}

#[tauri::command]
fn open_folder(path: String) {
    if std::path::Path::new(&path).is_dir() {
        focus::shell_open(&path);
    }
}

/// Opens the usage page in the user's default browser.
#[tauri::command]
fn open_usage_page() {
    focus::shell_open(USAGE_PAGE);
}

/* ---------- menu ---------- */

fn build_menu(app: &AppHandle, state: &AppState) -> tauri::Result<MenuHolder> {
    let check = |id: &str, text: &str, on: bool| CheckMenuItem::with_id(app, id, text, true, on, None::<&str>);
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);

    let manager = item("manager", "Open Claude Crab...")?;
    let settings = item("settings", "Settings...")?;
    let toggle = item("toggle", "Show / hide")?;
    let refresh = item("refresh", "Refresh now")?;
    let page = item("page", "Open usage page on claude.ai")?;
    let login = item("login", "Sign in to Claude...")?;
    let mini = check("mini", "Mini mode (rings only)", state.mini.load(Ordering::Relaxed))?;
    let remaining = check("remaining", "Show remaining instead of used", state.remaining.load(Ordering::Relaxed))?;
    let smooth = check("smooth", "Smooth animation (60 fps, more CPU)", state.smooth.load(Ordering::Relaxed))?;
    let click_through = check("click", "Click-through (turn off from tray)", false)?;
    let autostart = check("auto", "Start with Windows", app.autolaunch().is_enabled().unwrap_or(false))?;
    let sound = check("sound", "Sounds", state.sound.load(Ordering::Relaxed))?;
    let nudge = check("nudge", "Keep reminding until I answer", state.nudge.load(Ordering::Relaxed))?;
    let test_needs = item("test-needs", "Play the \"needs you\" sound")?;
    let test_done = item("test-done", "Play the \"finished\" sound")?;
    let alerts = Submenu::with_id(app, "alerts", "Alerts", true)?;
    alerts.append(&sound)?;
    alerts.append(&nudge)?;
    alerts.append(&PredefinedMenuItem::separator(app)?)?;
    alerts.append(&test_needs)?;
    alerts.append(&test_done)?;
    let chats = check("chats", "Track Claude chats (keeps a hidden page open)", usage::chats_enabled())?;
    let quit = item("quit", "Exit")?;

    let current = usage::interval();
    let mut intervals = Vec::new();
    for (secs, label) in INTERVALS {
        intervals.push((secs, check(&format!("every-{secs}"), label, secs == current)?));
    }
    let interval_menu = Submenu::with_id(app, "interval", "Check usage every", true)?;
    for (_, entry) in &intervals {
        interval_menu.append(entry)?;
    }

    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &manager, &settings, &toggle, &refresh, &page, &login, &sep1, &mini, &remaining, &smooth, &interval_menu, &chats, &alerts, &click_through, &autostart,
            &sep2, &quit,
        ],
    )?;
    Ok(MenuHolder { menu, mini, remaining, smooth, click_through, autostart, sound, nudge, chats, intervals })
}

fn on_menu(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let holder = app.state::<MenuHolder>();
    let main = app.get_webview_window("main");
    let flip = |flag: &AtomicBool, entry: &CheckMenuItem<Wry>| {
        let on = !flag.load(Ordering::Relaxed);
        flag.store(on, Ordering::Relaxed);
        let _ = entry.set_checked(on);
        on
    };
    let publish = || {
        save_settings(&state);
        let _ = app.emit_to("main", "mode", mode_payload(&state));
    };
    match id {
        "toggle" => {
            if let Some(w) = main {
                if w.is_visible().unwrap_or(true) {
                    let _ = w.hide();
                } else {
                    let _ = w.show();
                }
            }
        }
        "manager" => show_manager(app, "home"),
        "settings" => show_manager(app, "settings"),
        "refresh" => usage::force_refresh(),
        "page" => open_usage_page(),
        "login" => usage::open_login(app),
        "mini" => {
            flip(&state.mini, &holder.mini);
            publish();
        }
        "remaining" => {
            flip(&state.remaining, &holder.remaining);
            publish();
        }
        "smooth" => {
            flip(&state.smooth, &holder.smooth);
            publish();
        }
        "sound" => {
            if flip(&state.sound, &holder.sound) {
                sound::play(sound::Cue::Done);
            }
            save_settings(&state);
        }
        "chats" => {
            let on = !usage::chats_enabled();
            usage::set_chats(on);
            let _ = holder.chats.set_checked(on);
            if !on {
                chats::clear();
            }
            save_settings(&state);
        }
        "nudge" => {
            flip(&state.nudge, &holder.nudge);
            save_settings(&state);
        }
        "test-needs" => sound::play(sound::Cue::NeedsInput),
        "test-done" => sound::play(sound::Cue::Done),
        "click" => {
            let on = flip(&state.click_through, &holder.click_through);
            if let Some(w) = main {
                let _ = w.set_ignore_cursor_events(on);
            }
        }
        "auto" => {
            let launcher = app.autolaunch();
            let on = !launcher.is_enabled().unwrap_or(false);
            let _ = if on { launcher.enable() } else { launcher.disable() };
            let _ = holder.autostart.set_checked(launcher.is_enabled().unwrap_or(false));
        }
        "quit" => app.exit(0),
        other => {
            if let Some(secs) = other.strip_prefix("every-").and_then(|s| s.parse::<u64>().ok()) {
                usage::set_interval(secs);
                for (value, entry) in &holder.intervals {
                    let _ = entry.set_checked(*value == secs);
                }
                publish();
            }
        }
    }
}

fn main() {
    // `--open-session <id>`: jump to a session and exit. Same code path as a click on the board.
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--open-session") {
        let outcome = focus::open_session(args.get(at + 1).map(String::as_str).unwrap_or(""));
        std::process::exit(if outcome == "link" || outcome == "window" { 0 } else { 1 });
    }

    // `--board-tasks`: print what the manager's Board refresh would get, then exit.
    if args.iter().any(|a| a == "--board-tasks") {
        match agents::board_tasks() {
            Ok(tasks) => println!("{tasks}"),
            Err(why) => println!("ERROR: {why}"),
        }
        return;
    }

    // Hook mode: record one Claude Code event and exit, without starting the widget.
    if std::env::args().any(|a| a == "--hook") {
        hook::run();
        return;
    }

    agents::load();
    let saved = load_settings();
    usage::set_chats(saved["chats"].as_bool().unwrap_or(true));
    if let Some(secs) = saved["interval"].as_u64() {
        if INTERVALS.iter().any(|(value, _)| *value == secs) {
            usage::set_interval(secs);
        }
    }

    tauri::Builder::default()
        // must be the first plugin: a second launch exits and opens the app window of the running crab
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
            }
            show_manager(app, "home");
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::POSITION)
                .with_denylist(&[usage::WINDOW])
                .build(),
        )
        .manage(AppState {
            last_usage: Mutex::new(json!({ "state": "waiting" })),
            last_pct: Mutex::new(None),
            last_task: Mutex::new(json!({ "state": "idle", "count": 0 })),
            mini: AtomicBool::new(saved["mini"].as_bool().unwrap_or(false)),
            remaining: AtomicBool::new(saved["remaining"].as_bool().unwrap_or(false)),
            smooth: AtomicBool::new(saved["smooth"].as_bool().unwrap_or(false)),
            click_through: AtomicBool::new(false),
            sound: AtomicBool::new(saved["sound"].as_bool().unwrap_or(true)),
            nudge: AtomicBool::new(saved["nudge"].as_bool().unwrap_or(true)),
        })
        .invoke_handler(tauri::generate_handler![
            start_drag,
            set_panel,
            show_menu,
            get_state,
            open_login,
            open_usage_page,
            open_session,
            open_manager,
            get_settings,
            set_setting,
            agent_spawn,
            board_tasks,
            agent_list,
            agent_stop,
            agent_remove,
            pick_folder,
            open_folder
        ])
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .setup(|app| {
            let handle = app.handle().clone();
            let holder = build_menu(&handle, &app.state::<AppState>())?;
            let mut tray = TrayIconBuilder::with_id("tray").tooltip("Crab Widget").menu(&holder.menu);
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            app.manage(holder);

            // For checking the manager without clicking: CRAB_TEST_AGENT="folder|prompt" starts
            // one agent at launch, CRAB_OPEN_MANAGER=1 opens the manager window.
            if let Ok(spec) = std::env::var("CRAB_TEST_AGENT") {
                if let Some((folder, prompt)) = spec.split_once('|') {
                    let _ = agents::spawn(folder, prompt, true, false);
                }
            }
            if std::env::var_os("CRAB_OPEN_MANAGER").is_some() {
                show_manager(&handle, "home");
            }

            usage::start(handle.clone());
            tasks::start(handle.clone());
            system::start(handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Crab Widget");
}
