//! Fleet registry (`fleet.json`), config, engines and name rules.
//!
//! All state lives under the codename home: `%USERPROFILE%\.codename`
//! (or `$HOME/.codename`), overridable with env `CODENAME_HOME`.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SPECIES: &str = "codename";
/// Env var naming the sender for `report`/`remember` outside a home.
pub const SENDER_ENV: &str = "CODENAME_NAME";

/// The sender named by env `CODENAME_NAME`.
pub fn sender_from_env() -> Option<String> {
    std::env::var(SENDER_ENV).ok().filter(|s| !s.is_empty())
}

pub const CAPTAIN_TITLE: &str = "codename-captain";

/// Resolved locations. Built once in `main` and passed down explicitly
/// (tests build their own with a temp dir — no global env mutation).
#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
}

impl Paths {
    pub fn from_env() -> Result<Self> {
        if let Some(h) = std::env::var_os("CODENAME_HOME").filter(|s| !s.is_empty()) {
            return Ok(Self {
                home: PathBuf::from(h),
            });
        }
        let user = user_home().ok_or_else(|| anyhow!("cannot find USERPROFILE or HOME"))?;
        Ok(Self {
            home: user.join(".codename"),
        })
    }
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }
    pub fn fleet(&self) -> PathBuf {
        self.home.join("fleet.json")
    }
    pub fn history(&self) -> PathBuf {
        self.home.join("history")
    }
    pub fn config(&self) -> PathBuf {
        self.home.join("config.json")
    }
    pub fn engines(&self) -> PathBuf {
        self.home.join("engines.json")
    }
    pub fn templates(&self) -> PathBuf {
        self.home.join("templates")
    }
    pub fn backups(&self) -> PathBuf {
        self.home.join("backups")
    }
    pub fn teams(&self) -> PathBuf {
        self.home.join("teams")
    }
    pub fn registry_log(&self) -> PathBuf {
        self.home.join("REGISTRY-local.md")
    }
    pub fn logs(&self) -> PathBuf {
        self.home.join("logs")
    }
    pub fn dialogs_log(&self) -> PathBuf {
        self.home.join("first-run-dialogs.md")
    }
}

/// The user's home directory (USERPROFILE on Windows, HOME elsewhere).
pub fn user_home() -> Option<PathBuf> {
    ["USERPROFILE", "HOME"]
        .iter()
        .filter_map(std::env::var_os)
        .find(|v| !v.is_empty())
        .map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// atomic file IO
// ---------------------------------------------------------------------------

/// Writes via a temp file in the same directory, then renames over the target.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("no parent dir for {}", path.display()))?;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("rename to {}", path.display()))?;
    Ok(())
}

pub fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S-%6f").to_string()
}

// ---------------------------------------------------------------------------
// config + engines
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Where codename homes are born: `<root>/codenames/<name>`.
    pub root: Option<PathBuf>,
    /// arra memory server base URL.
    pub arra_url: String,
    /// Path (relative to arra_url) for storing a lesson. Route seen (`/api/learn`
    /// redirects here); the JSON body shape is UNVERIFIED.
    pub arra_learn_path: String,
    /// Path for searching (route + `results[].content` observed on one arra).
    pub arra_search_path: String,
    /// Path for health. From the install notes (`/api/health`).
    pub arra_health_path: String,
    /// MCP endpoint written into a newborn's project config. UNVERIFIED.
    pub arra_mcp_url: String,
    /// Windows scheduled task that runs `orca serve --no-pairing --json`
    /// (set by `codename serve-task install`); `startup` runs it when the
    /// Orca runtime is down.
    pub serve_task: Option<String>,
    /// Optional `orca terminal create --shell` (Windows: cmd.exe,
    /// powershell.exe, pwsh.exe, wsl.exe, bash.exe, git-bash).
    pub terminal_shell: Option<String>,
    /// Full path to the Orca CLI executable (env CODENAME_ORCA_BIN wins).
    pub orca_bin: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root: None,
            arra_url: "http://127.0.0.1:47778".into(),
            arra_learn_path: "/api/v1/learn".into(),
            arra_search_path: "/api/v1/search".into(),
            arra_health_path: "/api/health".into(),
            arra_mcp_url: "http://127.0.0.1:47778/mcp".into(),
            serve_task: None,
            terminal_shell: None,
            orca_bin: None,
        }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let p = paths.config();
        if !p.exists() {
            return Ok(Self::default());
        }
        let s = fs::read_to_string(&p)?;
        serde_json::from_str(&s).with_context(|| format!("{} is not valid JSON", p.display()))
    }
    pub fn save(&self, paths: &Paths) -> Result<()> {
        atomic_write(
            &paths.config(),
            serde_json::to_string_pretty(self)?.as_bytes(),
        )
    }
    pub fn root(&self, paths: &Paths) -> PathBuf {
        self.root.clone().unwrap_or_else(|| paths.home.clone())
    }
    pub fn codenames_dir(&self, paths: &Paths) -> PathBuf {
        self.root(paths).join("codenames")
    }
}

