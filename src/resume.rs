//! What restore types at a restored block's prompt (ADR-0005): the resume
//! command of the block's resumable program, if it can resume, else the
//! block's last command. Typed, never run.
//!
//! A block resumes only if save found its program in its foreground
//! ([`SavedBlock::resume`]), its program definition, read now, can build a
//! resume command for it (see [`ProgramDefinition::resume_command`]), and no
//! earlier block of this restore, in layout order, resumes the same program
//! with the same reported state: two agents writing to one conversation
//! would fight over it. Two Neovims loading one `Session.vim` are harmless,
//! so a program that reports no state can resume in any number of blocks.

use std::collections::HashSet;

use crate::create;
use crate::programs::{ProgramDefinition, Programs};
use crate::snapshot::SavedBlock;

/// Decides what restore types at each block's prompt, block by block in
/// layout order, across every session one restore creates.
pub struct Resuming<'a> {
    programs: &'a Programs,
    /// (definition name, state) of each block resumed so far with state.
    resumed: HashSet<(String, String)>,
}

impl Resuming<'_> {
    pub fn new(programs: &Programs) -> Resuming<'_> {
        Resuming {
            programs,
            resumed: HashSet::new(),
        }
    }

    /// What to type at the prompt of `block`, as restore creates it (see
    /// `create::as_created`): its resume command if it can resume, else its
    /// last command if that can be typed, else nothing.
    pub fn input(&mut self, block: &SavedBlock) -> Option<String> {
        self.resume_command(block)
            .or_else(|| create::typed_command(block).map(str::to_string))
    }

    fn resume_command(&mut self, block: &SavedBlock) -> Option<String> {
        let resume = block.resume.as_ref()?;
        let definition: &ProgramDefinition = self.programs.get(&resume.program)?;
        let command =
            definition.resume_command(&block.working_directory, resume.state.as_deref())?;
        if let Some(state) = &resume.state
            && !self
                .resumed
                .insert((definition.name.clone(), state.clone()))
        {
            return None;
        }
        Some(command)
    }
}
