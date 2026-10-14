//! JSON-lines command log: `<home>/logs/codename-YYYY-MM-DD.log`.
//!
//! One line per command: timestamp, command, codename, the orca argv vectors
//! that ran, exit status and duration. Every string is truncated to 200
//! characters so message bodies never land in the log in full. Files older
//! than 14 days are moved (never deleted) into `logs/old/`.

use crate::registry::Paths;
use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

pub const MAX_CHARS: usize = 200;
pub const KEEP_DAYS: i64 = 14;

pub fn truncate(s: &str) -> String {
    let n = s.chars().count();
    if n <= MAX_CHARS {
        s.to_string()
    } else {
        let head: String = s.chars().take(MAX_CHARS).collect();
        format!("{head}…(+{} chars)", n - MAX_CHARS)
    }
}

pub struct Record<'a> {
    pub command: &'a str,
    pub codename: Option<&'a str>,
    pub orca_argv: &'a [Vec<String>],
    pub exit: i32,
    pub duration: Duration,
    pub error: Option<&'a str>,
}

pub fn entry(r: &Record, now: DateTime<Local>) -> Value {
    let argv: Vec<Vec<String>> = r
        .orca_argv
        .iter()
        .map(|a| a.iter().map(|x| truncate(x)).collect())
        .collect();
    json!({
        "ts": now.to_rfc3339(),
        "command": r.command,
        "codename": r.codename.map(truncate),
        "orca_argv": argv,
        "exit": r.exit,
        "duration_ms": r.duration.as_millis() as u64,
        "error": r.error.map(truncate),
    })
}

pub fn file_for(paths: &Paths, day: NaiveDate) -> PathBuf {
    paths
        .logs()
        .join(format!("codename-{}.log", day.format("%Y-%m-%d")))
}

fn day_of(name: &str) -> Option<NaiveDate> {
    let d = name.strip_prefix("codename-")?.strip_suffix(".log")?;
    NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()
}

/// Moves logs older than KEEP_DAYS (today counts as day 1) into `logs/old/`.
pub fn rotate(paths: &Paths, today: NaiveDate) -> Result<Vec<PathBuf>> {
    let dir = paths.logs();
    let mut moved = vec![];
    let Ok(rd) = fs::read_dir(&dir) else {
        return Ok(moved);
    };
    for e in rd.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(day) = day_of(&name) else { continue };
        if (today - day).num_days() < KEEP_DAYS || !e.path().is_file() {
            continue;
        }
        let old = dir.join("old");
        fs::create_dir_all(&old)?;
        let mut dest = old.join(&name);
        if dest.exists() {
            dest = old.join(format!("{name}.{}", crate::registry::stamp()));
        }
        fs::rename(e.path(), &dest)?;
        moved.push(dest);
    }
    Ok(moved)
}

/// Appends one line (and rotates). Returns the log file path.
pub fn append(paths: &Paths, line: &Value, now: DateTime<Local>) -> Result<PathBuf> {
    fs::create_dir_all(paths.logs())?;
    rotate(paths, now.date_naive())?;
    let p = file_for(paths, now.date_naive());
    let mut f = OpenOptions::new().create(true).append(true).open(&p)?;
    writeln!(f, "{line}")?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn truncates_long_bodies() {
        let long = "ก".repeat(500);
        let t = truncate(&long);
        assert!(t.starts_with(&"ก".repeat(200)));
        assert!(t.contains("+300 chars"));
        assert_eq!(truncate("short"), "short");
    }

    #[test]
    fn appends_json_lines_with_truncated_argv() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path());
        let now = Local.with_ymd_and_hms(2026, 3, 10, 9, 0, 0).unwrap();
        let body = "x".repeat(1000);
        let argv = vec![vec![
            "terminal".into(),
            "send".into(),
            "--text".into(),
            body.clone(),
        ]];
        let r = Record {
            command: "ask",
            codename: Some("nova"),
            orca_argv: &argv,
            exit: 2,
            duration: Duration::from_millis(1234),
            error: None,
        };
        let f = append(&p, &entry(&r, now), now).unwrap();
        append(&p, &entry(&r, now), now).unwrap();
        assert!(f.ends_with("logs/codename-2026-03-10.log"));
        let text = fs::read_to_string(&f).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(!text.contains(&body), "full body must never be logged");
        let v: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(v["command"], "ask");
        assert_eq!(v["codename"], "nova");
        assert_eq!(v["exit"], 2);
        assert_eq!(v["duration_ms"], 1234);
        assert!(v["orca_argv"][0][3].as_str().unwrap().chars().count() < 230);
    }

    #[test]
    fn rotation_moves_old_files_and_keeps_14_days() {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::new(d.path());
        fs::create_dir_all(p.logs()).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 3, 20).unwrap();
        for day in [6, 7, 8, 19, 20] {
            let f = file_for(&p, NaiveDate::from_ymd_opt(2026, 3, day).unwrap());
            fs::write(f, "{}\n").unwrap();
        }
        fs::write(p.logs().join("notes.txt"), "keep").unwrap();
        let moved = rotate(&p, today).unwrap();
        // keep 14 days = 03-07..03-20; only 03-06 moves
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert!(p.logs().join("old/codename-2026-03-06.log").is_file());
        assert!(p.logs().join("codename-2026-03-07.log").is_file());
        assert!(p.logs().join("notes.txt").is_file());
        // a second move with the same name is kept, not overwritten
        fs::write(
            file_for(&p, NaiveDate::from_ymd_opt(2026, 3, 6).unwrap()),
            "again",
        )
        .unwrap();
        rotate(&p, today).unwrap();
        assert_eq!(fs::read_dir(p.logs().join("old")).unwrap().count(), 2);
    }
}
