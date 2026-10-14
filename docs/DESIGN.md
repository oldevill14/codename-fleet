# Design

## Goal

A thin, single-binary fleet layer over the Orca desktop app's CLI. Orca owns
terminals, worktrees and orchestration; `codename` owns names, identity,
homes, the registry, memory wiring and health checks.

Clean-room: written only from the project spec and general knowledge. No code
from other fleet tools or memory servers was read or copied.

## Modules

| file | responsibility |
|---|---|
| `src/main.rs` | builds the real context, prints text or `--json`, maps exit codes |
| `src/cli.rs` | clap definition, alias rules, reserved names, dispatch |
| `src/orca.rs` | **the only place that runs `orca`**: flag table, argv builders, `OrcaApi` trait, lenient JSON parsing, `RealOrca` |
| `src/registry.rs` | paths, `config.json`, `engines.json`, `fleet.json` (atomic + history), name rules |
| `src/seat.rs` | wakeup / sleep / hey / wait / ask / peek, first-run dialogs, latest-turn extraction |
| `src/birth.rs` | birth ritual, templates (embedded, overridable), `--choose` |
| `src/team.rs` | charter teams on Orca orchestration |
| `src/memory.rs` | arra over HTTP |
| `src/doctor.rs` | health checks, config backup + SHA256 manifest + drift |
| `src/fleet.rs` | setup / startup (runtime lifecycle) / shutdown / list / report |
| `src/service.rs` | `serve-task`: Windows scheduled task for `orca serve --no-pairing` |
| `src/log.rs` | JSON-lines command log + rotation |
| `src/fake.rs` | scripted in-memory `OrcaApi` for tests; answers with the live 1.4.224 envelopes |

## Calling Orca

- `Command::new(bin).args(argv)` only — never a shell, never a joined string.
  Message text (Thai, quotes, `&`, `;`, `|`) is one argv element.
- Every flag/subcommand string is a constant in the `flags` module at the top of
  `src/orca.rs`. Pure `argv_*` builders turn them into vectors; unit tests check
  the vectors.
- Every response is an envelope `{"id","ok","result"|"error":{"code","message"},"_meta"}`
  (verified live). `orca::envelope` returns `result` or an `OrcaError{code,message}`;
  `runtime_unavailable` becomes "the Orca runtime is not running — `codename startup`".
  Inside `result` decoding stays lenient: known keys first, then a depth-first
  search for well-known key names; unknown fields are ignored.
- Binary: env `CODENAME_ORCA_BIN`, else config `orca_bin`, else `orca`.
  codename resolves it itself (`orca::resolve_on_path`): on Windows each PATH
  directory is tried with the PATHEXT extensions in order (default
  `.COM;.EXE;.BAT;.CMD`); elsewhere the exact name. It then spawns the
  **resolved full path** with `Command::new(path).args(argv)`.
- **BatBadBut guard:** if the resolved file is `.cmd`/`.bat`, codename refuses
  to run it. Windows runs batch files through `cmd.exe`, which re-parses the
  command line, so message text passed as an argument could inject commands
  (CVE-2024-24576 class). Rust's std now escapes such arguments, but codename
  does not rely on it: the user points `CODENAME_ORCA_BIN`/`orca_bin` at the
  real `.exe`. Nothing is ever spawned through `cmd /C`. `doctor` reports the
  resolved orca path and kind (exe/batch/other).
- Engine commands (`claude`, `codex`, …) are not spawned by codename — Orca
  starts them inside its terminal with fixed arguments (no message text). On
  `wakeup` codename still resolves the first word on PATH and adds a **note**
  if it is missing or is a batch script (npm installs `claude.cmd`), instead
  of refusing, because refusing would block the normal npm install of
  Claude Code while adding no protection.
- `ORCA_TELEMETRY_DISABLED=1` is set on every child process.

### Orca CLI surface (verified against Orca 1.4.224)

Source of truth: `docs/orca-1.4.224/agent-context.json` (`orca agent-context
--json`, 249 commands), `help.txt`, and `live-roundtrip.txt` (real JSON from a
Windows host). A unit test (`every_flag_exists_in_the_1_4_224_schema`) checks
that every flag codename sends exists for that command in the schema.

