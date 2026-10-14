//! Fleet-wide commands: setup, startup, shutdown, list, report.

use crate::registry::{
    atomic_write, default_engines, free_engines, merge_engines, Config, Engines, Fleet, Paths,
    CAPTAIN_TITLE,
};
use crate::{doctor, memory, seat, Ctx, Outcome};
use anyhow::{anyhow, bail, Result};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Makes sure the captain's identity terminal (a plain shell, used as
/// `--from` for orchestration) exists. Returns its handle.
pub fn ensure_captain(ctx: &Ctx, fleet: &mut Fleet, cfg: &Config) -> Result<String> {
    let live = seat::live(ctx)?;
    if let Some(h) = &fleet.captain_terminal {
        if live.iter().any(|t| &t.handle == h) {
            return Ok(h.clone());
        }
    }
    let dir = cfg.root(&ctx.paths).join("captain");
    fs::create_dir_all(&dir)?;
    if !dir.join(".git").exists() {
        let ok = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["init", "-q"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            bail!("git init failed in {} — is Git installed?", dir.display());
        }
        fs::write(
            dir.join("README.md"),
            "# codename captain\n\nIdentity home of the captain terminal (codename-captain).\n",
        )?;
    }
    let path = dir.to_string_lossy().to_string();
    let repo = match &fleet.captain_repo_id {
        Some(r) => r.clone(),
        None => ctx.orca.repo_add(&path)?,
    };
    let t = ctx.orca.terminal_create(
        Some(&crate::orca::path_selector(&path)),
        CAPTAIN_TITLE,
        None,
    )?;
    fleet.captain_repo_id = Some(repo);
    fleet.captain_terminal = Some(t.handle.clone());
    fleet.save(&ctx.paths)?;
    Ok(t.handle)
}

pub fn setup(
    ctx: &Ctx,
    fleet: &mut Fleet,
    root: Option<PathBuf>,
    rebaseline: bool,
    free: bool,
    user_home: &Path,
) -> Result<Outcome> {
    let p: &Paths = &ctx.paths;
    for d in [
        p.home.clone(),
        p.history(),
        p.templates(),
        p.backups(),
        p.teams(),
    ] {
        fs::create_dir_all(d)?;
    }
    let mut cfg = Config::load(p)?;
    if let Some(r) = root {
        cfg.root = Some(r);
    }
    cfg.save(p)?;
    fs::create_dir_all(cfg.codenames_dir(p))?;
    if !p.engines().exists() {
        atomic_write(
            &p.engines(),
            serde_json::to_string_pretty(&default_engines())?.as_bytes(),
        )?;
    }
    let mut notes = vec![
        format!("home: {}", p.home.display()),
        format!("codenames: {}", cfg.codenames_dir(p).display()),
    ];
    notes.extend(doctor::path_warnings(&cfg.root(p)));
    if free {
        notes.extend(free_mode_notes(
            p,
            user_home,
            std::env::var_os("PATH").as_deref(),
            std::env::var("PATHEXT").ok().as_deref(),
            cfg!(windows),
        )?);
    }
    match doctor::latest_manifest(&p.backups()) {
        Some(m) if !rebaseline => notes.push(format!("config baseline kept: {}", m.display())),
        _ => notes.push(format!(
            "config baseline written: {}",
            doctor::backup(user_home, &p.backups())?.display()
        )),
    }
    if !fleet.codenames.is_empty() || p.fleet().exists() {
        notes.push(format!("registry: {} codename(s)", fleet.codenames.len()));
    } else {
        fleet.save(p)?;
        notes.push("registry created".into());
    }
    let code = match ctx.orca.status().and_then(|st| {
        if st.ready() {
            Ok(st)
        } else {
            Err(anyhow!(
                "runtime {}",
                st.state.unwrap_or_else(|| "not ready".into())
            ))
        }
    }) {
        Ok(_) => {
            let h = ensure_captain(ctx, fleet, &cfg)?;
            notes.push(format!("captain terminal: {h}"));
            0
        }
        Err(e) => {
            notes.push(format!("Orca not reachable ({e}) — open the Orca app or `codename serve-task install`, then rerun `codename setup`"));
            1
        }
    };
    Ok(Outcome::ok(
        format!("setup:\n  {}", notes.join("\n  ")),
        json!({"notes": notes}),
    )
    .with_code(code))
}

