# Ask before restoring into a restarted server

With `REXURRECT_ASK` set, nothing restores into a new server instance until the user chooses. Save's restore-first and auto-restore both leave a waiting server instance alone; save also saves nothing from it, so the snapshot on offer stays the latest. `rexurrect ask`, which the zsh recorder runs once at a block's first prompt, offers restore, start fresh or later. Starting fresh records the instance as restored into, which is all the existing code needs to save what's live from then on. Without the setting nothing changes: restoring stays silent and automatic.

## Considered Options

- **Gate only auto-restore**: save restores first on an unrestored server instance, and autosave runs save every 15 minutes, so the sessions would come back unasked within one interval.
- **Let save carry on saving while waiting**: it would replace the snapshot on offer with what's live (often nothing worth saving, which writes nothing, but any new session the user opened is), so "restore" would offer a different snapshot from the one the restart lost.
- **A native dialog from auto-restore** (`osascript`): it shows even before a block opens, but auto-restore runs from init.lua with no terminal and its output discarded, and whether a dialog from a process the server started can show depends on macOS privacy settings. A prompt in the first block's shell needs neither, and the user is looking at it.
- **Hold the restore lock while asking**: every save (autosave included) would queue behind an unanswered prompt. `ask` takes the lock only once answered, and acts only if nothing chose in the meantime; a second lock keeps two new blocks from asking at once.
- **On by default**: changes what existing users get after every restart. Opt-in keeps the current behaviour.
- **Ask before restoring, opt-in** (chosen).

## Consequences

- While a server instance waits, nothing is saved: work done before choosing isn't protected by autosave. Every new block asks again, and `ask`, `restore` and `fresh` choose from any shell.
- A server instance with nothing to restore (no snapshot, or every saved session live) never waits: it's restored into as before, which creates nothing.
- `ask` never restores without asking, whether or not the setting is on; without a terminal it does nothing.
- Starting fresh leaves the declined snapshot in `list` until retention removes it, so `restore --snapshot <id>` can still bring it back for a while.
- `autosave install` pins `REXURRECT_ASK`, like the other settings save reads.
