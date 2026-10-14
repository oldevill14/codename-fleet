//! codename — a fleet of named codenames on top of the Orca desktop app CLI.
#![forbid(unsafe_code)]

pub mod birth;
pub mod cli;
pub mod doctor;
pub mod fleet;
pub mod log;
pub mod memory;
pub mod orca;
pub mod registry;
pub mod seat;
pub mod service;
pub mod team;

use orca::OrcaApi;
use registry::Paths;
use serde_json::Value;
use std::time::Duration;

/// Polling knobs. Tests use zero delays and small counts.
#[derive(Clone, Copy)]
pub struct Pacing {
    pub poll: Duration,
    /// Polls to wait for an agent to be seen working after a send (ask trap).
    pub start_polls: u32,
    /// Polls for first-run dialogs after a wakeup (2s × 30 = 60s).
    pub dialog_polls: u32,
    /// Polls after `/exit` before closing a terminal.
    pub exit_polls: u32,
}

impl Default for Pacing {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(2),
            start_polls: 4,
            dialog_polls: 30,
            exit_polls: 3,
        }
    }
}

impl Pacing {
    pub fn nap(&self) {
        if !self.poll.is_zero() {
            std::thread::sleep(self.poll);
        }
    }
}

/// Everything a command needs. Built once in `main`.
pub struct Ctx<'a> {
    pub orca: &'a dyn OrcaApi,
    pub paths: Paths,
    pub pacing: Pacing,
}

/// Result of a command: exit code, human text, machine JSON.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub code: i32,
    pub text: String,
    pub json: Value,
}

impl Outcome {
    pub fn ok(text: impl Into<String>, json: Value) -> Self {
        Self {
            code: 0,
            text: text.into(),
            json,
        }
    }
    pub fn with_code(mut self, code: i32) -> Self {
        self.code = code;
        self
    }
}

/// In-memory OrcaApi used by unit and integration tests.
#[doc(hidden)]
pub mod fake;