/// PowerShell one-liner from the Free Claude Code README. Shown to the human,
/// never run by codename.
pub const FCC_INSTALL_PS: &str = r#"& ([scriptblock]::Create((irm "https://raw.githubusercontent.com/Alishahryar1/free-claude-code/main/scripts/install.ps1")))"#;

/// `setup --free`: registers claude-free / codex-free and reports what the
/// human still has to do. Free Claude Code (AGPL-3.0) is a separate tool:
/// codename never runs its installer, never writes `~/.fcc/.env`, and never
/// sets ANTHROPIC_* anywhere (user env, settings.json, engines.json) — the FCC
/// launchers set those per process themselves.
fn free_mode_notes(
    p: &Paths,
    user_home: &Path,
    path_var: Option<&std::ffi::OsStr>,
    pathext: Option<&str>,
    windows: bool,
) -> Result<Vec<String>> {
    let mut notes = vec![];
    let (added, kept) = merge_engines(p, &free_engines())?;
    if !added.is_empty() {
        notes.push(format!("free engines added: {}", added.join(", ")));
    }
    if !kept.is_empty() {
        notes.push(format!(
            "free engines kept (already in engines.json): {}",
            kept.join(", ")
        ));
    }
    let missing: Vec<&str> = doctor::FCC_LAUNCHERS
        .iter()
        .copied()
        .filter(|l| crate::orca::resolve_on_path(l, path_var, pathext, windows).is_none())
        .collect();
    if !missing.is_empty() {
        notes.push(format!(
            "Free Claude Code is not installed ({} not on PATH) — install it yourself in PowerShell: {FCC_INSTALL_PS}",
            missing.join(", ")
        ));
        notes.push(
            "when the installer asks about agents that are already installed (claude, codex), answer NO so a second copy does not shadow them on PATH".into(),
        );
        notes.push(
            "then open the FCC Admin UI, add an NVIDIA NIM key (build.nvidia.com/settings/api-keys) and click Apply".into(),
        );
    }
    match fs::read_to_string(user_home.join(".fcc").join(".env")) {
        Ok(t) => notes.extend(doctor::fcc_env_findings(&t)),
        Err(_) => notes.push("FCC not configured yet (no ~/.fcc/.env)".into()),
    }
    Ok(notes)
}

/// Makes sure the Orca runtime answers. If not: run the configured
/// `serve_task` (Windows scheduled task running `orca serve --no-pairing`),
/// else `orca open`; then poll `orca status` (30 × 2 s = 60 s).
pub fn ensure_runtime(ctx: &Ctx, cfg: &Config) -> Result<Vec<String>> {
    let st = ctx.orca.status()?;
    if st.ready() {
        return Ok(vec![format!(
            "orca runtime: {} ({})",
            st.state.as_deref().unwrap_or("ready"),
            st.host()
        )]);
    }
    let mut notes = vec![format!(
        "orca runtime not ready ({})",
        st.state.as_deref().unwrap_or("unknown")
    )];
    match cfg.serve_task.as_deref().filter(|_| cfg!(windows)) {
        Some(task) => {
            crate::service::run_task(task)?;
            notes.push(format!("started scheduled task {task}"));
        }
        None => {
            ctx.orca.open()?;
            notes.push("ran `orca open`".into());
        }
    }
    for _ in 0..30 {
        let st = ctx.orca.status()?;
        if st.ready() {
            notes.push(format!("orca runtime: ready ({})", st.host()));
            return Ok(notes);
        }
        ctx.pacing.nap();
    }
    bail!(
        "the Orca runtime did not become ready within 60 s ({}) — open the Orca app or \
         `codename serve-task install`",
        notes.join("; ")
    )
}