/// How to launch an engine. `resume_command` (optional) continues the last
/// conversation in the home and is used by `wakeup` once a codename has been
/// woken before (unless `--fresh`).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EngineSpec {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
    /// Orca agent id for `orchestration worker-start --agent` (team workers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orca_agent: Option<String>,
}

/// engines.json accepts `"claude": "claude"` or
/// `"claude": {"command": "claude", "resume_command": "claude --continue"}`.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawEngine {
    Plain(String),
    Full {
        command: String,
        #[serde(default)]
        resume_command: Option<String>,
        #[serde(default)]
        orca_agent: Option<String>,
    },
}

pub type Engines = BTreeMap<String, EngineSpec>;

/// Default engines. `glm` is absent on purpose (add it in engines.json if a
/// launcher exists on this machine). The codex resume command is UNVERIFIED.
pub fn default_engines() -> Engines {
    let resume = |e: &str| match e {
        "claude" => Some("claude --continue".to_string()),
        "codex" => Some("codex resume --last".to_string()),
        _ => None,
    };
    ["claude", "codex", "grok", "hermes"]
        .iter()
        .map(|e| {
            (
                e.to_string(),
                EngineSpec {
                    command: e.to_string(),
                    resume_command: resume(e),
                    orca_agent: matches!(*e, "claude" | "codex").then(|| e.to_string()),
                },
            )
        })
        .collect()
}

/// Free-model engines run through Free Claude Code (FCC, a separate AGPL tool
/// the human installs; codename only names its launchers, never bundles it).
/// Opt-in via `codename setup --free`; not part of `default_engines()`.
pub fn free_engines() -> Engines {
    [
        ("claude-free", "fcc-claude", "fcc-claude --continue"),
        ("codex-free", "fcc-codex", "fcc-codex resume --last"),
    ]
    .iter()
    .map(|(k, c, r)| {
        (
            k.to_string(),
            EngineSpec {
                command: c.to_string(),
                resume_command: Some(r.to_string()),
                orca_agent: None,
            },
        )
    })
    .collect()
}

