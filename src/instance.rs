//! Server instances and the restore lock.
//!
//! A server instance is one run of the Rex server, told apart by the
//! `instance_id` in `server.status`. rexurrect remembers the instances it
//! restored into, so it can tell a freshly started server (an unrestored
//! server instance) from one the user has been working in. It remembers
//! several, not just the last: another server can restore too (one started
//! by hand that runs the user's init.lua, auto-restore included), and that
//! mustn't make the server the user works in look unrestored.
//!
//! The restore lock lets one restore run at a time. Without it, two restores
//! (auto-restore and the restore a save starts first, say) could both create
//! the same session before either records its lineage (ADR-0001). A save
//! takes it too, so it never captures a half-restored server. The restored
//! instances are only read and written through the lock, so "is this
//! instance restored? if not, restore it and record it" can't interleave
//! with another process doing the same.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use crate::error::{Context, Result};
use crate::state::{self, StateDir};

const LOCK: &str = "restore.lock";
/// The restored instances, one per line, oldest first: an instance's ID,
/// followed by [`FAILED`] for [`Restored::Failed`]. Before #93 the file held
/// only the IDs of instances restored into.
const RESTORED: &str = "restored-instances";
/// What follows a [`Restored::Failed`] instance's ID in [`RESTORED`].
const FAILED: &str = "restore-failed";
/// How many restored instances to remember. Only the instances of servers
/// still running matter, and few run at once.
const REMEMBERED: usize = 32;

/// The restore lock, held until dropped (or until the process exits, however
/// it exits: it's an advisory `flock` on a file in the state directory).
#[derive(Debug)]
pub struct RestoreLock {
    _file: File,
    restored: PathBuf,
}

impl RestoreLock {
    /// Takes the restore lock, waiting for as long as another rexurrect
    /// process holds it. Save and restore both start here, so this is also
    /// where the state directory is created, or made, private.
    pub fn acquire(state: &StateDir) -> Result<RestoreLock> {
        state.make_private()?;
        let path = state.join(LOCK);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("opening the restore lock {}", path.display()))?;
        file.lock()
            .with_context(|| format!("taking the restore lock {}", path.display()))?;
        Ok(RestoreLock {
            _file: file,
            restored: state.join(RESTORED),
        })
    }

    /// What has been recorded about restoring into `instance_id`.
    pub fn restored(&self, instance_id: &str) -> Result<Restored> {
        Ok(self
            .restored_instances()?
            .into_iter()
            .find(|(id, _)| id == instance_id)
            .map_or(Restored::No, |(_, restored)| restored))
    }

    /// Whether a save should no longer restore into `instance_id` first: a
    /// restore into it finished, or a save recorded one that failed.
    pub fn is_restored(&self, instance_id: &str) -> Result<bool> {
        Ok(self.restored(instance_id)? != Restored::No)
    }

    /// Records that a restore into `instance_id` finished, at least in part,
    /// or that a snapshot was saved from it since a failed one.
    pub fn record_restored(&self, instance_id: &str) -> Result<()> {
        self.record(instance_id, Restored::Yes)
    }

    /// Records that every session a save tried to restore into `instance_id`
    /// first failed (see [`Restored::Failed`]).
    pub fn record_restore_failed(&self, instance_id: &str) -> Result<()> {
        self.record(instance_id, Restored::Failed)
    }

    /// Replaces `instance_id`'s record, forgetting the oldest instance once
    /// there are more than it remembers.
    fn record(&self, instance_id: &str, restored: Restored) -> Result<()> {
        let mut records = self.restored_instances()?;
        records.retain(|(id, _)| id != instance_id);
        records.push((instance_id.to_string(), restored));
        let kept = &records[records.len().saturating_sub(REMEMBERED)..];
        let contents: String = kept
            .iter()
            .map(|(id, restored)| match restored {
                Restored::Yes => format!("{id}\n"),
                Restored::Failed => format!("{id} {FAILED}\n"),
                Restored::No => unreachable!("only restores are recorded"),
            })
            .collect();
        state::write_file(&self.restored, contents.as_bytes())
    }

    /// Every instance's record, oldest first.
    fn restored_instances(&self) -> Result<Vec<(String, Restored)>> {
        let bytes = state::read_file(&self.restored)?.unwrap_or_default();
        Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| match line.split_once(' ') {
                Some((id, FAILED)) => (id.to_string(), Restored::Failed),
                _ => (line.to_string(), Restored::Yes),
            })
            .collect())
    }
}

/// What rexurrect has recorded about restoring into a server instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restored {
    /// Nothing: an unrestored server instance. A save restores first.
    No,
    /// A restore into it finished, at least in part, or found nothing to
    /// restore; or a snapshot was saved from it since a failed one. An
    /// empty server on it is an emptied server instance (ADR-0009).
    Yes,
    /// A save that restored first found every session it tried failed. It
    /// doesn't restore first again, but until a snapshot is saved from the
    /// instance, an empty server there doesn't replace the latest snapshot:
    /// the user never had the sessions to close.
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state directory of its own, removed when dropped.
    struct TempState(StateDir);

    impl TempState {
        fn new(name: &str) -> TempState {
            let root = std::env::temp_dir()
                .join(format!("rexurrect-instance-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            TempState(StateDir::at(root))
        }
    }

    impl Drop for TempState {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.path());
        }
    }

    #[test]
    fn an_instance_nothing_was_recorded_for_is_unrestored() {
        let state = TempState::new("unrestored");
        let lock = RestoreLock::acquire(&state.0).unwrap();
        assert_eq!(lock.restored("i1").unwrap(), Restored::No);
        assert!(!lock.is_restored("i1").unwrap());
    }

    #[test]
    fn an_instance_whose_restore_first_failed_counts_as_restored_but_not_as_restored_into() {
        let state = TempState::new("failed");
        let lock = RestoreLock::acquire(&state.0).unwrap();
        lock.record_restored("i1").unwrap();
        lock.record_restore_failed("i2").unwrap();
        assert_eq!(lock.restored("i1").unwrap(), Restored::Yes);
        assert_eq!(lock.restored("i2").unwrap(), Restored::Failed);
        assert!(
            lock.is_restored("i2").unwrap(),
            "a save would restore first"
        );
    }

    /// Once a snapshot is saved from an instance whose restore failed, it's
    /// recorded as restored.
    #[test]
    fn recording_an_instance_whose_restore_failed_as_restored_replaces_its_record() {
        let state = TempState::new("upgraded");
        let lock = RestoreLock::acquire(&state.0).unwrap();
        lock.record_restore_failed("i1").unwrap();
        lock.record_restored("i1").unwrap();
        assert_eq!(lock.restored("i1").unwrap(), Restored::Yes);
    }

    /// Before #93, the record held only the IDs of instances restored into.
    #[test]
    fn a_record_written_before_failed_restores_were_recorded_reads_as_restored() {
        let state = TempState::new("old-format");
        state::write_file(&state.0.join(RESTORED), b"i1\ni2\n").unwrap();
        let lock = RestoreLock::acquire(&state.0).unwrap();
        assert_eq!(lock.restored("i1").unwrap(), Restored::Yes);
        assert_eq!(lock.restored("i2").unwrap(), Restored::Yes);
    }
}
