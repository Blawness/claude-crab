//! Regular Claude chats (claude.ai, the desktop app's chat tab, mobile) have no hooks.
//! Their status comes from the claude.ai chat list, which the hidden claude.ai window polls.
//! Each active chat is written as a session file, so the board, the sounds and the reminders
//! treat it exactly like a Claude Code session. Only the list is read, never message text.

use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::hook::{now_ms, sessions_dir};

const PREFIX: &str = "chat-";
/// `live_status` values that mean nothing is being generated.
const IDLE_WORDS: [&str; 8] = ["idle", "settled", "complete", "done", "none", "stopped", "finished", "ended"];

/// A reply is in progress when the chat was updated well after it last settled.
const SETTLE_LAG_MS: u64 = 5_000;
/// An unsettled chat older than this is treated as abandoned, not as still generating.
const LAG_MAX_AGE_MS: u64 = 15 * 60 * 1000;

/// Last seen `updated_at` per chat, to notice a reply that started and finished between polls.
static SEEN: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

fn clean_id(raw: &str) -> String {
    raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(48).collect()
}

/// Updates the session files from one poll of the chat list.
pub fn record(payload: &Value) {
    let Some(list) = payload["chats"].as_array() else { return };
    let dir = sessions_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let now = now_ms();
    let mut guard = SEEN.lock().unwrap();
    // The first poll only takes stock: nothing that happened before the widget started counts.
    let first_poll = guard.is_none();
    let seen = guard.get_or_insert_with(HashMap::new);
    for chat in list {
        let id = clean_id(chat["id"].as_str().unwrap_or(""));
        if id.is_empty() {
            continue;
        }
        let path = dir.join(format!("{PREFIX}{id}.json"));
        let prev: Value = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let was_active = !prev.is_null() && prev["state"] != "done";

        let updated = chat["u"].as_u64().unwrap_or(0);
        let changed = !first_poll && seen.get(&id) != Some(&updated);
        seen.insert(id.clone(), updated);

        let live = chat["live"].as_str().unwrap_or("");
        let live_active = !live.is_empty() && !IDLE_WORDS.iter().any(|w| live.to_lowercase().contains(w));
        let unsettled = chat["s"].as_u64().map_or(true, |settled| updated > settled + SETTLE_LAG_MS)
            && now.saturating_sub(updated) < LAG_MAX_AGE_MS;
        let generating = live_active || (updated > 0 && unsettled);
        let state = if chat["needs"] == true {
            "waiting"
        } else if generating {
            "working"
        } else if was_active || changed {
            // `changed` alone means a reply came and went between two polls.
            "done"
        } else {
            continue; // an idle chat that was never seen active stays off the board
        };

        let out = json!({
            "session": format!("{PREFIX}{id}"),
            "kind": "chat",
            "chat": id,
            "state": state,
            "live": live,
            "tool": "",
            "coding": false,
            "cwd": "",
            "project": "Claude chat",
            "title": chat["name"].as_str().unwrap_or(""),
            "last": "",
            "done": 0,
            "total": 0,
            "agents": 0,
            "since": if was_active { prev["since"].as_u64().unwrap_or(now) } else { now },
            "updated": if state == "done" && prev["state"] == "done" { prev["updated"].as_u64().unwrap_or(now) } else { now },
        });
        let tmp = dir.join(format!("{PREFIX}{id}.tmp"));
        if fs::write(&tmp, out.to_string()).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

/// Removes every chat record, for when chat tracking is switched off.
pub fn clear() {
    let Ok(entries) = fs::read_dir(sessions_dir()) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(PREFIX) {
            let _ = fs::remove_file(entry.path());
        }
    }
}
