//! Programs that exited after the save (#60). A block is saved as resumable
//! when save finds its program in its foreground, but the user may quit the
//! program before the server stops, up to an autosave interval later.
//!
//! A recorder may rewrite the block's record with the same command each
//! time its shell's prompt comes back (the zsh one does). Save keeps when
//! the record was last written, read just after it found the program
//! ([`SavedResume::record_written_at`]). A record written since then means
//! the shell got control back after the save: its program had exited.
//! Restore reads the record before anything prunes it (save restores first,
//! and only a successful save prunes the files of blocks that aren't live),
//! and treats the block as if the save had seen its shell: not resumable,
//! with the record as its last command.
//!
//! Only that is evidence. A block with no record at the save (no recorder,
//! or one restore created and nothing has run in), a record not written
//! since, one that's gone, or one that can't be read says nothing: the
//! block resumes, as it did before. So does every block of a snapshot taken
//! before #60. A program suspended with Ctrl-Z counts as exited: its shell's
//! prompt came back too.
//!
//! Nor does a record written once the server instance restore runs in had
//! started, for a snapshot taken in an earlier one. The block can only have
//! been live since then as a carried-over block (ADR-0008), in which Rex
//! started a new shell, whose first prompt rewrote the record.
//!
//! [`SavedResume::record_written_at`]: crate::snapshot::SavedResume::record_written_at

use crate::recorder::{Recorded, RecorderFiles};
use crate::snapshot::{SavedBlock, SavedSession};
use crate::timestamp::Timestamp;

/// `saved` with each block whose program exited after the save (see the
/// module docs) made as the save would have found it with its shell in its
/// foreground: not resumable, with its record as its last command, or none
/// if the record says it has none. A record that can't be read leaves the
/// saved last command. Records written at or after `before`, when it's
/// given, say nothing.
pub fn without_exited_programs(
    saved: &SavedSession,
    recorder: &RecorderFiles,
    before: Option<Timestamp>,
) -> SavedSession {
    let mut without = saved.clone();
    for block in without.blocks_mut() {
        let Some(block_id) = exited_program_block_id(block, recorder, before) else {
            continue;
        };
        block.resume = None;
        match recorder.read(&block_id) {
            Ok(Recorded::LastCommand(command)) => block.last_command = Some(command),
            Ok(Recorded::NoLastCommand) => block.last_command = None,
            Ok(Recorded::Nothing) | Err(_) => {}
        }
    }
    without
}

/// The ID of `block`, if save found a program in its foreground and its
/// record has been written since save read it (and before `before`).
fn exited_program_block_id(
    block: &SavedBlock,
    recorder: &RecorderFiles,
    before: Option<Timestamp>,
) -> Option<String> {
    let read_at = block.resume.as_ref()?.record_written_at?;
    let block_id = block.block_id.as_ref()?;
    let written_at = Timestamp::from_system_time(recorder.modified(block_id).ok()??);
    let in_time = before.is_none_or(|before| written_at < before);
    (written_at > read_at && in_time).then(|| block_id.clone())
}
