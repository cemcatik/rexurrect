//! Program definitions: how rexurrect resumes a program (ADR-0005).
//!
//! A definition says which foreground programs it matches, what kind of
//! resume state the program reports, the command that resumes it, built
//! from that state, and a file that must be in the block's working
//! directory for it to resume. Save records a block whose foreground
//! program matches one as resumable; restore builds its resume command.
//!
//! Built in:
//! - Neovim, which reports no state and resumes from vim-obsession's
//!   `Session.vim`, as tmux-resurrect's `session` strategy does;
//! - Claude Code, which reports its agent conversation's ID from a
//!   `SessionStart` hook (see `report`).
//!
//! A reported value is never a command: it only ever stands for
//! [`STATE_PLACEHOLDER`] in a definition's resume command, after it passes
//! [`StateKind::accepts`], quoted for zsh where it needs it.

use std::path::Path;

use crate::create;
use crate::foreground;
use crate::rex::api::TerminalProcess;

/// What stands for the reported value in a resume command.
pub const STATE_PLACEHOLDER: &str = "{state}";

/// The longest ID a program can report.
const MAX_ID_LEN: usize = 256;
/// The longest path a program can report.
const MAX_PATH_LEN: usize = 4096;

/// The kind of resume state a program reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    /// None: the program keeps its own (Neovim, through its Neovim session).
    None,
    /// An ID, such as an agent conversation's.
    Id,
    /// The absolute path of a file, such as an agent conversation's session
    /// file.
    #[allow(dead_code)] // No built-in reports one; user definitions will (#57).
    Path,
}

impl StateKind {
    /// What a value of this kind is, as [`StateKind::accepts`] has it, for
    /// messages.
    pub fn described(self) -> &'static str {
        match self {
            StateKind::None => "no value",
            StateKind::Id => "an ID: letters, digits, '-', '_', '.' and ':', not starting with '-'",
            StateKind::Path => "an absolute path without control characters",
        }
    }

    /// Whether `value` is a value of this kind, and so safe to put in a
    /// resume command:
    /// - an ID is letters, digits, `-`, `_`, `.` and `:`, at most
    ///   [`MAX_ID_LEN`] of them, and doesn't start with `-`, so it can't be
    ///   read as an option (`--dangerously-skip-permissions`);
    /// - a path is absolute (so it can't be an option either), at most
    ///   [`MAX_PATH_LEN`] bytes, with no control characters.
    ///
    /// A program with no state accepts nothing.
    pub fn accepts(self, value: &str) -> bool {
        match self {
            StateKind::None => false,
            StateKind::Id => {
                !value.is_empty()
                    && value.len() <= MAX_ID_LEN
                    && !value.starts_with('-')
                    && value
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c))
            }
            StateKind::Path => {
                value.starts_with('/')
                    && value.len() <= MAX_PATH_LEN
                    && !value.contains(char::is_control)
            }
        }
    }
}

/// How a built-in agent's hook passes its resume state: its payload, on
/// `report`'s stdin (see `report`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    /// A Claude Code hook's JSON: the agent conversation's `session_id`,
    /// unless the payload has an `agent_id` (a subagent's).
    ClaudeCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramDefinition {
    /// The definition's name, which a snapshot records and `report` takes.
    pub name: String,
    /// The program names it matches, by any name a foreground program goes
    /// by (see [`crate::rex::api::ProcessInfo::names`]).
    pub matches: Vec<String>,
    pub state: StateKind,
    /// The resume command, a template: [`STATE_PLACEHOLDER`] stands for the
    /// reported value.
    pub resume: String,
    /// A file that must exist in the block's working directory at restore,
    /// else the program can't resume.
    pub requires_file: Option<String>,
    /// What a built-in agent's hook passes `report` on stdin.
    pub payload: Option<Payload>,
}

