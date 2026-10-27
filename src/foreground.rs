//! The program in a block's foreground, a shell block's or a command
//! block's own command: the last command of a block nothing has recorded
//! anything for (no recorder file), because no recorder is installed or its
//! shell started before one was, or it has no shell.

use std::path::Path;

use crate::error::Result;
use crate::process;
use crate::rex::api::{ProcessInfo, TerminalProcess};

/// rexurrect's own executable name. A save run from a block is that block's
/// foreground program while it saves, and never what the block was doing.
const REXURRECT: &str = "rexurrect";

/// The command line of the program in the foreground of the block whose
/// terminal `process` method answered `process`, as restore would type it
/// (see [`zsh_line`]), or `None` while the block's own shell, or
/// rexurrect, is in the foreground, or if the program's arguments can't be
/// read or typed. The arguments come from the kernel: the terminal `process`
/// method reports none.
pub fn foreground_command(process: &TerminalProcess) -> Result<Option<String>> {
    match process.foreground_program() {
        Some(program) if !program.names().any(|name| name == REXURRECT) => command_line(program),
        _ => Ok(None),
    }
}

/// Every name `program` goes by (see [`ProcessInfo::names`]) and, if it's
/// an interpreter, the file name of the script it runs, from its arguments
/// as the kernel holds them (see [`script_name`]): npm's Codex goes by
/// `node`, and only its script is `codex`.
pub fn program_names(program: &ProcessInfo) -> Result<Vec<String>> {
    let arguments = if program.names().any(|name| INTERPRETERS.contains(&name)) {
        process::arguments(program.pid)?
    } else {
        None
    };
    Ok(program
        .names()
        .chain(arguments.as_deref().and_then(script_name))
        .map(str::to_string)
        .collect())
}

/// The interpreters whose script, not the interpreter, is the program: an
/// npm-installed agent's process is `node`, running the agent's script.
const INTERPRETERS: &[&str] = &["node", "bun", "python", "python3", "ruby", "perl"];

/// The interpreters' options that run code, or a module, in place of a
/// script.
const CODE_OPTIONS: &[&str] = &["-e", "--eval", "-p", "--print", "-c", "-m"];

/// The interpreters' options whose value is the next argument, which
/// mustn't be taken for the script (`node -r dotenv/config app.js`).
const OPTIONS_WITH_VALUES: &[&str] = &[
    "-r",
    "--require",
    "--import",
    "--loader",
    "--experimental-loader",
    "-W",
    "-X",
];

/// The file name of the script an interpreter runs, from its `arguments`,
/// its own name first: `codex` for `node /opt/homebrew/bin/codex resume`.
/// The script is the first argument that isn't an option or an option's
/// value, or the one after `--`. `None` if it runs none: code given in its
/// arguments (`node -e …`, `python3 -c …`) or a module (`python3 -m …`), or
/// nothing at all (a REPL). The name is as it is: `node app.js` runs
/// `app.js`, never `app`, as every other name matches exactly.
fn script_name(arguments: &[String]) -> Option<&str> {
    let mut rest = arguments.iter().skip(1);
    let script = loop {
        let argument = rest.next()?;
        match argument.as_str() {
            "--" => break rest.next()?,
            option if CODE_OPTIONS.contains(&option) => return None,
            option if option.starts_with("--eval=") || option.starts_with("--print=") => {
                return None;
            }
            option if OPTIONS_WITH_VALUES.contains(&option) => {
                rest.next();
            }
            option if option.starts_with('-') => {}
            _ => break argument,
        }
    };
    Path::new(script).file_name()?.to_str()
}

/// `program`'s command line, as restore would type it (see [`zsh_line`]),
/// or `None` if its arguments can't be read or typed.
pub fn command_line(program: &ProcessInfo) -> Result<Option<String>> {
    Ok(process::arguments(program.pid)?.and_then(|arguments| zsh_line(&arguments)))
}

/// `arguments` as a line zsh runs as the same program with the same
/// arguments: joined by spaces, each quoted only if it needs it, so
/// `vim notes.txt` reads as it always did. `None` if any argument holds a
/// control character, the rule [`crate::create::typed_command`] applies:
/// typed, a line break would act as Enter.
fn zsh_line(arguments: &[String]) -> Option<String> {
    if arguments.is_empty()
        || arguments
            .iter()
            .any(|argument| argument.contains(char::is_control))
    {
        return None;
    }
    let quoted: Vec<String> = arguments.iter().map(|argument| quoted(argument)).collect();
    Some(quoted.join(" "))
}

