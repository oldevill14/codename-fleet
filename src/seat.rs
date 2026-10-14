//! Seats: wakeup / sleep / hey / wait / ask / peek, first-run dialogs and
//! latest-turn extraction.

use crate::orca::{SendPhase, TermState, TerminalInfo, WaitOutcome};
use crate::registry::{atomic_write, launch_command, Codename, Engines, Fleet};
use crate::{Ctx, Outcome};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use std::fs;

// ---------------------------------------------------------------------------
// live state
// ---------------------------------------------------------------------------

pub fn live(ctx: &Ctx) -> Result<Vec<TerminalInfo>> {
    ctx.orca.terminal_list()
}

pub fn find_live<'a>(live: &'a [TerminalInfo], handle: Option<&str>) -> Option<&'a TerminalInfo> {
    let h = handle?;
    live.iter().find(|t| t.handle == h)
}

/// rest = no live terminal; otherwise the terminal's state.
pub fn status_word(o: &Codename, live: &[TerminalInfo]) -> &'static str {
    match find_live(live, o.orca.terminal.as_deref()) {
        None => "rest",
        Some(t) => match t.state {
            TermState::Idle => "idle",
            TermState::Working => "working",
            TermState::Blocked => "blocked",
            // live terminal, agent state not reported by Orca
            TermState::Unknown => "awake",
        },
    }
}

fn state_of(ctx: &Ctx, handle: &str) -> Result<Option<TermState>> {
    Ok(live(ctx)?
        .into_iter()
        .find(|t| t.handle == handle)
        .map(|t| t.state))
}

/// Returns the live terminal handle of a codename or an error saying it rests.
pub fn handle_of(ctx: &Ctx, fleet: &Fleet, name: &str) -> Result<String> {
    let o = fleet.require(name)?;
    let h = o
        .orca
        .terminal
        .clone()
        .ok_or_else(|| anyhow!("`{}` is resting — run `codename wakeup {}`", o.name, o.name))?;
    if state_of(ctx, &h)?.is_none() {
        bail!(
            "`{}` has no live terminal (registry said {h}) — run `codename wakeup {}`",
            o.name,
            o.name
        );
    }
    Ok(h)
}

// ---------------------------------------------------------------------------
// first-run dialogs (4.4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Trust,
    Update,
    /// Hook review — never auto-accepted.
    Hooks,
    /// Login / OAuth — the human must do it.
    Login,
    /// "New MCP server found in this project" — approving a server is the
    /// human's call (birth pre-approves `arra` in project settings instead).
    McpApproval,
    /// Claude Code's "Detected a custom API key … Do you want to use this API
    /// key?" menu. Never answered: `1` would switch billing to that key.
    ApiKey,
}

/// Substring dialogs: (kind, lowercase substrings, keys to send or None = stop).
/// The Codex update menu and login screens are matched by dedicated functions
/// below (`is_update_menu`, `login_required`), never by a bare substring.
/// Record real screens in docs/first-run-dialogs.md and tests/fixtures/screens.
pub const DIALOGS: &[(Dialog, &[&str], Option<&str>)] = &[
    // first: the API-key menu is `1. Yes / 2. No`, so it must win over any
    // answerable dialog (Trust sends `1`) that might share the screen
    (
        Dialog::ApiKey,
        &[
            "detected a custom api key",
            "do you want to use this api key",
        ],
        None,
    ),
    (
        Dialog::Hooks,
        &[
            "hooks have changed",
            "review hooks",
            "hook changes",
            "new hooks",
        ],
        None,
    ),
    (
        Dialog::McpApproval,
        &[
            "mcp server found in this project",
            "mcp servers found in this project",
            "new mcp server",
        ],
        None,
    ),
    (
        Dialog::Trust,
        &[
            "trust this folder",
            "trust the files",
            "do you trust",
            "trust this directory",
        ],
        Some("1"),
    ),
];

/// Keys that skip the Codex update menu (`2. Skip`), sent WITHOUT Enter
/// (verified live with Codex 0.146.1).
pub const UPDATE_SKIP_KEYS: &str = "2";

/// Screen lines that matter for dialog detection: non-blocking hook error
/// lines (e.g. `SessionStart:startup hook error ... No such file or
/// directory`) are informational and dropped.
fn relevant_lines(screen: &str) -> Vec<String> {
    screen
        .lines()
        .map(|l| l.to_lowercase())
        .filter(|l| !(l.contains("hook error") || l.contains("hook failed")))
        .collect()
}

fn strip_box(l: &str) -> &str {
    l.trim()
        .trim_start_matches(['│', '┃', '|'])
        .trim_end_matches(['│', '┃', '|'])
        .trim()
}

