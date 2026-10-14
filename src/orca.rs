//! The ONLY place that talks to the Orca CLI (stablyai/orca).
//!
//! Every call is built as an argv vector and executed with
//! `std::process::Command::new(<resolved exe>).args(argv)` — never through a
//! shell and never as a concatenated command string.

// ============================================================================
// ORCA CLI FLAG TABLE
//
// VERIFIED against Orca 1.4.224: every subcommand and flag below appears in
// `orca agent-context --json` of that version (docs/orca-1.4.224/
// agent-context.json) and terminal create/send/wait/read/close were
// round-tripped live (docs/orca-1.4.224/live-roundtrip.txt).
// If a newer Orca changes them, run `orca <cmd> --help`, record it in
// docs/orca-cli-observed.md and fix the constants here — nothing else in the
// codebase hard-codes an Orca flag.
//
// Still UNVERIFIED (shape not seen live, parsed leniently): the JSON results
// of status, repo add, terminal list, worktree create/list/rm,
// orchestration run-create / worker-start / worker-list / worker-stop, and the
// error codes `terminal wait` uses for a blocked agent.
// ============================================================================
pub mod flags {
    /// Binary name; override with env `CODENAME_ORCA_BIN` or config `orca_bin`.
    pub const BIN: &str = "orca";
    /// Appended to every call whose output we parse.
    pub const JSON: &str = "--json";

    pub const VERSION: &[&str] = &["--version"];
    /// `orca status [--json]` → result.runtime.{state,reachable}, result.app.running
    pub const STATUS: &[&str] = &["status"];
    /// `orca open [--json]` — starts the desktop app.
    pub const OPEN: &[&str] = &["open"];
    /// `orca serve --no-pairing --json` — headless runtime (scheduled task).
    /// Binds a websocket on 0.0.0.0:6768; `--no-pairing` is mandatory here.
    pub const SERVE: &[&str] = &["serve", "--no-pairing", "--json"];

    /// `orca repo add --path <path>`
    pub const REPO_ADD: &[&str] = &["repo", "add"];
    pub const PATH: &str = "--path";

    pub const TERMINAL_LIST: &[&str] = &["terminal", "list"];
    /// `orca terminal create [--worktree <selector>] [--title] [--command] [--shell]`
    pub const TERMINAL_CREATE: &[&str] = &["terminal", "create"];
    /// `orca terminal send [--terminal] [--text] [--enter] [--wait-submit <seconds>]`
    pub const TERMINAL_SEND: &[&str] = &["terminal", "send"];
    /// `orca terminal wait [--terminal] --for exit|tui-idle [--timeout-ms <ms>]`
    /// (tui-idle only fires for agent TUIs, never for a plain shell)
    pub const TERMINAL_WAIT: &[&str] = &["terminal", "wait"];
    /// `orca terminal read [--terminal] [--cursor] [--limit] [--screen]`
    pub const TERMINAL_READ: &[&str] = &["terminal", "read"];
    /// `orca terminal close ([--terminal <handle>] [--tab] | --worktree <sel> --all)`
    pub const TERMINAL_CLOSE: &[&str] = &["terminal", "close"];

    pub const TERMINAL: &str = "--terminal";
    /// Worktree selector: `path:<path>` (forward slashes OK), `id:<repo-id>::<path>`,
    /// `branch:<b>`, `name:<n>`, `identity:<i>`, `issue:<n>`, `active`/`current`.
    pub const WORKTREE: &str = "--worktree";
    pub const TITLE: &str = "--title";
    /// Text typed into the terminal's shell after it starts.
    pub const COMMAND: &str = "--command";
    /// Windows host shell: cmd.exe | powershell.exe | pwsh.exe | wsl.exe | bash.exe | git-bash
    pub const SHELL: &str = "--shell";
    pub const TEXT: &str = "--text";
    pub const ENTER: &str = "--enter";
    /// Takes seconds: observe submission/turn start for up to this long.
    pub const WAIT_SUBMIT: &str = "--wait-submit";
    pub const WAIT_SUBMIT_SECS: &str = "15";
    pub const FOR: &str = "--for";
    pub const FOR_TUI_IDLE: &str = "tui-idle";
    pub const TIMEOUT_MS: &str = "--timeout-ms";
    pub const LIMIT: &str = "--limit";
    /// Rendered screen instead of accumulated stream (TUIs repaint lines).
    pub const SCREEN: &str = "--screen";

    /// `orca worktree create --name <n> [--repo <selector>]`
    pub const WORKTREE_CREATE: &[&str] = &["worktree", "create"];
    /// `orca worktree rm --worktree <selector> [--force]` (aliases remove/delete)
    pub const WORKTREE_RM: &[&str] = &["worktree", "rm"];
    pub const WORKTREE_LIST: &[&str] = &["worktree", "list"];
    /// Repo selector: `id:<id>`, `name:<name>`, `path:<path>`.
    pub const REPO: &str = "--repo";
    pub const NAME: &str = "--name";

    /// `orca orchestration run-create --objective <text> [--from <handle>]`
    pub const ORCH_RUN_CREATE: &[&str] = &["orchestration", "run-create"];
    pub const OBJECTIVE: &str = "--objective";
    /// `orca orchestration worker-start (--task|--spec) (--agent|--terminal)
    ///  [--worktree current|<selector>|new-child|new-top-level] [--name] [--repo]
    ///  [--display-name] [--run] [--from]`
    pub const ORCH_WORKER_START: &[&str] = &["orchestration", "worker-start"];
    pub const SPEC: &str = "--spec";
    pub const AGENT: &str = "--agent";
    pub const DISPLAY_NAME: &str = "--display-name";
    pub const NEW_TOP_LEVEL: &str = "new-top-level";
    /// `orca orchestration worker-list [--run <id>]`
    pub const ORCH_WORKER_LIST: &[&str] = &["orchestration", "worker-list"];
    /// `orca orchestration worker-stop --dispatch <id>` (never deletes worktrees)
    pub const ORCH_WORKER_STOP: &[&str] = &["orchestration", "worker-stop"];
    pub const DISPATCH: &str = "--dispatch";
    /// Orca rejects orchestration calls without a sender terminal
    /// (`no_active_sender_terminal`) even though help says it is optional.
    pub const FROM: &str = "--from";
    pub const RUN: &str = "--run";

