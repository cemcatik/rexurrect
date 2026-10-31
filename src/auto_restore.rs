//! `rexurrect auto-restore`: what the line in Rex's init.lua runs.
//!
//! The Rex server runs init.lua in its own process at every start, and
//! answers no request until init.lua returns. It runs init.lua again on every
//! `rex config reload`. So `auto-restore` can't wait for the server: it finds
//! the server process that started it, starts itself again in the background
//! aimed at that server (`--server-pid`), and returns.
//!
//! The background run waits for the server to be ready, then restores into
//! it unless rexurrect already has: once per server instance, so a config
//! reload restores nothing. At the server's start (init.lua's first run in
//! it), it replaces the sessions Rex carried over (ADR-0008). If that
//! created sessions, it then closes the app session (see [`app_session`]).
//! It logs the outcome either way, since nobody sees its output.
//!
//! It never relies on `REX_SERVER` to reach the server: init.lua runs with
//! whatever environment the server was started with, which can name another
//! server. It finds the endpoint from the server process itself, see
//! [`server_process`].

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use crate::account;
use crate::app_session::{self, Watched};
use crate::config::{self, CONFIG_VAR, Config, ConfigFile};
use crate::error::{Context, Error, Result};
use crate::instance::RestoreLock;
use crate::log::{self, Log};
use crate::process;
use crate::restore::{self, RestoreOptions, RestoreReport};
use crate::rex::Rex;
use crate::rex::server_process::{self, ServerProcess, Starter};
use crate::state::{STATE_DIR_VAR, StateDir};

#[derive(Debug, Default)]
pub struct AutoRestoreOptions {
    /// Wait for the server with this pid and restore into it, in the
    /// foreground. Without it: find the server that started this process
    /// and do that in the background.
    pub server_pid: Option<u32>,
    /// Print the hook, the line that turns auto-restore on in init.lua,
    /// and do nothing else.
    pub print_hook: bool,
}

/// How long one readiness check waits for the server to answer. A server
/// still running init.lua accepts the connection and answers nothing.
const CHECK_TIMEOUT: Duration = Duration::from_secs(1);
/// The pause between rounds of readiness checks.
const CHECK_INTERVAL: Duration = Duration::from_millis(200);

pub fn run(options: &AutoRestoreOptions) -> Result<ExitCode> {
    if options.print_hook {
        println!("{}", hook()?);
        return Ok(ExitCode::SUCCESS);
    }
    match options.server_pid {
        None => start_in_background(),
        Some(pid) => restore_when_ready(pid),
    }
}

/// The line that turns auto-restore on in init.lua:
///
/// ```lua
/// do local r = rex.exec({ [[<rexurrect>]], "auto-restore" }, { env = { REXURRECT_STATE_DIR = [[<state dir>]], REXURRECT_CONFIG = [[<config file>]] } }) if r then io.stderr:write(r.stderr) end end
/// ```
///
/// Rex runs commands from init.lua only through `rex.exec` (`os.execute`
/// raises an error, which stops init.lua there). It takes an argv list and
/// no shell, and waits for the command: `auto-restore` returns at once, so
/// that's fine. It captures the command's output, so the line passes stderr
/// on: to the terminal under `rex config check`, to the server's log when
/// the server runs init.lua. `r` is `nil` when the program never ran, and
/// Rex warns about that itself, with init.lua's file and line. `do ... end`
/// keeps `r` out of the rest of init.lua.
///
/// The three paths are pinned as they are here. The server runs init.lua in
/// its own environment, not the user's shell's: the Rex app starts it with a
/// bare `PATH`, so `rexurrect` needs its full path, and with no `XDG_*`, so
/// auto-restore could otherwise pick a different state directory from the
/// user's shells and their recorder, and a different config file from the
/// one they edit. `env` adds the state directory and the config file to the
/// server's environment. The file's path, not its settings: an edit takes
/// effect at the next server start.
pub fn hook() -> Result<String> {
    let program = std::env::current_exe().with_context(|| "finding rexurrect's own path")?;
    let state = StateDir::from_env()?;
    let config = ConfigFile::from_env()?;
    Ok(format!(
        "do local r = rex.exec({{ {}, \"auto-restore\" }}, \
         {{ env = {{ {STATE_DIR_VAR} = {}, {CONFIG_VAR} = {} }} }}) \
         if r then io.stderr:write(r.stderr) end end",
        lua_string(&program)?,
        lua_string(state.path())?,
        lua_string(config.path())?
    ))
}

/// `path` as a Lua string.
fn lua_string(path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .ok_or_else(|| Error::new(format!("{} isn't valid UTF-8", path.display())))?;
    Ok(lua_long_string(path))
}

/// `text` as a Lua long string (`[[...]]`), with as many `=` in its
/// brackets as it takes for nothing in `text` to end it early.
fn lua_long_string(text: &str) -> String {
    let mut level = String::new();
    while text.contains(&format!("]{level}]")) || text.ends_with(&format!("]{level}")) {
        level.push('=');
    }
    format!("[{level}[{text}]{level}]")
}