/// The interactive Codex update menu: a selectable `1.` option line, a
/// `Skip` option and `Press enter to continue`. The static banner left after
/// skipping ("Update available! … Run npm install …") does NOT match.
pub fn is_update_menu(screen: &str) -> bool {
    let lines = relevant_lines(screen);
    let option1 = lines.iter().any(|l| {
        let t = strip_box(l);
        let t = t.trim_start_matches(['›', '>', '❯']).trim();
        t.starts_with("1.") && t.contains("update")
    });
    let skip = lines.iter().any(|l| {
        let t = strip_box(l).trim_start_matches(['›', '>', '❯']).trim();
        t.starts_with("2.") && t.contains("skip")
    });
    let press = lines.iter().any(|l| l.contains("press enter to continue"));
    option1 && skip && press
}

/// The agent is not logged in (Claude: `Not logged in · Run /login`; Codex:
/// `Sign in with ChatGPT` / API-key menu). Best effort for Codex.
pub fn login_required(screen: &str) -> bool {
    let lines = relevant_lines(screen);
    lines.iter().any(|l| {
        l.contains("not logged in")
            || l.contains("select login method")
            || l.contains("sign in with chatgpt")
            || l.contains("log in with chatgpt")
            || l.contains("provide your own api key")
            || l.contains("anthropic console account")
            || l.contains("please run /login")
            // Free Claude Code launcher could not reach or start its proxy,
            // or cannot find the agent it wraps: same response, a human fixes it
            || l.contains("free claude code proxy is not reachable")
            || l.contains("free claude code did not become ready")
            || l.contains("could not find claude code command")
            || l.contains("could not find codex cli command")
    })
}

/// Codex/Claude ready: a prompt line (`›`/`>`/`❯`) and, for Codex, the
/// `<model> · <path>` footer, with no menu on screen.
pub fn is_ready(screen: &str) -> bool {
    if classify_dialog(screen).is_some() {
        return false;
    }
    let lines = relevant_lines(screen);
    let prompt = lines.iter().any(|l| {
        let t = l.trim();
        t.starts_with('›') || t.starts_with('❯') || (t.starts_with('>') && !t.starts_with(">_"))
    });
    let footer = lines
        .iter()
        .any(|l| l.contains(" · ") || l.contains("? for shortcuts"));
    prompt && footer
}

pub fn classify_dialog(screen: &str) -> Option<(Dialog, Option<&'static str>)> {
    if login_required(screen) {
        return Some((Dialog::Login, None));
    }
    if is_update_menu(screen) {
        return Some((Dialog::Update, Some(UPDATE_SKIP_KEYS)));
    }
    let s = relevant_lines(screen).join("\n");
    DIALOGS
        .iter()
        .find(|(_, pats, _)| pats.iter().any(|p| s.contains(p)))
        .map(|(d, _, k)| (*d, *k))
}

/// Error for a seat that is not logged in. Birth/wakeup close the terminal
/// and stop; codename never logs in for the human.
#[derive(Debug)]
pub struct NotLoggedIn(pub String);

impl std::fmt::Display for NotLoggedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let e = &self.0;
        if e.ends_with("-free") {
            // free engines must never fall back to a real login: the FCC
            // proxy is down, has no provider key, or did not take over
            return write!(
                f,
                "{e} did not get through Free Claude Code (proxy down, no provider key, or a \
                 login/API-key screen appeared) — open the FCC Admin UI, check a provider key \
                 is saved and applied, run `fcc-doctor`; do not /login here. codename never \
                 logs in for you"
            );
        }
        write!(
            f,
            "{e} is not logged in on this machine — run `{e}` and /login yourself; \
             codename never logs in for you"
        )
    }
}

impl std::error::Error for NotLoggedIn {}

/// Error for a seat stopped at a dialog only the human may answer (hook
/// review, MCP approval, custom API key). The seat is left open and nothing
/// more is typed into it — even `/exit` + Enter could select a menu option.
#[derive(Debug)]
pub struct DialogBlocked {
    pub name: String,
    pub kind: Dialog,
}

impl std::fmt::Display for DialogBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "blocked: `{}` is showing a {:?} dialog — codename will not answer it. \
             Open the terminal in Orca and let พี่เก่ง (the human) decide.",
            self.name, self.kind
        )
    }
}

impl std::error::Error for DialogBlocked {}