| use | argv we send |
|---|---|
| runtime state | `orca status --json` → `result.runtime.{state,reachable}`, `result.app.running` |
| start app | `orca open --json` |
| headless runtime | `orca serve --no-pairing --json` (scheduled task, see below) |
| register repo | `orca repo add --path <dir> --json` |
| list terminals | `orca terminal list --json` |
| open seat | `orca terminal create --worktree path:<home> --title <name> --command <engine> [--shell <sh>] --json` → `result.terminal.handle` |
| send | `orca terminal send --terminal <h> --text <msg> [--enter --wait-submit 15] --json` → `result.send.prompt.stages` |
| wait | `orca terminal wait --terminal <h> --for tui-idle --timeout-ms <ms> --json` |
| read | `orca terminal read --terminal <h> --limit <n> [--screen] --json` → `result.terminal.tail[]` |
| close | `orca terminal close --terminal <h> --json` |
| worktree (`wt`) | `orca worktree create --name <n> --repo id:<repoId> --json` · `worktree rm --worktree <selector>` · `worktree list --json` |
| team run | `orca orchestration run-create --objective <text> --from <captain> --json` |
| team worker | `orca orchestration worker-start --spec <task> --agent <agent> --worktree new-top-level --repo id:<repoId> --name team-<team>-<w> --display-name <team>-<w> --run <run> --from <captain> --json` |
| team status | `orca orchestration worker-list --run <run> --json` |
| team down | `orca orchestration worker-stop --dispatch <id> --json`, then WIP commit, then `worktree rm` |

Selectors (from `help.txt`): worktree `path:<path>` (forward slashes are fine
on Windows), `id:<repo-id>::<path>`, `branch:`, `name:`, `identity:`, `issue:`,
`active`/`current`; repo `id:<id>`, `name:<name>`, `path:<path>`. codename
registers each home with `repo add` once (cached `repoId`) and then targets it
as `path:<home>`.

Behaviour notes:
- `terminal wait`: success is `ok:true`; a timeout is
  `{"ok":false,"error":{"code":"timeout"}}` → exit 2. Codes containing
  block/permission/awaiting → exit 3 (those code names are **not yet seen
  live**). `tui-idle` never fires for a plain shell (only agent TUIs).
- `terminal send`: `--wait-submit` takes seconds. `stages` with a turn stage
  means delivered; `input_accepted` alone (e.g. plain shells:
  `provider: "unsupported"`) means typed only.
- `terminal read`: default output is the accumulated stream with repaints
  stacked (`eecho Cecho CN...`); codename reads `--screen` (rendered frame) for
  ask/peek/dialogs and falls back to the stream when the prompt scrolled off.
- `terminal create --command` is **typed into** the host shell; `--shell`
  chooses the shell itself on Windows (cmd.exe, powershell.exe, pwsh.exe,
  wsl.exe, bash.exe, git-bash) — set `terminal_shell` in config.json to use it.
- A terminal's `status` (`running`) is process state, not agent state; such
  terminals show as `awake` in `list`.
- `worker-start` has no `--title`/`--command`. Engines map to Orca agent ids
  (`claude`, `codex`, `cursor`, `antigravity`, `muse`, `zcode`, `opencode`,
  `opencode2`) via `orca_agent` in engines.json (claude/codex by default;
  grok/hermes need an explicit mapping).
- `worktree rm` also tries to delete the worktree's local branch unless Orca
  cannot prove it merged; codename commits WIP first, so unmerged work stays
  on a branch Orca keeps.
- Still unverified (not seen live): JSON results of status, repo add,
  terminal list, worktree create/list/rm and the orchestration verbs — parsed
  leniently.

## Orca runtime lifecycle

The CLI needs a running Orca **runtime**: either the desktop app or the
headless `orca serve`. The GUI is optional.

- `doctor` runs `orca status` and reports the state and whether the app or
  `serve` hosts it, plus the scheduled task if one is configured.
- `startup`: if `status` is not reachable, run the configured `serve_task`
  (`schtasks /Run /TN <task>`, Windows only), else `orca open`; then poll
  `status` every 2 s for up to 60 s.
