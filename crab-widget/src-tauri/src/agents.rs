//! Background agents: Claude Code runs started by the widget itself (`claude -p`), one
//! process per task. Each gets its own session id, so the hooks report it to the board like
//! any other session, and its own git worktree when the folder is a repository, so parallel
//! agents never write over each other.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;

use serde_json::{json, Value};

use crate::hook::{now_ms, sessions_dir};

/// Keeps a console window from flashing up for each child process.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const OUTPUT_TAIL: usize = 1200;
const MAX_KEPT: usize = 30;

static AGENTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn data_dir() -> PathBuf {
    sessions_dir().with_file_name("agents")
}

fn registry_path() -> PathBuf {
    data_dir().join("agents.json")
}

fn output_path(id: &str) -> PathBuf {
    data_dir().join(format!("{id}.out"))
}

fn save(agents: &[Value]) {
    let _ = fs::create_dir_all(data_dir());
    let _ = fs::write(registry_path(), json!(agents).to_string());
}

/// Loads the agents of earlier runs. One still marked running belongs to a widget that has
/// since closed, so nobody is waiting on it any more.
pub fn load() {
    let mut list: Vec<Value> = fs::read_to_string(registry_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    for agent in &mut list {
        if agent["status"] == "running" {
            agent["status"] = json!("detached");
        }
    }
    *AGENTS.lock().unwrap() = list;
}

fn claude_exe() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let exe = Path::new(&appdata).join("npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe");
    exe.exists().then_some(exe)
}

fn quiet(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null());
    cmd
}

