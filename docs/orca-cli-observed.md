# Orca CLI — observed (Orca 1.4.224)

> Generated from `docs/orca-1.4.224/agent-context.json` (`orca agent-context --json`,
> 249 commands, schema v1) and checked live on Windows for terminal
> create/send/wait/read/close (`docs/orca-1.4.224/live-roundtrip.txt`).
> `cargo test` fails if codename sends a flag the schema does not list.

## Envelope (verified live)

```json
{"id": "…", "ok": true,  "result": { … }, "_meta": {"runtimeId": "…"}}
{"id": "…", "ok": false, "error": {"code": "timeout", "message": "timeout"}, "_meta": { … }}
```

- `runtime_unavailable` = the Orca runtime (app or `orca serve`) is not running.
- `terminal create` → `result.terminal.handle` (`term_…`), `worktreeId` = `<repo-id>::<path>`.
- `terminal send` → `result.send.{accepted, prompt.stages[], prompt.provider}`; a plain shell reports `provider: "unsupported"` and only `input_accepted`.
- `terminal wait` timeout → `ok:false`, `error.code: "timeout"` (codename exit 2). `tui-idle` never fires for a plain shell.
- `terminal read` → `result.terminal.{status, tail[], nextCursor, source}`; default `source: stream` stacks repaints, `--screen` gives the rendered frame.
- `terminal close` → `result.close.{handle, ptyKilled}`.

## Selectors (help.txt)

- `--repo`: `id:<id>`, `name:<name>`, `path:<path>`
- `--worktree`: `identity:<identity>`, `id:<repo-id>::<path>`, `name:<displayName>`, `branch:<branch>`, `issue:<number>`, `path:<path>` (forward slashes OK on Windows, e.g. `path:D:/fleet/sandbox`), `active`/`current`

## Commands codename uses

### `orca status` — used by doctor, startup, setup

```
orca status [--json]
```
- caller.orcaSessionId is this agent's Orca session ID, as Orca resolved it, when the agent runs as an Orca session; otherwise caller is omitted.

### `orca open` — used by startup (no serve task)

```
orca open [--json]
```

### `orca serve` — used by serve-task (scheduled task)

```
orca serve [--port <port>] [--pairing-address <host>] [--mobile-pairing] [--no-pairing] [--grant-desktop-control] [--project-root <path>] [--recipe-json] [--json]
```
- Runs in the foreground and prints the bound endpoint, advertised endpoint, and pairing status. Stop it with Ctrl+C.
- --pairing-address changes only the client-advertised address; use a reachable LAN, Tailscale, SSH-forward, or reverse-proxy endpoint.
- Use --recipe-json with --project-root from VM recipes to print the recipe result JSON and leave the server running.
- Use --mobile-pairing to print a mobile-scoped pairing QR/link instead of the default runtime-environment pairing link.
- A paired client cannot drive this machine's desktop (orca computer) unless the offer was created with --grant-desktop-control.
- When the web client bundle is available, the server also prints a browser URL with the pairing data embedded.

### `orca repo add` — used by wakeup, birth, setup, team, wt

```
orca repo add --path <path> [--json]
```

### `orca terminal list` — used by list, doctor, every seat command

```
orca terminal list [--worktree <selector>] [--limit <n>] [--include-visual-layouts] [--json]
```
- JSON omits visualLayouts by default; pass --include-visual-layouts when machine-readable tab and pane topology is required.

### `orca terminal create` — used by wakeup, setup (captain), birth --choose

```
orca terminal create [--worktree <selector>] [--title <name>] [--command <text>] [--shell <shell>] [--focus] [--json]
```
- Creates a visible terminal tab without switching focus when possible; falls back to a background handle if the UI cannot adopt it. Pass --focus to switch to it.
- Use this, not worktree create, for a fresh agent in the current checkout.
- --shell picks the shell the terminal IS on a Windows host (cmd.exe, powershell.exe, pwsh.exe, wsl.exe, bash.exe, git-bash); --command is typed into whatever shell the host started, so `--command cmd.exe` leaves a cmd running INSIDE the default shell and exiting it drops back to that shell.
- A host that cannot apply --shell refuses the create rather than quietly spawning its default shell: macOS and Linux execution hosts spawn the login shell, terminals routed over SSH resolve their shell on the SSH host, a --shell that contradicts the project execution runtime (WSL vs Windows host) is refused, and an Orca host older than --shell is refused by the CLI.

