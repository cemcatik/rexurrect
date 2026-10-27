//! Program definitions: how rexurrect resumes a program (ADR-0005).
//!
//! A definition says which foreground programs it matches, what kind of
//! resume state the program reports, the command that resumes it, built
//! from that state, and a file that must be in the block's working
//! directory for it to resume. Save records a block whose foreground
//! program matches one as resumable; restore builds its resume command.
//!
//! Built in: Neovim, which reports no state and resumes from vim-obsession's
//! `Session.vim`, as tmux-resurrect's `session` strategy does.

use std::path::Path;

use crate::rex::api::TerminalProcess;

/// The kind of resume state a program reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    /// None: the program keeps its own (Neovim, through its Neovim session).
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramDefinition {
    /// The definition's name, which a snapshot records.
    pub name: String,
    /// The program names it matches, by any name a foreground program goes
    /// by (see [`crate::rex::api::ProcessInfo::names`]).
    pub matches: Vec<String>,
    pub state: StateKind,
    /// The resume command.
    pub resume: String,
    /// A file that must exist in the block's working directory at restore,
    /// else the program can't resume.
    pub requires_file: Option<String>,
}

impl ProgramDefinition {
    /// The command that resumes the program in a block whose working
    /// directory is `dir`, or `None` if it can't resume there: its required
    /// file is missing, or the command can't be typed (see
    /// [`crate::create::is_typeable`]).
    pub fn resume_command(&self, dir: &Path) -> Option<String> {
        if let Some(file) = &self.requires_file
            && !dir.join(file).is_file()
        {
            return None;
        }
        let command = match self.state {
            StateKind::None => self.resume.clone(),
        };
        crate::create::is_typeable(&command).then_some(command)
    }
}

/// Every program definition.
#[derive(Debug, Clone)]
pub struct Programs {
    definitions: Vec<ProgramDefinition>,
}

impl Default for Programs {
    /// The built-in definitions.
    fn default() -> Programs {
        Programs {
            definitions: vec![ProgramDefinition {
                name: "nvim".to_string(),
                matches: vec!["nvim".to_string()],
                state: StateKind::None,
                resume: "nvim -S".to_string(),
                requires_file: Some("Session.vim".to_string()),
            }],
        }
    }
}

impl Programs {
    /// The definition named `name`.
    pub fn get(&self, name: &str) -> Option<&ProgramDefinition> {
        self.definitions.iter().find(|d| d.name == name)
    }

    /// The definition matching the program in the foreground of the block
    /// whose terminal `process` method answered `process`, by any name it
    /// goes by, if there is one: the first that matches.
    pub fn matching(&self, process: &TerminalProcess) -> Option<&ProgramDefinition> {
        let program = process.foreground_program()?;
        self.definitions.iter().find(|definition| {
            program
                .names()
                .any(|name| definition.matches.iter().any(|m| m == name))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nvim() -> ProgramDefinition {
        Programs::default().get("nvim").unwrap().clone()
    }

    #[test]
    fn neovim_resumes_with_nvim_dash_s_where_a_session_vim_is() {
        let dir = std::env::temp_dir().join(format!("rexurrect-programs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(nvim().resume_command(&dir), None);
        std::fs::write(dir.join("Session.vim"), "").unwrap();
        assert_eq!(nvim().resume_command(&dir).as_deref(), Some("nvim -S"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
