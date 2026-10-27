//! Programs that exited after the save (#60). A block is saved as resumable
//! when save finds its program in its foreground, but the user may quit the
//! program before the server stops, up to an autosave interval later.
//!
//! A recorder may rewrite the block's record, unchanged, each time its
//! shell's prompt comes back (the zsh one does). A record written after the
//! snapshot was taken then means the shell got control back after the save:
//! its program had exited. Restore reads it before anything prunes it (save
//! restores first, and only a successful save prunes the files of blocks
//! that aren't live), and treats the block as if the save had seen its
//! shell: not resumable, with the record as its last command.
//!
//! No record, or one not written since the save, says nothing: the block
//! resumes, as it did before. So does every block of a snapshot taken before
//! save kept block IDs. A program suspended with Ctrl-Z counts as exited:
//! its shell's prompt came back too.

use crate::error::Result;
use crate::recorder::{Recorded, RecorderFiles};
use crate::snapshot::{SavedBlock, SavedSession};
use crate::timestamp::Timestamp;

/// `saved`, from a snapshot taken at `taken_at`, with each resumable block
/// whose recorder file was written after `taken_at` made as the save would
/// have found it with its shell in its foreground: not resumable, and with
/// its record as its last command (none, for a record that it has none).
/// A record that can't be read leaves the saved last command.
pub fn as_left(
    saved: &SavedSession,
    recorder: &RecorderFiles,
    taken_at: Timestamp,
) -> Result<SavedSession> {
    let mut left = saved.clone();
    for block in left.blocks_mut() {
        let Some(block_id) = exited_after(block, recorder, taken_at)? else {
            continue;
        };
        block.resume = None;
        match recorder.read(&block_id)? {
            Recorded::LastCommand(command) => block.last_command = Some(command),
            Recorded::NoLastCommand => block.last_command = None,
            Recorded::Nothing => {}
        }
    }
    Ok(left)
}

/// The block's ID, if `block` was saved as resumable and its recorder file
/// says its shell got its prompt back after `taken_at`.
fn exited_after(
    block: &SavedBlock,
    recorder: &RecorderFiles,
    taken_at: Timestamp,
) -> Result<Option<String>> {
    let (Some(_), Some(block_id)) = (&block.resume, &block.block_id) else {
        return Ok(None);
    };
    let modified = recorder.modified(block_id)?;
    Ok(modified
        .is_some_and(|modified| modified > taken_at.system_time())
        .then(|| block_id.clone()))
}