### `orca terminal send` — used by hey, ask, report, dialogs, sleep

```
orca terminal send [--terminal <handle>] [--text <text>] [--enter] [--interrupt] [--wait-submit <seconds>] [--retry-request <id>] [--json]
```
- For a text-plus-Enter agent prompt, the result separates input acceptance from observed submission and turn start.
- --wait-submit only observes the accepted prompt for the requested duration; timeout returns the queued/input-accepted receipt and never resends.
- After an ambiguous transport failure, reissue the exact command with the reported --retry-request ID. The ID is bound to the prompt payload and exact terminal process incarnation.
- Older hosts accept the legacy raw input but report provider old-host and do not offer idempotent retry or submission observation.

### `orca terminal wait` — used by wait, ask

```
orca terminal wait [--terminal <handle>] --for exit|tui-idle [--timeout-ms <ms>] [--json]
```

### `orca terminal read` — used by ask, peek, wakeup dialogs

```
orca terminal read [--terminal <handle>] [--cursor <n>] [--limit <n>] [--screen] [--json]
```
- Omit --terminal to target the active terminal in the current worktree.
- By default this returns accumulated terminal output with escape sequences stripped, not the rendered screen. Any program that repaints a line — shells, progress bars, TUIs — comes back as stacked fragments, so one `clear` keystroke by keystroke reads as `cclclecleaclear`, and spaces a prompt draws by moving the cursor are absent.
- Use --screen to read what the terminal actually renders. Prefer it whenever the answer depends on how output looks rather than what was emitted over time; the default is unsuitable for verifying rendered output.
- The result reports source: stream when it is accumulated output, screen when it is the rendered screen, and screen-unavailable when a screen was asked for but none could be rendered and the accumulated output is being returned instead. An absent source means the host predates the field.
- When present, draft is UI-only composer text excluded from tail; never treat it as terminal output or a submitted instruction.
- --screen and --cursor are mutually exclusive: a screen read is the current frame and has no history to page.
- Use --cursor with the nextCursor value from a previous read to get only new output since that read.
- Use --limit to request more retained lines for long agent responses; output reports oldestCursor when older lines were dropped.
- Useful for capturing the response to a command: read before sending, then read --cursor <prev> after waiting.

### `orca terminal close` — used by sleep, team down

```
orca terminal close ([--terminal <handle>] [--tab] | --worktree <selector> --all) [--json]
```
- Without --all, closes one terminal pane/session; add --tab to close its whole tab.
- With --worktree <selector> --all, stops every terminal process owned by that workspace and durably removes its terminal tabs, layouts, and resume records.
- Use workspace Sleep when the terminals and agent sessions should resume later.

### `orca worktree create` — used by wt

