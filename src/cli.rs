//! Command line definition and dispatch.
//!
//! Naming rule: commands that wrap something Orca already has keep the fleet's
//! original names with no alias (hey, wait, ask, peek, wt, team). Commands that
//! are new get a full-word name plus the old short name as a visible alias.

use crate::registry::{load_engines, validate_name, Config, Fleet};
use crate::{birth, doctor, fleet, memory, seat, service, team, Ctx, Outcome};
use anyhow::{bail, Result};
use clap::{CommandFactory, Parser, Subcommand};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser, PartialEq)]
#[command(
    name = "codename",
    version,
    about = "A fleet of named codenames on top of the Orca desktop app",
    after_help = "Orca must stay open with its CLI enabled. Orca flags used by this tool are unverified — see docs/DESIGN.md."
)]
pub struct Cli {
    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,
    /// Also print the orca calls and the log line to stderr.
    #[arg(long, global = true)]
    pub verbose: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum Cmd {
    /// First-time setup: home, config baseline backup, captain terminal.
    #[command(visible_alias = "init")]
    Setup {
        /// Fleet root; codenames are born in <root>/codenames (default: ~/.codename).
        #[arg(long)]
        root: Option<PathBuf>,
        /// Take a new agent-config baseline even if one exists.
        #[arg(long)]
        rebaseline: bool,
        /// Also register free-model engines claude-free / codex-free (Free Claude Code, installed separately).
        #[arg(long)]
        free: bool,
    },
    /// Bring the whole fleet back (after login / reboot).
    #[command(visible_alias = "up")]
    Startup,
    /// Put every codename to sleep, crew first, captain last.
    #[command(visible_alias = "down")]
    Shutdown,
    /// Fleet table: name, engine, status (rest/idle/working/blocked), home.
    #[command(visible_alias = "ls")]
    List,
    /// Give birth to a new codename (a new member of the fleet).
    #[command(visible_alias = "bud")]
    Birth {
        /// Name for the newborn (or use --choose).
        name: Option<String>,
        /// Let the newborn choose its own name.
        #[arg(long)]
        choose: bool,
        /// Engine key from engines.json.
        #[arg(long, default_value = "claude")]
        engine: String,
        /// One-line duty.
        #[arg(long)]
        role: String,
        /// Make this codename the captain (only one captain).
        #[arg(long)]
        captain: bool,
        /// Do not open a seat after birth.
        #[arg(long)]
        no_wake: bool,
        /// Skip the self-introduction acceptance check.
        #[arg(long)]
        no_verify: bool,
    },
    /// Open a codename's seat (never opens a second one).
    #[command(visible_alias = "wake")]
    Wakeup {
        name: String,
        /// Start a new conversation instead of the engine's resume command.
        #[arg(long)]
        fresh: bool,
    },
    /// Close a codename's seat; home and memory stay.
    #[command(visible_alias = "rest")]
    Sleep { name: String },
    /// Send a message and do not wait.
    Hey {
        name: String,
        #[arg(required = true, num_args = 1..)]
        message: Vec<String>,
    },
    /// Wait until the codename is idle. Exit 0 done, 2 timeout, 3 blocked.
    Wait {
        name: String,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Send, wait, print only the latest answer. Exit 0/2/3 like wait.
    Ask {
        name: String,
        #[arg(required = true, num_args = 1..)]
        message: Vec<String>,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Show the codename's latest screen.
    Peek {
        name: String,
        #[arg(short = 'n', long, default_value_t = 40)]
        lines: usize,
    },
    /// Create a separate worktree for a codename's code work.
    Wt { name: String, slug: String },
    /// Teams from a charter (lead + workers).
    Team {
        #[command(subcommand)]
        cmd: TeamCmd,
    },
    /// Report back to the captain: "REPORT <me> rc=<code>: ...".
    #[command(visible_alias = "done")]
    Report {
        #[arg(required = true, num_args = 1..)]
        message: Vec<String>,
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        rc: i32,
        /// Sender name (default: the codename whose home holds the cwd).
        #[arg(long = "as")]
        as_name: Option<String>,
    },
    /// Store a lesson in shared memory (arra).
    #[command(visible_alias = "learn")]
    Remember {
        #[arg(required = true, num_args = 1..)]
        lesson: Vec<String>,
    },
    /// Search shared memory (arra).
    #[command(visible_alias = "search")]
    Recall {
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// Windows scheduled task for the headless Orca runtime (no GUI needed).
    #[command(visible_alias = "svc")]
    ServeTask {
        #[command(subcommand)]
        cmd: ServeTaskCmd,
    },
    /// Health checks: Orca, telemetry, arra, registry, agent-config drift.
    #[command(visible_alias = "check")]
    Doctor {
        /// Also ask every live codename who it is.
        #[arg(long)]
        identity: bool,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum ServeTaskCmd {
    /// Register a logon task running `orca serve --no-pairing --json`.
    Install {
        #[arg(long, default_value = service::DEFAULT_TASK)]
        name: String,
    },
    /// Delete the scheduled task (the configured one unless --name).
    Remove {
        #[arg(long)]
        name: Option<String>,
    },
    /// Show whether the scheduled task exists.
    Status {
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Debug, Subcommand, PartialEq)]
pub enum TeamCmd {
    /// Start a team from a charter YAML.
    Up { charter: PathBuf },
    /// Show a team's workers.
    Status { name: Option<String> },
    /// Stop a team: close terminals, commit WIP, remove worktrees.
    Down { name: Option<String> },
}

impl Cmd {
    /// (command name, codename) for the log. Message bodies are not included.
    pub fn log_info(&self) -> (String, Option<String>) {
        let (c, o): (&str, Option<&String>) = match self {
            Cmd::Setup { .. } => ("setup", None),
            Cmd::Startup => ("startup", None),
            Cmd::Shutdown => ("shutdown", None),
            Cmd::List => ("list", None),
            Cmd::Birth { name, .. } => ("birth", name.as_ref()),
            Cmd::Wakeup { name, .. } => ("wakeup", Some(name)),
            Cmd::Sleep { name } => ("sleep", Some(name)),
            Cmd::Hey { name, .. } => ("hey", Some(name)),
            Cmd::Wait { name, .. } => ("wait", Some(name)),
            Cmd::Ask { name, .. } => ("ask", Some(name)),
            Cmd::Peek { name, .. } => ("peek", Some(name)),
            Cmd::Wt { name, .. } => ("wt", Some(name)),
            Cmd::Team { cmd } => match cmd {
                TeamCmd::Up { .. } => ("team up", None),
                TeamCmd::Status { name } => ("team status", name.as_ref()),
                TeamCmd::Down { name } => ("team down", name.as_ref()),
            },
            Cmd::Report { as_name, .. } => ("report", as_name.as_ref()),
            Cmd::Remember { .. } => ("remember", None),
            Cmd::Recall { .. } => ("recall", None),
            Cmd::Doctor { .. } => ("doctor", None),
            Cmd::ServeTask { cmd } => match cmd {
                ServeTaskCmd::Install { .. } => ("serve-task install", None),
                ServeTaskCmd::Remove { .. } => ("serve-task remove", None),
                ServeTaskCmd::Status { .. } => ("serve-task status", None),
            },
        };
        (c.to_string(), o.cloned())
    }
}

/// Every command name and alias (plus `help`): member names may not use them.
pub fn reserved_names() -> Vec<String> {
    let mut v = vec!["help".to_string()];
    for c in Cli::command().get_subcommands() {
        v.push(c.get_name().to_string());
        v.extend(c.get_all_aliases().map(str::to_string));
        for s in c.get_subcommands() {
            v.push(s.get_name().to_string());
        }
    }
    v
}

fn slug_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn run(cmd: Cmd, ctx: &Ctx, user_home: &Path, cwd: &Path) -> Result<Outcome> {
    let p = &ctx.paths;
    let cfg = Config::load(p)?;
    let mut f = Fleet::load(p)?;
    match cmd {
        Cmd::Setup {
            root,
            rebaseline,
            free,
        } => fleet::setup(ctx, &mut f, root, rebaseline, free, user_home),
        Cmd::Startup => fleet::startup(ctx, &mut f, &load_engines(p)?, &cfg),
        Cmd::Shutdown => fleet::shutdown(ctx, &mut f),
        Cmd::List => fleet::list(ctx, &f),
        Cmd::Birth {
            name,
            choose,
            engine,
            role,
            captain,
            no_wake,
            no_verify,
        } => birth::birth(
            ctx,
            &mut f,
            &load_engines(p)?,
            &cfg,
            &reserved_names(),
            birth::BirthArgs {
                name,
                choose,
                engine,
                role,
                captain,
                wake: !no_wake,
                verify: !no_verify,
            },
        ),
        Cmd::Wakeup { name, fresh } => seat::wakeup(ctx, &mut f, &load_engines(p)?, &name, fresh),
        Cmd::Sleep { name } => seat::sleep(ctx, &mut f, &name, false),
        Cmd::Hey { name, message } => seat::hey(ctx, &f, &name, &message.join(" ")),
        Cmd::Wait { name, timeout } => seat::wait(ctx, &f, &name, timeout),
        Cmd::Ask {
            name,
            message,
            timeout,
        } => seat::ask(ctx, &f, &name, &message.join(" "), timeout),
        Cmd::Peek { name, lines } => seat::peek(ctx, &f, &name, lines),
        Cmd::Wt { name, slug } => {
            if !slug_ok(&slug) {
                bail!("slug `{slug}` must be ASCII letters, digits, `-` or `_`");
            }
            let o = f.require(&name)?.clone();
            let repo = match &o.orca.repo_id {
                Some(r) => r.clone(),
                None => {
                    let r = ctx.orca.repo_add(&o.home.to_string_lossy())?;
                    f.get_mut(&name).expect("exists").orca.repo_id = Some(r.clone());
                    f.save(p)?;
                    r
                }
            };
            let w = ctx
                .orca
                .worktree_create(&repo, &format!("{}-{slug}", o.name))?;
            Ok(Outcome::ok(
                format!(
                    "worktree {} for {} ({}) — never remove it with uncommitted work; commit WIP to its branch first",
                    w.id,
                    o.name,
                    w.path.clone().unwrap_or_default()
                ),
                json!({"name": o.name, "worktree": w}),
            ))
        }
        Cmd::Team { cmd } => match cmd {
            TeamCmd::Up { charter } => team::up(ctx, &f, &load_engines(p)?, &charter),
            TeamCmd::Status { name } => team::status(ctx, name.as_deref()),
            TeamCmd::Down { name } => team::down(ctx, name.as_deref()),
        },
        Cmd::Report {
            message,
            rc,
            as_name,
        } => {
            let sender = fleet::resolve_sender(
                &f,
                as_name.as_deref(),
                cwd,
                crate::registry::sender_from_env(),
            )?;
            fleet::report(ctx, &f, &sender, rc, &message.join(" "))
        }
        Cmd::Remember { lesson } => {
            let source = fleet::resolve_sender(&f, None, cwd, crate::registry::sender_from_env())
                .map(|s| format!("codename:{s}"))
                .unwrap_or_else(|_| "codename".into());
            memory::remember(&cfg, &lesson.join(" "), &source)
        }
        Cmd::Recall { query, limit } => memory::recall(&cfg, &query.join(" "), limit),
        Cmd::Doctor { identity } => doctor::run(ctx, &f, &cfg, user_home, identity),
        Cmd::ServeTask { cmd } => serve_task(p, cfg, cmd),
    }
}

fn serve_task(p: &crate::registry::Paths, mut cfg: Config, cmd: ServeTaskCmd) -> Result<Outcome> {
    let root = service::system_root();
    match cmd {
        ServeTaskCmd::Install { name } => {
            service::validate_task_name(&name)?;
            let exe = crate::orca::resolve_orca(cfg.orca_bin.as_deref())?;
            crate::orca::guard_spawn(&exe)?;
            let (ok, out) =
                service::execute(&service::install_invocation(&root, &name, &exe.path))?;
            if !ok {
                bail!("registering task {name} failed: {out}");
            }
            cfg.serve_task = Some(name.clone());
            cfg.save(p)?;
            Ok(Outcome::ok(
                format!(
                    "scheduled task {name} registered: at logon runs `{} {}`\n  \
                     start it now: `codename startup` (or log off/on)\n  \
                     orca serve listens on 0.0.0.0:6768 — do NOT open that port in the Windows firewall; \
                     --no-pairing is always used",
                    exe.path.display(),
                    service::SERVE_ARGS
                ),
                json!({"task": name, "exe": exe.path, "args": service::SERVE_ARGS}),
            ))
        }
        ServeTaskCmd::Remove { name } => {
            let name = name
                .or_else(|| cfg.serve_task.clone())
                .unwrap_or_else(|| service::DEFAULT_TASK.into());
            service::validate_task_name(&name)?;
            let (ok, out) = service::execute(&service::remove_invocation(&root, &name))?;
            if !ok {
                bail!("removing task {name} failed: {out}");
            }
            if cfg.serve_task.as_deref() == Some(name.as_str()) {
                cfg.serve_task = None;
                cfg.save(p)?;
            }
            Ok(Outcome::ok(
                format!("scheduled task {name} removed"),
                json!({"task": name}),
            ))
        }
        ServeTaskCmd::Status { name } => {
            let name = name
                .or_else(|| cfg.serve_task.clone())
                .unwrap_or_else(|| service::DEFAULT_TASK.into());
            service::validate_task_name(&name)?;
            let (exists, out) = service::execute(&service::query_invocation(&root, &name))?;
            Ok(Outcome::ok(
                if exists { out.clone() } else { format!("task {name} not found") },
                json!({"task": name, "exists": exists, "configured": cfg.serve_task, "detail": out}),
            )
            .with_code(if exists { 0 } else { 1 }))
        }
    }
}

/// Validates a name against the live CLI definition (exposed for tests).
pub fn check_name(name: &str, fleet: &Fleet) -> Result<()> {
    validate_name(name, &reserved_names(), fleet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(a: &[&str]) -> Cmd {
        let mut v = vec!["codename"];
        v.extend_from_slice(a);
        Cli::try_parse_from(v).unwrap().cmd
    }

    #[test]
    fn log_info_never_carries_message() {
        let c = Cli::try_parse_from(["codename", "--verbose", "hey", "nova", "secret", "body"])
            .unwrap();
        assert!(c.verbose);
        assert_eq!(
            c.cmd.log_info(),
            ("hey".to_string(), Some("nova".to_string()))
        );
    }

    #[test]
    fn setup_free_flag_parses() {
        assert_eq!(
            parse(&["setup", "--free"]),
            Cmd::Setup {
                root: None,
                rebaseline: false,
                free: true
            }
        );
        assert_eq!(parse(&["init", "--free"]), parse(&["setup", "--free"]));
        assert_eq!(
            parse(&["setup"]),
            Cmd::Setup {
                root: None,
                rebaseline: false,
                free: false
            }
        );
    }

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn aliases_parse_to_same_command() {
        let pairs: &[(&[&str], &[&str])] = &[
            (&["setup"], &["init"]),
            (&["startup"], &["up"]),
            (&["shutdown"], &["down"]),
            (&["list"], &["ls"]),
            (&["birth", "x", "--role", "r"], &["bud", "x", "--role", "r"]),
            (&["wakeup", "x"], &["wake", "x"]),
            (&["wakeup", "x", "--fresh"], &["wake", "x", "--fresh"]),
            (&["sleep", "x"], &["rest", "x"]),
            (&["report", "ok"], &["done", "ok"]),
            (&["remember", "a"], &["learn", "a"]),
            (&["recall", "a"], &["search", "a"]),
            (&["doctor"], &["check"]),
            (&["serve-task", "install"], &["svc", "install"]),
            (
                &["serve-task", "status", "--name", "x"],
                &["svc", "status", "--name", "x"],
            ),
        ];
        for (full, alias) in pairs {
            assert_eq!(parse(full), parse(alias), "{full:?} vs {alias:?}");
        }
    }

    #[test]
    fn original_names_have_no_alias() {
        for name in ["hey", "wait", "ask", "peek", "wt", "team"] {
            let cmd = Cli::command();
            let sc = cmd.find_subcommand(name).unwrap();
            assert_eq!(
                sc.get_all_aliases().count(),
                0,
                "{name} must not have an alias"
            );
        }
    }

    #[test]
    fn new_commands_have_visible_aliases() {
        let cmd = Cli::command();
        for (name, alias) in [("setup", "init"), ("wakeup", "wake"), ("doctor", "check")] {
            let sc = cmd.find_subcommand(name).unwrap();
            assert!(sc.get_visible_aliases().any(|a| a == alias));
        }
    }

    #[test]
    fn message_args_join_and_json_is_global() {
        let c =
            Cli::try_parse_from(["codename", "ask", "nova", "ตอบ", "คำเดียว", "--json"]).unwrap();
        assert!(c.json);
        assert_eq!(
            c.cmd,
            Cmd::Ask {
                name: "nova".into(),
                message: vec!["ตอบ".into(), "คำเดียว".into()],
                timeout: 600
            }
        );
    }

    #[test]
    fn reserved_names_cover_commands_and_aliases() {
        let r = reserved_names();
        for n in [
            "setup", "init", "wake", "ls", "bud", "rest", "done", "learn", "search", "check",
            "hey", "team", "help",
        ] {
            assert!(r.iter().any(|x| x == n), "{n} not reserved");
            assert!(check_name(n, &Fleet::default()).is_err());
        }
        assert!(check_name("lyra", &Fleet::default()).is_ok());
    }
}
