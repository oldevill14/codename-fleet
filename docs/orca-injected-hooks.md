# Hooks injected by Orca into agent configs

> Observed on a Windows test machine with Orca 1.4.224. `codename setup` takes
> the baseline (`%USERPROFILE%\.codename\backups\pre-orca-*\manifest.json`);
> `codename doctor` lists files that changed since. Diff each against the copy
> in `files\`. `orca agent hooks status|on|off` manages these entries.

## Observed after the first Orca runtime start (Windows)

| file | what Orca did |
|---|---|
| `~/.claude/settings.json` | **modified** — added `hooks` on many events, including `PermissionRequest`, `PostCompact`, `TeammateIdle`, `PostToolUseFailure` (and others); each runs `C:/Users/<user>/.orca/agent-hooks/claude-hook.cmd` with `timeout: 10` |
| `~/.codex/config.toml` | **modified** |
| `~/.codex/hooks.json` | **created** — hooks on `SessionStart`, `UserPromptSubmit`, `PreToolUse`, … running `codex-hook.cmd` (under `~/.orca/agent-hooks/`) |
| `~/.gemini/settings.json` | **modified** |

Orca calls these "agent status hooks" (`orca agent hooks on|off|status`); they
are how Orca learns agent state (working / idle / waiting for a human) — `terminal wait --for tui-idle`, send turn-start observation and
`worker-show` waits depend on them. Removing them may break those features.

## Decision (human)

| option | effect |
|---|---|
| keep | Orca agent status works; agent configs carry Orca's hook scripts |
| remove (`orca agent hooks off`, or restore from the codename backup) | clean configs; codename's `ask`/`wait` lose Orca's idle/turn signals |
| remove + block re-adding | same as remove; check `doctor` after every Orca update |

Do not remove anything without the human's decision.