- `codename serve-task install|remove|status` (alias `svc`) registers a
  Windows scheduled task (at logon, current user, interactive session,
  limited rights, no time limit) that runs `<orca.exe> serve --no-pairing --json`.
  `orca serve` started over SSH dies with the session, which is why it runs as
  a task in the user's interactive session. Programs are started by full path
  from `%SystemRoot%\System32` (`powershell.exe` with a **constant** script
  that reads the task name and orca path from environment variables;
  `schtasks.exe` with plain argv) — user input is never parsed as code.
- `orca serve` binds a websocket on **0.0.0.0:6768**. `--no-pairing` is
  mandatory (always passed), and the Windows firewall must **not** open that
  port.

## The `ask` trap

If the agent is already idle, `terminal wait --for tui-idle` returns at once and
`ask` would print the previous answer. So `ask`:

1. sends with `--enter --wait-submit 15` (Orca observes submission/turn start);
2. polls terminal state until it is seen `working`/`blocked` (up to
   `start_polls` × 2 s); a `turn_started` send phase also counts as confirmed;
3. only then waits for idle;
4. reads the screen and prints the text after the **last** line that contains
   the sent message (whitespace-normalised prefix, so wrapped prompts match;
   continuation lines of a wrapped prompt are skipped; TUI chrome is dropped).
   If the prompt scrolled away it falls back to the last 40 lines.

Exit codes: 0 done, 2 timeout, 3 blocked (also for `wait`).

## Resume on wakeup

`engines.json` values are either a string (`"grok": "grok"`) or
`{"command": "...", "resume_command": "..."}`. Defaults: claude →
`claude --continue`; codex → `codex resume --last` (**UNVERIFIED** — check
`codex --help` on the target machine); grok and hermes have none. A plain
string equal to the default command keeps the default resume command.

The registry records `last_woken` on every wakeup. `wakeup` uses the resume
command when `last_woken` is set and the engine has one, unless `--fresh`.
`startup` therefore resumes every codename that was awake before; `birth` always
starts fresh. The chosen command and `resumed: true/false` are in `--json`.

## Free mode (Free Claude Code)

`codename setup --free` merges two engines into `engines.json`:
`claude-free` → `fcc-claude` (resume `fcc-claude --continue`) and
`codex-free` → `fcc-codex` (resume `fcc-codex resume --last`). They are not in
the defaults. The merge only inserts missing keys — an existing entry is
copied exactly as written, never overwritten — and writes (after snapshotting
the old file to `history/engines-<stamp>.json`) only when something was added.

