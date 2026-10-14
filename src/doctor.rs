//! Health checks and the agent-config backup manifest.
//!
//! Orca injects hooks into agent config files (Claude, Codex, ...). `setup`
//! takes a baseline copy + SHA256 manifest; `doctor` diffs against it.

use crate::orca::TermState;
use crate::registry::{atomic_write, stamp, Config, Fleet};
use crate::{memory, seat, Ctx, Outcome};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Agent config files (relative to the user's home) that Orca may touch.
pub const WATCHED: &[&str] = &[
    ".claude/settings.json",
    ".claude.json",
    ".codex/config.toml",
    ".codex/hooks.json",
    ".gemini/settings.json",
    ".cursor/mcp.json",
    ".cursor/hooks.json",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestEntry {
    /// Relative to the user's home, forward slashes.
    pub path: String,
    /// None = file was absent when the baseline was taken.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub created: String,
    pub files: Vec<ManifestEntry>,
}

pub fn sha256_file(p: &Path) -> Result<String> {
    let bytes = fs::read(p).with_context(|| format!("read {}", p.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Copies every watched file into `<backups>/<stamp>/files/` and writes
/// `manifest.json`. Missing files are recorded as absent.
pub fn backup(user_home: &Path, backups: &Path) -> Result<PathBuf> {
    let dir = backups.join(format!("pre-orca-{}", stamp()));
    let files_dir = dir.join("files");
    fs::create_dir_all(&files_dir)?;
    let mut m = Manifest {
        created: chrono::Local::now().to_rfc3339(),
        files: vec![],
    };
    for rel in WATCHED {
        let src = user_home.join(rel);
        let sha = if src.is_file() {
            let dst = files_dir.join(rel);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p)?;
            }
            fs::copy(&src, &dst)?;
            Some(sha256_file(&src)?)
        } else {
            None
        };
        m.files.push(ManifestEntry {
            path: rel.to_string(),
            sha256: sha,
        });
    }
    let mp = dir.join("manifest.json");
    atomic_write(&mp, serde_json::to_string_pretty(&m)?.as_bytes())?;
    Ok(mp)
}

pub fn latest_manifest(backups: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(backups)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("manifest.json"))
        .filter(|p| p.is_file())
        .collect();
    dirs.sort();
    dirs.pop()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    Same,
    Modified,
    Added,
    Removed,
}

pub fn diff(user_home: &Path, m: &Manifest) -> Vec<(String, Change)> {
    m.files
        .iter()
        .map(|e| {
            let p = user_home.join(&e.path);
            let now = if p.is_file() {
                sha256_file(&p).ok()
            } else {
                None
            };
            let c = match (&e.sha256, &now) {
                (None, None) => Change::Same,
                (None, Some(_)) => Change::Added,
                (Some(_), None) => Change::Removed,
                (Some(a), Some(b)) if a == b => Change::Same,
                _ => Change::Modified,
            };
            (e.path.clone(), c)
        })
        .collect()
}

/// Warnings for a fleet path: cloud-synced folders (OneDrive) are a real
/// risk (sync locks/rewrites `.git` and agent state); non-ASCII characters
/// and spaces are informational. `env` looks up environment variables.
pub fn path_warnings_with(path: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let norm = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let s = path.to_string_lossy();
    let n = norm(&s);
    let mut out = vec![];
    let under_env = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(|k| env(k))
        .map(|v| norm(&v))
        .filter(|v| !v.is_empty())
        .any(|v| n == v || n.starts_with(&format!("{v}/")));
    if under_env || n.contains("/onedrive") {
        out.push(format!(
            "WARNING: {s} is inside OneDrive (cloud sync) — sync can lock or rewrite .git \
             files and agent state. Use a local folder, e.g. `codename setup --root C:\\fleet`"
        ));
    }
    if !s.is_ascii() {
        out.push(format!(
            "info: {s} contains non-ASCII characters — some Windows tools mishandle them"
        ));
    }
    if s.contains(' ') {
        out.push(format!(
            "info: {s} contains spaces — quote it when typing commands"
        ));
    }
    out
}

pub fn path_warnings(path: &Path) -> Vec<String> {
    path_warnings_with(path, &|k| std::env::var(k).ok())
}

/// FCC (Free Claude Code) launchers that free-mode engines need on PATH.
pub const FCC_LAUNCHERS: &[&str] = &["fcc-claude", "fcc-codex"];

/// Exposure findings for Free Claude Code's `~/.fcc/.env` (KEY=VALUE lines).
/// Only HOST and PROXY_AUTH_ENABLED values are ever echoed — never a token or
/// API key (logs keep 200 chars of output).
pub fn fcc_env_findings(env_text: &str) -> Vec<String> {
    let get = |key: &str| {
        env_text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.strip_prefix("export ").unwrap_or(l).split_once('='))
            .filter(|(k, _)| k.trim() == key)
            .map(|(_, v)| {
                let v = v.trim();
                v.strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                    .unwrap_or(v)
                    .trim()
                    .to_string()
            })
            .next_back()
    };
    let mut out = vec![];
    let host = get("HOST");
    if host.as_deref().map_or(true, |h| {
        h.is_empty() || ["0.0.0.0", "::", "[::]"].contains(&h)
    }) {
        out.push(format!(
            "FCC proxy listens on all interfaces (HOST={}); set Host to 127.0.0.1 in the FCC \
             Admin UI so tailnet/LAN peers can't spend your provider keys",
            host.filter(|h| !h.is_empty())
                .unwrap_or_else(|| "unset, default 0.0.0.0".into())
        ));
    }
    let auth = get("PROXY_AUTH_ENABLED").unwrap_or_default().to_lowercase();
    if auth != "true" && auth != "1" {
        out.push("FCC Proxy Authentication is off; enable it in the FCC Admin UI".into());
    }
    out
}

/// True when the fleet uses free mode: an engine launched through `fcc-*` or
/// a codename on a `*-free` engine.
pub fn uses_free_mode(engines: &crate::registry::Engines, fleet: &Fleet) -> bool {
    engines.values().any(|e| e.command.starts_with("fcc-"))
        || fleet.codenames.iter().any(|o| o.engine.ends_with("-free"))
}

/// The "free mode" doctor check: FCC launchers on PATH and `~/.fcc/.env`
/// exposure. Pure apart from reading the file and probing PATH.
pub fn free_mode_check(
    user_home: &Path,
    path_var: Option<&std::ffi::OsStr>,
    pathext: Option<&str>,
    windows: bool,
) -> Check {
    let mut problems: Vec<String> = FCC_LAUNCHERS
        .iter()
        .filter(|l| crate::orca::resolve_on_path(l, path_var, pathext, windows).is_none())
        .map(|l| format!("{l} not on PATH (install Free Claude Code)"))
        .collect();
    match fs::read_to_string(user_home.join(".fcc").join(".env")) {
        Ok(t) => problems.extend(fcc_env_findings(&t)),
        Err(_) => problems.push("FCC not configured yet (no ~/.fcc/.env)".into()),
    }
    if problems.is_empty() {
        check(
            "free mode",
            Level::Pass,
            "fcc-claude/fcc-codex found; FCC not on all interfaces, proxy auth on",
        )
    } else {
        check("free mode", Level::Warn, problems.join("; "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub level: Level,
    pub detail: String,
}

fn check(name: &str, level: Level, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        level,
        detail: detail.into(),
    }
}

pub fn run(
    ctx: &Ctx,
    fleet: &Fleet,
    cfg: &Config,
    user_home: &Path,
    identity: bool,
) -> Result<Outcome> {
    let mut checks = vec![];

    match crate::orca::resolve_orca(cfg.orca_bin.as_deref()) {
        Ok(b) => match crate::orca::guard_spawn(&b) {
            Ok(()) => checks.push(check(
                "orca binary",
                Level::Pass,
                format!(
                    "{} ({})",
                    b.path.display(),
                    serde_json::to_value(b.kind)?.as_str().unwrap_or("?")
                ),
            )),
            Err(e) => checks.push(check("orca binary", Level::Fail, e.to_string())),
        },
        Err(e) => checks.push(check("orca binary", Level::Fail, e.to_string())),
    }

    let live = match ctx.orca.status() {
        Ok(st) if st.ready() => {
            checks.push(check(
                "orca runtime",
                Level::Pass,
                format!(
                    "{} via {}",
                    st.state.as_deref().unwrap_or("ready"),
                    st.host()
                ),
            ));
            match seat::live(ctx) {
                Ok(t) => Some(t),
                Err(e) => {
                    checks.push(check("orca terminals", Level::Fail, e.to_string()));
                    None
                }
            }
        }
        Ok(st) => {
            checks.push(check(
                "orca runtime",
                Level::Fail,
                format!(
                    "not ready (state {}, app running {}) — `codename startup`, open the Orca app, \
                     or `codename serve-task install`",
                    st.state.as_deref().unwrap_or("?"),
                    st.app_running.map(|b| b.to_string()).unwrap_or_else(|| "?".into())
                ),
            ));
            None
        }
        Err(e) => {
            checks.push(check("orca runtime", Level::Fail, e.to_string()));
            None
        }
    };
    if let Some(task) = &cfg.serve_task {
        let lvl_detail = if cfg!(windows) {
            match crate::service::execute(&crate::service::query_invocation(
                &crate::service::system_root(),
                task,
            )) {
                Ok((true, _)) => (Level::Pass, format!("{task} registered")),
                Ok((false, out)) => (Level::Warn, format!("{task} not found: {out}")),
                Err(e) => (Level::Warn, e.to_string()),
            }
        } else {
            (Level::Pass, format!("{task} (configured; Windows only)"))
        };
        checks.push(check("serve task", lvl_detail.0, lvl_detail.1));
    }

    match std::env::var("ORCA_TELEMETRY_DISABLED") {
        Ok(v) if v == "1" => checks.push(check(
            "telemetry off",
            Level::Pass,
            "ORCA_TELEMETRY_DISABLED=1",
        )),
        _ => checks.push(check(
            "telemetry off",
            Level::Fail,
            "set user env ORCA_TELEMETRY_DISABLED=1 (and turn it off in Orca Settings → Privacy)",
        )),
    }

    match memory::health(cfg) {
        Ok(_) => checks.push(check("arra memory", Level::Pass, cfg.arra_url.clone())),
        Err(e) => checks.push(check("arra memory", Level::Warn, e.to_string())),
    }

    if let Some(live) = &live {
        let mut problems = vec![];
        for o in &fleet.codenames {
            if let Some(h) = &o.orca.terminal {
                if !live.iter().any(|t| &t.handle == h) {
                    problems.push(format!("{} lists terminal {h} but it is not live", o.name));
                }
            }
        }
        for t in live {
            if let Some(title) = &t.title {
                let ours = fleet
                    .codenames
                    .iter()
                    .any(|o| o.orca.terminal.as_deref() == Some(&t.handle));
                if !ours && fleet.get(title).is_some() {
                    problems.push(format!(
                        "terminal {} titled `{title}` is not the registered seat (started twice?)",
                        t.handle
                    ));
                }
            }
        }
        match &fleet.captain_terminal {
            Some(h) if !live.iter().any(|t| &t.handle == h) => problems.push(
                "captain terminal is gone — run `codename setup` or `codename startup`".into(),
            ),
            None => problems.push("no captain terminal — run `codename setup`".into()),
            _ => {}
        }
        if problems.is_empty() {
            checks.push(check(
                "registry vs terminals",
                Level::Pass,
                format!("{} codename(s)", fleet.codenames.len()),
            ));
        } else {
            checks.push(check(
                "registry vs terminals",
                Level::Warn,
                problems.join("; "),
            ));
        }

        if identity {
            for o in &fleet.codenames {
                let alive = live.iter().any(|t| {
                    Some(&t.handle) == o.orca.terminal.as_ref() && t.state != TermState::Blocked
                });
                if !alive {
                    continue;
                }
                let r = seat::ask(ctx, fleet, &o.name, "ตอบสั้นๆ: หนูชื่ออะไร เผ่าอะไรคะ", 120);
                let (lvl, d) = match r {
                    Ok(r)
                        if r.text.contains(&o.name)
                            && r.text.to_lowercase().contains("codename") =>
                    {
                        (Level::Pass, "ok".to_string())
                    }
                    Ok(r) => (
                        Level::Fail,
                        format!("answered: {}", r.text.lines().next().unwrap_or("")),
                    ),
                    Err(e) => (Level::Warn, e.to_string()),
                };
                checks.push(check(&format!("identity {}", o.name), lvl, d));
            }
        }
    }

    match latest_manifest(&ctx.paths.backups()) {
        None => checks.push(check(
            "agent configs",
            Level::Warn,
            "no baseline — run `codename setup`",
        )),
        Some(mp) => {
            let m: Manifest = serde_json::from_str(&fs::read_to_string(&mp)?)?;
            let changed: Vec<String> = diff(user_home, &m)
                .into_iter()
                .filter(|(_, c)| *c != Change::Same)
                .map(|(p, c)| format!("{p} {c:?}"))
                .collect();
            if changed.is_empty() {
                checks.push(check(
                    "agent configs",
                    Level::Pass,
                    "unchanged since baseline",
                ));
            } else {
                checks.push(check(
                    "agent configs",
                    Level::Warn,
                    format!(
                        "changed since baseline ({}) — Orca may have injected hooks: {}",
                        mp.display(),
                        changed.join(", ")
                    ),
                ));
            }
        }
    }

    match crate::registry::load_engines(&ctx.paths) {
        Ok(engines) if uses_free_mode(&engines, fleet) => checks.push(free_mode_check(
            user_home,
            std::env::var_os("PATH").as_deref(),
            std::env::var("PATHEXT").ok().as_deref(),
            cfg!(windows),
        )),
        Ok(_) => {}
        Err(e) => checks.push(check("engines", Level::Warn, e.to_string())),
    }

    let mut pw: Vec<String> = path_warnings(&cfg.root(&ctx.paths));
    for o in &fleet.codenames {
        pw.extend(path_warnings(&o.home));
    }
    if pw.iter().any(|w| w.starts_with("WARNING")) {
        checks.push(check("fleet paths", Level::Warn, pw.join("; ")));
    } else if pw.is_empty() {
        checks.push(check("fleet paths", Level::Pass, "local, ASCII, no spaces"));
    } else {
        checks.push(check("fleet paths", Level::Pass, pw.join("; ")));
    }

    let fail = checks.iter().any(|c| c.level == Level::Fail);
    let text = checks
        .iter()
        .map(|c| {
            let tag = match c.level {
                Level::Pass => "PASS",
                Level::Warn => "WARN",
                Level::Fail => "FAIL",
            };
            format!("[{tag}] {:<22} {}", c.name, c.detail)
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(
        Outcome::ok(text, json!({"checks": checks, "ok": !fail})).with_code(if fail {
            1
        } else {
            0
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onedrive_and_path_warnings() {
        let env = |k: &str| match k {
            "OneDrive" => Some("C:\\Users\\x\\OneDrive".to_string()),
            "OneDriveCommercial" => Some("D:\\Work Sync".to_string()),
            _ => None,
        };
        let w = path_warnings_with(Path::new("C:\\Users\\x\\OneDrive\\fleet"), &env);
        assert!(w[0].starts_with("WARNING") && w[0].contains("OneDrive"));
        let w = path_warnings_with(Path::new("d:/work sync/fleet"), &env);
        assert!(w[0].starts_with("WARNING"), "{w:?}");
        assert!(w.iter().any(|x| x.contains("spaces")));
        // folder named like OneDrive even without env (e.g. "OneDrive - Company")
        let none = |_: &str| None;
        let w = path_warnings_with(Path::new("C:\\Users\\x\\OneDrive - Contoso\\f"), &none);
        assert!(w[0].starts_with("WARNING"));
        // a sibling sharing the prefix of a sync root is not inside it
        let w = path_warnings_with(Path::new("D:\\Work Syncer\\fleet"), &env);
        assert!(w.iter().all(|x| !x.starts_with("WARNING")), "{w:?}");
        let w = path_warnings_with(Path::new("C:\\fleet\\codenames\\อรุณา"), &none);
        assert_eq!(w.len(), 1);
        assert!(w[0].starts_with("info") && w[0].contains("non-ASCII"));
        assert!(path_warnings_with(Path::new("C:\\fleet\\codenames\\lyra"), &env).is_empty());
    }

    #[test]
    fn backup_and_diff() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("user");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(home.join(".claude/settings.json"), "{}").unwrap();
        fs::write(home.join(".claude.json"), "{}").unwrap();
        let backups = d.path().join("backups");
        let mp = backup(&home, &backups).unwrap();
        assert_eq!(latest_manifest(&backups).unwrap(), mp);
        let m: Manifest = serde_json::from_str(&fs::read_to_string(&mp).unwrap()).unwrap();
        assert!(m
            .files
            .iter()
            .any(|f| f.path == ".codex/config.toml" && f.sha256.is_none()));
        assert!(mp
            .parent()
            .unwrap()
            .join("files/.claude/settings.json")
            .is_file());
        assert!(diff(&home, &m).iter().all(|(_, c)| *c == Change::Same));

        fs::write(home.join(".claude/settings.json"), r#"{"hooks":{}}"#).unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(home.join(".codex/hooks.json"), "{}").unwrap();
        fs::remove_file(home.join(".claude.json")).unwrap();
        let dd = diff(&home, &m);
        let get = |p: &str| dd.iter().find(|x| x.0 == p).unwrap().1;
        assert_eq!(get(".claude/settings.json"), Change::Modified);
        assert_eq!(get(".codex/hooks.json"), Change::Added);
        assert_eq!(get(".claude.json"), Change::Removed);
    }

    #[test]
    fn fcc_env_findings_cases() {
        // defaults: all interfaces, auth off
        let f = fcc_env_findings("");
        assert_eq!(f.len(), 2, "{f:?}");
        assert!(f[0].contains("HOST=unset") && f[0].contains("127.0.0.1"));
        assert!(f[1].contains("Proxy Authentication is off"));
        for h in ["0.0.0.0", "::", "[::]", "\"0.0.0.0\""] {
            let f = fcc_env_findings(&format!("HOST={h}\nPROXY_AUTH_ENABLED=true"));
            assert_eq!(f.len(), 1, "{h}: {f:?}");
            assert!(f[0].contains("all interfaces"));
        }
        // safe config: quotes, comments, blanks, spacing all tolerated
        let ok = "# comment\n\n HOST = '127.0.0.1' \nPROXY_AUTH_ENABLED=\"1\"\nPORT=8082\n";
        assert!(
            fcc_env_findings(ok).is_empty(),
            "{:?}",
            fcc_env_findings(ok)
        );
        assert!(fcc_env_findings("HOST=127.0.0.1\nPROXY_AUTH_ENABLED=TRUE").is_empty());
        // commented-out values do not count
        assert_eq!(
            fcc_env_findings("#HOST=127.0.0.1\nPROXY_AUTH_ENABLED=false").len(),
            2
        );
    }

    #[test]
    fn fcc_findings_never_echo_secrets() {
        let secrets = [
            "freecc-SECRET-tok-123",
            "nvapi-SECRET-key-456",
            "or-SECRET-789",
        ];
        let env = format!(
            "ANTHROPIC_AUTH_TOKEN={}\nNVIDIA_NIM_API_KEY=\"{}\"\nOPENROUTER_TOKEN={}\nHOST=0.0.0.0\n",
            secrets[0], secrets[1], secrets[2]
        );
        let f = fcc_env_findings(&env);
        assert_eq!(f.len(), 2);
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".fcc")).unwrap();
        fs::write(home.path().join(".fcc/.env"), &env).unwrap();
        let c = free_mode_check(home.path(), None, None, false);
        let json = serde_json::to_string(&c).unwrap();
        for s in secrets {
            assert!(!f.iter().any(|x| x.contains(s)), "{s} leaked in findings");
            assert!(
                !c.detail.contains(s) && !json.contains(s),
                "{s} leaked in check"
            );
        }
    }

    #[test]
    fn free_mode_check_levels() {
        let home = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        // nothing installed, no .env
        let c = free_mode_check(home.path(), Some(bin.path().as_os_str()), None, true);
        assert_eq!(c.level, Level::Warn);
        assert!(c.detail.contains("fcc-claude not on PATH") && c.detail.contains("fcc-codex"));
        assert!(c.detail.contains("FCC not configured yet (no ~/.fcc/.env)"));
        // launchers present (Windows PATHEXT lookup) + safe .env → pass
        for l in FCC_LAUNCHERS {
            fs::write(bin.path().join(format!("{l}.exe")), "").unwrap();
        }
        fs::create_dir_all(home.path().join(".fcc")).unwrap();
        fs::write(
            home.path().join(".fcc/.env"),
            "HOST=127.0.0.1\nPROXY_AUTH_ENABLED=true\n",
        )
        .unwrap();
        let c = free_mode_check(home.path(), Some(bin.path().as_os_str()), None, true);
        assert_eq!(c.level, Level::Pass, "{}", c.detail);
        // exposed proxy → warn
        fs::write(home.path().join(".fcc/.env"), "HOST=0.0.0.0\n").unwrap();
        let c = free_mode_check(home.path(), Some(bin.path().as_os_str()), None, true);
        assert_eq!(c.level, Level::Warn);
        assert!(!c.detail.contains("not on PATH"));
    }

    #[test]
    fn free_mode_detection() {
        use crate::registry::{default_engines, free_engines, Codename};
        let mut f = Fleet::default();
        assert!(!uses_free_mode(&default_engines(), &f));
        assert!(uses_free_mode(&free_engines(), &f));
        f.codenames.push(Codename {
            name: "x".into(),
            engine: "claude-free".into(),
            ..Default::default()
        });
        assert!(uses_free_mode(&default_engines(), &f));
    }
}