fn log_dialog(ctx: &Ctx, name: &str, kind: Dialog, screen: &str) {
    let p = ctx.paths.dialogs_log();
    let prev = fs::read_to_string(&p).unwrap_or_default();
    let tail: Vec<&str> = screen
        .lines()
        .rev()
        .take(15)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let entry = format!(
        "\n## {} · {name} · {kind:?}\n```\n{}\n```\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        tail.join("\n")
    );
    let _ = atomic_write(&p, format!("{prev}{entry}").as_bytes());
}

/// Watches a fresh seat and answers known dialogs. Returns a list of notes;
/// errors when a human is needed (login → `NotLoggedIn`, hook review, MCP).
pub fn handle_first_run(ctx: &Ctx, name: &str, engine: &str, handle: &str) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    let mut answered = 0;
    for i in 0..ctx.pacing.dialog_polls {
        ctx.pacing.nap();
        let screen = ctx.orca.terminal_read(handle, 60, true)?;
        match classify_dialog(&screen) {
            Some((Dialog::Login, _)) => {
                log_dialog(ctx, name, Dialog::Login, &screen);
                return Err(NotLoggedIn(engine.to_string()).into());
            }
            Some((kind, None)) => {
                log_dialog(ctx, name, kind, &screen);
                return Err(DialogBlocked {
                    name: name.to_string(),
                    kind,
                }
                .into());
            }
            Some((kind, Some(keys))) => {
                log_dialog(ctx, name, kind, &screen);
                if answered >= 6 {
                    bail!("`{name}` keeps showing {kind:?} dialogs — check it in Orca");
                }
                // single keys, no Enter (verified for the Codex update menu)
                ctx.orca.terminal_send(handle, keys, false)?;
                answered += 1;
                notes.push(format!("answered {kind:?} dialog"));
            }
            None => {
                if is_ready(&screen) {
                    return Ok(notes);
                }
                let st = state_of(ctx, handle)?;
                if st == Some(TermState::Idle) || (i >= 2 && st != Some(TermState::Blocked)) {
                    return Ok(notes);
                }
            }
        }
    }
    notes.push("first-run window ended without a clear ready signal".into());
    Ok(notes)
}

// ---------------------------------------------------------------------------
// wakeup / sleep
// ---------------------------------------------------------------------------

/// Opens a seat. After the first wakeup the engine's `resume_command` is used
/// (continue the last conversation) unless `fresh` is set.
pub fn wakeup(
    ctx: &Ctx,
    fleet: &mut Fleet,
    engines: &Engines,
    name: &str,
    fresh: bool,
) -> Result<Outcome> {
    let terms = live(ctx)?;
    let o = fleet.require(name)?.clone();
    if let Some(t) = find_live(&terms, o.orca.terminal.as_deref()) {
        return Ok(Outcome::ok(
            format!("{} is already awake ({})", o.name, t.handle),
            json!({"name": o.name, "terminal": t.handle, "already": true}),
        ));
    }
    if !o.home.is_dir() {
        bail!("home of `{}` not found: {}", o.name, o.home.display());
    }
    let (cmd, resumed) = launch_command(engines, &o.engine, o.last_woken.is_some(), fresh)?;
    let mut pre_notes: Vec<String> = crate::orca::engine_note(&cmd).into_iter().collect();
    if resumed {
        pre_notes.push(format!("resumed with `{cmd}`"));
    }
    let home = o.home.to_string_lossy().to_string();
    let repo_id = match o.orca.repo_id.clone() {
        Some(r) => r,
        None => ctx.orca.repo_add(&home).context("orca repo add")?,
    };
    // Orca 1.4.224 worktree selector: `path:<home>` (forward slashes)
    let wt = crate::orca::path_selector(&home);
    let t = ctx.orca.terminal_create(Some(&wt), &o.name, Some(&cmd))?;
    {
        let e = fleet.get_mut(name).expect("checked above");
        e.orca.repo_id = Some(repo_id);
        e.orca.worktree = Some(wt);
        e.orca.terminal = Some(t.handle.clone());
        e.awake = true;
        e.last_woken = Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    }
    fleet.save(&ctx.paths)?;
    let mut notes = pre_notes;
    match handle_first_run(ctx, &o.name, &o.engine, &t.handle) {
        Ok(n) => notes.extend(n),
        // close (never answer) the seat: a later `sleep`/`shutdown` would type
        // `/exit` + Enter, which picks the highlighted option of the API-key menu
        Err(e)
            if e.downcast_ref::<NotLoggedIn>().is_some()
                || e.downcast_ref::<DialogBlocked>()
                    .is_some_and(|b| b.kind == Dialog::ApiKey) =>
        {
            let _ = ctx.orca.terminal_close(&t.handle);
            let entry = fleet.get_mut(name).expect("checked above");
            entry.orca.terminal = None;
            entry.awake = false;
            fleet.save(&ctx.paths)?;
            return Err(e);
        }
        Err(e) => return Err(e),
    }
    Ok(Outcome::ok(
        format!("{} is awake ({}){}", o.name, t.handle, fmt_notes(&notes)),
        json!({"name": o.name, "terminal": t.handle, "command": cmd, "resumed": resumed, "notes": notes}),
    ))
}

