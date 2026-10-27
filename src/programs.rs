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
//!   `SessionStart` hook (see `report`);
//! - Codex, which reports its thread's ID from a `SessionStart` hook too;
//! - pi, which reports its session file from an extension.
//!
//! A program goes by the names Rex gives its process and, when it's an
//! interpreter's script (an npm-installed agent runs as `node`), by its
//! script's file name (see [`foreground::program_names`]).
//!
//! The config file's `[programs.<name>]` tables change a built-in field by
//! field, turn one off (`enabled = false`), or add a program (see
//! [`Programs::configured`]). A foreground program matching more than one
//! definition gets the first: the user's own, in name order (TOML tables
//! don't keep the file's), then the built-ins.
//!
//! A reported value is never a command: it only ever stands for
//! [`STATE_PLACEHOLDER`] in a definition's resume command, after it passes
//! [`StateKind::accepts`], quoted for zsh where it needs it.

use std::path::Path;

use crate::config;
use crate::create;
use crate::error::{Error, Result};
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

/// How a built-in agent reports its resume state: what `report
/// --print-hook` prints, and what the hook passes `report` (see `report`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hook {
    /// A Claude Code `SessionStart` hook, its JSON on stdin: the agent
    /// conversation's `session_id`, unless the payload has an `agent_id` (a
    /// subagent's).
    ClaudeCode,
    /// A Codex `SessionStart` hook, its JSON on stdin: the thread's
    /// `session_id`, unless a `CODEX_THREAD_ID` it inherited names another
    /// (a Codex started from inside another's).
    Codex,
    /// A pi extension, which passes its session file as the value.
    Pi,
}

/// What an auto-resumed program that can't resume comes back as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// Its own command line from the save, run.
    Command,
    /// The block's last command, typed, as if it weren't auto-resumed.
    None,
}

/// How rexurrect resumes one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramDefinition {
    /// The definition's name, which a snapshot records and `report` takes.
    pub name: String,
    /// The program names it matches, by any name a foreground program goes
    /// by (see [`crate::rex::api::ProcessInfo::names`]).
    pub matches: Vec<String>,
    /// The kind of resume state the program reports.
    pub state: StateKind,
    /// The resume command, a template: [`STATE_PLACEHOLDER`] stands for the
    /// reported value.
    pub resume: String,
    /// A file that must exist in the block's working directory at restore,
    /// else the program can't resume.
    pub requires_file: Option<String>,
    /// How a built-in agent reports its resume state.
    pub hook: Option<Hook>,
    /// What the program comes back as when it auto-resumes but can't
    /// resume.
    pub fallback: Fallback,
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
                    hook: None,
                    fallback: Fallback::Command,
                },
                ProgramDefinition {
                    name: "claude".to_string(),
                    matches: vec!["claude".to_string()],
                    state: StateKind::Id,
                    resume: format!("claude --resume {STATE_PLACEHOLDER}"),
                    requires_file: None,
                    hook: Some(Hook::ClaudeCode),
                    fallback: Fallback::Command,
                },
                ProgramDefinition {
                    name: "codex".to_string(),
                    matches: vec!["codex".to_string()],
                    state: StateKind::Id,
                    resume: format!("codex resume {STATE_PLACEHOLDER}"),
                    requires_file: None,
                    hook: Some(Hook::Codex),
                    fallback: Fallback::Command,
                },
                ProgramDefinition {
                    name: "pi".to_string(),
                    matches: vec!["pi".to_string()],
                    state: StateKind::Path,
                    resume: format!("pi --session {STATE_PLACEHOLDER}"),
                    requires_file: None,
                    hook: Some(Hook::Pi),
                    fallback: Fallback::Command,
                },
            ],
        }
    }
}

/// Every field a `[programs.<name>]` table can set.
const FIELDS: [&str; 6] = [
    "match",
    "state",
    "resume",
    "requires_file",
    "fallback",
    "enabled",
];

