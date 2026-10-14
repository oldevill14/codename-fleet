# First-run dialogs seen during `wakeup`

`codename wakeup` (and `birth --choose`) reads the new seat's rendered screen
every 2 s for up to 60 s and answers only known safe dialogs (`src/seat.rs`).
Real screens are kept as test fixtures in `tests/fixtures/screens/`.

| dialog | how it is detected | action |
|---|---|---|
| Codex update menu (seen live, Codex 0.146.1) | interactive markers only: an option line `1. … update …` (may start with `›`), a `2. Skip` line **and** `Press enter to continue` | send `2` **without Enter** (verified to skip) |
| Codex update banner left after skipping | `Update available! … Run npm install …` box — **not** a dialog | none (stays on screen forever) |
| not logged in (seen live, Claude Code) | `Not logged in · Run /login` (header also shows `API Usage Billing`); Codex best effort: `Sign in with ChatGPT`, `Provide your own API key`, `Select login method` | **stop**: close the seat, exit non-zero, never retry: "`<engine>` is not logged in on this machine — run `<engine>` and /login yourself; codename never logs in for you" |
| Claude Code custom API key (reconstructed, not seen live) | "Detected a custom API key" or "Do you want to use this API key"; checked before every answerable dialog (its menu is `1. Yes / 2. No`, and Trust sends `1`) | **stop** (`blocked: …`): the seat is closed and nothing is typed — not even `/exit` — the human decides |
| Anthropic Console / login method (reconstructed, not seen live) | "Anthropic Console account" or "Select login method" | **stop** like not logged in |
| SessionStart hook error (seen live) | `SessionStart:startup hook error … No such file or directory` above the prompt | ignored — non-blocking, not a dialog or failure |
| trust this folder | "trust this folder", "trust the files", "do you trust", "trust this directory" | send `1` (no Enter) |
| hook review | "hooks have changed", "review hooks", "hook changes", "new hooks" | **stop** — human decides |
| MCP server approval | "mcp server found in this project", "mcp servers found in this project", "new mcp server" | **stop** — human decides (birth pre-approves `arra` via the home's `.claude/settings.json` → `enabledMcpjsonServers`) |

Ready: a prompt line (`›`, `>` or `❯`) plus a footer (Codex `<model> · <path>`,
Claude `? for shortcuts`) and no menu on screen.

Birth name prompt: each try must show that the agent took the turn (Orca
`turn_started`, a working state, or a non-empty answer after the prompt) and
`wait` must report idle. If the turn never started codename stops with
"agent did not start a turn" instead of re-asking; if the turn started but no
answer appeared yet it waits once more. It re-asks only after a real answer
that is not a usable name.

Every dialog actually seen is appended to
`%USERPROFILE%\.codename\first-run-dialogs.md`.
