//! `crab-widget.exe --hook`: run by Claude Code hooks. Reads the hook JSON on stdin and
//! records that session's state in a small file that the running widget watches.
//! It never prints anything and never blocks: a failure here must not disturb Claude Code.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const TITLE_CHARS: usize = 90;
const LAST_CHARS: usize = 140;
/// How much of the end of a transcript to scan for the last reply.
const TRANSCRIPT_TAIL: u64 = 200_000;

pub fn sessions_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("crab-widget")
        .join("sessions")
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// First meaningful line of a text, on one line, cut to `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let flat = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        flat.chars().take(max - 1).collect::<String>() + "…"
    }
}

/// The last thing Claude said in this session, read from the end of its transcript.
fn last_reply(transcript: &str) -> Option<String> {
    let mut file = fs::File::open(transcript).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TRANSCRIPT_TAIL))).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let tail = String::from_utf8_lossy(&bytes);
    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        if entry["type"] != "assistant" {
            continue;
        }
        let Some(blocks) = entry["message"]["content"].as_array() else { continue };
        let text = blocks.iter().rev().find_map(|b| (b["type"] == "text").then(|| b["text"].as_str()).flatten());
        if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
            return Some(one_line(text, LAST_CHARS));
        }
    }
    None
}

/// Step counts from a task-list tool call, if this is one.
fn steps_from(tool: &str, input: &Value, done: u64, total: u64) -> (u64, u64) {
    let is_plan = tool == "TodoWrite" || tool.ends_with("update_plan");
    if tool.ends_with("wrap_up") {
        return (total, total);
    }
    if !is_plan {
        return (done, total);
    }
    let list = input["todos"].as_array().or_else(|| input["items"].as_array());
    if let Some(list) = list {
        let finished = list.iter().filter(|i| i["status"] == "completed").count() as u64;
        return (finished, list.len() as u64);
    }
    let mut done = done;
    if input["advance"] == true {
        done += 1;
    }
    if let Some(marked) = input["done"].as_array() {
        done += marked.len() as u64;
    }
    (done.min(total), total)
}

pub fn run() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(&input) else { return };
    let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("");

    let session: String = text("session_id")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect();
    if session.is_empty() {
        return;
    }
    let dir = sessions_dir();
    let path = dir.join(format!("{session}.json"));
    let event = text("hook_event_name");

    let prev: Value = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let num = |key: &str| prev[key].as_u64().unwrap_or(0);
    let now = now_ms();
    let tool_name = text("tool_name");

    let mut state = prev["state"].as_str().unwrap_or("working").to_string();
    let mut tool = prev["tool"].as_str().unwrap_or("").to_string();
    let mut coding = prev["coding"].as_bool().unwrap_or(false);
    let mut since = prev["since"].as_u64().unwrap_or(now);
    let mut title = prev["title"].as_str().unwrap_or("").to_string();
    let mut last = prev["last"].as_str().unwrap_or("").to_string();
    let (mut done, mut total, mut agents) = (num("done"), num("total"), num("agents"));

    match event {
        "UserPromptSubmit" => {
            // A new turn: everything about the previous one is reset.
            state = "working".into();
            tool.clear();
            coding = false;
            since = now;
            title = one_line(text("prompt"), TITLE_CHARS);
            last.clear();
            done = 0;
            total = 0;
            agents = 0;
        }
        "PreToolUse" => {
            // These two tools exist to ask the user something, so the session waits on them.
            let asks = tool_name == "AskUserQuestion" || tool_name == "ExitPlanMode";
            if prev.is_null() || state == "done" {
                since = now;
            }
            state = if asks { "waiting" } else { "working" }.into();
            tool = tool_name.into();
            // A turn counts as coding once it uses a file or shell tool; until then it is just talking.
            if matches!(tool_name, "Edit" | "Write" | "NotebookEdit" | "Bash" | "PowerShell" | "Read" | "Grep" | "Glob") {
                coding = true;
            }
            (done, total) = steps_from(tool_name, &v["tool_input"], done, total);
        }
        "PostToolUse" => {
            state = "working".into();
            tool.clear();
        }
        "Notification" => {
            let needs_user = match text("notification_type") {
                "idle_prompt" | "auth_success" => false,
                "" => text("message").to_lowercase().contains("permission"),
                _ => true,
            };
            if !needs_user {
                return;
            }
            state = "waiting".into();
        }
        "Stop" => {
            state = "done".into();
            tool.clear();
            agents = 0;
            let reply = text("last_assistant_message");
            if !reply.is_empty() {
                last = one_line(reply, LAST_CHARS);
            } else if let Some(reply) = last_reply(text("transcript_path")) {
                last = reply;
            }
        }
        "SessionEnd" => {
            if prev.is_null() {
                return;
            }
            state = "done".into();
            tool.clear();
            agents = 0;
        }
        "SubagentStart" => agents += 1,
        "SubagentStop" => agents = agents.saturating_sub(1),
        "TaskCreated" => total += 1,
        "TaskCompleted" => done = (done + 1).min(total.max(done + 1)),
        _ => return,
    }

    // Recorded once per session, so a click on the board can jump to it.
    let host = match prev["host"].as_str() {
        Some(known) => known.to_string(),
        None => std::env::var("CLAUDE_CODE_HOST_SESSION_ID").unwrap_or_default(),
    };
    let ppid = prev["ppid"].as_u64().or_else(|| crate::focus::parent_pid().map(u64::from));

    let cwd = if text("cwd").is_empty() { prev["cwd"].as_str().unwrap_or("") } else { text("cwd") };
    let out = json!({
        "session": session,
        "state": state,
        "tool": tool,
        "coding": coding,
        "cwd": cwd,
        "title": title,
        "last": last,
        "done": done,
        "total": total,
        "agents": agents,
        "host": host,
        "ppid": ppid,
        "since": since,
        "updated": now,
    });
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let tmp = dir.join(format!("{session}.tmp"));
    if fs::write(&tmp, out.to_string()).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}