```
orca worktree create --name <name> [--repo <selector>|--project <id> [--host <host-id>]|--project-host-setup <id>] [--agent <id>] [--prompt <text>] [--setup run|skip|inherit] [--base-branch <ref>] [--issue <number>] [--pr <number>] [--linear-issue <identifier-or-url>] [--gitlab-issue <number-or-url>] [--gitlab-mr <number-or-url>] [--comment <text>] [--parent-worktree <selector>] [--no-parent] [--run-hooks] [--activate] [--json]
```
- This creates a new checkout. For a fresh agent in an existing worktree, use `orca terminal create --worktree active --command "codex"` instead.
- By default, Orca records the new worktree as a child of the caller context when it can infer one from the Orca terminal or current directory.
- If --repo is omitted, Orca infers the repo from the current Orca-managed worktree.
- Use --project with --host to create on a ready project host setup without spelling the backing repo id.
- --host runtime:<environment-id> creates on that paired Orca server; use the id from `orca environment list`, not the environment name.
- For related work, use the inferred parent or pass --parent-worktree active, folder:<id>, or worktree:<worktreeId> to make the relationship explicit. Worktree ids are the full <repo-id>::<path> values returned by `orca worktree list --json`.
- Use --no-parent when the new worktree should be independent of the current context.
- --no-parent only affects Orca lineage; omit --base-branch to use the repo default base, or pass the default base ref explicitly for independent top-level work.
- By default this creates the worktree and its first terminal without switching the active Orca view.
- Pass --agent to launch an agent in the first terminal; --prompt sends initial work to that agent.
- With --agent --json, read the new agent handle from result.agentTerminalHandle; older runtimes return only result.startupTerminal.handle, and may return neither for folder-based repos.
- Repo-defined setup hooks follow the repository setup policy; pass --setup run to force them.
- Pass --activate when the CLI caller intentionally wants to reveal the new worktree in the app.
- Passing --run-hooks is kept as a legacy alias for --setup run and reveals the worktree.
- Use --pr for GitHub pull requests; --gitlab-issue and --gitlab-mr write separate GitLab links. GitLab URLs must match the stored source project or remote; they cannot select a foreign project.

### `orca worktree rm` — used by team down

```
orca worktree rm --worktree <selector> [--force] [--run-hooks] [--allow-failed-archive-hook] [--json]
```
- Repo-defined orca.yaml archive hooks are skipped unless --run-hooks is passed.
- With --run-hooks, a failed archive hook blocks the removal: nothing is stopped, deleted or deregistered, and the command exits non-zero with error code worktree_archive_hook_failed. --force does not waive this.
- Pass --allow-failed-archive-hook to delete anyway after the hook has run and failed; the waived failure is reported back on result.archiveHookOverride. It requires --run-hooks and is rejected without it, because with no hook running there is no failure to waive.
- For Git worktrees, removal also attempts to delete the checked-out local branch, with or without --force. Orca retains branches it knows predated the worktree and any branch whose changes it cannot prove are already merged.

### `orca worktree list` — used by team down

```
orca worktree list [--repo <selector>] [--limit <n>] [--json]
```
- Each row carries the execution host that owns it (`host=`), and the trailing `scope:` line names every host the page covers plus the ones it does not.
- A host named under `not covered` may still have workspaces; an empty answer for it is not evidence that it has none. Each is annotated with the flag that reaches it, or marked not selectable from this machine.
- The row cap is shared across hosts, so a host whose rows sort last is not starved out of the page.

### `orca orchestration run-create` — used by team up

```
orca orchestration run-create --objective <text> [--from <handle>] [--retry-request <id>] [--json]
```
- A Run is a namespace and home inbox. It never schedules or places workers.
- --retry-request is only for exact recovery after an unknown mutation result.

### `orca orchestration worker-start` — used by team up

