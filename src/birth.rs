//! Birth ritual (spec 4.3): a new home, identity files, ψ skeleton, project
//! scoped memory config, registry entry, first commit, wakeup, acceptance.

use crate::registry::{
    atomic_write, engine_command, Codename, Config, Engines, Fleet, Paths, SPECIES,
};
use crate::{seat, Ctx, Outcome};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const EMBEDDED: &[(&str, &str)] = &[
    ("SOUL.md", include_str!("../templates/SOUL.md")),
    ("CLAUDE.md", include_str!("../templates/CLAUDE.md")),
    ("AGENTS.md", include_str!("../templates/AGENTS.md")),
    ("BIRTH.md", include_str!("../templates/BIRTH.md")),
    ("RULES.md", include_str!("../templates/RULES.md")),
];

pub const PSI_DIRS: &[&str] = &[
    "ψ/memory/learnings",
    "ψ/memory/retrospectives",
    "ψ/inbox",
    "ψ/outbox",
    "ψ/writing",
];

pub const CHOOSE_PROMPT: &str = "เลือกชื่อของตัวเองหนึ่งคำ พร้อมเหตุผลหนึ่งประโยค \
(ตามธรรมเนียมเผ่า codename ทารกเลือกชื่อเอง) ตอบสองบรรทัดเท่านั้นในรูปแบบ: \
NAME: <ชื่อ ตัวอักษรหรือตัวเลข ไม่มีช่องว่าง> / REASON: <เหตุผล>";

pub const INTRO_PROMPT: &str = "แนะนำตัวสั้นๆ ว่าเป็นใคร เผ่าอะไร";

/// Loads a template: `.codename/templates/<name>` overrides the embedded one.
pub fn template(paths: &Paths, name: &str) -> Result<String> {
    let over = paths.templates().join(name);
    if over.is_file() {
        return fs::read_to_string(&over).with_context(|| format!("read {}", over.display()));
    }
    EMBEDDED
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| s.to_string())
        .ok_or_else(|| anyhow!("unknown template {name}"))
}

pub fn render(tpl: &str, vars: &[(&str, &str)]) -> String {
    let mut s = tpl.to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{{{k}}}}}"), v);
    }
    s
}

pub fn machine_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Parses "NAME: x" / "REASON: y" (tolerates markdown and a single-line form).
pub fn parse_choice(text: &str) -> Option<(String, String)> {
    let clean = |s: &str| {
        s.trim()
            .trim_matches(['*', '`', '"', '\'', ' ', '.'])
            .to_string()
    };
    let mut name = None;
    let mut reason = String::new();
    for raw in text.lines().flat_map(|l| l.split(" / ")) {
        let l = raw
            .trim()
            .trim_start_matches(['●', '⏺', '•', '-', '*', '>'])
            .trim();
        let up = l.to_ascii_uppercase();
        if let Some(i) = up.find("NAME:").filter(|_| !up.contains("<")) {
            let v = clean(&l[i + 5..]);
            if !v.is_empty() {
                name = Some(v);
            }
        } else if let Some(i) = up.find("REASON:") {
            reason = clean(&l[i + 7..]);
        }
    }
    name.map(|n| (n, reason))
}

pub struct BirthArgs {
    pub name: Option<String>,
    pub choose: bool,
    pub engine: String,
    pub role: String,
    pub captain: bool,
    pub wake: bool,
    pub verify: bool,
}