pub fn startup(ctx: &Ctx, fleet: &mut Fleet, engines: &Engines, cfg: &Config) -> Result<Outcome> {
    let mut notes = ensure_runtime(ctx, cfg)?;
    match memory::health(cfg) {
        Ok(_) => notes.push("arra: ok".into()),
        Err(e) => notes.push(format!("arra: {e}")),
    }
    notes.push(format!(
        "captain terminal: {}",
        ensure_captain(ctx, fleet, cfg)?
    ));
    let mut order: Vec<String> = fleet
        .codenames
        .iter()
        .filter(|o| o.awake && o.captain)
        .map(|o| o.name.clone())
        .collect();
    order.extend(
        fleet
            .codenames
            .iter()
            .filter(|o| o.awake && !o.captain)
            .map(|o| o.name.clone()),
    );
    let mut failed = 0;
    for n in order {
        match seat::wakeup(ctx, fleet, engines, &n, false) {
            Ok(o) => notes.push(o.text),
            Err(e) => {
                failed += 1;
                notes.push(format!("{n}: FAILED {e}"));
            }
        }
    }
    Ok(Outcome::ok(
        format!("startup:\n  {}", notes.join("\n  ")),
        json!({"notes": notes, "failed": failed}),
    )
    .with_code(if failed > 0 { 1 } else { 0 }))
}

pub fn shutdown(ctx: &Ctx, fleet: &mut Fleet) -> Result<Outcome> {
    let mut order: Vec<String> = fleet
        .codenames
        .iter()
        .filter(|o| !o.captain)
        .map(|o| o.name.clone())
        .collect();
    order.extend(
        fleet
            .codenames
            .iter()
            .filter(|o| o.captain)
            .map(|o| o.name.clone()),
    );
    let mut notes = vec![];
    for n in order {
        match seat::sleep(ctx, fleet, &n, true) {
            Ok(o) => notes.push(o.text),
            Err(e) => notes.push(format!("{n}: {e}")),
        }
    }
    notes.push("Orca app left running; homes, memory and registry kept".into());
    Ok(Outcome::ok(
        format!("shutdown:\n  {}", notes.join("\n  ")),
        json!({"notes": notes}),
    ))
}

pub fn list(ctx: &Ctx, fleet: &Fleet) -> Result<Outcome> {
    let (live, warn) = match seat::live(ctx) {
        Ok(l) => (Some(l), None),
        Err(e) => (
            None,
            Some(format!("Orca not reachable ({e}) — states unknown")),
        ),
    };
    let mut rows = vec![];
    let mut lines = vec![format!(
        "{:<3} {:<14} {:<8} {:<8} {:<30} HOME",
        "#", "NAME", "ENGINE", "STATUS", "ROLE"
    )];
    for o in &fleet.codenames {
        let st = live
            .as_deref()
            .map(|l| seat::status_word(o, l))
            .unwrap_or("unknown");
        let name = if o.captain {
            format!("{}*", o.name)
        } else {
            o.name.clone()
        };
        lines.push(format!(
            "{:<3} {:<14} {:<8} {:<8} {:<30} {}",
            o.number,
            name,
            o.engine,
            st,
            o.role,
            o.home.display()
        ));
        rows.push(
            json!({"number": o.number, "name": o.name, "engine": o.engine, "status": st,
                         "role": o.role, "home": o.home, "captain": o.captain}),
        );
    }
    if fleet.codenames.is_empty() {
        lines.push(
            "(no codenames yet — `codename birth --choose --engine claude --role \"...\"`)".into(),
        );
    }
    if let Some(w) = &warn {
        lines.push(w.clone());
    }
    Ok(Outcome::ok(
        lines.join("\n"),
        json!({"codenames": rows, "warning": warn}),
    ))
}

/// Who is reporting: explicit flag, then the codename whose home contains the
/// cwd, then env CODENAME_NAME.
pub fn resolve_sender(
    fleet: &Fleet,
    explicit: Option<&str>,
    cwd: &Path,
    env: Option<String>,
) -> Result<String> {
    if let Some(n) = explicit {
        return Ok(fleet.require(n)?.name.clone());
    }
    if let Some(o) = fleet.codename_for_dir(cwd) {
        return Ok(o.name.clone());
    }
    if let Some(n) = env.filter(|s| !s.is_empty()) {
        return Ok(fleet.require(&n)?.name.clone());
    }
    bail!("cannot tell who is reporting — run inside a codename home or pass --as <name>")
}

pub fn report_text(sender: &str, rc: i32, msg: &str) -> String {
    format!("REPORT {sender} rc={rc}: {msg}")
}