/// `argument` as one zsh word: as it is if every character is plain, else in
/// single quotes, inside which zsh treats everything literally but `'`
/// itself (closed, escaped, reopened).
pub fn quoted(argument: &str) -> String {
    if is_plain(argument) {
        argument.to_string()
    } else {
        format!("'{}'", argument.replace('\'', r"'\''"))
    }
}

/// Whether zsh reads `argument` unquoted as itself: no whitespace, globbing,
/// expansion or redirection characters anywhere, nor `=` first (`=vim`
/// expands to vim's path).
fn is_plain(argument: &str) -> bool {
    !argument.is_empty()
        && !argument.starts_with('=')
        && argument
            .chars()
            .all(|c| c.is_alphanumeric() || "-_./:,+@%=".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(arguments: &[&str]) -> Option<String> {
        zsh_line(&arguments.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn ordinary_arguments_are_joined_unquoted() {
        assert_eq!(
            line(&["vim", "notes.txt"]).as_deref(),
            Some("vim notes.txt")
        );
        assert_eq!(
            line(&["npm", "run", "dev", "--port=3000", "user@host:~x"]).as_deref(),
            Some("npm run dev --port=3000 'user@host:~x'")
        );
    }

    #[test]
    fn arguments_zsh_would_read_otherwise_are_single_quoted() {
        assert_eq!(
            line(&["vim", "my file.txt"]).as_deref(),
            Some("vim 'my file.txt'")
        );
        assert_eq!(
            line(&["echo", "it's", "*", "$HOME", "", "=vim"]).as_deref(),
            Some(r"echo 'it'\''s' '*' '$HOME' '' '=vim'")
        );
    }

    /// Its arguments are never read: `pid` 1 isn't rexurrect.
    #[test]
    fn rexurrect_as_a_blocks_first_process_is_not_its_last_command() {
        let process = TerminalProcess {
            foreground: ProcessInfo {
                pid: 1,
                name: REXURRECT.to_string(),
                invoked_path: None,
                argv0: Some(REXURRECT.to_string()),
                cwd: None,
            },
            ancestors: Vec::new(),
            home: "/home/u".into(),
        };
        assert_eq!(foreground_command(&process).unwrap(), None);
    }

    fn script(arguments: &[&str]) -> Option<String> {
        let arguments: Vec<String> = arguments.iter().map(|a| a.to_string()).collect();
        script_name(&arguments).map(str::to_string)
    }

    /// An npm-installed program runs as its interpreter, with the path the
    /// shell found it by as the script.
    #[test]
    fn an_interpreters_script_is_its_first_argument_that_isnt_an_option() {
        assert_eq!(
            script(&["node", "/opt/homebrew/bin/codex", "resume", "x"]).as_deref(),
            Some("codex")
        );
        assert_eq!(
            script(&["/usr/local/bin/node", "--no-warnings", "/u/bin/codex"]).as_deref(),
            Some("codex")
        );
    }

    #[test]
    fn an_options_value_isnt_the_script_and_the_argument_after_dash_dash_is() {
        assert_eq!(
            script(&["node", "-r", "dotenv/config", "/u/bin/codex"]).as_deref(),
            Some("codex")
        );
        assert_eq!(script(&["node", "--", "/u/bin/pi"]).as_deref(), Some("pi"));
        assert_eq!(script(&["node", "app.js"]).as_deref(), Some("app.js"));
    }

    #[test]
    fn an_interpreter_running_code_from_its_arguments_or_nothing_runs_no_script() {
        assert_eq!(script(&["node", "-e", "require('/u/bin/codex')"]), None);
        assert_eq!(script(&["python3", "-c", "import codex"]), None);
        assert_eq!(script(&["python3", "-m", "codex"]), None);
        assert_eq!(script(&["node", "--eval=1", "/u/bin/codex"]), None);
        assert_eq!(script(&["node"]), None);
        assert_eq!(script(&[]), None);
    }

    #[test]
    fn arguments_holding_a_control_character_are_refused() {
        assert_eq!(line(&["sh", "-c", "tail -f /dev/null\ntrue"]), None);
        assert_eq!(line(&["printf", "a\tb"]), None);
        assert_eq!(line(&[]), None);
    }
}
