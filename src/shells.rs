//! Telling a shell from any other program: a block's first process is its
//! own shell only if it is one. A command block's first process is its
//! command, with no shell running.

use crate::rex::api::ProcessInfo;

/// The shells recognized by name. A shell missing here, as a block's first
/// process, counts as that block's foreground program.
const SHELL_NAMES: [&str; 9] = [
    "zsh", "bash", "fish", "sh", "dash", "ksh", "tcsh", "csh", "nu",
];

/// Whether `process` is a shell, by any name it goes by. A login shell's
/// `argv0` (`-zsh`) counts without its `-`. Matching shells rather than
/// programs is what makes this reliable: Claude Code's executable is named
/// for its version (`2.1.309`), and only its `argv0` is `claude`.
pub fn is_shell(process: &ProcessInfo) -> bool {
    process
        .names()
        .map(|name| name.strip_prefix('-').unwrap_or(name))
        .any(|name| SHELL_NAMES.contains(&name))
}