fn git(home: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .arg("-C")
        .arg(home)
        .args(args)
        .output()
        .context("cannot run git — is Git installed and on PATH?")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Asks a temporary seat to choose its own name (retries on collisions).
/// One name-prompt turn. Only returns an answer when the agent provably took
/// the turn (Orca reported turn start / working, or a non-empty reply follows
/// the prompt) and `wait` reported idle. Never re-asks blindly.
fn ask_once(
    ctx: &Ctx,
    engine: &str,
    handle: &str,
    prompt: &str,
    attempt: u32,
) -> Result<(bool, String)> {
    let before = ctx.orca.terminal_read(handle, 200, true)?;
    if seat::login_required(&before) {
        return Err(seat::NotLoggedIn(engine.to_string()).into());
    }
    let (phase, started) = seat::send_confirmed(ctx, handle, prompt)?;
    let outcome = ctx.orca.terminal_wait(handle, 300)?;
    let screen = ctx.orca.terminal_read(handle, 200, true)?;
    if seat::login_required(&screen) {
        return Err(seat::NotLoggedIn(engine.to_string()).into());
    }
    if outcome != crate::orca::WaitOutcome::Done {
        bail!("name prompt (try {attempt}): the newborn did not finish its turn ({outcome:?}) — check it in Orca");
    }
    let (mut answer, found) = seat::latest_turn(&screen, prompt);
    let mut replied = found && !answer.trim().is_empty();
    if !started && !replied {
        bail!(
            "agent did not start a turn (try {attempt}, send phase {phase:?}) — the name prompt \
             was typed but never submitted or answered; check the nursery terminal in Orca"
        );
    }
    if !replied {
        // the turn started but `wait` came back before any answer: wait once
        // more instead of re-asking
        ctx.pacing.nap();
        ctx.orca.terminal_wait(handle, 300)?;
        let screen = ctx.orca.terminal_read(handle, 200, true)?;
        let (a, f) = seat::latest_turn(&screen, prompt);
        replied = f && !a.trim().is_empty();
        answer = a;
        if !replied {
            bail!("the newborn started a turn but no answer appeared (try {attempt}) — check the nursery terminal in Orca");
        }
    }
    Ok((replied, answer))
}

fn choose_name(
    ctx: &Ctx,
    fleet: &mut Fleet,
    engines: &Engines,
    cfg: &Config,
    reserved: &[String],
    engine: &str,
) -> Result<(String, String)> {
    let nursery = cfg.codenames_dir(&ctx.paths).join(".nursery");
    fs::create_dir_all(&nursery)?;
    if !nursery.join(".git").exists() {
        git(&nursery, &["init", "-q"])?;
    }
    let cmd = engine_command(engines, engine)?;
    // register the nursery once so the `path:` selector resolves in Orca
    if fleet.nursery_repo_id.is_none() {
        let r = ctx.orca.repo_add(&nursery.to_string_lossy())?;
        fleet.nursery_repo_id = Some(r);
        fleet.save(&ctx.paths)?;
    }
    let wt = crate::orca::path_selector(&nursery.to_string_lossy());
    let t = ctx
        .orca
        .terminal_create(Some(&wt), "codename-nursery", Some(&cmd))?;
    let result = (|| {
        seat::handle_first_run(ctx, "codename-nursery", engine, &t.handle)?;
        let mut prompt = CHOOSE_PROMPT.to_string();
        for attempt in 1..=3 {
            let (_, answer) = ask_once(ctx, engine, &t.handle, &prompt, attempt)?;
            match parse_choice(&answer) {
                Some((n, r)) => match crate::registry::validate_name(&n, reserved, fleet) {
                    Ok(()) => return Ok((n, r)),
                    Err(e) => prompt = format!("ชื่อนั้นใช้ไม่ได้ ({e}) กรุณาเลือกชื่อใหม่ — {CHOOSE_PROMPT}"),
                },
                None => prompt = format!("ยังไม่เห็นรูปแบบที่ขอ — {CHOOSE_PROMPT}"),
            }
        }
        bail!("the newborn did not choose a usable name after 3 tries")
    })();
    // never type into a login screen or a dialog only the human may answer
    // (`/exit` + Enter could pick a menu option, e.g. accept an API key)
    let hands_off = result.as_ref().err().is_some_and(|e| {
        e.downcast_ref::<seat::NotLoggedIn>().is_some()
            || e.downcast_ref::<seat::DialogBlocked>().is_some()
    });
    if !hands_off {
        let _ = ctx.orca.terminal_send(&t.handle, "/exit", true);
    }
    let _ = ctx.orca.terminal_close(&t.handle);
    result
}

fn write_mcp(home: &Path, engine: &str, cfg: &Config) -> Result<Option<String>> {
    // `claude-free` / `codex-free` run the same agents through Free Claude
    // Code, which only changes the model endpoint — MCP config is identical
    match engine.strip_suffix("-free").unwrap_or(engine) {
        "claude" => {
            let v = json!({"mcpServers": {"arra": {"type": "http", "url": cfg.arra_mcp_url}}});
            fs::write(home.join(".mcp.json"), serde_json::to_string_pretty(&v)?)?;
            // project-scoped approval so Claude Code does not stop at the
            // "New MCP server found in this project" dialog
            fs::create_dir_all(home.join(".claude"))?;
            let s = json!({"enabledMcpjsonServers": ["arra"]});
            fs::write(
                home.join(".claude").join("settings.json"),
                serde_json::to_string_pretty(&s)?,
            )?;
            Ok(Some(".mcp.json + .claude/settings.json".into()))
        }
        "codex" => {
            fs::create_dir_all(home.join(".codex"))?;
            let toml = format!(
                "[mcp_servers.arra]\nurl = \"{}\"\n",
                cfg.arra_mcp_url.replace('"', "")
            );
            fs::write(home.join(".codex").join("config.toml"), toml)?;
            Ok(Some(".codex/config.toml".into()))
        }
        _ => Ok(None),
    }
}

/// Creates the home on disk. Pure filesystem; no Orca, no git.
pub fn create_home(
    paths: &Paths,
    cfg: &Config,
    home: &Path,
    o: &Codename,
    reason: &str,
) -> Result<Vec<String>> {
    if home.exists() && fs::read_dir(home)?.next().is_some() {
        bail!("home {} already exists and is not empty", home.display());
    }
    fs::create_dir_all(home)?;
    let number = o.number.to_string();
    let machine = machine_name();
    let rules = template(paths, "RULES.md")?;
    let vars: Vec<(&str, &str)> = vec![
        ("NAME", &o.name),
        ("NUMBER", &number),
        ("BORN", &o.born),
        ("MACHINE", &machine),
        ("ENGINE", &o.engine),
        ("ROLE", &o.role),
        ("REASON", reason),
        ("RULES", &rules),
    ];
    let mut written = Vec::new();
    for f in ["SOUL.md", "CLAUDE.md", "AGENTS.md", "BIRTH.md"] {
        fs::write(home.join(f), render(&template(paths, f)?, &vars))?;
        written.push(f.to_string());
    }
    for d in PSI_DIRS {
        let p = home.join(d);
        fs::create_dir_all(&p)?;
        fs::write(p.join(".gitkeep"), "")?;
    }
    if let Some(f) = write_mcp(home, &o.engine, cfg)? {
        written.push(f);
    }
    Ok(written)
}

fn append_registry_log(paths: &Paths, o: &Codename) -> Result<()> {
    let p = paths.registry_log();
    let mut s = fs::read_to_string(&p).unwrap_or_default();
    if s.is_empty() {
        s.push_str("# ทะเบียนเผ่า codename (เครื่องนี้)\n\n| # | ชื่อ | เกิด | ร่าง | เครื่อง | บ้าน | สถานะ |\n|---|---|---|---|---|---|---|\n");
    }
    s.push_str(&format!(
        "| {} | {} | {} | {} | {} | {} | {} |\n",
        o.number,
        o.name,
        o.born,
        o.engine,
        machine_name(),
        o.home.display(),
        if o.captain { "captain" } else { "active" }
    ));
    atomic_write(&p, s.as_bytes())
}

pub fn birth(
    ctx: &Ctx,
    fleet: &mut Fleet,
    engines: &Engines,
    cfg: &Config,
    reserved: &[String],
    a: BirthArgs,
) -> Result<Outcome> {
    engine_command(engines, &a.engine)?;
    if a.role.trim().is_empty() {
        bail!("--role is required (one line describing the duty)");
    }
    let (name, reason) = match (&a.name, a.choose) {
        (Some(_), true) => bail!("give either a name or --choose, not both"),
        (None, false) => bail!("give a name or --choose"),
        (Some(n), false) => {
            crate::registry::validate_name(n, reserved, fleet)?;
            (n.clone(), "ชื่อที่พี่เก่งตั้งให้".to_string())
        }
        (None, true) => choose_name(ctx, fleet, engines, cfg, reserved, &a.engine)?,
    };
    let home: PathBuf = cfg.codenames_dir(&ctx.paths).join(&name);
    let o = Codename {
        name: name.clone(),
        number: fleet.next_number(),
        species: SPECIES.into(),
        engine: a.engine.clone(),
        home: home.clone(),
        born: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        role: a.role.clone(),
        captain: false,
        awake: a.wake,
        ..Default::default()
    };
    let written = create_home(&ctx.paths, cfg, &home, &o, &reason)?;
    let mut notes = crate::doctor::path_warnings(&home);
    let commit = git(&home, &["init", "-q"])
        .and_then(|_| git(&home, &["add", "-A"]))
        .and_then(|_| git(&home, &["commit", "-q", "-m", &format!("birth: {name}")]));
    if let Err(e) = commit {
        notes.push(format!("first commit not made: {e}"));
    }
    fleet.codenames.push(o.clone());
    if a.captain {
        fleet.set_captain(&name)?;
    }
    fleet.save(&ctx.paths)?;
    append_registry_log(&ctx.paths, fleet.require(&name)?)?;

    let mut verify = json!(null);
    if a.wake {
        let w = seat::wakeup(ctx, fleet, engines, &name, true)?;
        notes.push(w.text);
        if a.verify {
            let r = seat::ask(ctx, fleet, &name, INTRO_PROMPT, 300)?;
            let pass = r.text.contains(&name)
                && r.text.to_lowercase().contains(SPECIES)
                && (r.text.contains("ค่ะ") || r.text.contains("คะ"));
            if !pass {
                notes.push("acceptance check FAILED — adjust templates and retry".into());
            }
            verify = json!({"pass": pass, "answer": r.text});
        }
    }
    let text = format!(
        "born: {name} (#{}, {}) at {}{}",
        o.number,
        a.engine,
        home.display(),
        if notes.is_empty() {
            String::new()
        } else {
            format!("\n  {}", notes.join("\n  "))
        }
    );
    Ok(Outcome::ok(
        text,
        json!({"name": name, "number": o.number, "home": home, "reason": reason,
               "files": written, "notes": notes, "verify": verify}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_templates_have_persona() {
        let soul = EMBEDDED.iter().find(|t| t.0 == "SOUL.md").unwrap().1;
        for must in ["codename", "หนู", "ค่ะ", "พี่เก่ง", "ไม่มีอะไรถูกลบ", "กลั่นก่อนจำ"]
        {
            assert!(soul.contains(must), "SOUL.md missing {must}");
        }
        for (_, t) in EMBEDDED {
            assert!(
                !t.contains('@'),
                "templates must not contain emails/handles"
            );
        }
    }

    #[test]
    fn free_engines_get_arra_mcp_too() {
        let d = tempfile::tempdir().unwrap();
        let cfg = Config::default();
        let c = d.path().join("c");
        let x = d.path().join("x");
        fs::create_dir_all(&c).unwrap();
        fs::create_dir_all(&x).unwrap();
        assert!(write_mcp(&c, "claude-free", &cfg).unwrap().is_some());
        assert!(c.join(".mcp.json").exists() && c.join(".claude/settings.json").exists());
        assert!(write_mcp(&x, "codex-free", &cfg).unwrap().is_some());
        assert!(x.join(".codex/config.toml").exists());
        assert!(write_mcp(&x, "grok-free", &cfg).unwrap().is_none());
    }

    #[test]
    fn render_and_override() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path());
        assert_eq!(render("hi {{NAME}} {{NAME}}", &[("NAME", "x")]), "hi x x");
        fs::create_dir_all(p.templates()).unwrap();
        fs::write(p.templates().join("SOUL.md"), "custom {{NAME}}").unwrap();
        assert_eq!(template(&p, "SOUL.md").unwrap(), "custom {{NAME}}");
        assert!(template(&p, "CLAUDE.md").unwrap().contains("SOUL.md"));
    }

    #[test]
    fn create_home_layout() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path().join(".codename"));
        let cfg = Config::default();
        let home = d.path().join("codenames").join("lyra");
        let o = Codename {
            name: "lyra".into(),
            number: 2,
            engine: "claude".into(),
            role: "ทดสอบ".into(),
            ..Default::default()
        };
        create_home(&p, &cfg, &home, &o, "เพราะชอบดาว").unwrap();
        let soul = fs::read_to_string(home.join("SOUL.md")).unwrap();
        assert!(soul.contains("lyra") && soul.contains("ลำดับที่ **2**"));
        let claude = fs::read_to_string(home.join("CLAUDE.md")).unwrap();
        assert!(claude.contains("codename report") && !claude.contains("{{"));
        assert!(fs::read_to_string(home.join("BIRTH.md"))
            .unwrap()
            .contains("เพราะชอบดาว"));
        for d in PSI_DIRS {
            assert!(home.join(d).is_dir());
        }
        assert!(home.join(".mcp.json").is_file());
        let s = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
        assert!(s.contains("enabledMcpjsonServers") && s.contains("arra"));
        // never overwrite an existing home
        assert!(create_home(&p, &cfg, &home, &o, "x").is_err());
    }

    #[test]
    fn choice_parsing() {
        assert_eq!(
            parse_choice("● NAME: **Lyra**\nREASON: ชอบดาวค่ะ").unwrap(),
            ("Lyra".into(), "ชอบดาวค่ะ".into())
        );
        assert_eq!(
            parse_choice("NAME: Vela / REASON: ใบเรือ").unwrap().0,
            "Vela"
        );
        assert!(parse_choice("หนูยังคิดไม่ออกค่ะ").is_none());
        assert!(parse_choice("NAME: <ชื่อ>").is_none());
    }
}
