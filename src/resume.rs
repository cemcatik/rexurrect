//! What restore types at a restored block's prompt (ADR-0005): the resume
//! command of the block's resumable program, if it can resume, else the
//! block's last command. Typed, never run, unless the program auto-resumes.
//!
//! A block resumes only if save found its program in its foreground
//! ([`SavedBlock::resume`]), its program definition, read now, can build a
//! resume command for it (see [`ProgramDefinition::resume_command`]), and no
//! earlier block of this restore, in layout order, resumes the same program
//! with the same reported state: two agents writing to one conversation
//! would fight over it. Two Neovims loading one `Session.vim` are harmless,
//! so a program that reports no state can resume in any number of blocks.
//!
//! A program listed in `auto_resume` auto-resumes (ADR-0006): its resume
//! command is typed followed by Enter, so the block's shell runs it, records
//! it, and is there when it exits. One that can't resume still comes back
//! running: its own command line as it was at the save, never the block's
//! last command, which may only have started it (`make edit`).

use std::collections::HashSet;

use crate::create;
use crate::programs::Programs;
use crate::snapshot::SavedBlock;

/// What restore types at a block's prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Typed, for the user to run.
    Typed(String),
    /// Typed followed by Enter: the block's shell runs it.
    Run(String),
}

impl Input {
    /// The command, which becomes the block's default last command.
    pub fn command(&self) -> &str {
        match self {
            Input::Typed(command) | Input::Run(command) => command,
        }
    }

    /// What Rex writes to the block's terminal (its `initial_input`).
    pub fn keystrokes(&self) -> String {
        match self {
            Input::Typed(command) => command.clone(),
            Input::Run(command) => format!("{command}\n"),
        }
    }
}

/// Decides what restore types at each block's prompt, block by block in
/// layout order, across every session one restore creates.
pub struct Resuming<'a> {
    programs: &'a Programs,
    /// The definitions whose programs auto-resume.
    auto_resume: &'a [String],
    /// (definition name, state) of each block resumed so far with state.
    resumed: HashSet<(String, String)>,
}

impl<'a> Resuming<'a> {
    pub fn new(programs: &'a Programs, auto_resume: &'a [String]) -> Resuming<'a> {
        Resuming {
            programs,
            auto_resume,
            resumed: HashSet::new(),
        }
    }

    /// What to type at the prompt of `block`, as restore creates it (see
    /// `create::as_created`): its resume command if it can resume, run if
    /// its program auto-resumes; else, for a program that auto-resumes, its
    /// command line from the save, run; else its last command if that can
    /// be typed; else nothing.
    pub fn input(&mut self, block: &SavedBlock) -> Option<Input> {
        let auto_resumes = block
            .resume
            .as_ref()
            .is_some_and(|resume| self.auto_resume.contains(&resume.program));
        let typed_or_run = |command: String| {
            if auto_resumes {
                Input::Run(command)
            } else {
                Input::Typed(command)
            }
        };
        if let Some(command) = self.resume_command(block) {
            return Some(typed_or_run(command));
        }
        let fallback = block
            .resume
            .as_ref()
            .and_then(|resume| resume.command_line.as_deref())
            .filter(|command| auto_resumes && create::is_typeable(command));
        match fallback {
            Some(command) => Some(Input::Run(command.to_string())),
            None => create::typed_command(block).map(|command| Input::Typed(command.to_string())),
        }
    }

    fn resume_command(&mut self, block: &SavedBlock) -> Option<String> {
        let resume = block.resume.as_ref()?;
        let definition = self.programs.get(&resume.program)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::SavedResume;

    fn block(last_command: &str, program: &str, state: Option<&str>, line: &str) -> SavedBlock {
        SavedBlock {
            label: None,
            working_directory: "/nonexistent-rexurrect-dir".into(),
            last_command: Some(last_command.to_string()),
            scrollback: None,
            resume: Some(SavedResume {
                program: program.to_string(),
                state: state.map(str::to_string),
                command_line: Some(line.to_string()),
            }),
        }
    }

    fn inputs(auto_resume: &[&str], blocks: &[SavedBlock]) -> Vec<Option<Input>> {
        let programs = Programs::default();
        let auto_resume: Vec<String> = auto_resume.iter().map(|s| s.to_string()).collect();
        let mut resuming = Resuming::new(&programs, &auto_resume);
        blocks.iter().map(|b| resuming.input(b)).collect()
    }

    fn typed(command: &str) -> Option<Input> {
        Some(Input::Typed(command.to_string()))
    }

    fn run(command: &str) -> Option<Input> {
        Some(Input::Run(command.to_string()))
    }

    #[test]
    fn a_program_that_can_resume_is_typed_unless_listed_then_run() {
        let claude = block("claude", "claude", Some("abc"), "claude --model opus");
        assert_eq!(
            inputs(&[], std::slice::from_ref(&claude)),
            [typed("claude --resume abc")]
        );
        assert_eq!(inputs(&["claude"], &[claude]), [run("claude --resume abc")]);
    }

    /// Never the last command, which may only have started the program.
    #[test]
    fn a_listed_program_that_cant_resume_runs_its_own_command_line() {
        let nvim = block("make edit", "nvim", None, "nvim src/main.rs");
        assert_eq!(
            inputs(&["nvim"], std::slice::from_ref(&nvim)),
            [run("nvim src/main.rs")]
        );
        assert_eq!(inputs(&[], &[nvim]), [typed("make edit")]);
    }

    #[test]
    fn a_listed_duplicate_runs_its_own_command_line() {
        let claude = block("claude", "claude", Some("abc"), "claude");
        assert_eq!(
            inputs(&["claude"], &[claude.clone(), claude]),
            [run("claude --resume abc"), run("claude")]
        );
    }

    #[test]
    fn a_listed_program_without_a_command_line_to_run_gets_its_last_command_typed() {
        let mut claude = block("claude", "claude", None, "x");
        claude.resume.as_mut().unwrap().command_line = None;
        assert_eq!(inputs(&["claude"], &[claude]), [typed("claude")]);
    }

    #[test]
    fn keystrokes_end_in_enter_only_for_a_command_to_run() {
        assert_eq!(Input::Typed("a".into()).keystrokes(), "a");
        assert_eq!(Input::Run("a".into()).keystrokes(), "a\n");
    }
}