impl Programs {
    /// The built-in definitions as the config file's `programs` table
    /// (`value`, in the file at `file`) changes and adds to them. Each
    /// `[programs.<name>]` table named like a built-in overrides it field by
    /// field (`requires_file = ""` removes its requirement, `enabled =
    /// false` turns it off); any other adds a program, which needs a
    /// `resume`, and whose other fields default to `match = [<name>]`,
    /// `state = "none"`, no required file and `fallback = "command"`.
    ///
    /// An error names the table and field: an unknown field, a value of the
    /// wrong kind, a new program with no `resume`, or a `resume` that uses
    /// `{state}` when `state` is `"none"`, or doesn't when it isn't.
    pub fn configured(value: &toml::Value, file: &Path) -> Result<Programs> {
        let file = file.display();
        let tables = value.as_table().ok_or_else(|| {
            Error::new(format!(
                "`programs` in {file} must be tables of program definitions, like \
                 [programs.nvim], not {value}"
            ))
        })?;
        let mut builtins = Programs::default().definitions;
        let mut added = Vec::new();
        for (name, fields) in tables {
            let fields = fields.as_table().ok_or_else(|| {
                Error::new(format!(
                    "`programs.{name}` in {file} must be a table of a program definition's \
                     fields, not {fields}"
                ))
            })?;
            let builtin = builtins.iter().position(|d| d.name == *name);
            let mut definition = match builtin {
                Some(i) => builtins[i].clone(),
                None => ProgramDefinition {
                    name: name.clone(),
                    matches: vec![name.clone()],
                    state: StateKind::None,
                    resume: String::new(),
                    requires_file: None,
                    hook: None,
                    fallback: Fallback::Command,
                },
            };
            let mut enabled = true;
            for (field, value) in fields {
                let wrong = |what: &str| {
                    Error::new(format!(
                        "`programs.{name}.{field}` in {file} must be {what}, not {value}"
                    ))
                };
                match field.as_str() {
                    "match" => {
                        definition.matches = config::program_names(value)
                            .filter(|names| !names.is_empty())
                            .ok_or_else(|| wrong("a list of program names, like [\"nvim\"]"))?;
                    }
                    "state" => {
                        definition.state = match value.as_str() {
                            Some("none") => StateKind::None,
                            Some("id") => StateKind::Id,
                            Some("path") => StateKind::Path,
                            _ => return Err(wrong("\"none\", \"id\" or \"path\"")),
                        };
                    }
                    "resume" => {
                        definition.resume = value
                            .as_str()
                            .filter(|command| create::is_typeable(command))
                            .ok_or_else(|| wrong("a command on one line"))?
                            .to_string();
                    }
                    "requires_file" => {
                        definition.requires_file = match value.as_str() {
                            Some("") => None,
                            Some(file) => Some(file.to_string()),
                            None => return Err(wrong("a file name, or \"\" for none")),
                        };
                    }
                    "fallback" => {
                        definition.fallback = match value.as_str() {
                            Some("command") => Fallback::Command,
                            Some("none") => Fallback::None,
                            _ => return Err(wrong("\"command\" or \"none\"")),
                        };
                    }
                    "enabled" => {
                        enabled = value.as_bool().ok_or_else(|| wrong("true or false"))?;
                    }
                    _ => {
                        return Err(Error::new(format!(
                            "`programs.{name}.{field}` in {file} isn't a field rexurrect knows; \
                             it knows {}",
                            FIELDS.join(", ")
                        )));
                    }
                }
            }
            if builtin.is_none() && definition.resume.is_empty() {
                return Err(Error::new(format!(
                    "`programs.{name}` in {file} adds a program, so it needs `resume`, the \
                     command that resumes it"
                )));
            }
            let uses_state = definition.resume.contains(STATE_PLACEHOLDER);
            if uses_state != (definition.state != StateKind::None) {
                return Err(Error::new(if uses_state {
                    format!(
                        "`programs.{name}.resume` in {file} uses {STATE_PLACEHOLDER}, but the \
                         program reports no state: set `state` to \"id\" or \"path\""
                    )
                } else {
                    format!(
                        "`programs.{name}.resume` in {file} must use {STATE_PLACEHOLDER} where \
                         the program's reported state goes, since `state` isn't \"none\""
                    )
                }));
            }
            match builtin {
                Some(i) if enabled => builtins[i] = definition,
                Some(i) => {
                    builtins.remove(i);
                }
                None if enabled => added.push(definition),
                None => {}
            }
        }
        added.append(&mut builtins);
        Ok(Programs { definitions: added })
    }

