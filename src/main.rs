#![forbid(unsafe_code)]

use clap::Parser;
use codename::cli::{run, Cli};
use codename::orca::RealOrca;
use codename::registry::{user_home, Config, Paths};
use codename::{log, Ctx, Outcome, Pacing};
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let started = Instant::now();
    let cli = Cli::parse();
    let (json, verbose) = (cli.json, cli.verbose);
    let (command, codename) = cli.cmd.log_info();
    let paths = Paths::from_env();
    let cfg = paths
        .as_ref()
        .ok()
        .and_then(|p| Config::load(p).ok())
        .unwrap_or_default();
    let orca = RealOrca::new(cfg.orca_bin.as_deref(), cfg.terminal_shell.clone(), verbose);
    let result: anyhow::Result<Outcome> = (|| {
        let paths = paths
            .as_ref()
            .map_err(|e| anyhow::anyhow!("{e:#}"))?
            .clone();
        let home = user_home().ok_or_else(|| anyhow::anyhow!("cannot find USERPROFILE or HOME"))?;
        let cwd = std::env::current_dir()?;
        let ctx = Ctx {
            orca: &orca,
            paths,
            pacing: Pacing::default(),
        };
        run(cli.cmd, &ctx, &home, &cwd)
    })();
    let (code, err) = match &result {
        Ok(o) => (o.code, None),
        Err(e) => (1, Some(format!("{e:#}"))),
    };
    if let Ok(p) = &paths {
        let calls = orca.calls.borrow();
        let rec = log::Record {
            command: &command,
            codename: codename.as_deref(),
            orca_argv: &calls,
            exit: code,
            duration: started.elapsed(),
            error: err.as_deref(),
        };
        let line = log::entry(&rec, chrono::Local::now());
        let written = log::append(p, &line, chrono::Local::now());
        if verbose {
            eprintln!("[codename] log {line}");
            if let Err(e) = written {
                eprintln!("[codename] could not write log: {e:#}");
            }
        }
    }
    match result {
        Ok(o) => {
            if json {
                let v = serde_json::json!({"code": o.code, "result": o.json});
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            } else if !o.text.is_empty() {
                println!("{}", o.text);
            }
            ExitCode::from(o.code.clamp(0, 255) as u8)
        }
        Err(e) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"code": 1, "error": format!("{e:#}")})
                );
            } else {
                eprintln!("error: {e:#}");
            }
            ExitCode::from(1)
        }
    }
}
