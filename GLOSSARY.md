# rexurrect

A standalone tool that saves Rex sessions and recreates them after the Rex server stops. Rex's API and UI name things differently; this glossary uses the API's words.

## Language

### Rex

**Server**:
The Rex process that owns every session on a machine. When it stops, all of its sessions are gone.

**Server instance**:
One run of the server, from start to stop. Rex gives each one its own ID; no session outlives the instance it was created in.
_Avoid_: boot, server session, lifetime

**Build**:
One numbered release of Rex, such as 1203, with its own server and `rex` CLI. Servers of two different builds can run at once.
_Avoid_: version (easily confused with a snapshot)

**Host**:
Another Rex server the user connects to by label.
_Avoid_: remote server, tailnet machine

**Session**:
A group of windows on a server, and the unit rexurrect saves and restores. Its label is only a display name: two sessions can share one, so rexurrect tells sessions apart by lineage, never by label.

**Window**:
One tab in a session, holding one layout.
_Avoid_: tab

**Block**:
One leaf of a layout. Today every block has Rex's shell flavor: a shell block, or a command block.
_Avoid_: pane

**Shell block**:
A block running an interactive terminal shell.
_Avoid_: terminal block

**Command block**:
A block Rex started with a command rather than a shell: the command is its first process, and no shell runs in it. It has a shell block's flavor and is saved like one, and comes back as a shell block with the command as its last command.

**Remote block**:
A block that lives on a host rather than on the local server.

**Layout**:
A window's tree of splits and blocks: how the window's space is divided. Floating layers aren't part of it.

**Split**:
A layout node that divides its space between two child nodes in one direction, at a ratio.

**Label**:
The display name Rex shows for a session, window or block. Labels aren't unique.
_Avoid_: name, title

**Default label**:
The label Rex generates for a window or block nobody labeled: `Window` plus the last 8 hex characters of the window's ID, or `shell` for a shell block. A block whose label was cleared gets `Block` plus the last 8 hex characters of its ID, also a default label. Any other label is user-set. Rex also generates labels for sessions (random word pairs such as `amber-ridge`), but those can't be told apart from user-set ones, so a session never has a default label.

**Working directory**:
The directory a shell block's shell was last in.
_Avoid_: cwd, pwd, last directory

### Saving

**Snapshot**:
A saved description of every live session at one point in time.
_Avoid_: backup, save file

**Latest snapshot**:
The most recently written snapshot, and the one restore uses unless told otherwise.

**Empty server**:
A server with no live sessions worth saving: none at all, or only untouched ones. Saving from one never replaces the latest snapshot.

**Untouched session**:
A session with nothing worth restoring: one window holding one shell block, default window and block labels, its working directory still home, and no last command. Its label doesn't count, so a session that was only renamed is still untouched. Untouched sessions aren't saved.

