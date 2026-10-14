# Screen fixtures

Rendered agent screens used by `src/seat.rs` tests.

| file | origin |
|---|---|
| `codex-update-dialog.txt` | verbatim from Codex 0.146.1 on Windows (live test of codename 0.1.1) |
| `codex-update-banner-ready.txt` | the first 3 banner lines are verbatim; the rest of the ready screen (prompt `›` + `<model> · <path>` footer) is reconstructed |
| `claude-not-logged-in.txt` | the line `Not logged in · Run /login` and the `API Usage Billing` header are verbatim; layout reconstructed |
| `claude-sessionstart-hook-error.txt` | only the phrases `SessionStart:startup hook error` and `No such file or directory` are from the live test; the rest of the line and the layout are reconstructed |
| `codex-sign-in.txt` | reconstructed (best effort) — not seen live |
| `claude-custom-api-key.txt` | reconstructed (not seen live) — Claude Code's "Detected a custom API key … Do you want to use this API key?" menu; the key line is a placeholder, never a real key shape |
| `claude-login-method.txt` | reconstructed (not seen live) — Claude Code's "Select login method" screen with the Anthropic Console option |
