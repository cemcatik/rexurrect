//! `rexurrect report <program> [value]`: a program reports its resume state
//! for the block it runs in (ADR-0005). An agent's hook runs it.
//!
//! It never prints and always exits 0: a hook's output reaches the agent,
//! and a problem in rexurrect must never show up in the agent's
//! conversation or block it. What it can't do it logs to `rexurrect.log`
//! instead, once per cause (see [`FailureLog`]). It does nothing at all
//! outside Rex (no `REX_BLOCK`, or an empty one).
//!
//! The value is the argument after the program's name. Without one, a
//! built-in agent's hook passes its own payload on stdin, and `report`
//! takes what it needs from it (see [`Payload`]). Either way the value is
//! dropped unless it's of the kind the program's definition expects (see
//! [`StateKind::accepts`]).
//!
//! `rexurrect report <program> --print-hook` prints what to add to the
//! program's own config to call `report`, and changes no file.

use std::io::{IsTerminal, Read};
use std::path::Path;
use std::process::ExitCode;

use serde::Deserialize;

use crate::config::{CONFIG_VAR, Config, ConfigFile};
use crate::error::{Context, Error, Result};
use crate::foreground;
use crate::log::Log;
use crate::programs::{Payload, StateKind};
use crate::resume_state::{Reported, ResumeStates};
use crate::settings;
use crate::state::{self, STATE_DIR_VAR, StateDir};

/// The variable Rex sets in each block's shell, and so in every program it
/// runs: the block's ID.
const BLOCK_VAR: &str = "REX_BLOCK";

/// The largest payload `report` reads from stdin. Hook payloads are a few
/// hundred bytes.
const MAX_PAYLOAD: u64 = 1 << 20;

/// The longest block ID `report` accepts. Rex's are `block:` and a UUID.
const MAX_BLOCK_ID_LEN: usize = 128;

#[derive(Debug, Default)]
pub struct ReportOptions {
    /// The program definition's name.
    pub program: Option<String>,
    /// The value, if given as an argument.
    pub value: Option<String>,
    /// Print the hook instead.
    pub print_hook: bool,
    /// Whether there were arguments beyond `<program> [value]`, which
    /// `report` ignores.
    pub extra_arguments: bool,
}

pub fn run(options: &ReportOptions) -> Result<ExitCode> {
    if options.print_hook {
        return print_hook(options);
    }
    report(options);
    Ok(ExitCode::SUCCESS)
}

/// Stores the block's resume state, or logs why it can't. Never fails and
/// never prints.
fn report(options: &ReportOptions) {
    let Some(block_id) = settings::env_non_empty(BLOCK_VAR) else {
        return;
    };
    let Ok(state) = StateDir::from_env() else {
        return;
    };
    let failures = FailureLog::new(&state);
    let program = options.program.as_deref().unwrap_or_default();
    match store(&state, program, &block_id.to_string_lossy(), options) {
        Ok(()) => failures.forget(),
        Err(Dropped::Quietly) => {}
        Err(Dropped::Because(cause)) => failures.log(&format!("report {program:?}: {cause}")),
    }
}

/// Why `report` stored nothing.
enum Dropped {
    /// Nothing went wrong: a subagent's payload, say.
    Quietly,
    /// Worth logging.
    Because(String),
}

impl<E: std::fmt::Display> From<E> for Dropped {
    fn from(e: E) -> Dropped {
        Dropped::Because(e.to_string())
    }
}

