//! Telling a shell from any other program: a block's first process is its
//! own shell only if it is one. A command block's first process is its
//! command, with no shell running.

use std::ffi::OsStr;
use std::path::Path;

use crate::rex::api::ProcessInfo;

/// Shells by name, whatever `$SHELL` says.
const SHELL_NAMES: [&str; 9] = [
    "zsh", "bash", "fish", "sh", "dash", "ksh", "tcsh", "csh", "nu",
];

/// The names a shell goes by: [`SHELL_NAMES`], and the file name of the
/// user's `$SHELL`.
#[derive(Debug, Clone)]
pub struct Shells {
    user_shell: Option<String>,
}

impl Shells {
    /// With `$SHELL`, if it's set. Unset, empty or not valid UTF-8, it adds
    /// nothing to the fixed names.
    pub fn from_env() -> Shells {
        Shells::new(std::env::var_os("SHELL").as_deref())
    }

    /// With `user_shell` (a `$SHELL` value) as one more shell.
    pub fn new(user_shell: Option<&OsStr>) -> Shells {
        let user_shell = user_shell
            .and_then(|shell| Path::new(shell).file_name())
            .and_then(OsStr::to_str)
            .map(str::to_string);
        Shells { user_shell }
    }

    /// Whether `process` is a shell, by any name it goes by. A login shell's
    /// `argv0` (`-zsh`) counts without its `-`. Matching shells rather than
    /// programs is what makes this reliable: Claude Code's executable is
    /// named for its version (`2.1.309`), and only its `argv0` is `claude`.
    pub fn is_shell(&self, process: &ProcessInfo) -> bool {
        process
            .names()
            .map(|name| name.strip_prefix('-').unwrap_or(name))
            .any(|name| SHELL_NAMES.contains(&name) || self.user_shell.as_deref() == Some(name))
    }
}
