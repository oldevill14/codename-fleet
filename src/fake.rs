//! A scripted, in-memory `OrcaApi`. Every call is recorded as the argv the
//! real implementation would have run (so tests also cover argv building),
//! and the terminal create/send/wait/read/close answers are the real Orca
//! 1.4.224 envelopes from docs/orca-1.4.224/live-roundtrip.txt, run through
//! the same decoders as the real implementation.

use crate::orca::*;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

pub const LIVE_ROUNDTRIP: &str = include_str!("../docs/orca-1.4.224/live-roundtrip.txt");

/// Returns the JSON envelope recorded live for `create`, `send`, `wait`,
/// `read` or `close`.
pub fn fixture(name: &str) -> Value {
    let marker = format!("--- {name}");
    let start = LIVE_ROUNDTRIP
        .lines()
        .position(|l| l.trim() == marker)
        .unwrap_or_else(|| panic!("no fixture {name}"));
    let body: Vec<&str> = LIVE_ROUNDTRIP
        .lines()
        .skip(start + 1)
        .take_while(|l| !l.starts_with("--- ") && !l.starts_with("HANDLE="))
        .collect();
    serde_json::from_str(&body.join("\n")).expect("fixture is JSON")
}

const LIVE_HANDLE: &str = "term_373e9bd9-d68e-4587-a4c0-55f46987b3ce";

/// A live fixture with its terminal handle replaced.
fn fixture_for(name: &str, handle: &str) -> Value {
    serde_json::from_str(&fixture(name).to_string().replace(LIVE_HANDLE, handle)).expect("json")
}

pub struct FakeOrca {
    pub up: Cell<bool>,
    /// What `orca status` reports (reachable runtime).
    pub runtime_ready: Cell<bool>,
    pub calls: RefCell<Vec<Vec<String>>>,
    pub terminals: RefCell<Vec<TerminalInfo>>,
    pub worktrees: RefCell<Vec<WorktreeInfo>>,
    /// Scripted states served one per `terminal_list` call (applied to all).
    pub states: RefCell<VecDeque<TermState>>,
    /// Scripted screens served one per `terminal_read`; then `screen`.
    pub screens: RefCell<VecDeque<String>>,
    pub screen: RefCell<String>,
    pub send_phase: Cell<SendPhase>,
    pub wait_outcome: Cell<WaitOutcome>,
    pub seen_working: Cell<bool>,
    /// Set if `terminal_wait` ran before any working state was observed.
    pub wait_too_early: Cell<bool>,
    /// Paths handed out (in order) by `worktree_create` / `worker_start`.
    pub worktree_paths: RefCell<VecDeque<String>>,
    next: Cell<u32>,
}

impl Default for FakeOrca {
    fn default() -> Self {
        Self {
            up: Cell::new(true),
            runtime_ready: Cell::new(true),
            calls: RefCell::default(),
            terminals: RefCell::default(),
            worktrees: RefCell::default(),
            states: RefCell::default(),
            screens: RefCell::default(),
            screen: RefCell::new(String::new()),
            send_phase: Cell::new(SendPhase::TurnStarted),
            wait_outcome: Cell::new(WaitOutcome::Done),
            seen_working: Cell::new(false),
            wait_too_early: Cell::new(false),
            worktree_paths: RefCell::default(),
            next: Cell::new(0),
        }
    }
}