impl ProgramDefinition {
    /// The command that resumes the program in a block whose working
    /// directory is `dir`, with `state` its reported resume state, or `None`
    /// if it can't resume there: its required file is missing, it expects
    /// state and `state` is missing or not of its kind, or the command can't
    /// be typed (see [`create::is_typeable`]).
    pub fn resume_command(&self, dir: &Path, state: Option<&str>) -> Option<String> {
        if let Some(file) = &self.requires_file
            && !dir.join(file).is_file()
        {
            return None;
        }
        let command = match self.state {
            StateKind::None => self.resume.clone(),
            kind => {
                let state = state.filter(|state| kind.accepts(state))?;
                self.resume
                    .replace(STATE_PLACEHOLDER, &foreground::quoted(state))
            }
        };
        create::is_typeable(&command).then_some(command)
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
            definitions: vec![
                ProgramDefinition {
                    name: "nvim".to_string(),
                    matches: vec!["nvim".to_string()],
                    state: StateKind::None,
                    resume: "nvim -S".to_string(),
                    requires_file: Some("Session.vim".to_string()),
                    payload: None,
                },
                ProgramDefinition {
                    name: "claude".to_string(),
                    matches: vec!["claude".to_string()],
                    state: StateKind::Id,
                    resume: format!("claude --resume {STATE_PLACEHOLDER}"),
                    requires_file: None,
                    payload: Some(Payload::ClaudeCode),
                },
            ],
        }
    }
}

impl Programs {
    /// Every definition's name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.definitions.iter().map(|d| d.name.as_str())
    }

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

    fn builtin(name: &str) -> ProgramDefinition {
        Programs::default().get(name).unwrap().clone()
    }

    /// A fresh empty directory, removed when the test ends.
    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let dir = std::env::temp_dir()
                .join(format!("rexurrect-programs-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn neovim_resumes_with_nvim_dash_s_where_a_session_vim_is() {
        let dir = Dir::new("nvim");
        assert_eq!(builtin("nvim").resume_command(&dir.0, None), None);
        std::fs::write(dir.0.join("Session.vim"), "").unwrap();
        assert_eq!(
            builtin("nvim").resume_command(&dir.0, None).as_deref(),
            Some("nvim -S")
        );
    }

    #[test]
    fn claude_resumes_its_conversation_by_id_and_only_with_one() {
        let dir = Dir::new("claude");
        let claude = builtin("claude");
        assert_eq!(
            claude
                .resume_command(&dir.0, Some("0b9e1f2a-61c4-4d1e-9a0f-3c2b1d4e5f60"))
                .as_deref(),
            Some("claude --resume 0b9e1f2a-61c4-4d1e-9a0f-3c2b1d4e5f60")
        );
        assert_eq!(claude.resume_command(&dir.0, None), None);
        assert_eq!(claude.resume_command(&dir.0, Some("$(reboot)")), None);
    }

    #[test]
    fn an_id_is_letters_digits_and_a_few_marks_and_never_an_option() {
        for id in ["abc", "0b9e1f2a-61c4", "thread_1.2:3", "A"] {
            assert!(StateKind::Id.accepts(id), "{id}");
        }
        let too_long = "a".repeat(MAX_ID_LEN + 1);
        for id in [
            "",
            "-x",
            "--dangerously-skip-permissions",
            "a b",
            "a;b",
            "$(x)",
            "a/b",
            "é",
            "a\n",
            &too_long,
        ] {
            assert!(!StateKind::Id.accepts(id), "{id:?}");
        }
    }

    #[test]
    fn a_path_is_absolute_without_control_characters() {
        assert!(StateKind::Path.accepts("/home/u/.pi/sessions/a b.jsonl"));
        let too_long = format!("/{}", "a".repeat(MAX_PATH_LEN));
        for path in ["", "relative/x", "-x", "/a\nb", "/a\tb", &too_long] {
            assert!(!StateKind::Path.accepts(path), "{path:?}");
        }
        assert!(!StateKind::None.accepts("anything"));
    }

    /// The value is one zsh word, whatever it holds: a path with a space
    /// stays one argument.
    #[test]
    fn a_reported_path_is_quoted_for_zsh_where_it_needs_it() {
        let dir = Dir::new("path");
        let pi = ProgramDefinition {
            name: "pi".to_string(),
            matches: vec!["pi".to_string()],
            state: StateKind::Path,
            resume: "pi --session {state}".to_string(),
            requires_file: None,
            payload: None,
        };
        assert_eq!(
            pi.resume_command(&dir.0, Some("/s/it's a.jsonl"))
                .as_deref(),
            Some(r"pi --session '/s/it'\''s a.jsonl'")
        );
    }
}