```
orca orchestration worker-start (--task <task_id> | --spec <text>) [--on <saved-environment>] [--worktree <current|selector|new-child|new-top-level>] (--agent <agent> | --terminal <handle>) [--task-title <text>] [--deps <json_array>] [--parent <task_id>] [--model <id>] [--effort <level>] [--name <name>] [--repo <selector>] [--base-branch <ref>] [--display-name <text>] [--comment <text>] [--setup <run|skip|inherit>] [--retry-of <dispatch_id>] [--timeout-ms <n>] [--run <run_id>] [--from <handle>] [--retry-request <id>] [--json]
```
- Current and existing worktrees never rerun setup; a fresh agent terminal is created unless --terminal is explicit.
- When reusing --terminal, pass --worktree for that terminal; current means the coordinator worktree.
- --agent takes an Orca agent id enabled on the worker server, such as claude, codex, cursor, antigravity, muse, zcode, opencode, or opencode2.
- --model supports Claude, Codex, Cursor, Antigravity, and Muse opaque provider model ids; --effort requires --model. OMP accepts --model only; --effort is unsupported. Neither can combine with --terminal. OpenCode model selection requires an existing worktree, a verified execution-host CLI, and an available model; effort is unsupported. Other agents, including zcode, launch with the model from their own config.
- New worktrees use agent-first creation and default --setup to run. Repository start-immediately runs setup beside the agent; wait-for-setup gates agent readiness and task input.
- Creation flags (--name, --repo, --base-branch, --display-name, --comment, --setup) are rejected for current/existing worktrees. Use exact --repo on the selected server; project/host convenience routing remains on worktree create.
- How the worker runs follows the user's own setting for new agent tabs; there is no flag for it and no caller needs to ask. A dispatch the setting cannot apply to still starts, so the placement, agent, and launch options passed here are always the ones honoured.
- Drive every worker the same way whichever way it was started: the same orchestration verbs, the same handle. Mail, dispatch, worker-show, worker-read and the whole lifecycle behave identically. The start receipt records which one ran, for operators and telemetry.
- Not every worker has a terminal. Read output with worker-read --source auto or --source transcript, which always work; --source terminal is refused when there is none, and orca terminal verbs do not accept every worker handle. Nothing above needs you to know which kind you have — the orchestration verbs cover all of them.
- --on selects only the worker server; the Run and this command remain on the current Orca server.
- Remote current and new-child are invalid; discover an exact remote selector or use new-top-level.
- --retry-of needs --task naming the failed Task (--spec creates a new one) and does not inherit placement; repeat the intended --on/worktree and --agent/terminal choices.
- The call exits 0 only for ready. Failed or outcome_unknown exits 1 and JSON includes stage/failedStage, setup, effects, residualResources, and recovery commands when needed.

### `orca orchestration worker-list` — used by team status

```
orca orchestration worker-list [--run <run_id>] [--terminal-state <active|reclaimable|retained|release_pending|release_unknown|released>] [--include-remote] [--cursor <cursor>] [--limit <1-100>] [--json]
```
- Terminal state is process accounting and is reported separately from Task status; a completed Task can still own a live terminal.
- Context-only Dispatches created by orchestration dispatch are included as unsupervised with terminal state retained.
- Returns at most 100 local rows, newest first, by default; --include-remote adds connected-server observations when the host supports fleet listing. Continue with the opaque page.nextCursor value unchanged.
- Without --run the list is scoped to the Run bound to the calling terminal, and to every Run when there is no binding; the receipt reports which in scope.source (flag, bound, or all).

### `orca orchestration worker-stop` — used by team down

```
orca orchestration worker-stop --dispatch <dispatch_id> [--retry-request <id>] [--json]
```
- A Dispatch created by orchestration dispatch is fenced without closing its unsupervised terminal process.
- Never deletes the worktree, setup terminal, configured tabs, or unrelated processes.

## Not used by codename but relevant

- `orca agent hooks status [--json]` — Show whether Orca-managed agent status hooks are enabled
- `orca agent hooks off [--json]` — Disable Orca-managed agent status hooks and remove local hook entries
- `orca terminal show [--terminal <handle>] [--json]` — Show terminal metadata and preview
- `orca orchestration check [--terminal <handle>] [--run <run_id>] [--ack <delivery_id>] [--unread | --peek | --all] [--types <type,...>] [--format] [--wait] [--timeout-ms <n>] [--retry-request <id>] [--json]
  default: return the bound Run's oldest unacknowledged FIFO batch.
  --ack: acknowledge the prior whole batch before checking/waiting.
  --peek: return only unread messages without marking them read.
  --all: return every message for the handle; does not mark read.
  --wait: block until a matching message arrives or --timeout-ms expires.
          Emits JSON keepalive lines to stderr every 15s so the caller can
          tell the process is alive. `_keepalive` is unrelated to heartbeat
          messages; `_heartbeat` remains as a deprecated compatibility alias.
          Filter with `jq "select(._keepalive|not)"` when merging streams.` — Check this agent's messages
- `orca worktree current [--json]` — Show the Orca-managed worktree for the current directory
