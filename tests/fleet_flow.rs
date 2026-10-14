//! End-to-end flow through `cli::run` with the fake Orca (no real Orca needed).

use clap::Parser;
use codename::cli::{run, Cli};
use codename::fake::FakeOrca;
use codename::orca::TermState;
use codename::registry::{Fleet, Paths};
use codename::{Ctx, Pacing};
use std::path::Path;
use std::time::Duration;

fn exec(ctx: &Ctx, user: &Path, args: &[&str]) -> codename::Outcome {
    let mut v = vec!["codename"];
    v.extend_from_slice(args);
    let cli = Cli::try_parse_from(v).expect("parse");
    run(cli.cmd, ctx, user, user).unwrap_or_else(|e| panic!("{args:?}: {e:#}"))
}

#[test]
fn setup_birth_list_ask_report_shutdown_startup() {
    let d = tempfile::tempdir().unwrap();
    let user = d.path().join("user");
    std::fs::create_dir_all(&user).unwrap();
    let fake = FakeOrca::default();
    let ctx = Ctx {
        orca: &fake,
        paths: Paths::new(user.join(".codename")),
        pacing: Pacing {
            poll: Duration::ZERO,
            start_polls: 2,
            dialog_polls: 2,
            exit_polls: 1,
        },
    };

    assert_eq!(exec(&ctx, &user, &["setup"]).code, 0);
    // keep tests off any real arra on this machine (port 9 = refused)
    let mut cfg = codename::registry::Config::load(&ctx.paths).unwrap();
    cfg.arra_url = "http://127.0.0.1:9".into();
    cfg.save(&ctx.paths).unwrap();

    *fake.screen.borrow_mut() =
        "> แนะนำตัวสั้นๆ ว่าเป็นใคร เผ่าอะไร\nหนูชื่อ lyra เป็นสมาชิกเผ่า codename ค่ะ".into();
    let b = exec(
        &ctx,
        &user,
        &[
            "birth",
            "lyra",
            "--engine",
            "claude",
            "--role",
            "กัปตัน",
            "--captain",
        ],
    );
    assert_eq!(b.code, 0, "{}", b.text);
    assert_eq!(b.json["verify"]["pass"], true, "{}", b.json);
    let home = user.join(".codename").join("codenames").join("lyra");
    assert!(home.join("SOUL.md").is_file());
    assert!(home.join("ψ").join("inbox").is_dir());

    exec(
        &ctx,
        &user,
        &[
            "bud",
            "vela",
            "--engine",
            "codex",
            "--role",
            "แรงงานโค้ด",
            "--no-verify",
        ],
    );
    let f = Fleet::load(&ctx.paths).unwrap();
    assert_eq!(f.codenames.len(), 2);
    assert_eq!(f.captain().unwrap().name, "lyra");
    assert_eq!(f.get("vela").unwrap().number, 2);
    assert!(std::fs::read_to_string(ctx.paths.registry_log())
        .unwrap()
        .contains("| 2 | vela |"));

    // name collisions are refused
    let cli = Cli::try_parse_from(["codename", "birth", "wake", "--role", "x"]).unwrap();
    assert!(run(cli.cmd, &ctx, &user, &user).is_err());
    let cli = Cli::try_parse_from(["codename", "birth", "Lyra", "--role", "x"]).unwrap();
    assert!(run(cli.cmd, &ctx, &user, &user).is_err());

    let l = exec(&ctx, &user, &["ls", "--json"]);
    assert_eq!(l.json["codenames"].as_array().unwrap().len(), 2);
    assert_eq!(l.json["codenames"][1]["status"], "idle");

    *fake.screen.borrow_mut() = "> ตอบคำเดียว: CN-OK\nCN-OK".into();
    fake.states
        .borrow_mut()
        .extend([TermState::Idle, TermState::Working, TermState::Idle]);
    let a = exec(&ctx, &user, &["ask", "vela", "ตอบคำเดียว:", "CN-OK"]);
    assert_eq!(a.text, "CN-OK");

    // report from inside vela's home goes to the captain
    let cli = Cli::try_parse_from(["codename", "report", "งานเสร็จ"]).unwrap();
    let vela_home = user.join(".codename").join("codenames").join("vela");
    run(cli.cmd, &ctx, &user, &vela_home).unwrap();
    let last_send = fake.calls_of("terminal", "send").pop().unwrap();
    assert!(last_send.iter().any(|a| a == "REPORT vela rc=0: งานเสร็จ"));

    let terminals_awake = fake.terminals.borrow().len();
    exec(&ctx, &user, &["down"]);
    assert_eq!(
        fake.terminals.borrow().len(),
        1,
        "only the captain shell stays"
    );
    exec(&ctx, &user, &["up"]);
    assert_eq!(fake.terminals.borrow().len(), terminals_awake);
    let l = exec(&ctx, &user, &["list"]);
    assert!(!l.text.contains("rest"), "{}", l.text);
    // startup reopened seats that had been woken before → resume commands
    let creates = fake.calls_of("terminal", "create");
    assert!(creates
        .iter()
        .any(|c| c.iter().any(|x| x == "codex resume --last")));
    assert!(creates
        .iter()
        .any(|c| c.iter().any(|x| x == "claude --continue")));
}

