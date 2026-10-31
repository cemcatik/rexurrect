# Replace the sessions Rex carried over

Rex builds with API revision 9 bring a server's sessions back by themselves when it starts. The sessions keep their IDs and their blocks' IDs, but not their scrollback or their programs. Every carried-over session then matches its saved session's lineage (ADR-0001), so restore skipped them all, counted the server instance as restored, and the next save replaced the latest snapshot with the bare copies. rexurrect now **replaces** them: it creates the saved session from the snapshot through the normal restore path, then closes Rex's copy, creating before closing so that a failed create leaves Rex's copy in place. Auto-restore does this by default, with no setting, but only when init.lua ran at server start (`server.status` `config.generation` is 1), never on a `rex config reload`. `restore --force` does the same on request. Neither tries to recognise Rex's copies: at server start auto-restore owns the server, and `--force` is the user saying so. Only save, which runs in the background whenever its timer fires, looks for a carried-over session: a live block that's also in a snapshot taken before the server instance started. When it finds one, it backs off without restoring or saving, so an existing snapshot is never replaced with Rex's bare copies. A plain `restore` skips carried-over sessions without counting the instance as restored. Settled in issue #82.

## Considered Options

- **Turn Rex's restore off**: `rex server run --restore=false` does it, but it's only a command-line flag. The Rex app (macOS) and the login server (Linux) start the server with arguments the user doesn't choose, and there's no init.lua or environment setting. We're not asking Rex for one.
- **Fill in Rex's blocks**: the block IDs line up with rexurrect's per-block state, but Rex has no way to print output into a running block (`com.superlogical.terminal.write` types input), so scrollback could only come back as a replay command typed at every prompt, left in the block and its shell history. Restore prints scrollback only because it creates each block running a replay script first.
- **Rebuild the windows inside Rex's session**: keeps the session ID, which lineage already makes unnecessary, at the cost of a second way of building layouts beside session creation.
- **Create beside Rex's copies**: leaves a duplicate of every session to close by hand after each restart.
- **A setting for auto-restore, or `--force` only by hand**: with Rex carrying sessions over, auto-restore without replacing does nothing, and by the time the user remembers to run `--force`, an autosave has replaced the latest snapshot.
- **Replace from save's restore-first too**: save can't tell a server start from a server that's been running for hours. After `restore --session work` (which doesn't count as restoring the instance), the next autosave would replace `work` while the user works in it.
- **Recognise Rex's copies by their IDs' age**: Rex's IDs happen to be UUIDv7, so a session older than `started_at` was carried over, but Rex doesn't document the format.

## Consequences

- Auto-restore closes sessions the user can already see in the Rex app, seconds after the server starts. Anything typed into one before then is lost.
- `restore --force` run from inside a session it replaces closes that session last, after recording the restore and writing its report to `rexurrect.log`, so the terminal it ran in disappears.
- On a server without auto-restore, autosaves pause after a restart (each logging why) until the user runs `restore --force`.
- Replaced sessions get new session and block IDs, as every restored session does.
- Rex starts a new shell in each block it carries over, and the zsh recorder rewrites the block's record at that shell's first prompt. So restoring from a snapshot taken in an earlier server instance ignores records written after the instance started when it asks whether a program exited after the save (#60): otherwise no replaced block would resume its program.
- If Rex refuses to close its copy, both sessions stay live, against lineage's one live session (ADR-0001). That's left as a note on the replaced session rather than a failure, for the user to close by hand.
- A Rex build that doesn't carry sessions over behaves as before: there's nothing live to replace.
