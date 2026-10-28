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
//! takes what it needs from it (see [`Hook`]). Either way the value is
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
use crate::programs::{Hook, StateKind};
use crate::resume_state::{Reported, ResumeStates};
use crate::settings;
use crate::state::{self, STATE_DIR_VAR, StateDir};

/// The variable Rex sets in each block's shell, and so in every program it
/// runs: the block's ID.
const BLOCK_VAR: &str = "REX_BLOCK";

/// The variable Codex sets for the commands its agent runs: its thread's
/// ID. A Codex started by one of them inherits it.
const CODEX_THREAD_VAR: &str = "CODEX_THREAD_ID";

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
    let value = match (&options.value, definition.hook) {
        (Some(value), _) => Some(value.clone()),
        (None, Some(Hook::ClaudeCode)) => claude_code_value(&read_stdin()?)?,
        (None, Some(Hook::Codex)) => {
            let inherited_thread = settings::env_non_empty(CODEX_THREAD_VAR);
            codex_value(
                &read_stdin()?,
                inherited_thread.as_deref().and_then(|id| id.to_str()),
            )?
        }
        (None, _) => return because("no value was given"),
    };
    let Some(value) = value else {
        return Err(Dropped::Quietly);
    };
    if !definition.state.accepts(&value) {
        return because(&format!(
            "the value {} isn't {}",
            shown(&value),
            definition.state.described()
        ));
    }
    let reported = Reported {
        program: definition.name.clone(),
        state: value,
    };
    ResumeStates::new(state).write(block_id, &reported)?;
    Ok(())
}

/// The longest part of a value that `report` shows in the log.
const MAX_SHOWN: usize = 100;

/// `value` as the log shows it: quoted, with control characters escaped,
/// and cut short past [`MAX_SHOWN`] characters.
fn shown(value: &str) -> String {
    match value.char_indices().nth(MAX_SHOWN) {
        Some((end, _)) => format!("{:?}…", &value[..end]),
        None => format!("{value:?}"),
    }
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

/// A Claude Code or Codex hook's payload, as far as `report` reads it.
#[derive(Deserialize)]
struct HookPayload {
    session_id: Option<String>,
    /// Set when a Claude Code subagent runs the hook.
    agent_id: Option<serde_json::Value>,
}

impl HookPayload {
    /// The payload in `bytes`, from `agent`'s hook ("a Claude Code"), if it
    /// has a `session_id`.
    fn read(bytes: &[u8], agent: &str) -> std::result::Result<(HookPayload, String), Dropped> {
        let mut payload: HookPayload = serde_json::from_slice(bytes).map_err(|_| {
            Dropped::Because(format!("the hook's payload isn't {agent} hook's JSON"))
        })?;
        let session_id = payload
            .session_id
            .take()
            .ok_or_else(|| Dropped::Because("the hook's payload has no session_id".into()))?;
        Ok((payload, session_id))
    }
}

/// The agent conversation in a Claude Code hook's payload: `None` for a
/// subagent's, to ignore.
fn claude_code_value(bytes: &[u8]) -> std::result::Result<Option<String>, Dropped> {
    let (payload, session_id) = HookPayload::read(bytes, "a Claude Code")?;
    let subagent = payload.agent_id.is_some_and(|id| !id.is_null());
    Ok((!subagent).then_some(session_id))
}

/// The thread in a Codex hook's payload: `None`, to ignore, if
/// `inherited_thread`, the hook's `CODEX_THREAD_ID`, names another, as
/// herdr does: a Codex started from inside another's (by its shell tool)
/// inherits that one's, and mustn't take over the block.
fn codex_value(
    bytes: &[u8],
    inherited_thread: Option<&str>,
) -> std::result::Result<Option<String>, Dropped> {
    let (_, session_id) = HookPayload::read(bytes, "a Codex")?;
    let nested = inherited_thread.is_some_and(|thread| thread != session_id);
    Ok((!nested).then_some(session_id))
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
    match definition.hook {
        Some(Hook::ClaudeCode) => print!("{}", session_start_hook(&command)),
        Some(Hook::Codex) => print!("{}", codex_hook(&command)),
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

/// The hooks for Codex: its `SessionStart` hook for `~/.codex/hooks.json`,
/// which has the shape of Claude Code's, and the feature flag in
/// `~/.codex/config.toml` that turns hooks on. Codex's `SessionStart` fires
/// on `startup`, `resume`, `clear`, `compact` and `fork`, as Claude Code's
/// does.
fn codex_hook(command: &str) -> String {
    format!(
        "# In ~/.codex/hooks.json:\n{}\n# In ~/.codex/config.toml:\n[features]\nhooks = true\n",
        session_start_hook(command)
    )
}

/// The `SessionStart` hook for `~/.claude/settings.json` (and Codex's
/// `hooks.json`), running `command`. `SessionStart` fires on `startup`,
/// `resume`, `clear`, `compact` and `fork`; with no matcher, the hook runs
/// on each, so the block always holds the conversation on screen.
fn session_start_hook(command: &str) -> String {
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
    fn a_value_is_shown_quoted_escaped_and_cut_short() {
        assert_eq!(shown("a b"), r#""a b""#);
        assert_eq!(shown("a\nb"), r#""a\nb""#);
        let long = "é".repeat(MAX_SHOWN + 1);
        assert_eq!(shown(&long), format!("{:?}…", "é".repeat(MAX_SHOWN)));
    }

    #[test]
    fn a_block_id_is_rexs_and_nothing_that_reaches_elsewhere() {
        assert!(is_block_id("block:0b9e1f2a-61c4-4d1e-9a0f-3c2b1d4e5f60"));
        for id in ["", ".", "..", "../x", "a/b", ".hidden", "block:x y", "a\0b"] {
            assert!(!is_block_id(id), "{id:?}");
        }
        assert!(!is_block_id(&"b".repeat(MAX_BLOCK_ID_LEN + 1)));
    }

    fn outcome(value: std::result::Result<Option<String>, Dropped>) -> Option<String> {
        value.unwrap_or_else(|dropped| match dropped {
            Dropped::Quietly => None,
            Dropped::Because(cause) => Some(format!("dropped: {cause}")),
        })
    }

    fn claude(payload: &str) -> Option<String> {
        outcome(claude_code_value(payload.as_bytes()))
    }

    fn codex(payload: &str, inherited_thread: Option<&str>) -> Option<String> {
        outcome(codex_value(payload.as_bytes(), inherited_thread))
    }

    /// As Codex 0.165 sends it.
    const CODEX_SESSION_START: &str = r#"{"session_id":"019a3c2b-1d4e","transcript_path":null,"cwd":"/w/a","hook_event_name":"SessionStart","model":"gpt-5.5","permission_mode":"default","source":"startup"}"#;

    #[test]
    fn a_codex_payload_reports_its_session_id() {
        assert_eq!(
            codex(CODEX_SESSION_START, None).as_deref(),
            Some("019a3c2b-1d4e")
        );
        assert_eq!(
            codex(CODEX_SESSION_START, Some("019a3c2b-1d4e")).as_deref(),
            Some("019a3c2b-1d4e")
        );
    }

    #[test]
    fn a_codex_payload_for_another_thread_than_the_inherited_one_is_ignored() {
        assert_eq!(codex(CODEX_SESSION_START, Some("019-outer")), None);
    }

    #[test]
    fn a_codex_payload_without_a_session_id_is_dropped() {
        assert!(codex("{}", None).unwrap().starts_with("dropped:"));
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