#[test]
fn every_orca_call_is_plain_argv() {
    // Whatever ran above, the fake recorded argv vectors; none may be a shell.
    let fake = FakeOrca::default();
    let d = tempfile::tempdir().unwrap();
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
    exec(&ctx, d.path(), &["setup"]);
    for c in fake.calls.borrow().iter() {
        assert!(!c.is_empty());
        for bad in ["cmd", "/C", "sh", "-c", "powershell"] {
            assert_ne!(c[0], bad);
        }
        assert!(c.iter().all(|a| !a.contains(" && ")));
    }
}

#[test]
fn birth_choose_lets_newborn_pick_and_reuses_nursery_repo() {
    let d = tempfile::tempdir().unwrap();
    let user = d.path().join("user");
    std::fs::create_dir_all(&user).unwrap();
    let fake = FakeOrca::default();
    let ctx = Ctx {
        orca: &fake,
        paths: Paths::new(user.join(".codename")),
        pacing: Pacing {
            poll: Duration::ZERO,
            start_polls: 1,
            dialog_polls: 1,
            exit_polls: 1,
        },
    };
    exec(&ctx, &user, &["setup"]);
    let prompt = codename::birth::CHOOSE_PROMPT;
    for name in ["nova", "vela"] {
        *fake.screen.borrow_mut() = format!("> {prompt}\nNAME: {name}\nREASON: ชอบดาวค่ะ");
        let o = exec(
            &ctx,
            &user,
            &[
                "birth",
                "--choose",
                "--engine",
                "codex",
                "--role",
                "งาน",
                "--no-verify",
            ],
        );
        assert_eq!(o.json["name"], name, "{}", o.text);
    }
    let f = Fleet::load(&ctx.paths).unwrap();
    assert!(f.get("nova").is_some() && f.get("vela").is_some());
    // captain repo + nursery once + one per home = 4 (not 5)
    assert_eq!(fake.calls_of("repo", "add").len(), 4);
    let birth_md = std::fs::read_to_string(
        user.join(".codename")
            .join("codenames")
            .join("vela")
            .join("BIRTH.md"),
    )
    .unwrap();
    assert!(birth_md.contains("ชอบดาวค่ะ"));
}

#[test]
fn binary_logs_every_command_and_refuses_batch_orca() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("cn");
    let shim = d.path().join("orca.cmd");
    std::fs::write(&shim, "@echo off\r\n").unwrap();
    // keep the test off any real arra on this machine (port 9 = refused)
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.json"),
        r#"{"arra_url":"http://127.0.0.1:9"}"#,
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_codename"))
        .args(["--verbose", "doctor"])
        .env("CODENAME_HOME", &home)
        .env("CODENAME_ORCA_BIN", &shim)
        .env("HOME", d.path())
        .env("USERPROFILE", d.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stdout.contains("[FAIL] orca binary"), "{stdout}");
    assert!(stdout.contains("CVE-2024-24576"), "{stdout}");
    assert!(stderr.contains("[codename] log "), "{stderr}");
    let logs: Vec<_> = std::fs::read_dir(home.join("logs"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(logs.len(), 1);
    let text = std::fs::read_to_string(logs[0].path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(v["command"], "doctor");
    assert_eq!(v["exit"], 1);
    assert_eq!(
        v["orca_argv"].as_array().unwrap().len(),
        0,
        "batch orca never spawned"
    );
}

fn quick_ctx<'a>(fake: &'a FakeOrca, user: &Path) -> Ctx<'a> {
    Ctx {
        orca: fake,
        paths: Paths::new(user.join(".codename")),
        pacing: Pacing {
            poll: Duration::ZERO,
            start_polls: 2,
            dialog_polls: 3,
            exit_polls: 1,
        },
    }
}

fn birth_choose(ctx: &Ctx, user: &Path, engine: &str) -> anyhow::Result<codename::Outcome> {
    let cli = Cli::try_parse_from([
        "codename",
        "birth",
        "--choose",
        "--engine",
        engine,
        "--role",
        "x",
        "--no-verify",
    ])
    .unwrap();
    run(cli.cmd, ctx, user, user)
}

