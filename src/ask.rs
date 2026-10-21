//! Asking before restoring: `rexurrect ask` and `rexurrect fresh`.
//!
//! With `REXURRECT_ASK` set, nothing restores into a server instance on its
//! own: not auto-restore, and not the restore a save runs first. Instead the
//! instance waits for a choice, which `ask` puts to the user at a terminal
//! (the zsh recorder runs it at the first prompt of each new block):
//!
//! - restore: a whole restore of the snapshot it offered, as `restore` does;
//! - start fresh (`fresh` does this without asking): the instance counts as
//!   restored into, so nothing ever restores into it and saves save what's
//!   live;
//! - later, or no answer: nothing. The instance keeps waiting, and saves
//!   save nothing, so the snapshot on offer stays the latest.
//!
//! An instance waits only while there's something to restore: the latest
//! snapshot holds a session whose lineage isn't live. Otherwise it's
//! restored into as usual, which creates nothing.
//!
//! `ask` holds no lock while it waits for an answer, so autosave never
//! queues up behind a prompt. It takes the restore lock once answered and
//! acts only if nothing chose in the meantime. A second lock, held while
//! asking, keeps two new blocks from asking at once.

use std::fs::OpenOptions;
use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::process::ExitCode;

use crate::error::{Context, Result};
use crate::instance::RestoreLock;
use crate::log::Log;
use crate::restore::{self, Outcome, RestoreOptions, RestoreReport};
use crate::rex::Rex;
use crate::rex::api::ServerStatus;
use crate::settings;
use crate::state::StateDir;
use crate::store::SnapshotStore;

/// Ask before restoring, when set to anything but empty or `0`.
pub const ASK_VAR: &str = "REXURRECT_ASK";
/// Held while a prompt is up, so only one block asks.
const ASK_LOCK: &str = "ask.lock";

/// Whether `REXURRECT_ASK` is on.
pub fn enabled() -> bool {
    settings::env_non_empty(ASK_VAR).is_some_and(|value| value != "0")
}

/// What restoring into the server instance `server` describes would do,
/// as a dry run, if that instance is waiting for a choice: asking is on,
/// nothing has restored into it, and the latest snapshot has a session to
/// create. `None` otherwise.
pub fn awaiting_choice(
    rex: &Rex,
    state: &StateDir,
    lock: &RestoreLock,
    server: &ServerStatus,
) -> Result<Option<RestoreReport>> {
    if !enabled() || lock.is_restored(&server.instance_id)? {
        return Ok(None);
    }
    let dry_run = RestoreOptions {
        dry_run: true,
        ..RestoreOptions::default()
    };
    let report = restore::restore(rex, state, lock, server, &dry_run)?;
    let creates = report
        .outcomes
        .iter()
        .any(|o| matches!(o, Outcome::WouldCreate { .. }));
    Ok(creates.then_some(report))
}

enum Choice {
    Restore,
    Fresh,
    Later,
}