fn is_git_repo(folder: &Path) -> bool {
    quiet("git")
        .arg("-C")
        .arg(folder)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn set(id: &str, change: impl FnOnce(&mut Value)) {
    let mut agents = AGENTS.lock().unwrap();
    if let Some(agent) = agents.iter_mut().find(|a| a["id"] == id) {
        change(agent);
    }
    save(&agents);
}

/// Starts one agent on `prompt` in `folder`. Returns its session id.
pub fn spawn(folder: &str, prompt: &str, worktree: bool, board: bool) -> Result<String, String> {
    if !Path::new(folder).is_dir() {
        return Err(format!("Folder not found: {folder}"));
    }
    // Use the folder's one true spelling. Git and Claude Code compare paths as text, so a
    // short name (LAPTOP~1) or different letter case makes the worktree check fail.
    let resolved = fs::canonicalize(folder).map_err(|e| e.to_string())?;
    let resolved = resolved.to_string_lossy();
    let folder = resolved.strip_prefix(r"\\?\").unwrap_or(&resolved);
    let folder_path = Path::new(folder);
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Write what the agent should do.".into());
    }
    let exe = claude_exe().ok_or("Claude Code is not installed where the widget expects it (npm global).")?;

    let id = uuid::Uuid::new_v4().to_string();
    let isolated = worktree && is_git_repo(folder_path);
    let tree = format!("crab-{}", &id[..8]);
    let name: String = prompt.lines().next().unwrap_or("").chars().take(60).collect();

    let _ = fs::create_dir_all(data_dir());
    let out = fs::File::create(output_path(&id)).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;

    let mut cmd = Command::new(exe);
    cmd.current_dir(folder_path)
        .args(["-p", prompt, "--session-id", &id, "--permission-mode", "acceptEdits", "-n", &name])
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .creation_flags(CREATE_NO_WINDOW);
    if isolated {
        cmd.args(["--worktree", &tree]);
    }
    if board {
        // A task that came from Board may report back there.
        cmd.args(["--allowedTools", BOARD_TOOLS]);
    }
    own_login(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("Could not start Claude Code: {e}"))?;

    {
        let mut agents = AGENTS.lock().unwrap();
        agents.push(json!({
            "id": id,
            "prompt": prompt,
            "folder": folder,
            "worktree": if isolated { json!(tree) } else { Value::Null },
            "pid": child.id(),
            "started": now_ms(),
            "status": "running",
        }));
        let extra = agents.len().saturating_sub(MAX_KEPT);
        for old in agents.drain(..extra) {
            let _ = fs::remove_file(output_path(old["id"].as_str().unwrap_or("")));
        }
        save(&agents);
    }

    let watched = id.clone();
    thread::spawn(move || {
        let code = child.wait().ok().and_then(|s| s.code());
        set(&watched, |agent| {
            // A stop request already set the status; do not overwrite it.
            if agent["status"] == "running" {
                agent["status"] = json!(if code == Some(0) { "done" } else { "failed" });
            }
            agent["exit"] = json!(code);
            agent["ended"] = json!(now_ms());
        });
    });
    Ok(id)
}

/// The agent must sign in as a normal Claude Code run, not inherit this session's wiring.
fn own_login(cmd: &mut Command) {
    for (key, _) in std::env::vars() {
        if key.starts_with("CLAUDE") || key.starts_with("ANTHROPIC") || key.ends_with("_OAUTH") {
            cmd.env_remove(key);
        }
    }
}

/// The Board connector of the user's Claude account, as Claude Code names it.
const BOARD_TOOLS: &str = "mcp__claude_ai_Board";
const BOARD_PROMPT: &str = r#"Use the Board MCP tools to list the tasks that are not finished yet. Reply with ONLY a JSON array, no prose and no code fence. Each item: {"id": string (the exact id the Board tools return for the task and accept back, never a made-up or shortened code), "title": string, "status": string, "workspace": string, "due": string or null, "assignee": string or null, "description": string (at most 300 characters)}. At most 30 items."#;

/// Pulls the first JSON array out of a reply that may have stray text around it.
fn json_array(text: &str) -> Option<Value> {
    let (start, end) = (text.find('[')?, text.rfind(']')?);
    serde_json::from_str::<Value>(text.get(start..=end)?).ok().filter(Value::is_array)
}

/// Open tasks from Board. The widget cannot call Board itself (access belongs to the Claude
/// account), so one short Claude Code run fetches them. Slow: expect most of a minute.
pub fn board_tasks() -> Result<Value, String> {
    let exe = claude_exe().ok_or("Claude Code is not installed where the widget expects it (npm global).")?;
    let mut cmd = Command::new(exe);
    cmd.args(["-p", BOARD_PROMPT, "--allowedTools", BOARD_TOOLS, "--no-session-persistence"])
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    if let Some(home) = std::env::var_os("USERPROFILE") {
        cmd.current_dir(home);
    }
    own_login(&mut cmd);
    let out = cmd.output().map_err(|e| format!("Could not start Claude Code: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    json_array(&text).ok_or_else(|| {
        let shown: String = text.trim().chars().take(200).collect();
        format!("Board did not return a task list. Claude said: {}", if shown.is_empty() { "(nothing)" } else { &shown })
    })
}

#[cfg(test)]
mod tests {
    use super::json_array;

    #[test]
    fn finds_the_array_inside_extra_text() {
        let found = json_array("Here you go:\n[{\"id\":\"a\"}]\nDone.").unwrap();
        assert_eq!(found[0]["id"], "a");
        assert!(json_array("no list here").is_none());
        assert!(json_array("{\"not\": \"an array\"}").is_none());
    }
}

/// Stops a running agent and everything it started.
pub fn stop(id: &str) -> bool {
    let pid = AGENTS
        .lock()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id && a["status"] == "running")
        .and_then(|a| a["pid"].as_u64());
    let Some(pid) = pid else { return false };
    set(id, |agent| agent["status"] = json!("stopped"));
    quiet("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status().is_ok_and(|s| s.success())
}

/// Forgets an agent that is no longer running.
pub fn remove(id: &str) {
    let mut agents = AGENTS.lock().unwrap();
    agents.retain(|a| a["id"] != id || a["status"] == "running");
    save(&agents);
    if !agents.iter().any(|a| a["id"] == id) {
        let _ = fs::remove_file(output_path(id));
    }
}

/// Every known agent, newest first, each with the end of what it printed.
pub fn list() -> Value {
    let agents = AGENTS.lock().unwrap().clone();
    let with_output: Vec<Value> = agents
        .into_iter()
        .rev()
        .map(|mut agent| {
            let text = fs::read(output_path(agent["id"].as_str().unwrap_or("")))
                .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
                .unwrap_or_default();
            let skip = text.chars().count().saturating_sub(OUTPUT_TAIL);
            agent["output"] = json!(text.chars().skip(skip).collect::<String>());
            agent
        })
        .collect();
    json!(with_output)
}

pub fn is_agent(id: &str) -> bool {
    AGENTS.lock().unwrap().iter().any(|a| a["id"] == id)
}