fn fmt_notes(n: &[String]) -> String {
    if n.is_empty() {
        String::new()
    } else {
        format!(" — {}", n.join("; "))
    }
}

/// Sends `/exit`, waits a moment, closes the terminal. `keep_awake` keeps the
/// startup flag (used by `shutdown` so `startup` brings it back).
pub fn sleep(ctx: &Ctx, fleet: &mut Fleet, name: &str, keep_awake: bool) -> Result<Outcome> {
    let terms = live(ctx)?;
    let o = fleet.require(name)?.clone();
    let mut closed = false;
    if let Some(t) = find_live(&terms, o.orca.terminal.as_deref()) {
        let _ = ctx.orca.terminal_send(&t.handle, "/exit", true);
        for _ in 0..ctx.pacing.exit_polls {
            ctx.pacing.nap();
            if state_of(ctx, &t.handle)?.is_none() {
                break;
            }
        }
        if state_of(ctx, &t.handle)?.is_some() {
            ctx.orca.terminal_close(&t.handle)?;
        }
        closed = true;
    }
    let e = fleet.get_mut(name).expect("checked above");
    e.orca.terminal = None;
    if !keep_awake {
        e.awake = false;
    }
    fleet.save(&ctx.paths)?;
    let msg = if closed {
        format!("{} is resting (home and memory kept)", o.name)
    } else {
        format!("{} was already resting", o.name)
    };
    Ok(Outcome::ok(msg, json!({"name": o.name, "closed": closed})))
}

// ---------------------------------------------------------------------------
// hey / wait / ask / peek
// ---------------------------------------------------------------------------

/// Polls until the agent is seen working (or blocked). Returns true if seen.
fn seen_working(ctx: &Ctx, handle: &str) -> Result<bool> {
    for _ in 0..ctx.pacing.start_polls.max(1) {
        match state_of(ctx, handle)? {
            Some(TermState::Working) | Some(TermState::Blocked) => return Ok(true),
            None => bail!("terminal {handle} disappeared"),
            _ => {}
        }
        ctx.pacing.nap();
    }
    Ok(false)
}

/// Sends text. Delivered only when Orca reports `turn_started` or the agent
/// is seen working shortly after.
pub fn send_confirmed(ctx: &Ctx, handle: &str, text: &str) -> Result<(SendPhase, bool)> {
    let phase = ctx.orca.terminal_send(handle, text, true)?;
    let started = phase == SendPhase::TurnStarted;
    let working = seen_working(ctx, handle)?;
    Ok((phase, started || working))
}

pub fn hey(ctx: &Ctx, fleet: &Fleet, name: &str, text: &str) -> Result<Outcome> {
    let h = handle_of(ctx, fleet, name)?;
    let (phase, ok) = send_confirmed(ctx, &h, text)?;
    let j = json!({"to": name, "terminal": h, "phase": phase, "delivered": ok});
    if ok {
        Ok(Outcome::ok(format!("sent to {name}"), j))
    } else {
        Ok(Outcome::ok(
            format!("sent to {name} but no turn start was seen (phase {phase:?}) — check with `codename peek {name}`"),
            j,
        )
        .with_code(1))
    }
}

pub fn wait(ctx: &Ctx, fleet: &Fleet, name: &str, timeout: u64) -> Result<Outcome> {
    let h = handle_of(ctx, fleet, name)?;
    let w = ctx.orca.terminal_wait(&h, timeout)?;
    Ok(Outcome::ok(
        format!("{name}: {w:?}"),
        json!({"name": name, "outcome": w}),
    )
    .with_code(w.exit_code()))
}

