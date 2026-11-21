# Leave excluded sessions to the tools that make them

Some sessions are made and kept up by another tool, not by the user: tr3x keeps one, **T3 Code**, in step with T3 Code, one window per thread, each a command block running `tr3x chat`. The tool remembers the session by its ID and remakes whatever's missing. At a server start, auto-restore replaced it like any carried-over session (ADR-0008): it created a copy with new IDs, whose blocks came back as shells with `tr3x chat …` typed at the prompt, and closed Rex's copy. The tool then found its session gone and made another, so every restart left two. Restoring such a session can't help: its blocks only make sense running the tool's programs, and the tool is still around to remake them.

A new setting, `exclude_sessions`, lists the labels of the sessions rexurrect leaves alone, each `*` matching any run of characters: `["T3 Code", "T3 Code · *"]`. A pattern needs some text other than `*`: `"*"` would exclude every session, so that each save on a restored server instance replaced the latest snapshot with an empty one (ADR-0009). Save doesn't save an excluded session. Restore, auto-restore and `restore --force` included, neither creates nor replaces one, from any snapshot, even one taken before it was excluded, and leaves alone a live session whose own label is excluded though its saved session's isn't, as when a tool takes over a session, and reports it as `excluded  <label>  (exclude_sessions)`, counted with the skipped ones. Save's check for carried-over sessions ignores excluded live sessions and the blocks of excluded saved sessions, so a snapshot from before the exclusion doesn't make every autosave back off. Nothing is excluded by default. Settled in issues #111 and #113.

## Considered Options

- **Recognise managed sessions by their blocks**: leave alone a carried-over session whose blocks are all command blocks, since a command block comes back as a shell. It needs no setting, but it changes what every user gets, and command blocks the user started by hand (`rex run`) would stop being replaced, which is the case ADR-0008 exists for.
- **Exclude by block label or command**: robust to a renamed session, but the tools that need this today label their sessions consistently, and a label is what the user sees in Rex and can write down.
- **A marker in Rex** (session metadata a tool sets): the cleanest signal, but Rex has nowhere to store it.
- **Match labels exactly, without patterns**: tr3x labels one session per machine `T3 Code · <machine>` in one of its modes, which would need a line per machine.
- **Exclude by label** (chosen): the user names what to leave alone, by what Rex shows.

## Consequences

- This narrows ADR-0008: auto-restore and `restore --force` no longer replace every carried-over session, and save's back-off no longer counts every one. An excluded session is left to its tool even when the user says `--force`.
- Labels aren't identity (ADR-0001), and this doesn't make them one: an exclusion is a filter the user writes, applied before lineage matching. A session of the user's own that happens to share an excluded label is excluded too.
- An excluded session's lineage stays in the lineage map until upkeep finds it no longer live, as for any session.
- Excluding a session doesn't remove it from existing snapshots; restoring from them skips it all the same. Removing it from `exclude_sessions` brings it back at the next restore that finds it missing.
