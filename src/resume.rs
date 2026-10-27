//! What restore types at a restored block's prompt (ADR-0005): the resume
//! command of the block's resumable program, if it can resume, else the
//! block's last command. Typed, never run.
//!
//! A block resumes only if save found its program in its foreground
//! ([`SavedBlock::resume`]), and its program definition, read now, can
//! build a resume command for it (see [`ProgramDefinition::resume_command`]).

use crate::create;
use crate::programs::{ProgramDefinition, Programs};
use crate::snapshot::SavedBlock;

/// Decides what restore types at each block's prompt.
pub struct Resuming<'a> {
    programs: &'a Programs,
}

impl Resuming<'_> {
    pub fn new(programs: &Programs) -> Resuming<'_> {
        Resuming { programs }
    }

    /// What to type at the prompt of `block`, as restore creates it (see
    /// `create::as_created`): its resume command if it can resume, else its
    /// last command if that can be typed, else nothing.
    pub fn input(&self, block: &SavedBlock) -> Option<String> {
        self.resume_command(block)
            .or_else(|| create::typed_command(block).map(str::to_string))
    }

    fn resume_command(&self, block: &SavedBlock) -> Option<String> {
        self.definition(block)?
            .resume_command(&block.working_directory)
    }

    /// The program definition `block`'s resumable program matched at the
    /// save, if it still exists.
    fn definition(&self, block: &SavedBlock) -> Option<&ProgramDefinition> {
        self.programs.get(&block.resume.as_ref()?.program)
    }
}