The launchers come from Free Claude Code
(https://github.com/Alishahryar1/free-claude-code, AGPL-3.0-only). codename is
MIT, so it never vendors FCC code or its install script and never runs the
installer; it only names the installed launchers. Each launcher strips every
`ANTHROPIC_*` variable and sets `ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN`
for its own process, so codename never sets `ANTHROPIC_*` anywhere (user env,
`settings.json`, `engines.json`) and never writes `~/.fcc/.env`.

`setup --free` and `doctor` (check `free mode`, shown only when an engine
command starts with `fcc-` or a codename uses a `*-free` engine) report missing
launchers on PATH (PATHEXT lookup as for orca) and two exposure findings from
`~/.fcc/.env`: `HOST` unset or all-interfaces (`0.0.0.0`, `::`, `[::]`), and
`PROXY_AUTH_ENABLED` not `true`/`1`. Values of tokens and `*_API_KEY` /
`*_TOKEN` keys are never printed (logs keep 200 characters).

Known gaps: `birth` writes the arra MCP config only for the exact engine
names `claude`/`codex`, so `*-free` codenames get none; free engines have no
`orca_agent`, so `team up` rejects them unless one is set in `engines.json`.

## Logging

Every command appends one JSON line to
`<home>/logs/codename-YYYY-MM-DD.log`:
`{ts, command, codename, orca_argv, exit, duration_ms, error}`. The command name
and codename come from the parsed CLI (message arguments are not logged
directly); `orca_argv` holds the argv vectors that actually ran, with every
string — and the error — truncated to 200 characters. Before each append,
files older than 14 days (today counts as day 1) are moved into
`logs/old/` — never deleted. `--verbose` also prints each orca argv and the
log line to stderr. Log write failures never fail the command.

## Path warnings

`setup` (root), `birth` (new home) and `doctor` (root + every home) warn when
the path is inside OneDrive — under `%OneDrive%`, `%OneDriveConsumer%` or
`%OneDriveCommercial%`, or containing a `\OneDrive` path segment prefix —
because sync can lock or rewrite `.git` objects and agent state. Non-ASCII
characters and spaces produce informational notes only.

## State on disk

`%USERPROFILE%\.codename\` (or `$HOME/.codename`, or `CODENAME_HOME`):

```
config.json          root, arra URLs, orca_bin, serve_task, terminal_shell
engines.json         engine -> launch command (editable, no rebuild)
fleet.json           the registry
history/             a copy of fleet.json (and engines.json) before every change
templates/           optional overrides of the embedded templates
backups/pre-orca-*/  agent-config copies + manifest.json (SHA256)
teams/               running team state; teams/history/ after `team down`
logs/                daily JSON-lines logs; logs/old/ after 14 days
REGISTRY-local.md    birth log, same columns as the species registry
first-run-dialogs.md dialogs seen during wakeup
codenames/<name>/    homes (or <root>/codenames when setup --root is given)
captain/             tiny git repo hosting the captain's shell terminal
```

Writes go to a temp file in the same directory, then `rename` (replaces on
Windows too). A corrupt `fleet.json` is reported and never overwritten.

## Names

Member names: 1–32 chars, start with a letter, letters/digits/`-`/`_` (Unicode
letters such as Thai are fine), not a Windows device name, not
`codename-captain`, and not any command or alias. The reserved list is built
from the clap definition at runtime so it cannot drift. Comparison with
existing codenames is case-insensitive.

## Memory (arra) — partly verified API

arra's code was not read. Endpoints are relative to `arra_url`
(default `http://127.0.0.1:47778`), all editable in `config.json`. The routes
were checked with read-only HTTP requests against one running arra (status
codes and key names only); nothing was written to it.

- `GET /api/health` → 200 (observed)
- `GET /api/v1/search?q=<query>&limit=<n>` → 200 with `results[]` whose items
  carry `content` (observed; `/api/search` 308-redirects here). The parser also
  accepts `items`/`hits`/`documents` and `pattern`/`text`/`snippet`/`title`.
- `POST /api/v1/learn` (route observed: `/api/learn` 308-redirects here) with
  JSON `{"pattern": "...", "source": "codename:<name>", "concepts": ["codename"]}`
  — **body shape UNVERIFIED** (not POSTed during development, to avoid writing
  into a real memory store)
- MCP for newborn homes: `arra_mcp_url` (default `http://127.0.0.1:47778/mcp`),
  written as an HTTP MCP server in `.mcp.json` (Claude) or
  `.codex/config.toml` (Codex), project scope only — user-level configs are
  never touched. For Claude the home also gets `.claude/settings.json` with
  `enabledMcpjsonServers: ["arra"]` so the "New MCP server found in this
  project" dialog does not block the first wakeup (setting name confirmed in
  the Claude Code binary). Codex may only load a project `.codex/config.toml`
  once the folder is trusted — unverified.

If arra is unreachable, `remember`/`recall` fail with a clear message;
`doctor` and `startup` only warn.

## Crates

clap (derive), serde, serde_json, serde_yaml, anyhow, ureq, chrono, sha2;
tempfile for tests. No others.

- `ureq` is built with `default-features = false`: arra is plain HTTP on
  loopback, so no TLS stack (and no C toolchain dependency from it) is needed.
- `chrono` uses only `clock` + `std`.
- `serde_yaml` 0.9 is marked deprecated upstream but is the crate the spec
  allows; it is only used to read charters.

## Testing

`cargo test` uses `fake::FakeOrca`, which records each call as the argv the
real implementation would run and serves scripted states/screens. Covered:
registry read/write (incl. corrupt file and history), latest-turn extraction,
the idle-before-send trap, exit codes, alias parsing, name collisions, argv
construction, first-run dialogs, birth layout, team up/down balance with WIP
commit, setup idempotency, report routing, shutdown order.