/// Adds the engines in `add` whose keys are missing from engines.json (the
/// defaults when the file is absent). Existing entries are never touched —
/// they are copied as written, even if their content differs. Writes only
/// when something was added, snapshotting the old file to
/// `history/engines-<stamp>.json` first. Returns (added, kept) keys of `add`.
pub fn merge_engines(paths: &Paths, add: &Engines) -> Result<(Vec<String>, Vec<String>)> {
    let p = paths.engines();
    let existing = p.exists();
    let mut map: serde_json::Map<String, serde_json::Value> = if existing {
        let s = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        // validity gate: a corrupt file is an error and is never rewritten
        parse_engines(&s).with_context(|| format!("{} is not valid engines JSON", p.display()))?;
        serde_json::from_str(&s)?
    } else {
        match serde_json::to_value(default_engines())? {
            serde_json::Value::Object(m) => m,
            _ => unreachable!("engines serialize to an object"),
        }
    };
    let (mut added, mut kept) = (vec![], vec![]);
    for (k, spec) in add {
        if map.contains_key(k) {
            kept.push(k.clone());
        } else {
            map.insert(k.clone(), serde_json::to_value(spec)?);
            added.push(k.clone());
        }
    }
    if !added.is_empty() {
        if existing {
            fs::create_dir_all(paths.history())?;
            let snap = paths.history().join(format!("engines-{}.json", stamp()));
            fs::copy(&p, &snap).with_context(|| format!("snapshot to {}", snap.display()))?;
        }
        atomic_write(&p, serde_json::to_string_pretty(&map)?.as_bytes())?;
    }
    Ok((added, kept))
}

pub fn parse_engines(json: &str) -> Result<Engines> {
    let raw: BTreeMap<String, RawEngine> = serde_json::from_str(json)?;
    let defaults = default_engines();
    Ok(raw
        .into_iter()
        .map(|(k, v)| {
            let spec = match v {
                // a plain string keeps the default resume command when it
                // still launches the default binary
                RawEngine::Plain(command) => {
                    let d = defaults.get(&k).filter(|d| d.command == command);
                    EngineSpec {
                        resume_command: d.and_then(|d| d.resume_command.clone()),
                        orca_agent: d.and_then(|d| d.orca_agent.clone()),
                        command,
                    }
                }
                RawEngine::Full {
                    command,
                    resume_command,
                    orca_agent,
                } => EngineSpec {
                    command,
                    resume_command: resume_command.filter(|r| !r.trim().is_empty()),
                    orca_agent: orca_agent.filter(|r| !r.trim().is_empty()),
                },
            };
            (k, spec)
        })
        .collect())
}

pub fn load_engines(paths: &Paths) -> Result<Engines> {
    let p = paths.engines();
    if !p.exists() {
        return Ok(default_engines());
    }
    let s = fs::read_to_string(&p)?;
    parse_engines(&s).with_context(|| format!("{} is not valid engines JSON", p.display()))
}

pub fn engine_command(engines: &Engines, engine: &str) -> Result<String> {
    engines
        .get(engine)
        .map(|e| e.command.clone())
        .ok_or_else(|| anyhow!("engine `{engine}` has no launch command — add it to engines.json"))
}

/// Orca agent id for team workers: `orca_agent` from engines.json, else the
/// engine name when Orca knows it (claude, codex, cursor, antigravity, ...).
pub fn orca_agent(engines: &Engines, engine: &str) -> Result<String> {
    let spec = engines
        .get(engine)
        .ok_or_else(|| anyhow!("engine `{engine}` is not in engines.json"))?;
    if let Some(a) = &spec.orca_agent {
        return Ok(a.clone());
    }
    if crate::orca::flags::KNOWN_AGENTS.contains(&engine) {
        return Ok(engine.to_string());
    }
    bail!(
        "engine `{engine}` has no Orca agent id for team workers — set \"orca_agent\" in \
         engines.json (Orca accepts e.g. {})",
        crate::orca::flags::KNOWN_AGENTS.join(", ")
    )
}

/// Chooses the launch command: resume when the codename was woken before, the
/// engine has a resume command, and `fresh` is not set. Returns (cmd, resumed).
pub fn launch_command(
    engines: &Engines,
    engine: &str,
    woken_before: bool,
    fresh: bool,
) -> Result<(String, bool)> {
    let spec = engines.get(engine).ok_or_else(|| {
        anyhow!("engine `{engine}` has no launch command — add it to engines.json")
    })?;
    match (&spec.resume_command, woken_before && !fresh) {
        (Some(r), true) => Ok((r.clone(), true)),
        _ => Ok((spec.command.clone(), false)),
    }
}

