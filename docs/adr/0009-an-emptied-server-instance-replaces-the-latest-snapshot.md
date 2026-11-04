# An emptied server instance replaces the latest snapshot

On an **emptied server instance** (one rexurrect restored into, at least in part, or has saved from since a failed restore), a save from an empty server writes a snapshot with no sessions and points latest at it, like any other snapshot: retention, upkeep of what's no longer live, and scrollback cleanup all run. The next restore, auto-restore or restore-first save then creates nothing. Before this, a save from an empty server never wrote anything, on any server instance. So closing all but one session was respected, but closing every session wasn't: the next start brought them all back. Settled in issue #93.

The empty-server guard (#8) came first and stood in for a signal rexurrect didn't have then: whether the server instance was fresh or one the user had been working in. #15 added that signal. A save on an unrestored server instance restores first, so on any instance a save reaches without restoring, rexurrect has restored into it, and an empty server there means the user closed its sessions. The guard only still matters on the instances #15 doesn't cover:

- **An unrestored server instance**: a save restores first, as before, so the guard is never reached there with a snapshot to lose.
- **An instance whose restore failed**: a save that restores first and finds every session failed still records the instance (#15), so that later saves don't restore first and fail again. Nothing was restored there, so the user never had the sessions to close. The record now says the restore failed, and the instance becomes emptied only once a snapshot is saved from it. Until then an empty server there keeps the latest snapshot. `restore` and auto-restore don't record an instance whose restore failed, so a save there still restores first.

A restore that finds nothing to restore (no snapshot yet, or a latest snapshot with no sessions) is recorded as before, so that instance is emptied. Restoring a snapshot with no sessions creates nothing, so auto-restore leaves the app session alone (ADR-0004).

No new snapshot with no sessions is written while the latest one already has none. Otherwise autosave, every 15 minutes on an empty server, would push every real snapshot out of retention within hours. That save says nothing changed. A latest snapshot that can't be read doesn't count as having no sessions: the save writes a new one, and logs why, rather than fail every autosave.

This is on by default, with no setting.

## Considered Options

- **An opt-in setting**: closing everything and having it stick is what the user asked for, and nothing on an emptied instance needs protecting. The setting would only keep the surprise for those who don't find it.
- **Remove latest instead of writing a snapshot with no sessions**: then "no latest snapshot" would mean both "never saved" and "the user emptied the server", and restore, `list` and the restore-first save would each have to tell them apart. A snapshot with no sessions needs no new case: restore creates nothing, and `list` shows `(no sessions)`.
- **Accept the failed-restore hole**: treat every recorded instance as emptied, failed restores included. A restart into a Rex build that can't recreate the saved sessions would then let the next autosave on the empty server replace the latest snapshot. That's the loss the guard exists to prevent, on the snapshot the user most needs back.

## Consequences

- The restored-instance record (`restored-instances`) now marks an instance whose restore failed with `restore-failed` after its ID. A rexurrect from before this change reads such a line as an unrestored server instance, so its next save restores first again.
- Earlier snapshots stay listed and restorable with `restore --snapshot <id>` until retention removes them, so an accidental "close everything" can be undone by hand.
- Closing everything sticks only if a save runs while the server is empty. A server that stops when empty (`--exit-empty` with no client connected), or a restart before the next autosave, still brings the sessions back: #33.