fn store(
    state: &StateDir,
    program: &str,
    block_id: &str,
    options: &ReportOptions,
) -> std::result::Result<(), Dropped> {
    let because = |cause: &str| Err(Dropped::Because(cause.to_string()));
    if options.program.is_none() || options.extra_arguments {
        return because("usage: rexurrect report <program> [value]");
    }
    if !is_block_id(block_id) {
        return because("REX_BLOCK isn't a plain block ID");
    }
    let config = Config::load()?;
    let Some(definition) = config.programs.get(program) else {
        return because("no program definition has this name");
    };
    if definition.state == StateKind::None {
        return because("this program reports no resume state");
    }
    let value = match (&options.value, definition.payload) {
        (Some(value), _) => value.clone(),
        (None, Some(payload)) => match payload_value(payload, &read_stdin()?)? {
            Some(value) => value,
            None => return Err(Dropped::Quietly),
        },
        (None, None) => return because("no value was given"),
    };
    if !definition.state.accepts(&value) {
        return because(&format!("the value isn't {}", definition.state.described()));
    }
    let reported = Reported {
        program: definition.name.clone(),
        state: value,
    };
    ResumeStates::new(state).write(block_id, &reported)?;
    Ok(())
}

/// Whether `id` can be a block ID Rex gave: what `REX_BLOCK` holds in a
/// block (`block:<uuid>`). Only letters, digits, `:`, `-` and `_`, so it
/// can't name anything but its own file.
fn is_block_id(id: &str) -> bool {
    state::is_plain_file_name(id)
        && id.len() <= MAX_BLOCK_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":-_".contains(c))
}

/// The hook's payload, from stdin; empty if stdin is a terminal (someone
/// ran `report` by hand), which would otherwise wait for input.
fn read_stdin() -> std::result::Result<Vec<u8>, Dropped> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Ok(Vec::new());
    }
    let mut payload = Vec::new();
    stdin
        .lock()
        .take(MAX_PAYLOAD)
        .read_to_end(&mut payload)
        .map_err(|e| Dropped::Because(format!("reading the hook's payload: {e}")))?;
    Ok(payload)
}

/// A Claude Code hook's payload, as far as `report` reads it.
#[derive(Deserialize)]
struct ClaudeCodePayload {
    session_id: Option<String>,
    /// Set when a subagent runs the hook.
    agent_id: Option<serde_json::Value>,
}

/// The value in a built-in agent's hook payload: `None` for a payload to
/// ignore (a subagent's).
fn payload_value(payload: Payload, bytes: &[u8]) -> std::result::Result<Option<String>, Dropped> {
    match payload {
        Payload::ClaudeCode => {
            let payload: ClaudeCodePayload = serde_json::from_slice(bytes).map_err(|_| {
                Dropped::Because("the hook's payload isn't a Claude Code hook's JSON".into())
            })?;
            if payload.agent_id.is_some_and(|id| !id.is_null()) {
                return Ok(None);
            }
            payload
                .session_id
                .map(Some)
                .ok_or_else(|| Dropped::Because("the hook's payload has no session_id".into()))
        }
    }
}

/// Logs `report`'s failures to `rexurrect.log`, each cause once: a hook
/// runs at every agent conversation, and one broken the same way would
/// otherwise fill the log. Remembers the causes it has logged in
/// `report-failures.json` until a report succeeds, after which a failure is
/// news again.
struct FailureLog {
    state: StateDir,
}

const FAILURES_FILE: &str = "report-failures.json";

/// How many causes [`FailureLog`] remembers; older ones may be logged again.
const MAX_FAILURES: usize = 32;

impl FailureLog {
    fn new(state: &StateDir) -> FailureLog {
        FailureLog {
            state: state.clone(),
        }
    }

    /// Logs `cause` unless it's logged already. Best effort.
    fn log(&self, cause: &str) {
        let path = self.state.join(FAILURES_FILE);
        let mut logged: Vec<String> = state::read_json(&path).unwrap_or_default();
        if logged.iter().any(|c| c == cause) {
            return;
        }
        if Log::new(&self.state)
            .append(&format!("report failed: {cause}"))
            .is_err()
        {
            return;
        }
        logged.push(cause.to_string());
        let excess = logged.len().saturating_sub(MAX_FAILURES);
        logged.drain(..excess);
        let _ = state::write_json(&path, &logged);
    }

    /// Forgets the causes logged so far. Best effort.
    fn forget(&self) {
        let _ = state::remove_if_exists(&self.state.join(FAILURES_FILE));
    }
}