pub fn ask(ctx: &Ctx, fleet: &Fleet, name: &str, text: &str, timeout: u64) -> Result<Outcome> {
    let h = handle_of(ctx, fleet, name)?;
    // Trap: if the agent was idle before, `wait` would return at once. So we
    // first require a turn start / working state, then wait for idle.
    let (phase, started) = send_confirmed(ctx, &h, text)?;
    let w: WaitOutcome = ctx.orca.terminal_wait(&h, timeout)?;
    // Rendered screen first (TUIs repaint); if the prompt scrolled off it,
    // fall back to the accumulated stream.
    let screen = ctx.orca.terminal_read(&h, 400, true)?;
    if login_required(&screen) {
        let engine = fleet.require(name)?.engine.clone();
        return Err(NotLoggedIn(engine).into());
    }
    let (mut answer, mut found) = latest_turn(&screen, text);
    if !found {
        let stream = ctx.orca.terminal_read(&h, 400, false)?;
        let (a2, f2) = latest_turn(&stream, text);
        if f2 {
            (answer, found) = (a2, f2);
        }
    }
    let j = json!({
        "name": name, "outcome": w, "phase": phase, "started": started,
        "matched_prompt": found, "answer": answer,
    });
    let mut text_out = answer.clone();
    if !started {
        text_out.push_str(
            "\n[codename] warning: the turn start was never seen; the answer may be stale",
        );
    }
    if w != WaitOutcome::Done {
        text_out.push_str(&format!("\n[codename] {w:?}"));
    }
    Ok(Outcome::ok(text_out, j).with_code(w.exit_code()))
}

pub fn peek(ctx: &Ctx, fleet: &Fleet, name: &str, lines: usize) -> Result<Outcome> {
    let h = handle_of(ctx, fleet, name)?;
    let s = ctx.orca.terminal_read(&h, lines, true)?;
    let tail = last_lines(&s, lines);
    Ok(Outcome::ok(
        tail.clone(),
        json!({"name": name, "terminal": h, "screen": tail}),
    ))
}