/// `rexurrect ask`: if this server instance is waiting for a choice, asks
/// for one at the terminal and acts on it. Silent when there's nothing to
/// ask, when stdin isn't a terminal, or when another block is asking.
pub fn run() -> Result<ExitCode> {
    let state = StateDir::from_env()?;
    let rex = Rex::from_env()?;
    let (server, offer) = {
        let lock = RestoreLock::acquire(&state)?;
        let server = rex.server_status()?;
        let offer = awaiting_choice(&rex, &state, &lock, &server)?;
        if offer.is_none() && !lock.is_restored(&server.instance_id)? {
            // Nothing to restore (or asking is off): restore as usual, which
            // records the instance and creates whatever there is.
            let report =
                restore::restore(&rex, &state, &lock, &server, &RestoreOptions::default())?;
            if report.created_a_session() {
                return Ok(report.finish(&state, "ask", &mut std::io::stdout().lock()));
            }
            return Ok(ExitCode::SUCCESS);
        }
        (server, offer)
    };
    let Some(offer) = offer else {
        return Ok(ExitCode::SUCCESS);
    };
    if !std::io::stdin().is_terminal() {
        return Ok(ExitCode::SUCCESS);
    }
    let path = state.join(ASK_LOCK);
    let asking = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    if asking.try_lock().is_err() {
        return Ok(ExitCode::SUCCESS);
    }
    let snapshot = offer.snapshot.clone().unwrap_or_default();
    let choice = prompt(&state, &snapshot, &offer)?;

    let lock = RestoreLock::acquire(&state)?;
    let now = rex.server_status()?;
    if now.instance_id != server.instance_id || lock.is_restored(&now.instance_id)? {
        println!("rexurrect: already decided for this server; nothing to do");
        return Ok(ExitCode::SUCCESS);
    }
    let mut out = std::io::stdout().lock();
    match choice {
        Choice::Restore => {
            let options = RestoreOptions {
                snapshot: Some(snapshot),
                ..RestoreOptions::default()
            };
            let report = restore::restore(&rex, &state, &lock, &now, &options)?;
            Ok(report.finish(&state, "ask", &mut out))
        }
        Choice::Fresh => {
            lock.record_restored(&now.instance_id)?;
            let message = format!(
                "started fresh on server instance {}; snapshot {snapshot} was not restored",
                now.instance_id
            );
            Log::new(&state).append(&format!("ask: {message}"))?;
            let _ = writeln!(
                out,
                "rexurrect: starting fresh. Snapshot {snapshot} stays in `rexurrect list` until \
                 retention removes it; `rexurrect restore --snapshot {snapshot}` brings it back."
            );
            Ok(ExitCode::SUCCESS)
        }
        Choice::Later => {
            let _ = writeln!(
                out,
                "rexurrect: deciding later. Nothing restores and nothing is saved until you \
                 choose: a new block asks again, or run `rexurrect restore` or `rexurrect fresh`."
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Puts the choice to the user: Enter restores.
fn prompt(state: &StateDir, snapshot: &str, offer: &RestoreReport) -> Result<Choice> {
    let taken_at = SnapshotStore::new(state)
        .read(snapshot)?
        .map(|s| s.taken_at.display())
        .unwrap_or_default();
    let labels: Vec<&str> = offer
        .outcomes
        .iter()
        .filter_map(|o| match o {
            Outcome::WouldCreate { label } => Some(label.as_str()),
            _ => None,
        })
        .collect();
    let mut out = std::io::stdout().lock();
    let _ = write!(
        out,
        "\nrexurrect: the Rex server restarted. Snapshot {snapshot} ({taken_at}) has {} \
         session(s) that aren't live:\n  {}\nRestore them? [R]estore / [f]resh start / [l]ater: ",
        labels.len(),
        labels.join(", ")
    );
    let _ = out.flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).unwrap_or(0) == 0 {
        let _ = writeln!(out);
        return Ok(Choice::Later);
    }
    Ok(match answer.trim().to_ascii_lowercase().as_str() {
        "" | "r" | "restore" | "y" | "yes" => Choice::Restore,
        "f" | "fresh" | "n" | "no" => Choice::Fresh,
        _ => Choice::Later,
    })
}

/// `rexurrect fresh`: start fresh on this server instance without asking.
/// Nothing restores into it from now on, and saves save what's live.
pub fn run_fresh() -> Result<ExitCode> {
    let state = StateDir::from_env()?;
    let rex = Rex::from_env()?;
    let lock = RestoreLock::acquire(&state)?;
    let server = rex.server_status()?;
    if lock.is_restored(&server.instance_id)? {
        println!("already decided for this server instance; nothing to do");
        return Ok(ExitCode::SUCCESS);
    }
    lock.record_restored(&server.instance_id)?;
    Log::new(&state).append(&format!(
        "fresh: started fresh on server instance {}",
        server.instance_id
    ))?;
    println!("starting fresh: nothing will restore into this server instance");
    Ok(ExitCode::SUCCESS)
}