/// Starts the background run, aimed at the server that started this
/// process, and returns without waiting for it. The background run is
/// detached from the server: its own process group, no output, and none of
/// the server's open files. It doesn't get the moved variables either: this
/// run has warned about them, and they're read by neither.
fn start_in_background() -> Result<ExitCode> {
    // Checked here, so a bad setting is reported where init.lua's output
    // goes, not only in the background run's log entry.
    Config::load()?;
    let server = match server_process::find_starter()? {
        Starter::Server(server) => server,
        Starter::OtherRex { command } => {
            eprintln!(
                "rexurrect auto-restore: started by `{command}`, not by a Rex server, \
                 so there's no server to restore into"
            );
            return Ok(ExitCode::SUCCESS);
        }
        Starter::NotRex => {
            return Err(Error::new(
                "auto-restore restores into the Rex server whose init.lua runs it, and no Rex \
                 server started it; to restore by hand, run `rexurrect restore`",
            ));
        }
    };
    let program = std::env::current_exe().with_context(|| "finding rexurrect's own path")?;
    process::stop_passing_on_open_files();
    let mut background = Command::new(program);
    background
        .args(["auto-restore", "--server-pid", &server.pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    for var in config::moved_variables() {
        background.env_remove(var);
    }
    background
        .spawn()
        .with_context(|| "starting auto-restore in the background")?;
    Ok(ExitCode::SUCCESS)
}

/// Waits for the server with this pid to be ready, then restores into it
/// unless this server instance was already restored into. If that created
/// any session, looks for the app session and closes it (see
/// `app_session`). Logs what happened as one entry, once it stops looking
/// (after an entry for each failure).
fn restore_when_ready(pid: u32) -> Result<ExitCode> {
    let config = Config::load()?;
    let state = StateDir::from_env()?;
    let server = ServerProcess::read(pid)?
        .ok_or_else(|| Error::new(format!("there's no Rex server with pid {pid}")))?;
    let ready = wait_until_ready(&server, config.ready_timeout)?;
    let lock = RestoreLock::acquire(&state)?;
    // Read again under the lock: the instance to restore into is whichever
    // the server is running once it's this run's turn.
    let server = ready.rex.server_status()?;
    // Sessions live at the server's start are Rex's copies of the saved
    // ones (ADR-0008): replaced. A config reload runs init.lua again in a
    // server the user may have been working in: nothing is replaced then.
    let options = RestoreOptions {
        force: server.config.generation == 1,
        ..RestoreOptions::default()
    };
    let restored = restore::restore_unrestored_instance(
        &ready.rex, &state, &lock, &server, &config, &options,
    )?;
    // The instance is recorded as restored by now. A save or restore that
    // starts while this run looks for the app session needn't wait for it.
    drop(lock);
    let created = restored
        .as_ref()
        .is_some_and(RestoreReport::created_a_session);
    let (mut outcome, mut status) = match restored {
        None => (
            "already restored into; nothing to do".to_string(),
            ExitCode::SUCCESS,
        ),
        Some(report) => {
            let mut printed = Vec::new();
            let status = report.finish(&state, "auto-restore", &mut printed);
            (String::from_utf8_lossy(&printed).into_owned(), status)
        }
    };
    if created {
        match app_session::close_app_session(&ready.rex, &state, &server, &config) {
            Watched::Closed(session) => outcome.push_str(&format!("\nclosed {session}")),
            Watched::NoneFound => {}
            Watched::ServerStopped => {
                outcome.push_str("\nstopped looking for the app session because the server stopped")
            }
            Watched::CloseFailed(session, e) => {
                log::log_failure(&state, "auto-restore", &format!("closing {session}: {e}"));
                status = ExitCode::FAILURE;
            }
        }
    }
    let message = format!(
        "auto-restore into {} (server instance {}): {outcome}",
        ready.endpoint, server.instance_id
    );
    println!("{message}");
    Log::new(&state).append(&message)?;
    Ok(status)
}

/// A server that's ready, reached at the endpoint it answered on.
struct Ready {
    rex: Rex,
    endpoint: String,
}

/// Checks the endpoints the server may be listening on until one answers
/// with the server's own pid, which makes it the server's. Gives up after
/// `timeout`, or as soon as the server has exited or every endpoint turned
/// out to be another server's.
fn wait_until_ready(server: &ServerProcess, timeout: Duration) -> Result<Ready> {
    let deadline = Instant::now() + timeout;
    let mut candidates = server.candidate_endpoints(&account::home()?);
    let mut other_servers = Vec::new();
    loop {
        let mut not_ready = Vec::new();
        for endpoint in candidates.clone() {
            let rex = Rex::at(&server.program, &endpoint);
            match rex.with_timeout(CHECK_TIMEOUT).server_status() {
                Ok(status) if status.pid == server.pid => {
                    return Ok(Ready { rex, endpoint });
                }
                Ok(status) => {
                    candidates.retain(|c| *c != endpoint);
                    other_servers.push(format!("{endpoint} (pid {})", status.pid));
                }
                Err(e) => not_ready.push(format!("{endpoint}: {e}")),
            }
        }
        let gave_up = if !process::is_running(server.pid) {
            Some("it stopped before it was ready".to_string())
        } else if candidates.is_empty() {
            Some(format!(
                "every endpoint it may listen on is another server's: {}",
                other_servers.join(", ")
            ))
        } else if Instant::now() >= deadline {
            Some(format!(
                "it wasn't ready after {}s; last tried {}",
                timeout.as_secs(),
                not_ready.join("; ")
            ))
        } else {
            None
        };
        if let Some(why) = gave_up {
            return Err(Error::new(format!(
                "gave up waiting for the Rex server (pid {}): {why}",
                server.pid
            )));
        }
        std::thread::sleep(CHECK_INTERVAL);
    }
}