fn last_lines(s: &str, n: usize) -> String {
    let v: Vec<&str> = s.lines().collect();
    v[v.len().saturating_sub(n)..].join("\n")
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extracts the latest turn: the text after the LAST line showing the sent
/// message. TUIs wrap long prompts, so we match on a whitespace-normalised
/// prefix of the message's first line. Returns (answer, matched).
pub fn latest_turn(screen: &str, sent: &str) -> (String, bool) {
    let first = sent
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let key: String = squash(first).chars().take(30).collect();
    let lines: Vec<&str> = screen.lines().collect();
    if !key.is_empty() {
        if let Some(i) = lines.iter().rposition(|l| squash(l).contains(&key)) {
            // skip continuation lines of a wrapped prompt
            let full = squash(sent);
            let mut acc = squash(lines[i]);
            let mut j = i + 1;
            while j < lines.len() && !acc.contains(&full) {
                let l = squash(lines[j]);
                if !l.is_empty() && full.contains(&l) && !is_chrome(lines[j]) {
                    acc.push(' ');
                    acc.push_str(&l);
                    j += 1;
                } else {
                    break;
                }
            }
            return (clean(&lines[j..]), true);
        }
    }
    (clean(&lines[lines.len().saturating_sub(40)..]), false)
}

fn is_chrome(l: &str) -> bool {
    let t = l.trim();
    t.is_empty()
        || t.chars().all(|c| "─━│┃╭╮╰╯┌┐└┘├┤═║>❯ ".contains(c))
        || t.contains("? for shortcuts")
        || t.contains("esc to interrupt")
}

/// Drops TUI chrome (borders, empty prompt, status hints) and trims blanks.
fn clean(lines: &[&str]) -> String {
    let mut kept: Vec<String> = lines
        .iter()
        .filter(|l| !is_chrome(l) || l.trim().is_empty())
        .map(|l| {
            l.trim_end()
                .trim_start_matches(['│', '┃'])
                .trim_end_matches(['│', '┃'])
                .trim_end()
                .to_string()
        })
        .collect();
    while kept.first().is_some_and(|l| l.trim().is_empty()) {
        kept.remove(0);
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    kept.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeOrca;
    use crate::registry::{Codename, OrcaRef, Paths};
    use crate::Pacing;
    use std::time::Duration;

    fn pacing() -> Pacing {
        Pacing {
            poll: Duration::ZERO,
            start_polls: 5,
            dialog_polls: 5,
            exit_polls: 1,
        }
    }

    fn setup(fake: &FakeOrca, dir: &std::path::Path) -> Fleet {
        fake.add_terminal("t1", "nova", TermState::Idle);
        let mut f = Fleet::default();
        f.codenames.push(Codename {
            name: "nova".into(),
            number: 1,
            home: dir.to_path_buf(),
            orca: OrcaRef {
                terminal: Some("t1".into()),
                ..Default::default()
            },
            ..Default::default()
        });
        f
    }

    #[test]
    fn latest_turn_takes_only_last_answer() {
        let screen = "\
> ตอบคำเดียว: CN-OK
● CN-OK
> ตอบคำเดียว: CN-OK
● CN-OK (second)
more line
╭──────────╮
│ >        │
╰──────────╯
  ? for shortcuts";
        let (a, found) = latest_turn(screen, "ตอบคำเดียว: CN-OK");
        assert!(found);
        assert_eq!(a, "● CN-OK (second)\nmore line");
    }

    #[test]
    fn latest_turn_handles_wrapped_prompt() {
        let sent = "please summarise the whole repository structure in one short paragraph";
        let screen = "old stuff\n> please summarise the whole repository\nstructure in one short paragraph\nThe repo has src and docs.";
        let (a, found) = latest_turn(screen, sent);
        assert!(found);
        assert_eq!(a, "The repo has src and docs.");
    }

    #[test]
    fn latest_turn_keeps_answer_equal_to_part_of_prompt() {
        let (a, found) = latest_turn("> ตอบคำเดียว: CN-OK\nCN-OK\n> ", "ตอบคำเดียว: CN-OK");
        assert!(found);
        assert_eq!(a, "CN-OK");
    }

    #[test]
    fn latest_turn_fallback_when_prompt_scrolled_away() {
        let (a, found) = latest_turn("x\ny\nz", "not on screen");
        assert!(!found);
        assert_eq!(a, "x\ny\nz");
    }

    #[test]
    fn ask_waits_for_working_before_idle() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let fleet = setup(&fake, d.path());
        // Already idle before the send; Orca only "accepted" the keys.
        fake.send_phase.set(SendPhase::Accepted);
        fake.states.borrow_mut().extend([
            TermState::Idle, // handle_of check
            TermState::Idle,
            TermState::Idle,
            TermState::Working,
            TermState::Idle,
        ]);
        *fake.screen.borrow_mut() = "> hello\nHi พี่เก่ง ค่ะ".into();
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        let out = ask(&ctx, &fleet, "nova", "hello", 30).unwrap();
        assert!(
            !fake.wait_too_early.get(),
            "wait ran before the agent was seen working"
        );
        assert_eq!(out.code, 0);
        assert_eq!(out.text, "Hi พี่เก่ง ค่ะ");
    }

    #[test]
    fn ask_exit_codes() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let fleet = setup(&fake, d.path());
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        fake.wait_outcome.set(WaitOutcome::Timeout);
        assert_eq!(ask(&ctx, &fleet, "nova", "x", 1).unwrap().code, 2);
        fake.wait_outcome.set(WaitOutcome::Blocked);
        assert_eq!(wait(&ctx, &fleet, "nova", 1).unwrap().code, 3);
    }

    #[test]
    fn hey_unconfirmed_is_not_success() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let fleet = setup(&fake, d.path());
        fake.send_phase.set(SendPhase::Accepted);
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        assert_eq!(hey(&ctx, &fleet, "nova", "x").unwrap().code, 1);
        fake.send_phase.set(SendPhase::TurnStarted);
        assert_eq!(hey(&ctx, &fleet, "nova", "x").unwrap().code, 0);
    }

    #[test]
    fn resting_codename_errors() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let mut fleet = setup(&fake, d.path());
        fleet.codenames[0].orca.terminal = Some("gone".into());
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        assert!(hey(&ctx, &fleet, "nova", "x").is_err());
    }

    #[test]
    fn dialogs_classified() {
        assert_eq!(
            classify_dialog("Do you trust the files in this folder?")
                .unwrap()
                .0,
            Dialog::Trust
        );
        assert_eq!(classify_dialog("Select login method:").unwrap().1, None);
        assert_eq!(
            classify_dialog("Hooks have changed since last run")
                .unwrap()
                .0,
            Dialog::Hooks
        );
        assert_eq!(
            classify_dialog("New MCP server found in this project: arra").unwrap(),
            (Dialog::McpApproval, None)
        );
        assert!(classify_dialog("> ready").is_none());
    }

    #[test]
    fn wakeup_creates_once_and_answers_trust() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let mut fleet = Fleet::default();
        fleet.codenames.push(Codename {
            name: "lyra".into(),
            home: d.path().to_path_buf(),
            ..Default::default()
        });
        fake.screens
            .borrow_mut()
            .push_back("Do you trust the files in this folder?\n1. Yes".into());
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        let engines = crate::registry::default_engines();
        let out = wakeup(&ctx, &mut fleet, &engines, "lyra", false).unwrap();
        assert_eq!(out.code, 0);
        assert_eq!(fake.calls_of("terminal", "create").len(), 1);
        assert!(fake
            .calls_of("terminal", "send")
            .iter()
            .any(|c| c.contains(&"1".to_string())));
        assert!(fleet.get("lyra").unwrap().awake);
        // second wakeup must not open another seat
        wakeup(&ctx, &mut fleet, &engines, "lyra", false).unwrap();
        assert_eq!(fake.calls_of("terminal", "create").len(), 1);
        // sleep closes it and clears the handle
        sleep(&ctx, &mut fleet, "lyra", false).unwrap();
        assert!(fleet.get("lyra").unwrap().orca.terminal.is_none());
        assert!(fake.terminals.borrow().is_empty());
        let cmd_of = |i: usize| {
            let c = &fake.calls_of("terminal", "create")[i];
            let k = c.iter().position(|a| a == "--command").unwrap();
            c[k + 1].clone()
        };
        assert_eq!(cmd_of(0), "claude", "first wakeup starts fresh");
        // woken before → resume
        let out = wakeup(&ctx, &mut fleet, &engines, "lyra", false).unwrap();
        assert_eq!(cmd_of(1), "claude --continue");
        assert_eq!(out.json["resumed"], true);
        sleep(&ctx, &mut fleet, "lyra", false).unwrap();
        // --fresh overrides resume
        wakeup(&ctx, &mut fleet, &engines, "lyra", true).unwrap();
        assert_eq!(cmd_of(2), "claude");
        assert!(fleet.get("lyra").unwrap().last_woken.is_some());
    }

    #[test]
    fn wakeup_stops_on_login_dialog() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let mut fleet = Fleet::default();
        fleet.codenames.push(Codename {
            name: "lyra".into(),
            home: d.path().to_path_buf(),
            ..Default::default()
        });
        *fake.screen.borrow_mut() = "Select login method:\n1. Claude account".into();
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        let err = wakeup(
            &ctx,
            &mut fleet,
            &crate::registry::default_engines(),
            "lyra",
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("claude is not logged in"), "{err}");
        assert!(fake.calls_of("terminal", "send").is_empty());
        // the seat is closed and the registry does not point at it
        assert!(fake.terminals.borrow().is_empty());
        assert!(fleet.get("lyra").unwrap().orca.terminal.is_none());
    }

    const CODEX_UPDATE: &str = include_str!("../tests/fixtures/screens/codex-update-dialog.txt");
    const CODEX_BANNER_READY: &str =
        include_str!("../tests/fixtures/screens/codex-update-banner-ready.txt");
    const CLAUDE_NOT_LOGGED_IN: &str =
        include_str!("../tests/fixtures/screens/claude-not-logged-in.txt");
    const CLAUDE_HOOK_ERROR: &str =
        include_str!("../tests/fixtures/screens/claude-sessionstart-hook-error.txt");
    const CODEX_SIGN_IN: &str = include_str!("../tests/fixtures/screens/codex-sign-in.txt");
    const CLAUDE_API_KEY: &str =
        include_str!("../tests/fixtures/screens/claude-custom-api-key.txt");
    const CLAUDE_LOGIN_METHOD: &str =
        include_str!("../tests/fixtures/screens/claude-login-method.txt");

    #[test]
    fn api_key_and_console_login_are_never_answered() {
        assert_eq!(
            classify_dialog(CLAUDE_API_KEY),
            Some((Dialog::ApiKey, None))
        );
        assert!(!is_ready(CLAUDE_API_KEY));
        // even with a trust prompt on the same screen the API-key menu wins
        let both = format!("Do you trust the files in this folder?\n{CLAUDE_API_KEY}");
        assert_eq!(classify_dialog(&both), Some((Dialog::ApiKey, None)));
        // the Console / login-method choice is a login screen → stop
        assert_eq!(
            classify_dialog(CLAUDE_LOGIN_METHOD).unwrap().0,
            Dialog::Login
        );
        // "API Usage Billing" alone (header of an API-billed session) is not a dialog
        assert_eq!(classify_dialog(CLAUDE_HOOK_ERROR), None);
        assert!(!login_required("│   API Usage Billing   │"));
    }

    #[test]
    fn wakeup_stops_at_api_key_prompt_without_sending() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let mut fleet = one_codename(d.path(), "claude");
        *fake.screen.borrow_mut() =
            format!("Do you trust the files in this folder?\n{CLAUDE_API_KEY}");
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        let err = wakeup(
            &ctx,
            &mut fleet,
            &crate::registry::default_engines(),
            "vela",
            false,
        )
        .unwrap_err();
        let b = err.downcast_ref::<DialogBlocked>().expect("blocked error");
        assert_eq!(b.kind, Dialog::ApiKey);
        assert!(err.to_string().starts_with("blocked:"), "{err}");
        assert!(fake.calls_of("terminal", "send").is_empty());
        // the seat is closed, so no later sleep/shutdown types into the menu
        assert_eq!(fake.calls_of("terminal", "close").len(), 1);
        let o = fleet.get("vela").unwrap();
        assert!(o.orca.terminal.is_none() && !o.awake);
        sleep(&ctx, &mut fleet, "vela", false).unwrap();
        assert!(fake.calls_of("terminal", "send").is_empty());
    }

    #[test]
    fn real_screens_classified() {
        // the interactive Codex update menu → skip with "2"
        assert_eq!(
            classify_dialog(CODEX_UPDATE),
            Some((Dialog::Update, Some("2")))
        );
        assert!(!is_ready(CODEX_UPDATE));
        // the static banner left after skipping is NOT a dialog; screen is ready
        assert!(!is_update_menu(CODEX_BANNER_READY));
        assert_eq!(classify_dialog(CODEX_BANNER_READY), None);
        assert!(is_ready(CODEX_BANNER_READY));
        // "Update available!" alone never counts
        assert_eq!(
            classify_dialog("✨ Update available! 0.146.1 -> 0.162.1"),
            None
        );
        // not logged in
        assert!(login_required(CLAUDE_NOT_LOGGED_IN));
        assert_eq!(
            classify_dialog(CLAUDE_NOT_LOGGED_IN).unwrap().0,
            Dialog::Login
        );
        assert!(login_required(CODEX_SIGN_IN));
        // non-blocking SessionStart hook errors are not dialogs or failures
        assert_eq!(classify_dialog(CLAUDE_HOOK_ERROR), None);
        assert!(!login_required(CLAUDE_HOOK_ERROR));
        assert!(is_ready(CLAUDE_HOOK_ERROR));
        let free = NotLoggedIn("claude-free".into()).to_string();
        assert!(
            free.contains("Free Claude Code") && free.contains("fcc-doctor"),
            "{free}"
        );
        assert!(free.contains("do not /login") && free.contains("never logs in for you"));
        for screen in [
            "Free Claude Code proxy is not reachable at http://127.0.0.1:8082: refused\nStart it manually with: fcc-desktop",
            "Could not find Claude Code command: claude",
        ] {
            assert!(login_required(screen), "{screen}");
        }
        let msg = NotLoggedIn("codex".into()).to_string();
        assert!(msg.contains("codex is not logged in") && msg.contains("never logs in for you"));
    }

    fn one_codename(d: &std::path::Path, engine: &str) -> Fleet {
        let mut fleet = Fleet::default();
        fleet.codenames.push(Codename {
            name: "vela".into(),
            engine: engine.into(),
            home: d.to_path_buf(),
            ..Default::default()
        });
        fleet
    }

    #[test]
    fn wakeup_skips_codex_update_once_without_enter_then_ready() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let mut fleet = one_codename(d.path(), "codex");
        fake.screens.borrow_mut().push_back(CODEX_UPDATE.into());
        // after "2" the banner stays forever, with the ready prompt below it
        *fake.screen.borrow_mut() = CODEX_BANNER_READY.into();
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        let out = wakeup(
            &ctx,
            &mut fleet,
            &crate::registry::default_engines(),
            "vela",
            false,
        )
        .unwrap();
        let sends = fake.calls_of("terminal", "send");
        assert_eq!(sends.len(), 1, "{sends:?}");
        assert!(sends[0].windows(2).any(|w| w[0] == "--text" && w[1] == "2"));
        assert!(!sends[0].contains(&"--enter".to_string()));
        assert!(out.text.contains("answered Update dialog"));
    }

    #[test]
    fn wakeup_ignores_sessionstart_hook_errors() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let mut fleet = one_codename(d.path(), "claude");
        *fake.screen.borrow_mut() = CLAUDE_HOOK_ERROR.into();
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        wakeup(
            &ctx,
            &mut fleet,
            &crate::registry::default_engines(),
            "vela",
            false,
        )
        .unwrap();
        assert!(fake.calls_of("terminal", "send").is_empty());
        assert!(fleet.get("vela").unwrap().orca.terminal.is_some());
    }

    #[test]
    fn wakeup_not_logged_in_closes_seat() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let mut fleet = one_codename(d.path(), "claude");
        *fake.screen.borrow_mut() = CLAUDE_NOT_LOGGED_IN.into();
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join("h")),
            pacing: pacing(),
        };
        let err = wakeup(
            &ctx,
            &mut fleet,
            &crate::registry::default_engines(),
            "vela",
            false,
        )
        .unwrap_err();
        assert!(err.downcast_ref::<NotLoggedIn>().is_some());
        assert_eq!(fake.calls_of("terminal", "close").len(), 1);
        assert!(!fleet.get("vela").unwrap().awake);
    }
}