**Scrollback**:
A shell block's earlier output, saved with a snapshot and printed back above the block's new prompt when it's restored. Three kinds of block have none saved: one showing a full-screen program (on the terminal's alternate screen), one running a program that draws its interface on the main screen (`claude`, by default), and one whose scrollback is too large for Rex to hand over (larger than Rex's limit on one response).
_Avoid_: history (that's the shell's command history), pane contents

**Autosave**:
A save that runs on a timer without the user asking.
_Avoid_: scheduled save, periodic save

**Recorder**:
The hook in a shell block's outermost shell that records each command as it starts, whatever the shell, and may rewrite that record each time the prompt comes back, so restore can tell the shell got control back after a save. Shells nested inside it (`nix develop`, `poetry shell`) don't record. Only shells record last commands: a program reports its resume state instead.

**Ignored command**:
A command the recorder doesn't record, leaving the previous last command in place: a command on the ignore list (by name, or by the last part of its path, as `/bin/ls`) run on its own, rexurrect itself by default, and any multi-line command.
_Avoid_: skip list, skipped command

**Last command**:
The command a restored shell block gets typed at its prompt, never run, unless the block resumes a program: the most recent command the block's recorder recorded. A block the recorder has recorded nothing for (no recorder installed, or its shell started before it was) falls back to the program in its foreground, a command block's own command included (its arguments, each quoted where zsh needs it, and never one with a control character in an argument), and keeps the last such program once it exits; failing that, a restored block keeps the last command it was restored with, or none. That way it carries forward across any number of server instances. rexurrect itself running in a block is never its last command.
_Avoid_: last history entry

### Restoring

**Live session**:
A session that exists on the server right now.
_Avoid_: existing session, current session

**Saved session**:
A session as recorded in a snapshot.

**Restored session**:
A live session that restore created from a saved session. It belongs to the saved session's lineage, whatever it's labeled.

**Lineage**:
A session plus every session restore has created from it, across any number of restarts. Only one session of a lineage is ever live.
_Avoid_: version (easily confused with a snapshot)

**Lineage map**:
rexurrect's record of which lineage each restored session belongs to. A live session missing from it starts a lineage of its own.
_Avoid_: lineage (that's the chain of sessions), lineage file

**Startup window**:
The first two minutes after restore creates a session, while its blocks' shells may still be starting. A save in it saves the session's blocks as restore created them, except a block the recorder has recorded a command in since.

**Skipped session**:
A saved session that restore doesn't create because a session of its lineage is already live. A live session with the same label but a different lineage doesn't count.

**Unrestored server instance**:
A server instance rexurrect hasn't restored into yet. Save restores before saving from one, so a freshly started server never replaces the latest snapshot.

**Auto-restore**:
A restore that runs by itself once per server instance, as soon as the server is ready.

**App session**:
A session the Rex app creates on its own when it finds the server with no sessions: an untouched session with a label Rex generated. Auto-restore closes one while it's still untouched.
_Avoid_: extra session, empty session, default session

### Resuming

**Resume**:
Bringing a program back in a restored block as it was, not just started again: its Neovim session, its agent conversation. Only a program that was in the block's foreground at the last save resumes; one that had exited by then doesn't, even if it's still the block's last command, and neither does one whose block's recorder has rewritten its record since that save (the program exited before the server stopped). Restore types the program's resume command at the block's prompt in place of its last command, or runs it when the program auto-resumes.
_Avoid_: restore (that's sessions), reopen, relaunch

**Program definition**:
How rexurrect resumes one program: which foreground program it matches, what resume state (if any) the program reports, and the command that resumes it, built from that state. A few ship built in; the user can override them or add their own.
_Avoid_: plugin (that's the zsh plugin, or a Neovim or Rex plugin), strategy, integration

**Auto-resume**:
A resume whose command restore runs rather than types. Only programs the user has listed auto-resume; by default, none do. A listed program that can't resume (it reported no resume state, or its Neovim session isn't there) still comes back running: restore runs the program's own command line as it was at the save, never the block's last command, which may only have started it (`make edit`, `git commit`). A definition can opt out of that (`fallback = "none"`): its program then gets the block's last command typed, as if it weren't listed, never run.
_Avoid_: auto-start, allow-list (bare)

**Resume state**:
What a program reports to rexurrect about itself, per block, so that it can resume: an agent conversation's ID, or the path of its session file. It's a value, never a command: rexurrect builds the command from it, and accepts only the kind of value the program's definition expects. A program that keeps its own state (Neovim, through its Neovim session) reports none.
_Avoid_: program recorder, record (that's the recorder's)

**Neovim session**:
Neovim's own record of its buffers, windows, tab pages and directory, which reopens them when Neovim loads it. Neovim, or a plugin in it, keeps it; rexurrect only has Neovim load it when Neovim resumes. It has nothing to do with a **Session**.
_Avoid_: session (bare), editor session

**Agent conversation**:
A coding agent's conversation (Claude Code's, Codex's, pi's), which the agent can reopen by its ID or its session file. The agents call it a session; it has nothing to do with a **Session** either.
_Avoid_: agent session, session (bare)
