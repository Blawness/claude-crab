//! Jumps to a Claude Code session from the board: the Claude desktop app is asked to open
//! that exact session through its own `claude://` link; a terminal session gets the window
//! that owns its process brought to the front.

use std::fs;

use serde_json::Value;
use sysinfo::{Pid, ProcessesToUpdate, System};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE, SW_SHOWNORMAL,
};

use crate::hook::sessions_dir;

/// The desktop app only accepts its own local session ids in a link.
fn is_host_id(id: &str) -> bool {
    id.strip_prefix("local_")
        .is_some_and(|rest| !rest.is_empty() && rest.len() <= 64 && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// Opens a link with whatever app is registered for it (a browser for https, Claude for
/// claude://). Passing a link to explorer.exe is not the same thing: for a protocol owned by
/// a Store app it just opens a File Explorer window.
pub fn shell_open(link: &str) -> bool {
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (verb, target) = (wide("open"), wide(link));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success as a value above 32.
    result as isize > 32
}

/// The pid of the process that ran this hook's parent, i.e. the Claude Code process itself.
pub fn parent_pid() -> Option<u32> {
    let me = sysinfo::get_current_pid().ok()?;
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[me]), true);
    sys.process(me)?.parent().map(|p| p.as_u32())
}

unsafe extern "system" fn collect(hwnd: HWND, out: LPARAM) -> i32 {
    // Only real top-level windows: visible, titled, and not owned by another window.
    if IsWindowVisible(hwnd) != 0 && GetWindowTextLengthW(hwnd) > 0 && GetWindow(hwnd, GW_OWNER).is_null() {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let list = &mut *(out as *mut Vec<(u32, usize)>);
        list.push((pid, hwnd as usize));
    }
    1
}

/// Brings forward the window of `pid`, or of the nearest ancestor that has one
/// (a shell has no window of its own; its terminal does).
fn focus_owner(pid: u32) -> bool {
    let mut windows: Vec<(u32, usize)> = Vec::new();
    unsafe { EnumWindows(Some(collect), &mut windows as *mut _ as LPARAM) };

    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut current = Some(Pid::from_u32(pid));
    for _ in 0..12 {
        let Some(at) = current else { break };
        if let Some((_, hwnd)) = windows.iter().find(|(owner, _)| *owner == at.as_u32()) {
            let hwnd = *hwnd as HWND;
            unsafe {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                return SetForegroundWindow(hwnd) != 0;
            }
        }
        current = sys.process(at).and_then(|p| p.parent());
    }
    false
}

/// Opens the session with this id. Returns a short word saying what was done.
pub fn open_session(session: &str) -> &'static str {
    let clean: String = session.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect();
    let Some(record) = fs::read_to_string(sessions_dir().join(format!("{clean}.json")))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        return "unknown";
    };

    if let Some(chat) = record["chat"].as_str() {
        let ok = !chat.is_empty() && chat.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        return if ok && shell_open(&format!("claude://claude.ai/chat/{chat}")) { "link" } else { "missing" };
    }
    if let Some(host) = record["host"].as_str().filter(|h| is_host_id(h)) {
        let link = format!("claude://code/continue?session={host}&source=crab_widget");
        if shell_open(&link) {
            return "link";
        }
    }
    match record["ppid"].as_u64() {
        Some(pid) if focus_owner(pid as u32) => "window",
        _ => "missing",
    }
}
