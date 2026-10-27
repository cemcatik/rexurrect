//! Resume state: what programs report about themselves, per block, so they
//! can resume (ADR-0005). `rexurrect report` writes it; save reads it.
//!
//! Private to rexurrect, unlike recorder files: only `report` writes it,
//! and its format can change. On disk, in the state directory:
//! - one file per block, `resume/<block ID>`, holding the program that
//!   reported and its value as JSON, so a block whose program changed
//!   (Claude Code, then Codex) never resumes one with the other's state;
//! - written atomically through `state::write_file`, in a private (`0700`)
//!   directory: an agent conversation's ID is as private as a command;
//! - a save that writes a snapshot removes those of blocks that aren't
//!   live, as it does recorder files.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::state::{self, StateDir};

const DIR: &str = "resume";

/// One block's reported resume state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reported {
    /// The name of the program definition it was reported for.
    pub program: String,
    /// The value, as reported and accepted.
    pub state: String,
}

pub struct ResumeStates {
    dir: PathBuf,
}

impl ResumeStates {
    pub fn new(state: &StateDir) -> ResumeStates {
        ResumeStates {
            dir: state.join(DIR),
        }
    }

    /// The block's reported resume state, if any. A file that can't be
    /// parsed counts as none: it only ever costs a resume.
    pub fn read(&self, block_id: &str) -> Result<Option<Reported>> {
        let Some(path) = self.file(block_id) else {
            return Ok(None);
        };
        Ok(state::read_file(&path)?.and_then(|bytes| serde_json::from_slice(&bytes).ok()))
    }

    /// Makes `reported` the block's resume state, replacing any earlier one.
    pub fn write(&self, block_id: &str, reported: &Reported) -> Result<()> {
        let path = self
            .file(block_id)
            .expect("callers check that a block ID is a plain file name");
        state::write_json(&path, reported)
    }

    /// Removes the resume state of blocks `is_live` says aren't live, unless
    /// it changed at or after `kept_since`: a program may have reported it
    /// for a block that appeared since save read the server (see
    /// [`state::remove_files_of_gone_blocks`]).
    pub fn retain_live(
        &self,
        is_live: impl Fn(&str) -> bool,
        kept_since: SystemTime,
    ) -> Result<()> {
        state::remove_files_of_gone_blocks(&self.dir, is_live, kept_since)
    }

    /// Removes the temporary files that crashed writes left behind (see
    /// [`state::remove_stale_temp_files`]).
    pub fn remove_stale_temp_files(&self) -> Result<()> {
        state::remove_stale_temp_files(&self.dir, state::is_temp_name)
    }

    /// The block's file. `None` for an ID that isn't a plain file name, so
    /// no ID can reach outside the directory.
    fn file(&self, block_id: &str) -> Option<PathBuf> {
        state::is_plain_file_name(block_id).then(|| self.dir.join(block_id))
    }
}