pub fn report(ctx: &Ctx, fleet: &Fleet, sender: &str, rc: i32, msg: &str) -> Result<Outcome> {
    let cap = fleet
        .captain()
        .ok_or_else(|| anyhow!("no captain in the registry — `codename birth ... --captain`"))?;
    let text = report_text(sender, rc, msg);
    let mut o = seat::hey(ctx, fleet, &cap.name.clone(), &text)?;
    o.text = format!("reported to {}: {text}", cap.name);
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeOrca;
    use crate::orca::TermState;
    use crate::registry::Codename;
    use crate::Pacing;
    use std::time::Duration;

    fn ctx<'a>(fake: &'a FakeOrca, d: &Path) -> Ctx<'a> {
        Ctx {
            orca: fake,
            paths: Paths::new(d.join(".codename")),
            pacing: Pacing {
                poll: Duration::ZERO,
                start_polls: 1,
                dialog_polls: 1,
                exit_polls: 1,
            },
        }
    }

    #[test]
    fn setup_creates_layout_and_captain() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let c = ctx(&fake, d.path());
        let mut f = Fleet::load(&c.paths).unwrap();
        let o = setup(&c, &mut f, None, false, false, &d.path().join("user")).unwrap();
        assert_eq!(o.code, 0, "{}", o.text);
        assert!(c.paths.engines().is_file() && c.paths.config().is_file());
        assert!(doctor::latest_manifest(&c.paths.backups()).is_some());
        let cap = Fleet::load(&c.paths).unwrap().captain_terminal.unwrap();
        // rerun is idempotent: no second captain terminal
        setup(&c, &mut f, None, false, false, &d.path().join("user")).unwrap();
        assert_eq!(f.captain_terminal.as_deref(), Some(cap.as_str()));
        assert_eq!(fake.calls_of("terminal", "create").len(), 1);
    }

    #[test]
    fn setup_free_registers_engines_and_notes() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        let c = ctx(&fake, d.path());
        let user = d.path().join("user");
        let mut f = Fleet::load(&c.paths).unwrap();
        // plain setup does not add free engines
        let o = setup(&c, &mut f, None, false, false, &user).unwrap();
        assert!(!o.text.contains("free"), "{}", o.text);
        assert!(!crate::registry::load_engines(&c.paths)
            .unwrap()
            .contains_key("claude-free"));
        let o = setup(&c, &mut f, None, false, true, &user).unwrap();
        assert!(o
            .text
            .contains("free engines added: claude-free, codex-free"));
        let e = crate::registry::load_engines(&c.paths).unwrap();
        assert_eq!(e["codex-free"].command, "fcc-codex");
        // rerun keeps them
        let o = setup(&c, &mut f, None, false, true, &user).unwrap();
        assert!(o
            .text
            .contains("free engines kept (already in engines.json): claude-free, codex-free"));
        assert!(!o.text.contains("free engines added"));
    }

    #[test]
    fn free_mode_notes_missing_fcc_and_exposure() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path().join(".codename"));
        let bin = d.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let user = d.path().join("user");
        let n = free_mode_notes(&p, &user, Some(bin.as_os_str()), None, true).unwrap();
        let t = n.join("\n");
        assert!(t.contains("fcc-claude, fcc-codex not on PATH"), "{t}");
        assert!(t.contains(FCC_INSTALL_PS) && t.contains("answer NO"));
        assert!(t.contains("build.nvidia.com/settings/api-keys"));
        assert!(t.contains("no ~/.fcc/.env"));
        // installed + exposed .env
        for l in doctor::FCC_LAUNCHERS {
            fs::write(bin.join(format!("{l}.exe")), "").unwrap();
        }
        fs::create_dir_all(user.join(".fcc")).unwrap();
        fs::write(
            user.join(".fcc/.env"),
            "HOST=0.0.0.0\nANTHROPIC_AUTH_TOKEN=freecc-secret\n",
        )
        .unwrap();
        let n = free_mode_notes(&p, &user, Some(bin.as_os_str()), None, true).unwrap();
        let t = n.join("\n");
        assert!(!t.contains("not on PATH") && !t.contains("irm "), "{t}");
        assert!(t.contains("all interfaces") && t.contains("Proxy Authentication is off"));
        assert!(!t.contains("freecc-secret"));
    }

    #[test]
    fn startup_starts_runtime_with_orca_open_then_polls_status() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        fake.runtime_ready.set(false);
        let c = ctx(&fake, d.path());
        let cfg = Config {
            arra_url: "http://127.0.0.1:9".into(),
            ..Default::default()
        };
        // runtime down: every runtime command reports runtime_unavailable
        let err = seat::live(&c).unwrap_err().to_string();
        assert!(err.contains("runtime is not running"), "{err}");
        let mut f = Fleet::default();
        let o = startup(&c, &mut f, &default_engines(), &cfg).unwrap();
        assert_eq!(o.code, 0, "{}", o.text);
        assert_eq!(fake.calls_of("open", "--json").len(), 1);
        assert!(o.text.contains("ran `orca open`") && o.text.contains("ready"));
        assert!(f.captain_terminal.is_some());
        // already ready → no second open
        startup(&c, &mut f, &default_engines(), &cfg).unwrap();
        assert_eq!(fake.calls_of("open", "--json").len(), 1);
    }

    #[test]
    fn setup_offline_still_prepares_home() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        fake.up.set(false);
        let c = ctx(&fake, d.path());
        let o = setup(&c, &mut Fleet::default(), None, false, false, d.path()).unwrap();
        assert_eq!(o.code, 1);
        assert!(c.paths.engines().is_file());
    }

    #[test]
    fn list_shows_rest_and_idle() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        fake.add_terminal("t1", "a", TermState::Working);
        let mut f = Fleet::default();
        f.codenames.push(Codename {
            name: "a".into(),
            orca: crate::registry::OrcaRef {
                terminal: Some("t1".into()),
                ..Default::default()
            },
            ..Default::default()
        });
        f.codenames.push(Codename {
            name: "b".into(),
            ..Default::default()
        });
        let o = list(&ctx(&fake, d.path()), &f).unwrap();
        assert_eq!(o.json["codenames"][0]["status"], "working");
        assert_eq!(o.json["codenames"][1]["status"], "rest");
    }

    #[test]
    fn report_goes_to_captain() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        fake.add_terminal("tc", "cap", TermState::Idle);
        let mut f = Fleet::default();
        f.codenames.push(Codename {
            name: "cap".into(),
            captain: true,
            home: "/r/cap".into(),
            orca: crate::registry::OrcaRef {
                terminal: Some("tc".into()),
                ..Default::default()
            },
            ..Default::default()
        });
        f.codenames.push(Codename {
            name: "w".into(),
            home: "/r/w".into(),
            ..Default::default()
        });
        let sender = resolve_sender(&f, None, Path::new("/r/w/sub"), None).unwrap();
        assert_eq!(sender, "w");
        assert!(resolve_sender(&f, None, Path::new("/elsewhere"), None).is_err());
        assert_eq!(
            resolve_sender(&f, None, Path::new("/elsewhere"), Some("cap".into())).unwrap(),
            "cap"
        );
        report(&ctx(&fake, d.path()), &f, &sender, 0, "done").unwrap();
        let sends = fake.calls_of("terminal", "send");
        assert!(sends[0].iter().any(|a| a == "REPORT w rc=0: done"));
        assert!(sends[0]
            .windows(2)
            .any(|w| w[0] == "--terminal" && w[1] == "tc"));
    }

    #[test]
    fn shutdown_sleeps_captain_last_and_keeps_awake_flag() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeOrca::default();
        fake.add_terminal("tc", "cap", TermState::Idle);
        fake.add_terminal("tw", "w", TermState::Idle);
        let mut f = Fleet::default();
        f.codenames.push(Codename {
            name: "cap".into(),
            captain: true,
            awake: true,
            orca: crate::registry::OrcaRef {
                terminal: Some("tc".into()),
                ..Default::default()
            },
            ..Default::default()
        });
        f.codenames.push(Codename {
            name: "w".into(),
            awake: true,
            orca: crate::registry::OrcaRef {
                terminal: Some("tw".into()),
                ..Default::default()
            },
            ..Default::default()
        });
        let c = ctx(&fake, d.path());
        shutdown(&c, &mut f).unwrap();
        let closes = fake.calls_of("terminal", "close");
        assert!(closes[0].contains(&"tw".to_string()) && closes[1].contains(&"tc".to_string()));
        assert!(f
            .codenames
            .iter()
            .all(|o| o.awake && o.orca.terminal.is_none()));
    }
}