impl FakeOrca {
    fn rec(&self, argv: Vec<String>) -> Result<()> {
        if !self.up.get() {
            bail!("cannot run `orca` — not installed");
        }
        let runtime_cmd = !matches!(
            argv.first().map(String::as_str),
            Some("--version" | "status" | "open")
        );
        self.calls.borrow_mut().push(argv.clone());
        if runtime_cmd && !self.runtime_ready.get() {
            let e = OrcaError {
                code: RUNTIME_UNAVAILABLE.into(),
                message: "Orca runtime is not running".into(),
            };
            return Err(orca_error(&argv, &e));
        }
        Ok(())
    }
    fn id(&self, p: &str) -> String {
        self.next.set(self.next.get() + 1);
        format!("{p}{}", self.next.get())
    }
    pub fn calls_of(&self, a: &str, b: &str) -> Vec<Vec<String>> {
        self.calls
            .borrow()
            .iter()
            .filter(|c| {
                c.first().map(String::as_str) == Some(a) && c.get(1).map(String::as_str) == Some(b)
            })
            .cloned()
            .collect()
    }
    pub fn add_terminal(&self, handle: &str, title: &str, state: TermState) {
        self.terminals.borrow_mut().push(TerminalInfo {
            handle: handle.into(),
            title: Some(title.into()),
            state,
            worktree: None,
        });
    }
    fn live(&self, handle: &str) -> Result<()> {
        if !self.terminals.borrow().iter().any(|t| t.handle == handle) {
            bail!("orca terminal failed [terminal_not_found]: {handle}");
        }
        Ok(())
    }
}