/// Prints what to add to `program`'s own config so that it reports its
/// resume state, naming rexurrect by its full path and pinning the state
/// directory and config file as they are here (as `auto-restore
/// --print-hook` does): a hook runs in the agent's environment, which may
/// not be the shell's that save and autosave see. Changes no file.
fn print_hook(options: &ReportOptions) -> Result<ExitCode> {
    let program = options.program.as_deref().ok_or_else(|| {
        Error::new("--print-hook needs a program: rexurrect report <program> --print-hook")
    })?;
    let config = Config::load()?;
    let definition = config
        .programs
        .get(program)
        .ok_or_else(|| Error::new(format!("no program definition is named {program:?}")))?;
    let command = report_command(program)?;
    match definition.payload {
        Some(Payload::ClaudeCode) => print!("{}", claude_code_hook(&command)),
        None if definition.state == StateKind::None => {
            return Err(Error::new(format!(
                "{program} reports no resume state, so it needs no hook"
            )));
        }
        None => println!("{command} <value>"),
    }
    Ok(ExitCode::SUCCESS)
}

/// The command a hook runs: `REXURRECT_STATE_DIR=… REXURRECT_CONFIG=…
/// <rexurrect> report <program>`, each value quoted for the shell.
fn report_command(program: &str) -> Result<String> {
    let rexurrect = std::env::current_exe().with_context(|| "finding rexurrect's own path")?;
    let state = StateDir::from_env()?;
    let config = ConfigFile::from_env()?;
    Ok(format!(
        "{STATE_DIR_VAR}={} {CONFIG_VAR}={} {} report {}",
        shell_word(state.path())?,
        shell_word(config.path())?,
        shell_word(&rexurrect)?,
        foreground::quoted(program),
    ))
}

/// `path` as one word for `sh` (and zsh): single quotes, where needed, mean
/// the same to both.
fn shell_word(path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .ok_or_else(|| Error::new(format!("{} isn't valid UTF-8", path.display())))?;
    Ok(foreground::quoted(path))
}

/// The `SessionStart` hook for `~/.claude/settings.json`, running
/// `command`. `SessionStart` fires on `startup`, `resume`, `clear`,
/// `compact` and `fork`; with no matcher, the hook runs on each, so the
/// block always holds the conversation on screen.
fn claude_code_hook(command: &str) -> String {
    let hook = serde_json::json!({
        "hooks": {
            "SessionStart": [
                { "hooks": [{ "type": "command", "command": command }] }
            ]
        }
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&hook).expect("JSON values serialize")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_id_is_rexs_and_nothing_that_reaches_elsewhere() {
        assert!(is_block_id("block:0b9e1f2a-61c4-4d1e-9a0f-3c2b1d4e5f60"));
        for id in ["", ".", "..", "../x", "a/b", ".hidden", "block:x y", "a\0b"] {
            assert!(!is_block_id(id), "{id:?}");
        }
        assert!(!is_block_id(&"b".repeat(MAX_BLOCK_ID_LEN + 1)));
    }

    fn claude(payload: &str) -> Option<String> {
        payload_value(Payload::ClaudeCode, payload.as_bytes()).unwrap_or_else(|dropped| {
            match dropped {
                Dropped::Quietly => None,
                Dropped::Because(cause) => Some(format!("dropped: {cause}")),
            }
        })
    }

    #[test]
    fn a_claude_code_payload_reports_its_session_id() {
        assert_eq!(
            claude(r#"{"session_id":"abc","source":"clear","hook_event_name":"SessionStart"}"#)
                .as_deref(),
            Some("abc")
        );
        assert_eq!(
            claude(r#"{"session_id":"abc","agent_id":null}"#).as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn a_subagents_payload_is_ignored() {
        assert_eq!(claude(r#"{"session_id":"abc","agent_id":"agent-1"}"#), None);
    }

    #[test]
    fn a_payload_without_a_session_id_or_that_isnt_json_is_dropped() {
        assert!(claude("{}").unwrap().starts_with("dropped:"));
        assert!(claude("not json").unwrap().starts_with("dropped:"));
        assert!(claude("").unwrap().starts_with("dropped:"));
    }
}