    /// Agent ids `worker-start --agent` accepts (Orca 1.4.224 notes).
    pub const KNOWN_AGENTS: &[&str] = &[
        "claude",
        "codex",
        "cursor",
        "antigravity",
        "muse",
        "zcode",
        "opencode",
        "opencode2",
    ];
}

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Agent state of a terminal, when Orca reports one. A plain process status
/// such as `running` says nothing about the agent and maps to `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TermState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl TermState {
    pub fn from_str_lenient(s: &str) -> Self {
        let s = s.to_ascii_lowercase();
        if s.contains("block") || s.contains("permission") || s.contains("awaiting") {
            TermState::Blocked
        } else if s.contains("idle") || s.contains("ready") {
            TermState::Idle
        } else if s.contains("working") || s.contains("busy") || s.contains("thinking") {
            TermState::Working
        } else {
            TermState::Unknown
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TerminalInfo {
    pub handle: String,
    pub title: Option<String>,
    pub state: TermState,
    pub worktree: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SendPhase {
    /// Orca saw the agent start a turn — the only state we treat as delivered.
    TurnStarted,
    /// Orca only accepted the keystrokes (`input_accepted`).
    Accepted,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WaitOutcome {
    Done,
    Timeout,
    Blocked,
}

impl WaitOutcome {
    pub fn exit_code(self) -> i32 {
        match self {
            WaitOutcome::Done => 0,
            WaitOutcome::Timeout => 2,
            WaitOutcome::Blocked => 3,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct WorktreeInfo {
    /// Orca worktree id: `<repo-id>::<path>`.
    pub id: String,
    pub path: Option<String>,
    pub branch: Option<String>,
}

impl WorktreeInfo {
    /// Best selector for `worktree rm`: `id:<id>` when known, else `path:`.
    pub fn selector(&self) -> String {
        if !self.id.is_empty() {
            format!("id:{}", self.id)
        } else {
            path_selector(self.path.as_deref().unwrap_or_default())
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct RuntimeStatus {
    pub state: Option<String>,
    pub reachable: bool,
    pub app_running: Option<bool>,
}

impl RuntimeStatus {
    pub fn ready(&self) -> bool {
        self.reachable
    }
    /// "app" when the desktop app hosts the runtime, else "serve".
    pub fn host(&self) -> &'static str {
        if self.app_running == Some(true) {
            "app"
        } else {
            "serve (headless)"
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkerStart {
    pub from: String,
    pub run: String,
    pub spec: String,
    pub agent: String,
    pub repo_id: String,
    pub name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct WorkerInfo {
    pub dispatch: String,
    pub terminal: Option<String>,
    pub worktree_id: Option<String>,
    pub worktree_path: Option<String>,
}

/// Abstraction over the Orca CLI so tests can plug in a fake.
pub trait OrcaApi {
    fn version(&self) -> Result<String>;
    fn status(&self) -> Result<RuntimeStatus>;
    fn open(&self) -> Result<()>;
    fn repo_add(&self, path: &str) -> Result<String>;
    fn terminal_list(&self) -> Result<Vec<TerminalInfo>>;
    fn terminal_create(
        &self,
        worktree: Option<&str>,
        title: &str,
        command: Option<&str>,
    ) -> Result<TerminalInfo>;
    fn terminal_send(&self, handle: &str, text: &str, enter: bool) -> Result<SendPhase>;
    fn terminal_wait(&self, handle: &str, timeout_secs: u64) -> Result<WaitOutcome>;
    fn terminal_read(&self, handle: &str, limit: usize, screen: bool) -> Result<String>;
    fn terminal_close(&self, handle: &str) -> Result<()>;
    fn worktree_create(&self, repo_id: &str, name: &str) -> Result<WorktreeInfo>;
    fn worktree_rm(&self, selector: &str) -> Result<()>;
    fn worktree_list(&self) -> Result<Vec<WorktreeInfo>>;
    fn run_create(&self, from: &str, objective: &str) -> Result<String>;
    fn worker_start(&self, w: &WorkerStart) -> Result<WorkerInfo>;
    fn worker_list(&self, run: &str) -> Result<Value>;
    fn worker_stop(&self, dispatch: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// selectors + argv builders (pure; unit-tested)
// ---------------------------------------------------------------------------

/// `path:<path>` with forward slashes (accepted by Orca on Windows).
pub fn path_selector(p: &str) -> String {
    format!("path:{}", p.replace('\\', "/"))
}

/// `id:<repo-id>` repo selector (passes through an existing `id:`/`path:`).
pub fn repo_selector(repo: &str) -> String {
    if ["id:", "name:", "path:"]
        .iter()
        .any(|p| repo.starts_with(p))
    {
        repo.to_string()
    } else {
        format!("id:{repo}")
    }
}

fn v(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn with_json(mut a: Vec<String>) -> Vec<String> {
    a.push(flags::JSON.to_string());
    a
}

pub fn argv_version() -> Vec<String> {
    v(flags::VERSION)
}

pub fn argv_status() -> Vec<String> {
    with_json(v(flags::STATUS))
}

pub fn argv_open() -> Vec<String> {
    with_json(v(flags::OPEN))
}

pub fn argv_serve() -> Vec<String> {
    v(flags::SERVE)
}

pub fn argv_repo_add(path: &str) -> Vec<String> {
    let mut a = v(flags::REPO_ADD);
    a.extend(v(&[flags::PATH, path]));
    with_json(a)
}

pub fn argv_terminal_list() -> Vec<String> {
    with_json(v(flags::TERMINAL_LIST))
}

pub fn argv_terminal_create(
    worktree: Option<&str>,
    title: &str,
    command: Option<&str>,
    shell: Option<&str>,
) -> Vec<String> {
    let mut a = v(flags::TERMINAL_CREATE);
    if let Some(w) = worktree {
        a.extend(v(&[flags::WORKTREE, w]));
    }
    a.extend(v(&[flags::TITLE, title]));
    if let Some(c) = command {
        a.extend(v(&[flags::COMMAND, c]));
    }
    if let Some(s) = shell {
        a.extend(v(&[flags::SHELL, s]));
    }
    with_json(a)
}

pub fn argv_terminal_send(handle: &str, text: &str, enter: bool) -> Vec<String> {
    let mut a = v(flags::TERMINAL_SEND);
    a.extend(v(&[flags::TERMINAL, handle, flags::TEXT, text]));
    if enter {
        a.push(flags::ENTER.into());
        a.extend(v(&[flags::WAIT_SUBMIT, flags::WAIT_SUBMIT_SECS]));
    }
    with_json(a)
}

pub fn argv_terminal_wait(handle: &str, timeout_secs: u64) -> Vec<String> {
    let mut a = v(flags::TERMINAL_WAIT);
    let ms = timeout_secs.saturating_mul(1000).to_string();
    a.extend(v(&[
        flags::TERMINAL,
        handle,
        flags::FOR,
        flags::FOR_TUI_IDLE,
        flags::TIMEOUT_MS,
        &ms,
    ]));
    with_json(a)
}

pub fn argv_terminal_read(handle: &str, limit: usize, screen: bool) -> Vec<String> {
    let mut a = v(flags::TERMINAL_READ);
    let l = limit.to_string();
    a.extend(v(&[flags::TERMINAL, handle, flags::LIMIT, &l]));
    if screen {
        a.push(flags::SCREEN.into());
    }
    with_json(a)
}

pub fn argv_terminal_close(handle: &str) -> Vec<String> {
    let mut a = v(flags::TERMINAL_CLOSE);
    a.extend(v(&[flags::TERMINAL, handle]));
    with_json(a)
}

pub fn argv_worktree_create(repo_id: &str, name: &str) -> Vec<String> {
    let mut a = v(flags::WORKTREE_CREATE);
    let repo = repo_selector(repo_id);
    a.extend(v(&[flags::NAME, name, flags::REPO, &repo]));
    with_json(a)
}

pub fn argv_worktree_rm(selector: &str) -> Vec<String> {
    let mut a = v(flags::WORKTREE_RM);
    a.extend(v(&[flags::WORKTREE, selector]));
    with_json(a)
}

pub fn argv_worktree_list() -> Vec<String> {
    with_json(v(flags::WORKTREE_LIST))
}

pub fn argv_run_create(from: &str, objective: &str) -> Vec<String> {
    let mut a = v(flags::ORCH_RUN_CREATE);
    a.extend(v(&[flags::OBJECTIVE, objective, flags::FROM, from]));
    with_json(a)
}

pub fn argv_worker_start(w: &WorkerStart) -> Vec<String> {
    let mut a = v(flags::ORCH_WORKER_START);
    let repo = repo_selector(&w.repo_id);
    a.extend(v(&[
        flags::SPEC,
        &w.spec,
        flags::AGENT,
        &w.agent,
        flags::WORKTREE,
        flags::NEW_TOP_LEVEL,
        flags::REPO,
        &repo,
        flags::NAME,
        &w.name,
        flags::DISPLAY_NAME,
        &w.display_name,
        flags::RUN,
        &w.run,
        flags::FROM,
        &w.from,
    ]));
    with_json(a)
}

pub fn argv_worker_list(run: &str) -> Vec<String> {
    let mut a = v(flags::ORCH_WORKER_LIST);
    a.extend(v(&[flags::RUN, run]));
    with_json(a)
}

pub fn argv_worker_stop(dispatch: &str) -> Vec<String> {
    let mut a = v(flags::ORCH_WORKER_STOP);
    a.extend(v(&[flags::DISPATCH, dispatch]));
    with_json(a)
}

// ---------------------------------------------------------------------------
// envelope + lenient decoders
// ---------------------------------------------------------------------------

/// An Orca error: `{"ok":false,"error":{"code","message"}}`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrcaError {
    pub code: String,
    pub message: String,
}

pub const RUNTIME_UNAVAILABLE: &str = "runtime_unavailable";

/// Every Orca response is `{"id","ok","result"|"error","_meta"}`. Returns the
/// `result` (or the value itself when there is no envelope).
pub fn envelope(v: &Value) -> std::result::Result<&Value, OrcaError> {
    match v.get("ok").and_then(Value::as_bool) {
        Some(false) => {
            let e = v.get("error").cloned().unwrap_or(Value::Null);
            Err(OrcaError {
                code: pick_str(&e, &["code"]).unwrap_or_else(|| "unknown".into()),
                message: pick_str(&e, &["message"]).unwrap_or_else(|| e.to_string()),
            })
        }
        Some(true) => Ok(v.get("result").unwrap_or(v)),
        None => Ok(v),
    }
}

pub fn orca_error(argv: &[String], e: &OrcaError) -> anyhow::Error {
    let cmd = argv
        .iter()
        .take_while(|a| a.starts_with(|c: char| c.is_ascii_lowercase()))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if e.code == RUNTIME_UNAVAILABLE {
        anyhow!(
            "orca {cmd}: the Orca runtime is not running ({}). Run `codename startup` \
             (starts the `serve_task` scheduled task or `orca open`), or open the Orca app.",
            e.message
        )
    } else {
        anyhow!("orca {cmd} failed [{}]: {}", e.code, e.message)
    }
}

pub fn pick_str(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        match v.get(*k) {
            Some(Value::String(s)) if !s.is_empty() => return Some(s.clone()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

/// Depth-first search for the first string under any of `keys`.
pub fn find_str(v: &Value, keys: &[&str]) -> Option<String> {
    if let Some(s) = pick_str(v, keys) {
        return Some(s);
    }
    match v {
        Value::Object(m) => m.values().find_map(|x| find_str(x, keys)),
        Value::Array(a) => a.iter().find_map(|x| find_str(x, keys)),
        _ => None,
    }
}

/// Finds the first array: the value itself or under a known key (one level of
/// nesting is searched too, e.g. `result.terminals`).
pub fn pick_array<'a>(v: &'a Value, keys: &[&str]) -> Vec<&'a Value> {
    if let Some(a) = v.as_array() {
        return a.iter().collect();
    }
    for k in keys {
        if let Some(Value::Array(a)) = v.get(*k) {
            return a.iter().collect();
        }
    }
    if let Value::Object(m) = v {
        for inner in m.values().filter(|x| x.is_object()) {
            for k in keys {
                if let Some(Value::Array(a)) = inner.get(*k) {
                    return a.iter().collect();
                }
            }
        }
    }
    Vec::new()
}

const STATE_KEYS: &[&str] = &["agentState", "agentStatus", "tuiState", "state", "status"];

/// Decodes `result.terminal` (or a bare terminal row).
pub fn decode_terminal(v: &Value) -> Option<TerminalInfo> {
    let v = match v.get("terminal") {
        Some(t) if t.is_object() => t,
        _ => v,
    };
    let handle = pick_str(v, &["handle", "terminalHandle", "terminalId", "id"])?;
    let state = pick_str(v, STATE_KEYS)
        .map(|s| TermState::from_str_lenient(&s))
        .unwrap_or(TermState::Unknown);
    Some(TerminalInfo {
        handle,
        title: pick_str(v, &["title", "name", "label"]),
        state,
        worktree: pick_str(v, &["worktreeId", "worktree", "cwd", "path"]),
    })
}

pub fn decode_terminal_list(v: &Value) -> Vec<TerminalInfo> {
    pick_array(v, &["terminals", "items", "list"])
        .into_iter()
        .filter_map(decode_terminal)
        .collect()
}

/// `result.send.prompt.stages` — `turn_started` (or any turn stage) means
/// delivered; `input_accepted` / `accepted: true` only means typed.
pub fn decode_send(v: &Value) -> SendPhase {
    let send = v.get("send").unwrap_or(v);
    let stages: Vec<String> = send
        .get("prompt")
        .and_then(|p| p.get("stages"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_ascii_lowercase))
                .collect()
        })
        .unwrap_or_default();
    if stages.iter().any(|s| s.contains("turn")) {
        SendPhase::TurnStarted
    } else if !stages.is_empty() || send.get("accepted").and_then(Value::as_bool) == Some(true) {
        SendPhase::Accepted
    } else {
        SendPhase::Unknown
    }
}

/// `terminal wait`: ok → done; error.code `timeout` → 2; a blocked/permission
/// code → 3 (code names unverified); anything else is an error.
pub fn decode_wait(full: &Value) -> std::result::Result<WaitOutcome, OrcaError> {
    match envelope(full) {
        Ok(_) => Ok(WaitOutcome::Done),
        Err(e) => {
            let c = e.code.to_ascii_lowercase();
            if c.contains("timeout") || c.contains("timed_out") {
                Ok(WaitOutcome::Timeout)
            } else if c.contains("block") || c.contains("permission") || c.contains("awaiting") {
                Ok(WaitOutcome::Blocked)
            } else {
                Err(e)
            }
        }
    }
}

/// `result.terminal.tail` (array of lines) joined; tolerant of other shapes.
pub fn decode_read(v: &Value) -> String {
    let t = v.get("terminal").filter(|t| t.is_object()).unwrap_or(v);
    for k in ["tail", "lines", "rows", "screen"] {
        if let Some(Value::Array(a)) = t.get(k) {
            return a
                .iter()
                .map(|l| {
                    l.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| l.to_string())
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
    if let Some(s) = pick_str(t, &["text", "content", "screen", "output"]) {
        return s;
    }
    match t {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `result.runtime.{state,reachable}`, `result.app.running`.
pub fn decode_status(v: &Value) -> RuntimeStatus {
    let rt = v.get("runtime").unwrap_or(&Value::Null);
    RuntimeStatus {
        state: pick_str(rt, &["state", "status"]),
        reachable: rt
            .get("reachable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        app_running: v
            .get("app")
            .and_then(|a| a.get("running"))
            .and_then(Value::as_bool),
    }
}

pub fn decode_repo_id(v: &Value) -> Option<String> {
    v.get("repo")
        .and_then(|r| pick_str(r, &["id", "repoId"]))
        .or_else(|| pick_str(v, &["repoId", "id"]))
        .or_else(|| find_str(v, &["repoId"]))
}

pub fn decode_worktree(v: &Value) -> Option<WorktreeInfo> {
    let w = v.get("worktree").filter(|t| t.is_object()).unwrap_or(v);
    let id = pick_str(w, &["worktreeId", "id"]).unwrap_or_default();
    let path = pick_str(w, &["path", "worktreePath", "cwd"]);
    if id.is_empty() && path.is_none() {
        return None;
    }
    Some(WorktreeInfo {
        id,
        path,
        branch: pick_str(w, &["branch", "branchName"]),
    })
}

pub fn decode_worker(v: &Value) -> Option<WorkerInfo> {
    let dispatch = v
        .get("dispatch")
        .and_then(|d| pick_str(d, &["id", "dispatchId"]))
        .or_else(|| find_str(v, &["dispatchId"]))
        .or_else(|| pick_str(v, &["id"]))?;
    let terminal = v
        .get("terminal")
        .and_then(|t| pick_str(t, &["handle"]))
        .or_else(|| find_str(v, &["agentTerminalHandle", "terminalHandle"]));
    let wt = v.get("worktree").filter(|w| w.is_object());
    Some(WorkerInfo {
        dispatch,
        terminal,
        worktree_id: wt
            .and_then(|w| pick_str(w, &["id", "worktreeId"]))
            .or_else(|| find_str(v, &["worktreeId"])),
        worktree_path: wt
            .and_then(|w| pick_str(w, &["path"]))
            .or_else(|| find_str(v, &["worktreePath"])),
    })
}

// ---------------------------------------------------------------------------
// executable resolution + BatBadBut guard
// ---------------------------------------------------------------------------

/// What a resolved executable is. `.cmd`/`.bat` files are run by Windows
/// through `cmd.exe`, whose argument parsing differs from normal programs;
/// passing untrusted text to them allows argument injection
/// (CVE-2024-24576, "BatBadBut"). codename never does that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BinKind {
    Exe,
    Batch,
    Other,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ResolvedBin {
    pub path: PathBuf,
    pub kind: BinKind,
}

pub fn bin_kind(p: &Path) -> BinKind {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("exe") | Some("com") => BinKind::Exe,
        Some("cmd") | Some("bat") => BinKind::Batch,
        _ => BinKind::Other,
    }
}

pub const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Resolves `name` like the OS would. On Windows (`windows = true`) a bare
/// name is tried with each PATHEXT extension in order (a name that already
/// has an extension is tried as-is first); elsewhere the exact file name is
/// looked up. Names containing a path separator are not searched on PATH.
pub fn resolve_on_path(
    name: &str,
    path_var: Option<&OsStr>,
    pathext: Option<&str>,
    windows: bool,
) -> Option<PathBuf> {
    let exts: Vec<String> = pathext
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(DEFAULT_PATHEXT)
        .split(';')
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();
    let has_ext = Path::new(name).extension().is_some();
    let candidates = |base: PathBuf| -> Vec<PathBuf> {
        let mut c = vec![];
        if !windows || has_ext {
            c.push(base.clone());
        }
        if windows {
            // Windows file names are case-insensitive; try the PATHEXT
            // spelling and its lowercase form so this also works off Windows.
            for e in &exts {
                for v in [e.clone(), e.to_ascii_lowercase()] {
                    let mut s = base.clone().into_os_string();
                    s.push(&v);
                    let p = PathBuf::from(s);
                    if !c.contains(&p) {
                        c.push(p);
                    }
                }
            }
        }
        c
    };
    if name.contains('/') || name.contains('\\') || Path::new(name).is_absolute() {
        return candidates(PathBuf::from(name))
            .into_iter()
            .find(|p| p.is_file());
    }
    let path_var = path_var?;
    std::env::split_paths(path_var)
        .flat_map(|dir| candidates(dir.join(name)))
        .find(|p| p.is_file())
}

/// Picks the orca executable: env `CODENAME_ORCA_BIN`, then config
/// `orca_bin`, then `orca` on PATH.
pub fn resolve_orca_with(
    env_override: Option<String>,
    cfg_bin: Option<&Path>,
    path_var: Option<&OsStr>,
    pathext: Option<&str>,
    windows: bool,
) -> Result<ResolvedBin> {
    let wanted: String = env_override
        .filter(|s| !s.trim().is_empty())
        .or_else(|| cfg_bin.map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| flags::BIN.to_string());
    let path = resolve_on_path(&wanted, path_var, pathext, windows).ok_or_else(|| {
        anyhow!(
            "cannot find `{wanted}` — install the Orca app, enable its CLI \
             (Settings → General → Orca CLI) or set CODENAME_ORCA_BIN / `orca_bin` \
             in config.json to the full path of the Orca CLI executable"
        )
    })?;
    let kind = bin_kind(&path);
    Ok(ResolvedBin { path, kind })
}

pub fn resolve_orca(cfg_bin: Option<&Path>) -> Result<ResolvedBin> {
    resolve_orca_with(
        std::env::var("CODENAME_ORCA_BIN").ok(),
        cfg_bin,
        std::env::var_os("PATH").as_deref(),
        std::env::var("PATHEXT").ok().as_deref(),
        cfg!(windows),
    )
}

/// Refuses batch files: codename passes message text as arguments, and
/// `cmd.exe` cannot receive arbitrary text safely.
pub fn guard_spawn(b: &ResolvedBin) -> Result<()> {
    if b.kind == BinKind::Batch {
        bail!(
            "refusing to run {} — it is a .cmd/.bat script. Windows runs those through \
             cmd.exe, which re-parses arguments, so message text could inject commands \
             (CVE-2024-24576 / \"BatBadBut\"). Point CODENAME_ORCA_BIN (env) or `orca_bin` \
             (config.json) at the real Orca CLI .exe instead. codename never spawns via `cmd /C`.",
            b.path.display()
        );
    }
    Ok(())
}

/// Checks an engine launch command (first word) on PATH. Engines are started
/// by Orca inside its terminal with fixed arguments — codename never spawns
/// them — so problems are warnings, not errors.
pub fn engine_note(command: &str) -> Option<String> {
    let first = command.split_whitespace().next()?;
    match resolve_on_path(
        first,
        std::env::var_os("PATH").as_deref(),
        std::env::var("PATHEXT").ok().as_deref(),
        cfg!(windows),
    ) {
        None => Some(format!(
            "engine `{first}` not found on PATH (Orca may still find it)"
        )),
        Some(p) if bin_kind(&p) == BinKind::Batch => Some(format!(
            "engine `{first}` is a batch script ({}); fine for Orca's terminal (fixed args, \
             no message text), but codename itself will never spawn it",
            p.display()
        )),
        Some(_) => None,
    }
}

// ---------------------------------------------------------------------------
// real implementation
// ---------------------------------------------------------------------------

pub struct RealOrca {
    bin: std::result::Result<ResolvedBin, String>,
    /// Optional `--shell` for terminal create (config `terminal_shell`).
    shell: Option<String>,
    /// Every argv run (for the log). Elements are truncated by the logger.
    pub calls: RefCell<Vec<Vec<String>>>,
    pub verbose: bool,
}

impl RealOrca {
    pub fn new(cfg_bin: Option<&Path>, shell: Option<String>, verbose: bool) -> Self {
        Self {
            bin: resolve_orca(cfg_bin).map_err(|e| format!("{e:#}")),
            shell,
            calls: RefCell::default(),
            verbose,
        }
    }

    /// The resolved executable (for `serve-task`), refusing batch files.
    pub fn exe(&self) -> Result<PathBuf> {
        let b = self.bin.as_ref().map_err(|e| anyhow!("{e}"))?;
        guard_spawn(b)?;
        Ok(b.path.clone())
    }

    /// Runs orca with argv. Returns (exit success, stdout, stderr).
    fn exec(&self, argv: &[String]) -> Result<(bool, String, String)> {
        let exe = self.exe()?;
        self.calls.borrow_mut().push(argv.to_vec());
        if self.verbose {
            eprintln!("[codename] orca {}", argv.join(" "));
        }
        let out = Command::new(&exe)
            .args(argv)
            .env("ORCA_TELEMETRY_DISABLED", "1")
            .output()
            .with_context(|| format!("cannot run `{}`", exe.display()))?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }

    /// Runs and parses the JSON envelope (whatever the exit code).
    fn call(&self, argv: &[String]) -> Result<Value> {
        let (ok, stdout, stderr) = self.exec(argv)?;
        // tolerate keepalive/log lines: take the last line that parses
        let parsed = serde_json::from_str::<Value>(stdout.trim())
            .ok()
            .or_else(|| {
                stdout
                    .lines()
                    .rev()
                    .find_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
            });
        match parsed {
            Some(v) => Ok(v),
            None if ok => Ok(Value::String(stdout)),
            None => bail!(
                "orca {} failed: {}{}",
                argv.join(" "),
                stderr.trim(),
                stdout.trim()
            ),
        }
    }

    /// Runs and returns `result`, turning `ok:false` into an error.
    fn result(&self, argv: Vec<String>) -> Result<Value> {
        let full = self.call(&argv)?;
        envelope(&full).cloned().map_err(|e| orca_error(&argv, &e))
    }
}

impl OrcaApi for RealOrca {
    fn version(&self) -> Result<String> {
        let (ok, out, err) = self.exec(&argv_version())?;
        if !ok {
            bail!("orca --version failed: {}", err.trim());
        }
        Ok(out.trim().to_string())
    }

    fn status(&self) -> Result<RuntimeStatus> {
        // `status` may answer ok:false when nothing runs; read what it says.
        let full = self.call(&argv_status())?;
        match envelope(&full) {
            Ok(r) => Ok(decode_status(r)),
            Err(e) if e.code == RUNTIME_UNAVAILABLE => Ok(RuntimeStatus {
                state: Some(e.code),
                ..Default::default()
            }),
            Err(e) => Err(orca_error(&argv_status(), &e)),
        }
    }

    fn open(&self) -> Result<()> {
        self.result(argv_open()).map(|_| ())
    }

    fn repo_add(&self, path: &str) -> Result<String> {
        let r = self.result(argv_repo_add(path))?;
        decode_repo_id(&r).ok_or_else(|| anyhow!("orca repo add: no repo id in output: {r}"))
    }

    fn terminal_list(&self) -> Result<Vec<TerminalInfo>> {
        Ok(decode_terminal_list(&self.result(argv_terminal_list())?))
    }

    fn terminal_create(
        &self,
        worktree: Option<&str>,
        title: &str,
        command: Option<&str>,
    ) -> Result<TerminalInfo> {
        let r = self.result(argv_terminal_create(
            worktree,
            title,
            command,
            self.shell.as_deref(),
        ))?;
        decode_terminal(&r).ok_or_else(|| anyhow!("orca terminal create: no handle in output: {r}"))
    }

    fn terminal_send(&self, handle: &str, text: &str, enter: bool) -> Result<SendPhase> {
        Ok(decode_send(
            &self.result(argv_terminal_send(handle, text, enter))?,
        ))
    }

    fn terminal_wait(&self, handle: &str, timeout_secs: u64) -> Result<WaitOutcome> {
        let argv = argv_terminal_wait(handle, timeout_secs);
        let full = self.call(&argv)?;
        decode_wait(&full).map_err(|e| orca_error(&argv, &e))
    }

    fn terminal_read(&self, handle: &str, limit: usize, screen: bool) -> Result<String> {
        Ok(decode_read(
            &self.result(argv_terminal_read(handle, limit, screen))?,
        ))
    }

    fn terminal_close(&self, handle: &str) -> Result<()> {
        self.result(argv_terminal_close(handle)).map(|_| ())
    }

    fn worktree_create(&self, repo_id: &str, name: &str) -> Result<WorktreeInfo> {
        let r = self.result(argv_worktree_create(repo_id, name))?;
        decode_worktree(&r)
            .ok_or_else(|| anyhow!("orca worktree create: no worktree in output: {r}"))
    }

    fn worktree_rm(&self, selector: &str) -> Result<()> {
        self.result(argv_worktree_rm(selector)).map(|_| ())
    }

    fn worktree_list(&self) -> Result<Vec<WorktreeInfo>> {
        let r = self.result(argv_worktree_list())?;
        Ok(pick_array(&r, &["worktrees", "items", "rows"])
            .into_iter()
            .filter_map(decode_worktree)
            .collect())
    }

    fn run_create(&self, from: &str, objective: &str) -> Result<String> {
        let r = self.result(argv_run_create(from, objective))?;
        r.get("run")
            .and_then(|x| pick_str(x, &["id", "runId"]))
            .or_else(|| find_str(&r, &["runId"]))
            .or_else(|| pick_str(&r, &["id"]))
            .ok_or_else(|| anyhow!("orca orchestration run-create: no run id: {r}"))
    }

    fn worker_start(&self, w: &WorkerStart) -> Result<WorkerInfo> {
        let r = self.result(argv_worker_start(w))?;
        decode_worker(&r)
            .ok_or_else(|| anyhow!("orca orchestration worker-start: no dispatch id: {r}"))
    }

    fn worker_list(&self, run: &str) -> Result<Value> {
        self.result(argv_worker_list(run))
    }

    fn worker_stop(&self, dispatch: &str) -> Result<()> {
        self.result(argv_worker_stop(dispatch)).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::fixture;
    use serde_json::json;

    fn no_shell(a: &[String]) {
        for bad in ["cmd", "/C", "/c", "sh", "-c", "bash", "powershell"] {
            assert!(!a.iter().any(|x| x == bad), "shell token {bad} in {a:?}");
        }
    }

    fn pair(a: &[String], k: &str, val: &str) -> bool {
        a.windows(2).any(|w| w[0] == k && w[1] == val)
    }

    /// Every flag we send must exist for that command in the 1.4.224 schema.
    #[test]
    fn every_flag_exists_in_the_1_4_224_schema() {
        let schema: Value =
            serde_json::from_str(include_str!("../docs/orca-1.4.224/agent-context.json")).unwrap();
        let cmds = schema["commands"].as_array().unwrap();
        let ws = WorkerStart {
            from: "cap".into(),
            run: "r".into(),
            spec: "s".into(),
            agent: "codex".into(),
            repo_id: "rid".into(),
            name: "n".into(),
            display_name: "d".into(),
        };
        let all = [
            argv_status(),
            argv_open(),
            argv_serve(),
            argv_repo_add("p"),
            argv_terminal_list(),
            argv_terminal_create(Some("path:D:/x"), "t", Some("claude"), Some("pwsh.exe")),
            argv_terminal_send("h", "x", true),
            argv_terminal_wait("h", 1),
            argv_terminal_read("h", 10, true),
            argv_terminal_close("h"),
            argv_worktree_create("rid", "n"),
            argv_worktree_rm("id:a::b"),
            argv_worktree_list(),
            argv_run_create("cap", "o"),
            argv_worker_start(&ws),
            argv_worker_list("r"),
            argv_worker_stop("d"),
        ];
        for argv in all {
            let words: Vec<&String> = argv.iter().take_while(|a| !a.starts_with("--")).collect();
            let cmd = cmds
                .iter()
                .find(|c| {
                    let p: Vec<&str> = c["path"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_str().unwrap())
                        .collect();
                    p.len() == words.len() && p.iter().zip(&words).all(|(a, b)| a == b)
                })
                .unwrap_or_else(|| panic!("command {words:?} not in schema"));
            let known: Vec<&str> = cmd["flags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap())
                .collect();
            for f in argv.iter().filter(|a| a.starts_with("--")) {
                assert!(
                    known.contains(&f.trim_start_matches("--")),
                    "{f} not a flag of {words:?} in Orca 1.4.224"
                );
            }
        }
    }

    #[test]
    fn send_argv_keeps_text_as_one_element() {
        let text = "สวัสดี \"พี่\" & echo pwned; rm -rf / | x";
        let a = argv_terminal_send("t1", text, true);
        no_shell(&a);
        assert_eq!(a[..2], ["terminal", "send"]);
        assert!(a.iter().any(|x| x == text));
        assert!(pair(&a, "--wait-submit", "15"));
        assert!(a.contains(&"--enter".to_string()));
        assert_eq!(a.last().unwrap(), "--json");
        let keys = argv_terminal_send("t1", "1", false);
        assert!(!keys.contains(&"--enter".to_string()));
    }

    #[test]
    fn create_argv_shape_and_selectors() {
        assert_eq!(path_selector("D:\\fleet\\sandbox"), "path:D:/fleet/sandbox");
        assert_eq!(repo_selector("abc"), "id:abc");
        assert_eq!(repo_selector("path:/x"), "path:/x");
        let a = argv_terminal_create(Some("path:D:/fleet/x y"), "nova", Some("claude"), None);
        no_shell(&a);
        assert!(pair(&a, "--worktree", "path:D:/fleet/x y"));
        assert!(pair(&a, "--title", "nova"));
        assert!(pair(&a, "--command", "claude"));
        assert!(!a.contains(&"--shell".to_string()));
        let b = argv_terminal_create(None, "codename-captain", None, Some("pwsh.exe"));
        assert!(pair(&b, "--shell", "pwsh.exe"));
    }

    #[test]
    fn wait_read_worktree_argv() {
        let a = argv_terminal_wait("h", 600);
        assert!(pair(&a, "--for", "tui-idle"));
        assert!(pair(&a, "--timeout-ms", "600000"));
        assert!(pair(&argv_terminal_read("h", 40, true), "--limit", "40"));
        assert!(argv_terminal_read("h", 40, true).contains(&"--screen".to_string()));
        assert!(pair(
            &argv_worktree_rm("id:r::C:/w"),
            "--worktree",
            "id:r::C:/w"
        ));
        assert!(pair(&argv_worktree_create("rid", "n"), "--repo", "id:rid"));
        let w = WorktreeInfo {
            id: "r::C:/w".into(),
            ..Default::default()
        };
        assert_eq!(w.selector(), "id:r::C:/w");
        let p = WorktreeInfo {
            path: Some("C:\\w".into()),
            ..Default::default()
        };
        assert_eq!(p.selector(), "path:C:/w");
    }

    #[test]
    fn orchestration_argv() {
        assert!(pair(&argv_run_create("cap", "o"), "--from", "cap"));
        assert!(pair(&argv_run_create("cap", "o"), "--objective", "o"));
        let ws = WorkerStart {
            from: "cap".into(),
            run: "r1".into(),
            spec: "do it; rm -rf /".into(),
            agent: "codex".into(),
            repo_id: "rid".into(),
            name: "team-demo-a".into(),
            display_name: "a".into(),
        };
        let a = argv_worker_start(&ws);
        no_shell(&a);
        for (k, val) in [
            ("--from", "cap"),
            ("--run", "r1"),
            ("--spec", "do it; rm -rf /"),
            ("--agent", "codex"),
            ("--worktree", "new-top-level"),
            ("--repo", "id:rid"),
            ("--name", "team-demo-a"),
            ("--display-name", "a"),
        ] {
            assert!(pair(&a, k, val), "{k} {val} in {a:?}");
        }
        assert!(!a.contains(&"--title".to_string()) && !a.contains(&"--command".to_string()));
    }

    #[test]
    fn live_fixtures_decode() {
        let create = fixture("create");
        let t = decode_terminal(envelope(&create).unwrap()).unwrap();
        assert_eq!(t.handle, "term_373e9bd9-d68e-4587-a4c0-55f46987b3ce");
        assert_eq!(t.title.as_deref(), Some("cn-probe"));
        assert!(t.worktree.unwrap().ends_with("::D:/fleet/sandbox"));
        assert_eq!(t.state, TermState::Unknown);

        let send = fixture("send");
        assert_eq!(decode_send(envelope(&send).unwrap()), SendPhase::Accepted);

        let wait = fixture("wait");
        assert_eq!(decode_wait(&wait).unwrap(), WaitOutcome::Timeout);

        let read = fixture("read");
        let text = decode_read(envelope(&read).unwrap());
        assert!(text.lines().nth(1) == Some("CN-PROBE-OK"), "{text}");

        let close = fixture("close");
        assert!(envelope(&close).is_ok());
    }

    #[test]
    fn envelope_errors_and_codes() {
        let e = envelope(
            &json!({"ok": false, "error": {"code": "runtime_unavailable", "message": "no"}}),
        )
        .unwrap_err();
        assert_eq!(e.code, RUNTIME_UNAVAILABLE);
        let msg = orca_error(&argv_terminal_list(), &e).to_string();
        assert!(
            msg.contains("not running") && msg.contains("terminal list"),
            "{msg}"
        );
        let other = orca_error(
            &argv_terminal_list(),
            &OrcaError {
                code: "bad".into(),
                message: "m".into(),
            },
        )
        .to_string();
        assert!(other.contains("[bad]"));
        assert_eq!(
            decode_wait(&json!({"ok": false, "error": {"code": "agent_blocked", "message": ""}}))
                .unwrap(),
            WaitOutcome::Blocked
        );
        assert!(
            decode_wait(&json!({"ok": false, "error": {"code": "terminal_not_found"}})).is_err()
        );
        assert_eq!(
            decode_wait(&json!({"ok": true, "result": {}})).unwrap(),
            WaitOutcome::Done
        );
        assert_eq!(envelope(&json!({"x": 1})).unwrap()["x"], 1);
        let turn = json!({"send": {"accepted": true, "prompt": {"stages": ["input_accepted", "submitted", "turn_started"]}}});
        assert_eq!(decode_send(&turn), SendPhase::TurnStarted);
        assert_eq!(WaitOutcome::Blocked.exit_code(), 3);
        assert_eq!(WaitOutcome::Timeout.exit_code(), 2);
    }

    #[test]
    fn status_and_lenient_decoders() {
        let s = decode_status(
            &json!({"runtime": {"state": "ready", "reachable": true}, "app": {"running": false}}),
        );
        assert!(s.ready());
        assert_eq!(s.host(), "serve (headless)");
        assert_eq!(s.state.as_deref(), Some("ready"));
        assert!(!decode_status(&json!({})).ready());
        let list = decode_terminal_list(&json!({"terminals": [
            {"handle": "a", "title": "nova", "status": "running"},
            {"handle": "b", "agentState": "working"},
            {"handle": "c", "agentStatus": "awaiting-permission"},
            {"nothing": true}
        ]}));
        assert_eq!(list.len(), 3);
        assert_eq!(
            list[0].state,
            TermState::Unknown,
            "process status is not agent state"
        );
        assert_eq!(list[1].state, TermState::Working);
        assert_eq!(list[2].state, TermState::Blocked);
        assert_eq!(
            decode_repo_id(&json!({"repo": {"id": "r1"}})).as_deref(),
            Some("r1")
        );
        let w = decode_worker(
            &json!({"dispatch": {"id": "d1"}, "agentTerminalHandle": "term_x",
            "worktree": {"id": "r::C:/w", "path": "C:/w"}}),
        )
        .unwrap();
        assert_eq!(w.dispatch, "d1");
        assert_eq!(w.terminal.as_deref(), Some("term_x"));
        assert_eq!(w.worktree_id.as_deref(), Some("r::C:/w"));
        assert_eq!(decode_read(&json!({"text": "hi"})), "hi");
    }

    fn same(a: &Path, b: &Path) -> bool {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    #[test]
    fn resolver_follows_pathext_order_on_windows() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        touch(&a.join("orca.cmd"));
        touch(&a.join("orca")); // npm-style sh shim: not runnable on Windows
        touch(&b.join("orca.exe"));
        let path = std::env::join_paths([&a, &b]).unwrap();
        // first dir wins, and within it PATHEXT order: .cmd is found before b/orca.exe
        let r = resolve_on_path("orca", Some(&path), Some(".COM;.EXE;.BAT;.CMD"), true).unwrap();
        assert!(same(&r, &a.join("orca.cmd")), "{r:?}");
        assert_eq!(bin_kind(&r), BinKind::Batch);
        touch(&a.join("orca.exe"));
        let r = resolve_on_path("orca", Some(&path), None, true).unwrap();
        assert!(same(&r, &a.join("orca.exe")), "{r:?}");
        // custom PATHEXT order
        let r = resolve_on_path("orca", Some(&path), Some(".CMD;.EXE"), true).unwrap();
        assert!(same(&r, &a.join("orca.cmd")), "{r:?}");
    }

    #[test]
    fn resolver_non_windows_and_explicit_paths() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("orca"));
        let path = std::env::join_paths([d.path()]).unwrap();
        assert_eq!(
            resolve_on_path("orca", Some(&path), None, false).unwrap(),
            d.path().join("orca")
        );
        assert!(resolve_on_path("missing", Some(&path), None, false).is_none());
        let exe = d.path().join("real").join("orca.exe");
        touch(&exe);
        let s = exe.to_string_lossy().into_owned();
        assert_eq!(resolve_on_path(&s, None, None, true).unwrap(), exe);
        let no_ext = d
            .path()
            .join("real")
            .join("orca")
            .to_string_lossy()
            .into_owned();
        assert!(same(
            &resolve_on_path(&no_ext, None, None, true).unwrap(),
            &exe
        ));
    }

    #[test]
    fn guard_refuses_batch_and_override_wins() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("orca.cmd"));
        let exe = d.path().join("x").join("Orca CLI.exe");
        touch(&exe);
        let path = std::env::join_paths([d.path()]).unwrap();
        let r = resolve_orca_with(None, None, Some(&path), None, true).unwrap();
        let err = guard_spawn(&r).unwrap_err().to_string();
        assert!(
            err.contains("CVE-2024-24576") && err.contains("CODENAME_ORCA_BIN"),
            "{err}"
        );
        let cfg = resolve_orca_with(None, Some(&exe), Some(&path), None, true).unwrap();
        assert_eq!(cfg.kind, BinKind::Exe);
        assert!(guard_spawn(&cfg).is_ok());
        let env = resolve_orca_with(
            Some(exe.to_string_lossy().into_owned()),
            Some(Path::new("nope")),
            Some(&path),
            None,
            true,
        )
        .unwrap();
        assert_eq!(env.path, exe);
        assert!(resolve_orca_with(
            None,
            None,
            Some(&std::env::join_paths([d.path().join("x")]).unwrap()),
            None,
            true
        )
        .is_err());
    }

    #[test]
    fn real_orca_refuses_batch_without_spawning() {
        let d = tempfile::tempdir().unwrap();
        let cmd = d.path().join("orca.bat");
        touch(&cmd);
        let o = RealOrca {
            bin: Ok(ResolvedBin {
                path: cmd,
                kind: BinKind::Batch,
            }),
            shell: None,
            calls: RefCell::default(),
            verbose: false,
        };
        assert!(o.version().unwrap_err().to_string().contains("BatBadBut"));
        assert!(o.calls.borrow().is_empty());
    }
}
