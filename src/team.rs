//! Teams from a charter (lead + workers) on top of Orca orchestration.
//! Every orchestration call carries `--from <captain terminal>`.

use crate::orca::{WorkerStart, WorktreeInfo};
use crate::registry::{atomic_write, orca_agent, stamp, Engines, Fleet};
use crate::{seat, Ctx, Outcome};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Charter {
    pub name: String,
    /// Repo the workers branch from: a codename's name or a path to a git repo.
    pub repo: String,
    /// Optional existing codename who receives the briefing.
    #[serde(default)]
    pub lead: Option<String>,
    pub workers: Vec<WorkerSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct WorkerSpec {
    pub name: String,
    #[serde(default = "default_engine")]
    pub engine: String,
    pub task: String,
}

fn default_engine() -> String {
    "codex".into()
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct TeamState {
    pub name: String,
    pub run: String,
    pub from: String,
    pub repo_id: String,
    pub lead: Option<String>,
    pub workers: Vec<WorkerState>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct WorkerState {
    pub name: String,
    pub engine: String,
    pub task: String,
    /// Orca dispatch id from `orchestration worker-start`.
    pub dispatch: String,
    pub terminal: String,
    pub worktree_id: String,
    pub worktree_path: Option<String>,
}

pub fn parse_charter(yaml: &str) -> Result<Charter> {
    let c: Charter = serde_yaml::from_str(yaml).context("charter is not valid YAML")?;
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    };
    if !ok(&c.name) {
        bail!(
            "charter name `{}` must be letters, digits, `-` or `_`",
            c.name
        );
    }
    if c.workers.is_empty() {
        bail!("charter has no workers");
    }
    for w in &c.workers {
        if !ok(&w.name) {
            bail!(
                "worker name `{}` must be letters, digits, `-` or `_`",
                w.name
            );
        }
    }
    Ok(c)
}

fn state_path(ctx: &Ctx, name: &str) -> PathBuf {
    ctx.paths.teams().join(format!("{name}.json"))
}

fn captain_from(ctx: &Ctx, fleet: &Fleet) -> Result<String> {
    let h = fleet.captain_terminal.clone().ok_or_else(|| {
        anyhow!("no captain terminal — run `codename setup` first (needed as --from)")
    })?;
    if !seat::live(ctx)?.iter().any(|t| t.handle == h) {
        bail!("captain terminal {h} is not live — run `codename startup` (or `codename setup`)");
    }
    Ok(h)
}

pub fn up(ctx: &Ctx, fleet: &Fleet, engines: &Engines, charter_file: &Path) -> Result<Outcome> {
    let c = parse_charter(
        &fs::read_to_string(charter_file)
            .with_context(|| format!("read {}", charter_file.display()))?,
    )?;
    for w in &c.workers {
        orca_agent(engines, &w.engine)?;
    }
    let sp = state_path(ctx, &c.name);
    if sp.exists() {
        bail!(
            "team `{}` is already up — `codename team down {}` first",
            c.name,
            c.name
        );
    }
    let from = captain_from(ctx, fleet)?;
    let repo_id = match fleet.get(&c.repo) {
        Some(o) => match &o.orca.repo_id {
            Some(r) => r.clone(),
            None => ctx.orca.repo_add(&o.home.to_string_lossy())?,
        },
        None => ctx.orca.repo_add(&c.repo)?,
    };
    let run = ctx
        .orca
        .run_create(&from, &format!("codename team {}", c.name))?;
    let mut st = TeamState {
        name: c.name.clone(),
        run,
        from: from.clone(),
        repo_id: repo_id.clone(),
        lead: c.lead.clone(),
        workers: vec![],
    };
    for w in &c.workers {
        // worker-start creates a new top-level worktree + agent terminal
        let info = ctx.orca.worker_start(&WorkerStart {
            from: from.clone(),
            run: st.run.clone(),
            spec: w.task.clone(),
            agent: orca_agent(engines, &w.engine)?,
            repo_id: repo_id.clone(),
            name: format!("team-{}-{}", c.name, w.name),
            display_name: format!("{}-{}", c.name, w.name),
        })?;
        st.workers.push(WorkerState {
            name: w.name.clone(),
            engine: w.engine.clone(),
            task: w.task.clone(),
            dispatch: info.dispatch,
            terminal: info.terminal.unwrap_or_default(),
            worktree_id: info.worktree_id.unwrap_or_default(),
            worktree_path: info.worktree_path,
        });
        // persist after each worker so `team down` can clean up a partial start
        atomic_write(&sp, serde_json::to_string_pretty(&st)?.as_bytes())?;
    }
    let mut notes = vec![];
    if let Some(lead) = &c.lead {
        let brief = format!(
            "TEAM {} (run {}) started: {}",
            c.name,
            st.run,
            c.workers
                .iter()
                .map(|w| format!("{}={}", w.name, w.task))
                .collect::<Vec<_>>()
                .join("; ")
        );
        if let Err(e) = seat::hey(ctx, fleet, lead, &brief) {
            notes.push(format!("lead briefing failed: {e}"));
        }
    }
    Ok(Outcome::ok(
        format!(
            "team {} up: run {} with {} worker(s){}",
            c.name,
            st.run,
            st.workers.len(),
            if notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", notes.join("; "))
            }
        ),
        serde_json::to_value(&st)?,
    ))
}

fn load_state(ctx: &Ctx, name: Option<&str>) -> Result<TeamState> {
    let p = match name {
        Some(n) => state_path(ctx, n),
        None => {
            let mut all: Vec<PathBuf> = fs::read_dir(ctx.paths.teams())
                .map(|r| {
                    r.filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x == "json"))
                        .collect()
                })
                .unwrap_or_default();
            match all.len() {
                0 => bail!("no team is up"),
                1 => all.remove(0),
                _ => bail!("several teams are up — name one"),
            }
        }
    };
    let s = fs::read_to_string(&p).with_context(|| format!("no team state at {}", p.display()))?;
    serde_json::from_str(&s).with_context(|| format!("{} is corrupt", p.display()))
}