#[test]
fn birth_stops_when_agent_is_not_logged_in() {
    let d = tempfile::tempdir().unwrap();
    let user = d.path().join("user");
    std::fs::create_dir_all(&user).unwrap();
    let fake = FakeOrca::default();
    let ctx = quick_ctx(&fake, &user);
    exec(&ctx, &user, &["setup"]);
    *fake.screen.borrow_mut() =
        include_str!("fixtures/screens/claude-not-logged-in.txt").to_string();
    let err = birth_choose(&ctx, &user, "claude").unwrap_err().to_string();
    assert!(
        err.contains("claude is not logged in on this machine")
            && err.contains("never logs in for you"),
        "{err}"
    );
    // the name prompt was never typed, and the nursery seat is closed
    let sends = fake.calls_of("terminal", "send");
    assert!(sends.is_empty(), "{sends:?}");
    assert_eq!(
        fake.terminals.borrow().len(),
        1,
        "only the captain shell stays"
    );
    assert!(Fleet::load(&ctx.paths).unwrap().codenames.is_empty());
}

#[test]
fn birth_named_free_engine_stops_at_api_key_prompt_and_closes_seat() {
    let d = tempfile::tempdir().unwrap();
    let user = d.path().join("user");
    std::fs::create_dir_all(&user).unwrap();
    let fake = FakeOrca::default();
    let ctx = quick_ctx(&fake, &user);
    exec(&ctx, &user, &["setup", "--free"]);
    *fake.screen.borrow_mut() =
        include_str!("fixtures/screens/claude-custom-api-key.txt").to_string();
    let cli = Cli::try_parse_from([
        "codename",
        "birth",
        "lyra",
        "--engine",
        "claude-free",
        "--role",
        "x",
    ])
    .unwrap();
    let err = run(cli.cmd, &ctx, &user, &user).unwrap_err().to_string();
    assert!(err.contains("blocked:") && err.contains("ApiKey"), "{err}");
    assert!(fake.calls_of("terminal", "send").is_empty());
    assert_eq!(
        fake.terminals.borrow().len(),
        1,
        "only the captain shell stays"
    );
    // a later shutdown types nothing into any seat
    exec(&ctx, &user, &["shutdown"]);
    assert!(fake.calls_of("terminal", "send").is_empty());
}

#[test]
fn birth_stops_blocked_at_custom_api_key_prompt_and_types_nothing() {
    let d = tempfile::tempdir().unwrap();
    let user = d.path().join("user");
    std::fs::create_dir_all(&user).unwrap();
    let fake = FakeOrca::default();
    let ctx = quick_ctx(&fake, &user);
    exec(&ctx, &user, &["setup", "--free"]);
    *fake.screen.borrow_mut() =
        include_str!("fixtures/screens/claude-custom-api-key.txt").to_string();
    let err = birth_choose(&ctx, &user, "claude-free")
        .unwrap_err()
        .to_string();
    assert!(
        err.starts_with("blocked:") && err.contains("ApiKey"),
        "{err}"
    );
    // nothing typed: no name prompt, no `1`, not even `/exit` + Enter
    let sends = fake.calls_of("terminal", "send");
    assert!(sends.is_empty(), "{sends:?}");
    assert!(Fleet::load(&ctx.paths).unwrap().codenames.is_empty());
}

#[test]
fn birth_fails_fast_when_agent_never_starts_a_turn() {
    let d = tempfile::tempdir().unwrap();
    let user = d.path().join("user");
    std::fs::create_dir_all(&user).unwrap();
    let fake = FakeOrca::default();
    let ctx = quick_ctx(&fake, &user);
    exec(&ctx, &user, &["setup"]);
    // ready codex screen that never changes; Orca only accepts the keys
    *fake.screen.borrow_mut() =
        include_str!("fixtures/screens/codex-update-banner-ready.txt").to_string();
    fake.send_phase.set(codename::orca::SendPhase::Accepted);
    let err = birth_choose(&ctx, &user, "codex").unwrap_err().to_string();
    assert!(err.contains("agent did not start a turn"), "{err}");
    let prompts: Vec<_> = fake
        .calls_of("terminal", "send")
        .into_iter()
        .filter(|c| c.iter().any(|a| a.contains("NAME:")))
        .collect();
    assert_eq!(prompts.len(), 1, "never re-asks blindly");
    assert_eq!(fake.terminals.borrow().len(), 1, "nursery closed");
}