    /// Every definition's name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.definitions.iter().map(|d| d.name.as_str())
    }

    /// The definition named `name`.
    pub fn get(&self, name: &str) -> Option<&ProgramDefinition> {
        self.definitions.iter().find(|d| d.name == name)
    }

    /// The definition matching the program in the foreground of the block
    /// whose terminal `process` method answered `process`, if there is one:
    /// the first that matches any name it goes by, its script's included
    /// (see [`foreground::program_names`]).
    pub fn matching(&self, process: &TerminalProcess) -> Result<Option<&ProgramDefinition>> {
        let Some(program) = process.foreground_program() else {
            return Ok(None);
        };
        let names = foreground::program_names(program)?;
        Ok(self.definitions.iter().find(|definition| {
            names
                .iter()
                .any(|name| definition.matches.iter().any(|m| m == name))
        }))
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
    fn codex_resumes_its_thread_by_id() {
        let dir = Dir::new("codex");
        assert_eq!(
            builtin("codex")
                .resume_command(&dir.0, Some("019a3c2b-1d4e-7f60-9a0f-3c2b1d4e5f60"))
                .as_deref(),
            Some("codex resume 019a3c2b-1d4e-7f60-9a0f-3c2b1d4e5f60")
        );
        assert_eq!(builtin("codex").resume_command(&dir.0, None), None);
    }

    #[test]
    fn pi_resumes_its_session_file() {
        let dir = Dir::new("pi");
        assert_eq!(
            builtin("pi")
                .resume_command(&dir.0, Some("/u/.pi/agent/sessions/--w--/s.jsonl"))
                .as_deref(),
            Some("pi --session /u/.pi/agent/sessions/--w--/s.jsonl")
        );
        assert_eq!(builtin("pi").resume_command(&dir.0, Some("s.jsonl")), None);
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
            hook: None,
            fallback: Fallback::Command,
        };
        assert_eq!(
            pi.resume_command(&dir.0, Some("/s/it's a.jsonl"))
                .as_deref(),
            Some(r"pi --session '/s/it'\''s a.jsonl'")
        );
    }

    fn configured(toml: &str) -> Result<Programs> {
        let table: toml::Table = toml.parse().unwrap();
        Programs::configured(&table["programs"], Path::new("/c/config.toml"))
    }

    #[test]
    fn an_override_changes_only_the_fields_it_sets() {
        let programs =
            configured("[programs.claude]\nresume = \"claude --resume {state} --yolo\"\n").unwrap();
        let claude = programs.get("claude").unwrap();
        assert_eq!(claude.resume, "claude --resume {state} --yolo");
        assert_eq!(claude.state, StateKind::Id);
        assert_eq!(claude.hook, Some(Hook::ClaudeCode));
        assert_eq!(programs.get("nvim"), Programs::default().get("nvim"));
    }

    #[test]
    fn a_new_programs_fields_default_and_it_comes_before_the_built_ins() {
        let programs = configured(
            "[programs.myvim]\nmatch = [\"nvim\"]\nresume = \"nvim -c Resume\"\n\
             [programs.off]\nresume = \"off\"\nenabled = false\n",
        )
        .unwrap();
        assert_eq!(
            programs.names().collect::<Vec<_>>(),
            ["myvim", "nvim", "claude", "codex", "pi"]
        );
        let myvim = programs.get("myvim").unwrap();
        assert_eq!(myvim.state, StateKind::None);
        assert_eq!(myvim.requires_file, None);
        assert_eq!(myvim.fallback, Fallback::Command);
    }

    #[test]
    fn an_empty_requires_file_removes_the_built_ins_and_fallback_can_be_none() {
        let programs =
            configured("[programs.nvim]\nrequires_file = \"\"\nfallback = \"none\"\n").unwrap();
        let nvim = programs.get("nvim").unwrap();
        assert_eq!(nvim.requires_file, None);
        assert_eq!(nvim.fallback, Fallback::None);
    }

    #[test]
    fn errors_name_the_file_the_table_and_the_field() {
        for (toml, name) in [
            ("[programs.nvim]\nresmue = \"x\"", "`programs.nvim.resmue`"),
            ("[programs.nvim]\nmatch = []", "`programs.nvim.match`"),
            ("[programs.nvim]\nmatch = \"nvim\"", "`programs.nvim.match`"),
            (
                "[programs.nvim]\nenabled = \"no\"",
                "`programs.nvim.enabled`",
            ),
            (
                "[programs.nvim]\nfallback = \"last\"",
                "`programs.nvim.fallback`",
            ),
            (
                "[programs.nvim]\nresume = \"a\\nb\"",
                "`programs.nvim.resume`",
            ),
            (
                "[programs.nvim]\nresume = \"nvim {state}\"",
                "`programs.nvim.resume`",
            ),
            (
                "[programs.claude]\nstate = \"none\"",
                "`programs.claude.resume`",
            ),
            ("[programs.x]\nstate = \"id\"", "`programs.x`"),
            ("programs = 3", "`programs`"),
            ("[programs]\nx = 3", "`programs.x`"),
        ] {
            let error = configured(toml).expect_err(toml).to_string();
            assert!(
                error.contains(name) && error.contains("/c/config.toml"),
                "{toml}: {error}"
            );
        }
    }
}