pub fn status(ctx: &Ctx, name: Option<&str>) -> Result<Outcome> {
    let st = load_state(ctx, name)?;
    let check = ctx.orca.worker_list(&st.run)?;
    let live = seat::live(ctx)?;
    let mut lines = vec![format!("team {} · run {}", st.name, st.run)];
    let mut ws = vec![];
    for w in &st.workers {
        let state = live
            .iter()
            .find(|t| t.handle == w.terminal)
            .map(|t| format!("{:?}", t.state).to_lowercase())
            .unwrap_or_else(|| "gone".into());
        lines.push(format!(
            "  {:<14} {:<8} {:<8} {}",
            w.name, w.engine, state, w.task
        ));
        ws.push(json!({"name": w.name, "state": state, "terminal": w.terminal}));
    }
    Ok(Outcome::ok(
        lines.join("\n"),
        json!({"team": st.name, "run": st.run, "workers": ws, "orca": check}),
    ))
}

fn git_out(dir: &str, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("cannot run git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Commits uncommitted work to the worktree's branch so nothing is lost.
fn save_wip(path: &str, team: &str) -> Result<bool> {
    if git_out(path, &["status", "--porcelain"])?.trim().is_empty() {
        return Ok(false);
    }
    git_out(path, &["add", "-A"])?;
    git_out(
        path,
        &[
            "commit",
            "-q",
            "--no-verify",
            "-m",
            &format!("WIP: team {team} down (saved by codename)"),
        ],
    )?;
    Ok(true)
}

pub fn down(ctx: &Ctx, name: Option<&str>) -> Result<Outcome> {
    let st = load_state(ctx, name)?;
    let live = seat::live(ctx)?;
    let known_wts = ctx.orca.worktree_list().unwrap_or_default();
    let mut lines = vec![];
    let mut kept = 0;
    for w in &st.workers {
        if !w.dispatch.is_empty() {
            if let Err(e) = ctx.orca.worker_stop(&w.dispatch) {
                lines.push(format!("{}: worker-stop failed ({e})", w.name));
            }
        }
        if live.iter().any(|t| t.handle == w.terminal) {
            let _ = ctx.orca.terminal_send(&w.terminal, "/exit", true);
            ctx.orca.terminal_close(&w.terminal)?;
        }
        let path = w.worktree_path.clone().or_else(|| {
            known_wts
                .iter()
                .find(|k| k.id == w.worktree_id)
                .and_then(|k| k.path.clone())
        });
        let selector = WorktreeInfo {
            id: w.worktree_id.clone(),
            path: path.clone(),
            branch: None,
        }
        .selector();
        match path {
            Some(p) if Path::new(&p).is_dir() => match save_wip(&p, &st.name) {
                Ok(saved) => {
                    ctx.orca.worktree_rm(&selector)?;
                    lines.push(format!(
                        "{}: closed{}",
                        w.name,
                        if saved {
                            ", WIP committed to its branch"
                        } else {
                            ""
                        }
                    ));
                }
                Err(e) => {
                    kept += 1;
                    lines.push(format!("{}: worktree KEPT ({e})", w.name));
                }
            },
            Some(_) => {
                ctx.orca.worktree_rm(&selector)?;
                lines.push(format!("{}: closed (worktree dir already gone)", w.name));
            }
            None => {
                kept += 1;
                lines.push(format!(
                    "{}: worktree {} KEPT — path unknown, cannot check for uncommitted work",
                    w.name, w.worktree_id
                ));
            }
        }
    }
    let sp = state_path(ctx, &st.name);
    let hist = ctx.paths.teams().join("history");
    fs::create_dir_all(&hist)?;
    fs::rename(&sp, hist.join(format!("{}-{}.json", st.name, stamp())))?;
    Ok(Outcome::ok(
        format!("team {} down\n  {}", st.name, lines.join("\n  ")),
        json!({"team": st.name, "workers": lines, "kept": kept}),
    )
    .with_code(if kept > 0 { 1 } else { 0 }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeOrca;
    use crate::orca::TermState;
    use crate::registry::Paths;
    use crate::Pacing;
    use std::time::Duration;

    #[test]
    fn example_charter_parses() {
        let c = parse_charter(include_str!("../examples/charter.yaml")).unwrap();
        assert!(!c.workers.is_empty());
        assert!(parse_charter("name: bad name\nrepo: x\nworkers: []").is_err());
    }

    #[test]
    fn team_up_down_balances_terminals_and_worktrees() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        fake.add_terminal("cap", "codename-captain", TermState::Idle);
        let fleet = Fleet {
            captain_terminal: Some("cap".into()),
            ..Default::default()
        };
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join(".codename")),
            pacing: Pacing {
                poll: Duration::ZERO,
                start_polls: 1,
                dialog_polls: 1,
                exit_polls: 1,
            },
        };
        let charter = d.path().join("c.yaml");
        fs::write(&charter, "name: demo\nrepo: /tmp/x\nworkers:\n  - name: a\n    task: t1\n  - name: b\n    engine: claude\n    task: t2\n").unwrap();
        let before = (fake.terminals.borrow().len(), fake.worktrees.borrow().len());
        let out = up(&ctx, &fleet, &crate::registry::default_engines(), &charter).unwrap();
        assert_eq!(out.code, 0);
        assert_eq!(fake.terminals.borrow().len(), before.0 + 2);
        for c in fake
            .calls_of("orchestration", "run-create")
            .iter()
            .chain(fake.calls_of("orchestration", "worker-start").iter())
        {
            assert!(c.windows(2).any(|w| w[0] == "--from" && w[1] == "cap"));
        }
        let starts = fake.calls_of("orchestration", "worker-start");
        assert_eq!(starts.len(), 2);
        let has = |c: &Vec<String>, k: &str, v: &str| c.windows(2).any(|w| w[0] == k && w[1] == v);
        assert!(has(&starts[0], "--agent", "codex") && has(&starts[1], "--agent", "claude"));
        assert!(has(&starts[0], "--display-name", "demo-a"));
        assert!(has(&starts[0], "--worktree", "new-top-level"));
        assert!(has(&starts[0], "--spec", "t1"));
        assert!(
            up(&ctx, &fleet, &crate::registry::default_engines(), &charter).is_err(),
            "double up"
        );
        status(&ctx, None).unwrap();
        // the fake has no worktree paths → worktrees are KEPT (safety), terminals closed
        let o = down(&ctx, Some("demo")).unwrap();
        assert_eq!(o.code, 1);
        assert_eq!(fake.terminals.borrow().len(), before.0);
    }

    fn git_repo(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        for a in [
            vec!["init", "-q"],
            vec!["config", "user.name", "test"],
            vec!["config", "user.email", "test@example.invalid"],
            vec!["commit", "-q", "--allow-empty", "-m", "base"],
        ] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(&a)
                .status()
                .unwrap()
                .success());
        }
    }

    #[test]
    fn team_down_commits_wip_then_removes_worktree() {
        let d = tempfile::tempdir().unwrap();
        let wt = d.path().join("wt-a");
        git_repo(&wt);
        fs::write(wt.join("work.txt"), "unsaved").unwrap();
        let fake = FakeOrca::default();
        fake.add_terminal("cap", "codename-captain", TermState::Idle);
        fake.worktree_paths
            .borrow_mut()
            .push_back(wt.to_string_lossy().into_owned());
        let fleet = Fleet {
            captain_terminal: Some("cap".into()),
            ..Default::default()
        };
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path().join(".codename")),
            pacing: Pacing {
                poll: Duration::ZERO,
                start_polls: 1,
                dialog_polls: 1,
                exit_polls: 1,
            },
        };
        let charter = d.path().join("c.yaml");
        fs::write(
            &charter,
            "name: demo\nrepo: /tmp/x\nworkers:\n  - name: a\n    task: t1\n",
        )
        .unwrap();
        let before = (fake.terminals.borrow().len(), fake.worktrees.borrow().len());
        up(&ctx, &fleet, &crate::registry::default_engines(), &charter).unwrap();
        let o = down(&ctx, None).unwrap();
        assert_eq!(o.code, 0, "{}", o.text);
        assert_eq!(
            (fake.terminals.borrow().len(), fake.worktrees.borrow().len()),
            before
        );
        let log = git_out(&wt.to_string_lossy(), &["log", "--oneline"]).unwrap();
        assert!(log.contains("WIP: team demo down"), "{log}");
        assert!(
            wt.join("work.txt").exists(),
            "user files must never be deleted"
        );
    }

    #[test]
    fn team_up_requires_captain_terminal() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let ctx = Ctx {
            orca: &fake,
            paths: Paths::new(d.path()),
            pacing: Pacing::default(),
        };
        let charter = d.path().join("c.yaml");
        fs::write(
            &charter,
            "name: demo\nrepo: /tmp/x\nworkers:\n  - name: a\n    task: t\n",
        )
        .unwrap();
        let err = up(
            &ctx,
            &Fleet::default(),
            &crate::registry::default_engines(),
            &charter,
        )
        .unwrap_err();
        assert!(err.to_string().contains("setup"));
    }
}