impl OrcaApi for FakeOrca {
    fn version(&self) -> Result<String> {
        self.rec(argv_version())?;
        Ok("1.4.224 (fake)".into())
    }
    fn status(&self) -> Result<RuntimeStatus> {
        self.rec(argv_status())?;
        let ready = self.runtime_ready.get();
        let env = json!({"id": "x", "ok": true, "result": {
            "runtime": {"state": if ready { "ready" } else { "stopped" }, "reachable": ready},
            "app": {"running": false}}});
        Ok(decode_status(
            envelope(&env).map_err(|e| anyhow!(e.message))?,
        ))
    }
    fn open(&self) -> Result<()> {
        self.rec(argv_open())?;
        self.runtime_ready.set(true);
        Ok(())
    }
    fn repo_add(&self, path: &str) -> Result<String> {
        self.rec(argv_repo_add(path))?;
        Ok(self.id("repo"))
    }
    fn terminal_list(&self) -> Result<Vec<TerminalInfo>> {
        self.rec(argv_terminal_list())?;
        if let Some(s) = self.states.borrow_mut().pop_front() {
            if s == TermState::Working {
                self.seen_working.set(true);
            }
            for t in self.terminals.borrow_mut().iter_mut() {
                t.state = s;
            }
        }
        Ok(self.terminals.borrow().clone())
    }
    fn terminal_create(
        &self,
        worktree: Option<&str>,
        title: &str,
        command: Option<&str>,
    ) -> Result<TerminalInfo> {
        self.rec(argv_terminal_create(worktree, title, command, None))?;
        let handle = self.id("term_");
        let mut env = fixture_for("create", &handle);
        env["result"]["terminal"]["title"] = json!(title);
        let mut t = decode_terminal(envelope(&env).map_err(|e| anyhow!(e.message))?)
            .ok_or_else(|| anyhow!("fixture"))?;
        t.state = TermState::Idle;
        self.terminals.borrow_mut().push(t.clone());
        Ok(t)
    }
    fn terminal_send(&self, handle: &str, text: &str, enter: bool) -> Result<SendPhase> {
        self.rec(argv_terminal_send(handle, text, enter))?;
        self.live(handle)?;
        let mut env = fixture_for("send", handle);
        if self.send_phase.get() == SendPhase::TurnStarted {
            env["result"]["send"]["prompt"]["stages"] =
                json!(["input_accepted", "submitted", "turn_started"]);
        }
        Ok(decode_send(envelope(&env).map_err(|e| anyhow!(e.message))?))
    }
    fn terminal_wait(&self, handle: &str, timeout_secs: u64) -> Result<WaitOutcome> {
        self.rec(argv_terminal_wait(handle, timeout_secs))?;
        if !self.seen_working.get() {
            self.wait_too_early.set(true);
        }
        let env = match self.wait_outcome.get() {
            WaitOutcome::Timeout => fixture("wait"),
            WaitOutcome::Blocked => {
                json!({"ok": false, "error": {"code": "agent_blocked", "message": "waiting for permission"}})
            }
            WaitOutcome::Done => json!({"ok": true, "result": {"wait": {"handle": handle}}}),
        };
        decode_wait(&env).map_err(|e| anyhow!(e.message))
    }
    fn terminal_read(&self, handle: &str, limit: usize, screen: bool) -> Result<String> {
        self.rec(argv_terminal_read(handle, limit, screen))?;
        let text = match self.screens.borrow_mut().pop_front() {
            Some(s) => s,
            None => self.screen.borrow().clone(),
        };
        let mut env = fixture_for("read", handle);
        env["result"]["terminal"]["tail"] = json!(text.lines().collect::<Vec<_>>());
        Ok(decode_read(envelope(&env).map_err(|e| anyhow!(e.message))?))
    }
    fn terminal_close(&self, handle: &str) -> Result<()> {
        self.rec(argv_terminal_close(handle))?;
        envelope(&fixture_for("close", handle)).map_err(|e| anyhow!(e.message))?;
        self.terminals.borrow_mut().retain(|t| t.handle != handle);
        Ok(())
    }
    fn worktree_create(&self, repo_id: &str, name: &str) -> Result<WorktreeInfo> {
        self.rec(argv_worktree_create(repo_id, name))?;
        let path = self.worktree_paths.borrow_mut().pop_front();
        let w = WorktreeInfo {
            id: format!(
                "{repo_id}::{}",
                path.clone().unwrap_or_else(|| self.id("wt"))
            ),
            path,
            branch: Some(name.into()),
        };
        self.worktrees.borrow_mut().push(w.clone());
        Ok(w)
    }
    fn worktree_rm(&self, selector: &str) -> Result<()> {
        self.rec(argv_worktree_rm(selector))?;
        let before = self.worktrees.borrow().len();
        self.worktrees
            .borrow_mut()
            .retain(|w| w.selector() != selector);
        if self.worktrees.borrow().len() == before {
            bail!("orca worktree rm failed [worktree_not_found]: {selector}");
        }
        Ok(())
    }
    fn worktree_list(&self) -> Result<Vec<WorktreeInfo>> {
        self.rec(argv_worktree_list())?;
        Ok(self.worktrees.borrow().clone())
    }
    fn run_create(&self, from: &str, objective: &str) -> Result<String> {
        self.rec(argv_run_create(from, objective))?;
        Ok(self.id("run_"))
    }
    fn worker_start(&self, w: &WorkerStart) -> Result<WorkerInfo> {
        self.rec(argv_worker_start(w))?;
        let path = self.worktree_paths.borrow_mut().pop_front();
        let wt = WorktreeInfo {
            id: format!(
                "{}::{}",
                w.repo_id,
                path.clone().unwrap_or_else(|| w.name.clone())
            ),
            path: path.clone(),
            branch: Some(w.name.clone()),
        };
        self.worktrees.borrow_mut().push(wt.clone());
        let handle = self.id("term_");
        self.terminals.borrow_mut().push(TerminalInfo {
            handle: handle.clone(),
            title: Some(w.display_name.clone()),
            state: TermState::Working,
            worktree: Some(wt.id.clone()),
        });
        let env = json!({"ok": true, "result": {
            "dispatch": {"id": self.id("dispatch_")},
            "agentTerminalHandle": handle,
            "worktree": {"id": wt.id, "path": path}}});
        decode_worker(envelope(&env).map_err(|e| anyhow!(e.message))?)
            .ok_or_else(|| anyhow!("decode"))
    }
    fn worker_list(&self, run: &str) -> Result<Value> {
        self.rec(argv_worker_list(run))?;
        Ok(json!({"workers": [], "scope": {"source": "flag"}}))
    }
    fn worker_stop(&self, dispatch: &str) -> Result<()> {
        self.rec(argv_worker_stop(dispatch))?;
        Ok(())
    }
}
