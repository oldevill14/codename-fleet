//! `codename serve-task`: a Windows scheduled task that runs the headless
//! Orca runtime (`orca serve --no-pairing --json`) at logon, in the user's
//! interactive session, so the fleet works without the Orca app GUI.
//!
//! Safety:
//! - Programs are started by full path from %SystemRoot%\System32 with an
//!   argv list — never through `cmd /C`, never a concatenated command line.
//! - The PowerShell script is a constant; the task name and orca path reach
//!   it only through environment variables, so no user input is ever parsed
//!   as PowerShell code.
//! - `orca serve` binds a websocket on 0.0.0.0:6768. `--no-pairing` is
//!   mandatory and the Windows firewall must NOT open that port.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DEFAULT_TASK: &str = "codename-orca-serve";
/// Arguments the task passes to the orca executable (fixed, never user input).
pub const SERVE_ARGS: &str = "serve --no-pairing --json";

/// Fixed PowerShell script. Inputs: $env:CN_TASK, $env:CN_EXE, $env:CN_ARGS.
pub const INSTALL_PS: &str = r#"$ErrorActionPreference = 'Stop'
$user = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$action = New-ScheduledTaskAction -Execute $env:CN_EXE -Argument $env:CN_ARGS
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $env:CN_TASK -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description 'codename: headless Orca runtime (orca serve --no-pairing). Do not open port 6768 in the firewall.' -Force | Out-Null
Write-Output ('registered ' + $env:CN_TASK)"#;

#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub fn validate_task_name(n: &str) -> Result<()> {
    if n.is_empty() || n.len() > 64 {
        bail!("task name must be 1-64 characters");
    }
    if let Some(c) = n
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        bail!("task name `{n}` contains `{c}` — use ASCII letters, digits, `-`, `_` or `.`");
    }
    Ok(())
}

fn system32(system_root: &str) -> PathBuf {
    Path::new(system_root).join("System32")
}

pub fn install_invocation(system_root: &str, task: &str, orca_exe: &Path) -> Invocation {
    Invocation {
        program: system32(system_root)
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe"),
        args: [
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            INSTALL_PS,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        env: vec![
            ("CN_TASK".into(), task.into()),
            ("CN_EXE".into(), orca_exe.to_string_lossy().into_owned()),
            ("CN_ARGS".into(), SERVE_ARGS.into()),
        ],
    }
}

fn schtasks(system_root: &str, args: &[&str]) -> Invocation {
    Invocation {
        program: system32(system_root).join("schtasks.exe"),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: vec![],
    }
}

pub fn remove_invocation(system_root: &str, task: &str) -> Invocation {
    schtasks(system_root, &["/Delete", "/TN", task, "/F"])
}

pub fn query_invocation(system_root: &str, task: &str) -> Invocation {
    schtasks(system_root, &["/Query", "/TN", task, "/FO", "LIST"])
}

pub fn run_invocation(system_root: &str, task: &str) -> Invocation {
    schtasks(system_root, &["/Run", "/TN", task])
}

pub fn system_root() -> String {
    std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())
}

/// Runs an invocation; returns (success, combined output).
pub fn execute(inv: &Invocation) -> Result<(bool, String)> {
    if !cfg!(windows) {
        bail!("scheduled tasks are Windows-only — on macOS/Linux run `orca serve --no-pairing` under your own service manager");
    }
    let out = Command::new(&inv.program)
        .args(&inv.args)
        .envs(inv.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .output()
        .with_context(|| format!("cannot run {}", inv.program.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), text.trim().to_string()))
}

/// Starts the configured task (used by `startup`).
pub fn run_task(task: &str) -> Result<String> {
    validate_task_name(task)?;
    let (ok, out) = execute(&run_invocation(&system_root(), task))?;
    if !ok {
        bail!("schtasks /Run {task} failed: {out}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_names_are_strict() {
        assert!(validate_task_name(DEFAULT_TASK).is_ok());
        for bad in ["", "a b", "x\"; Remove-Item C:\\", "$(whoami)", "a/b", "ชื่อ"] {
            assert!(validate_task_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn install_passes_inputs_only_via_env() {
        let exe = Path::new("C:\\Program Files\\Orca\\orca.exe");
        let inv = install_invocation("C:\\Windows", "codename-orca-serve", exe);
        assert!(
            inv.program
                .ends_with("System32/WindowsPowerShell/v1.0/powershell.exe")
                || inv.program.to_string_lossy().ends_with("powershell.exe")
        );
        assert_eq!(inv.args.last().unwrap(), INSTALL_PS);
        assert!(!INSTALL_PS.contains("codename-orca-serve"));
        assert!(!inv.args.iter().any(|a| a.contains("Program Files")));
        assert!(inv
            .env
            .contains(&("CN_EXE".into(), exe.to_string_lossy().into_owned())));
        assert!(inv
            .env
            .contains(&("CN_ARGS".into(), "serve --no-pairing --json".into())));
        assert!(
            INSTALL_PS.contains("$env:CN_TASK") && INSTALL_PS.contains("-LogonType Interactive")
        );
        assert!(!inv.args.iter().any(|a| a == "/C" || a == "cmd"));
    }

    #[test]
    fn schtasks_argv() {
        let r = run_invocation("C:\\Windows", "t1");
        assert!(r.program.to_string_lossy().ends_with("schtasks.exe"));
        assert_eq!(r.args, ["/Run", "/TN", "t1"]);
        assert_eq!(
            remove_invocation("C:\\Windows", "t1").args,
            ["/Delete", "/TN", "t1", "/F"]
        );
        assert_eq!(
            query_invocation("C:\\Windows", "t1").args[..2],
            ["/Query", "/TN"]
        );
    }

    #[test]
    fn non_windows_refuses_to_execute() {
        if !cfg!(windows) {
            assert!(execute(&run_invocation("C:\\Windows", "t")).is_err());
        }
    }
}