// ---------------------------------------------------------------------------
// fleet
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct OrcaRef {
    pub repo_id: Option<String>,
    pub worktree: Option<String>,
    pub terminal: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Codename {
    pub name: String,
    pub number: u32,
    pub species: String,
    pub engine: String,
    pub home: PathBuf,
    pub born: String,
    pub role: String,
    pub orca: OrcaRef,
    pub captain: bool,
    /// Should `startup` wake this codename? Set by wakeup, cleared by sleep.
    pub awake: bool,
    /// When the seat was last opened (resume is used after the first time).
    pub last_woken: Option<String>,
}

impl Default for Codename {
    fn default() -> Self {
        Self {
            name: String::new(),
            number: 0,
            species: SPECIES.into(),
            engine: "claude".into(),
            home: PathBuf::new(),
            born: String::new(),
            role: String::new(),
            orca: OrcaRef::default(),
            captain: false,
            awake: false,
            last_woken: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Fleet {
    pub version: u32,
    /// Handle of the captain's identity terminal (used as `--from`).
    pub captain_terminal: Option<String>,
    /// Orca repo id of the captain's home repo.
    pub captain_repo_id: Option<String>,
    /// Orca repo id of the `.nursery` repo used by `birth --choose`.
    pub nursery_repo_id: Option<String>,
    pub codenames: Vec<Codename>,
}

impl Fleet {
    /// Loads fleet.json. A missing file is an empty fleet; a corrupt file is a
    /// hard error and is never overwritten.
    pub fn load(paths: &Paths) -> Result<Self> {
        let p = paths.fleet();
        if !p.exists() {
            return Ok(Self {
                version: 1,
                ..Default::default()
            });
        }
        let s = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        serde_json::from_str(&s).map_err(|e| {
            anyhow!(
                "{} is corrupt ({e}). It was left untouched. Restore a copy from {} \
                 or fix it by hand.",
                p.display(),
                paths.history().display()
            )
        })
    }

    /// Snapshots the current file into history/, then writes atomically.
    pub fn save(&self, paths: &Paths) -> Result<()> {
        let p = paths.fleet();
        if p.exists() {
            fs::create_dir_all(paths.history())?;
            let snap = paths.history().join(format!("fleet-{}.json", stamp()));
            fs::copy(&p, &snap).with_context(|| format!("snapshot to {}", snap.display()))?;
        }
        let mut out = self.clone();
        out.version = 1;
        atomic_write(&p, serde_json::to_string_pretty(&out)?.as_bytes())
    }

    pub fn get(&self, name: &str) -> Option<&Codename> {
        self.codenames
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name) || o.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Codename> {
        self.codenames
            .iter_mut()
            .find(|o| o.name.eq_ignore_ascii_case(name) || o.name == name)
    }

    pub fn require(&self, name: &str) -> Result<&Codename> {
        self.get(name)
            .ok_or_else(|| anyhow!("no codename named `{name}` in the fleet"))
    }

    pub fn captain(&self) -> Option<&Codename> {
        self.codenames.iter().find(|o| o.captain)
    }

    pub fn next_number(&self) -> u32 {
        self.codenames.iter().map(|o| o.number).max().unwrap_or(0) + 1
    }

    /// Exactly one captain: setting one clears the others.
    pub fn set_captain(&mut self, name: &str) -> Result<()> {
        if self.get(name).is_none() {
            bail!("no codename named `{name}`");
        }
        for o in &mut self.codenames {
            o.captain = o.name.eq_ignore_ascii_case(name) || o.name == name;
        }
        Ok(())
    }

    /// Finds the codename whose home contains `dir` (used to infer the sender).
    pub fn codename_for_dir(&self, dir: &Path) -> Option<&Codename> {
        let dir = norm(dir);
        self.codenames
            .iter()
            .filter(|o| !o.home.as_os_str().is_empty())
            .find(|o| {
                let h = norm(&o.home);
                dir == h || dir.starts_with(&format!("{h}/"))
            })
    }
}

fn norm(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/').to_string();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// name rules
// ---------------------------------------------------------------------------

const WINDOWS_DEVICE_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Validates the name of a new codename. `reserved` is every command name and alias
/// (built from the clap definition so it cannot drift).
pub fn validate_name(name: &str, reserved: &[String], fleet: &Fleet) -> Result<()> {
    let n = name.trim();
    if n.is_empty() {
        bail!("name is empty");
    }
    if n.chars().count() > 32 {
        bail!("name `{n}` is longer than 32 characters");
    }
    if n != name {
        bail!("name `{name}` has leading/trailing spaces");
    }
    let first = n.chars().next().unwrap_or('-');
    if !first.is_alphabetic() {
        bail!("name `{n}` must start with a letter");
    }
    if let Some(c) = n
        .chars()
        .find(|c| !(c.is_alphanumeric() || *c == '-' || *c == '_'))
    {
        bail!("name `{n}` contains `{c}` — use letters, digits, `-` or `_` only");
    }
    let lower = n.to_lowercase();
    if WINDOWS_DEVICE_NAMES.contains(&lower.as_str()) {
        bail!("name `{n}` is a reserved Windows device name");
    }
    if lower == CAPTAIN_TITLE || reserved.iter().any(|r| r.to_lowercase() == lower) {
        bail!("name `{n}` collides with a codename command or alias");
    }
    if fleet
        .codenames
        .iter()
        .any(|o| o.name.to_lowercase() == lower)
    {
        bail!("a codename named `{n}` already exists");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path().join(".codename"));
        (d, p)
    }

    fn codename(name: &str, n: u32) -> Codename {
        Codename {
            name: name.into(),
            number: n,
            home: PathBuf::from(format!("/r/codenames/{name}")),
            ..Default::default()
        }
    }

    #[test]
    fn missing_registry_is_empty() {
        let (_d, p) = tmp();
        let f = Fleet::load(&p).unwrap();
        assert!(f.codenames.is_empty());
    }

    #[test]
    fn roundtrip_and_history() {
        let (_d, p) = tmp();
        let mut f = Fleet::load(&p).unwrap();
        f.codenames.push(codename("nova", 1));
        f.save(&p).unwrap();
        assert!(!p.history().exists(), "first write has nothing to snapshot");
        f.codenames.push(codename("lyra", 2));
        f.save(&p).unwrap();
        let snaps: Vec<_> = fs::read_dir(p.history()).unwrap().collect();
        assert_eq!(snaps.len(), 1);
        let back = Fleet::load(&p).unwrap();
        assert_eq!(back.codenames.len(), 2);
        assert_eq!(back.codenames[1].species, "codename");
        assert_eq!(back.next_number(), 3);
        // no temp files left behind
        let leftovers: Vec<_> = fs::read_dir(&p.home)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn corrupt_registry_errors_and_is_untouched() {
        let (_d, p) = tmp();
        fs::create_dir_all(&p.home).unwrap();
        let junk = b"{ \"codenames\": [ {\"name\": ";
        fs::write(p.fleet(), junk).unwrap();
        let err = Fleet::load(&p).unwrap_err().to_string();
        assert!(err.contains("corrupt"), "{err}");
        assert_eq!(fs::read(p.fleet()).unwrap(), junk);
    }

    #[test]
    fn tolerant_of_unknown_and_missing_fields() {
        let (_d, p) = tmp();
        fs::create_dir_all(&p.home).unwrap();
        fs::write(
            p.fleet(),
            r#"{"codenames":[{"name":"x","future":1}],"other":true}"#,
        )
        .unwrap();
        let f = Fleet::load(&p).unwrap();
        assert_eq!(f.codenames[0].species, "codename");
        assert!(!f.codenames[0].captain);
    }

    #[test]
    fn example_registry_parses() {
        let f: Fleet = serde_json::from_str(include_str!("../fleet.example.json")).unwrap();
        assert_eq!(f.captain().unwrap().name, "lyra");
        assert_eq!(f.codenames[0].orca.terminal.as_deref(), Some("term-2"));
    }

    #[test]
    fn single_captain() {
        let mut f = Fleet::default();
        f.codenames.push(codename("a", 1));
        f.codenames.push(codename("b", 2));
        f.set_captain("a").unwrap();
        f.set_captain("b").unwrap();
        assert_eq!(f.codenames.iter().filter(|o| o.captain).count(), 1);
        assert_eq!(f.captain().unwrap().name, "b");
        assert!(f.set_captain("zzz").is_err());
    }

    #[test]
    fn name_rules() {
        let reserved: Vec<String> = ["wakeup", "wake", "list", "ls"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut f = Fleet::default();
        f.codenames.push(codename("Nova", 1));
        assert!(validate_name("lyra", &reserved, &f).is_ok());
        assert!(validate_name("อรุณา", &reserved, &f).is_ok());
        assert!(validate_name("WAKE", &reserved, &f).is_err());
        assert!(validate_name("ls", &reserved, &f).is_err());
        assert!(validate_name("nova", &reserved, &f).is_err());
        assert!(validate_name("con", &reserved, &f).is_err());
        assert!(validate_name("Lpt1", &reserved, &f).is_err());
        assert!(validate_name("a/b", &reserved, &f).is_err());
        assert!(validate_name("a b", &reserved, &f).is_err());
        assert!(validate_name("1abc", &reserved, &f).is_err());
        assert!(validate_name("", &reserved, &f).is_err());
        assert!(validate_name("codename-captain", &reserved, &f).is_err());
    }

    #[test]
    fn codename_for_dir_matches_home_prefix() {
        let mut f = Fleet::default();
        f.codenames.push(codename("nova", 1));
        assert_eq!(
            f.codename_for_dir(Path::new("/r/codenames/nova/src"))
                .unwrap()
                .name,
            "nova"
        );
        assert!(f
            .codename_for_dir(Path::new("/r/codenames/other"))
            .is_none());
        assert!(f
            .codename_for_dir(Path::new("/r/codenames/novax"))
            .is_none());
    }

    #[test]
    fn engines_default_and_override() {
        let (_d, p) = tmp();
        let e = load_engines(&p).unwrap();
        assert_eq!(e.get("claude").unwrap().command, "claude");
        assert!(!e.contains_key("glm"));
        fs::create_dir_all(&p.home).unwrap();
        fs::write(p.engines(), r#"{"glm":"claude-glm","claude":"claude"}"#).unwrap();
        let e = load_engines(&p).unwrap();
        assert_eq!(engine_command(&e, "glm").unwrap(), "claude-glm");
        assert!(engine_command(&e, "codex").is_err());
        // plain "claude" keeps the default resume command
        assert_eq!(
            e["claude"].resume_command.as_deref(),
            Some("claude --continue")
        );
        assert_eq!(e["glm"].resume_command, None);
    }

    #[test]
    fn engines_full_form_and_resume_selection() {
        let e = parse_engines(
            r#"{"codex":{"command":"codex","resume_command":"codex resume --last"},
                "grok":"grok","x":{"command":"x-cli","resume_command":""}}"#,
        )
        .unwrap();
        assert_eq!(
            launch_command(&e, "codex", false, false).unwrap(),
            ("codex".into(), false)
        );
        assert_eq!(
            launch_command(&e, "codex", true, false).unwrap(),
            ("codex resume --last".into(), true)
        );
        assert_eq!(
            launch_command(&e, "codex", true, true).unwrap(),
            ("codex".into(), false)
        );
        assert_eq!(
            launch_command(&e, "grok", true, false).unwrap(),
            ("grok".into(), false)
        );
        assert_eq!(
            launch_command(&e, "x", true, false).unwrap(),
            ("x-cli".into(), false)
        );
        assert!(launch_command(&e, "nope", false, false).is_err());
        let d = default_engines();
        assert_eq!(
            d["claude"].resume_command.as_deref(),
            Some("claude --continue")
        );
        assert_eq!(d["grok"].resume_command, None);
        assert_eq!(orca_agent(&d, "codex").unwrap(), "codex");
        assert!(orca_agent(&d, "grok").is_err());
        let g = parse_engines(r#"{"grok":{"command":"grok","orca_agent":"opencode"}}"#).unwrap();
        assert_eq!(orca_agent(&g, "grok").unwrap(), "opencode");
        // defaults round-trip through the file format
        let back = parse_engines(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }

    fn engine_snaps(p: &Paths) -> usize {
        fs::read_dir(p.history())
            .map(|r| {
                r.filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().starts_with("engines-"))
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn free_engines_are_opt_in() {
        let f = free_engines();
        assert_eq!(f["claude-free"].command, "fcc-claude");
        assert_eq!(
            f["claude-free"].resume_command.as_deref(),
            Some("fcc-claude --continue")
        );
        assert_eq!(f["codex-free"].command, "fcc-codex");
        assert_eq!(
            f["codex-free"].resume_command.as_deref(),
            Some("fcc-codex resume --last")
        );
        assert!(f.values().all(|e| e.orca_agent.is_none()));
        let d = default_engines();
        assert!(!d.contains_key("claude-free") && !d.contains_key("codex-free"));
    }

    #[test]
    fn merge_keeps_user_entry_and_snapshots_once() {
        let (_d, p) = tmp();
        fs::create_dir_all(&p.home).unwrap();
        let original = r#"{"claude":"claude","claude-free":{"command":"my-fcc","x":1}}"#;
        fs::write(p.engines(), original).unwrap();
        let (added, kept) = merge_engines(&p, &free_engines()).unwrap();
        assert_eq!(added, vec!["codex-free".to_string()]);
        assert_eq!(kept, vec!["claude-free".to_string()]);
        assert_eq!(engine_snaps(&p), 1);
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(p.engines()).unwrap()).unwrap();
        // user entries copied exactly as written
        assert_eq!(
            v["claude-free"],
            serde_json::json!({"command":"my-fcc","x":1})
        );
        assert_eq!(v["claude"], "claude");
        assert_eq!(v["codex-free"]["command"], "fcc-codex");
        let e = load_engines(&p).unwrap();
        assert_eq!(e["claude-free"].command, "my-fcc");
        // the snapshot holds the old file
        let snap = fs::read_dir(p.history()).unwrap().next().unwrap().unwrap();
        assert_eq!(fs::read_to_string(snap.path()).unwrap(), original);
        // second run: nothing added, nothing written
        let before = fs::read_to_string(p.engines()).unwrap();
        let (added, kept) = merge_engines(&p, &free_engines()).unwrap();
        assert!(added.is_empty());
        assert_eq!(kept.len(), 2);
        assert_eq!(engine_snaps(&p), 1);
        assert_eq!(fs::read_to_string(p.engines()).unwrap(), before);
    }

    #[test]
    fn merge_absent_file_writes_defaults_plus_free() {
        let (_d, p) = tmp();
        let (added, kept) = merge_engines(&p, &free_engines()).unwrap();
        assert_eq!(added.len(), 2);
        assert!(kept.is_empty());
        assert_eq!(engine_snaps(&p), 0);
        let e = load_engines(&p).unwrap();
        for k in [
            "claude",
            "codex",
            "grok",
            "hermes",
            "claude-free",
            "codex-free",
        ] {
            assert!(e.contains_key(k), "{k} missing");
        }
        assert_eq!(e["claude"], default_engines()["claude"]);
    }

    #[test]
    fn merge_refuses_corrupt_file() {
        let (_d, p) = tmp();
        fs::create_dir_all(&p.home).unwrap();
        fs::write(p.engines(), "{ nope").unwrap();
        assert!(merge_engines(&p, &free_engines()).is_err());
        assert_eq!(fs::read_to_string(p.engines()).unwrap(), "{ nope");
        assert_eq!(engine_snaps(&p), 0);
    }
}
