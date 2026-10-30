//! `rexurrect restore`: recreate a snapshot's sessions, the latest by default.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::account;
use crate::config::Config;
use crate::create;
use crate::defaults::DefaultCommands;
use crate::error::{Context, Error, Result};
use crate::exited;
use crate::instance::RestoreLock;
use crate::lineage::{Lineage, LineageMap};
use crate::log::{self, Log};
use crate::recorder::RecorderFiles;
use crate::resume::{Input, Resuming};
use crate::rex::Rex;
use crate::rex::api::{CreateFloatingLayer, CreatedSession, NewLayer, ServerStatus};
use crate::scrollback::Replay;
use crate::settings::{self, BLOCK_VAR};
use crate::snapshot::{SavedSession, Snapshot};
use crate::startup::StartingSessions;
use crate::state::StateDir;
use crate::store::SnapshotStore;
use crate::timestamp::Timestamp;

#[derive(Debug, Default, Clone)]
pub struct RestoreOptions {
    /// Restore from this snapshot (an identifier `list` shows) instead of the latest.
    pub snapshot: Option<String>,
    /// Restore only the saved sessions with this label (every one of them).
    pub session: Option<String>,
    /// Print what would be created and skipped, and create nothing.
    pub dry_run: bool,
    /// Replace each saved session's live session instead of skipping it
    /// (ADR-0008): create the saved session, then close the live one.
    pub force: bool,
    /// The block this restore runs in (`REX_BLOCK`). With `force`, the
    /// session holding it is closed last, once the restore has finished.
    pub own_block: Option<String>,
}

impl RestoreOptions {
    /// Whether this restore counts as restoring the server instance. A dry
    /// run creates nothing and a single-session restore leaves the other
    /// sessions out, so a save after either still restores first. Restoring
    /// a whole older snapshot counts.
    ///
    /// This deliberately departs from #15, which counts any restore: after
    /// `--session X` on a fresh server instance, the next autosave would
    /// otherwise replace the latest snapshot with one missing every other
    /// session, the very loss restoring first exists to prevent (story 48).
    fn restores_whole_snapshot(&self) -> bool {
        !self.dry_run && self.session.is_none()
    }
}

/// What happened to one saved session.
#[derive(Debug)]
pub enum Outcome {
    Created {
        label: String,
        session_id: String,
        /// Something that went wrong after it was created, too small to
        /// count as failing: it's still restored.
        note: Option<String>,
    },
    /// Created in place of a live session of its lineage, which restore
    /// then closed (a replaced session, ADR-0008).
    Replaced {
        label: String,
        session_id: String,
        /// The live session it replaced.
        closed_session_id: String,
        closed: Closed,
        /// As for [`Outcome::Created`].
        note: Option<String>,
    },
    /// A dry run would create it.
    WouldCreate { label: String },
    /// A dry run with `force` would replace this live session with it.
    WouldReplace {
        label: String,
        live_session_id: String,
    },
    /// A session of its lineage is already live.
    Skipped {
        label: String,
        live_session_id: String,
        /// Rex carried the live session over (ADR-0008): it holds a block
        /// of a snapshot taken before this server instance started.
        carried_over: bool,
    },
    Failed {
        label: String,
        error: String,
        /// The session restore created for it all the same, if it got that
        /// far. Its lineage is recorded, so no restore creates it again.
        session_id: Option<String>,
    },
}

/// Whether the live session that an [`Outcome::Replaced`] replaced was closed.
#[derive(Debug)]
pub enum Closed {
    Yes,
    /// Rex refused, so both are live: too small to count as failing.
    Failed(String),
    /// It holds the block this restore runs in, so it's closed last, once
    /// the restore has finished (see [`RestoreReport::close_last`]).
    Last,
}

#[derive(Debug, Default)]
pub struct RestoreReport {
    /// The identifier of the snapshot it restored from; `None` when there
    /// was no snapshot yet.
    pub snapshot: Option<String>,
    pub dry_run: bool,
    pub outcomes: Vec<Outcome>,
}

impl RestoreReport {
    /// Prints the report to `out`, logs each session that failed as a
    /// failure of `command`, and returns how the run should exit: non-zero
    /// if any session failed. Every command that restores ends this way.
    pub fn finish(&self, state: &StateDir, command: &str, out: &mut impl Write) -> ExitCode {
        let _ = self.print(out);
        self.log_failures(state, command);
        if self.failed() > 0 {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }

    fn failed(&self) -> usize {
        self.count(|o| matches!(o, Outcome::Failed { .. }))
    }

    /// Whether restore tried to create at least one session and every try
    /// failed before creating anything. Such a restore restored nothing, so
    /// it doesn't count as restoring the server instance, and a save after
    /// it mustn't replace the latest snapshot.
    pub fn every_attempt_failed(&self) -> bool {
        let attempted = self.count(|o| {
            matches!(
                o,
                Outcome::Created { .. } | Outcome::Replaced { .. } | Outcome::Failed { .. }
            )
        });
        attempted > 0 && !self.created_a_session()
    }

    /// Whether restore created at least one session, counting one that
    /// replaced a live one and one that failed after it was created. Only
    /// then does auto-restore look for the app session.
    pub fn created_a_session(&self) -> bool {
        self.outcomes.iter().any(|o| {
            matches!(
                o,
                Outcome::Created { .. }
                    | Outcome::Replaced { .. }
                    | Outcome::Failed {
                        session_id: Some(_),
                        ..
                    }
            )
        })
    }

    /// Whether restore skipped a session Rex carried over. Rex's copy is
    /// still live, without its scrollback or programs, so such a restore
    /// doesn't count as restoring the server instance: a save after it
    /// backs off rather than replace the latest snapshot with the copy.
    fn skipped_a_carried_over_session(&self) -> bool {
        self.outcomes.iter().any(|o| {
            matches!(
                o,
                Outcome::Skipped {
                    carried_over: true,
                    ..
                }
            )
        })
    }

    /// The live session to close once the restore has finished: the one
    /// that holds the block it runs in, which it replaced.
    pub fn close_last(&self) -> Option<&str> {
        self.outcomes.iter().find_map(|o| match o {
            Outcome::Replaced {
                closed_session_id,
                closed: Closed::Last,
                ..
            } => Some(closed_session_id.as_str()),
            _ => None,
        })
    }

    /// Appends each session that failed to the log, as a failure of `command`.
    fn log_failures(&self, state: &StateDir, command: &str) {
        for outcome in &self.outcomes {
            if let Outcome::Failed { label, error, .. } = outcome {
                log::log_failure(state, command, &format!("session {label:?}: {error}"));
            }
        }
    }

    fn count(&self, pred: impl Fn(&Outcome) -> bool) -> usize {
        self.outcomes.iter().filter(|o| pred(o)).count()
    }

    fn print(&self, out: &mut impl Write) -> std::io::Result<()> {
        if self.snapshot.is_none() {
            return writeln!(out, "nothing to restore: no snapshot yet");
        }
        for outcome in &self.outcomes {
            match outcome {
                Outcome::Created {
                    label,
                    session_id,
                    note,
                } => match note {
                    Some(note) => writeln!(out, "created  {label}  {session_id}  ({note})")?,
                    None => writeln!(out, "created  {label}  {session_id}")?,
                },
                Outcome::Replaced {
                    label,
                    session_id,
                    closed_session_id,
                    closed,
                    note,
                } => {
                    let mut notes = vec![match closed {
                        Closed::Yes => format!("closed {closed_session_id}"),
                        Closed::Failed(e) => format!("couldn't close {closed_session_id}: {e}"),
                        Closed::Last => {
                            format!("closing {closed_session_id} last: this restore runs in it")
                        }
                    }];
                    notes.extend(note.clone());
                    writeln!(
                        out,
                        "replaced  {label}  {session_id}  ({})",
                        notes.join("; ")
                    )?
                }
                Outcome::WouldCreate { label } => writeln!(out, "would create  {label}")?,
                Outcome::WouldReplace {
                    label,
                    live_session_id,
                } => writeln!(out, "would replace  {label}  (live as {live_session_id})")?,
                Outcome::Skipped {
                    label,
                    carried_over: true,
                    ..
                } => writeln!(
                    out,
                    "skipped  {label}  (carried over by Rex; --force replaces it)"
                )?,
                Outcome::Skipped {
                    label,
                    live_session_id,
                    ..
                } => writeln!(out, "skipped  {label}  (live as {live_session_id})")?,
                Outcome::Failed { label, error, .. } => writeln!(out, "failed   {label}: {error}")?,
            }
        }
        let skipped = self.count(|o| matches!(o, Outcome::Skipped { .. }));
        if self.dry_run {
            return writeln!(
                out,
                "dry run: {} would be created, {} would be replaced, {skipped} skipped; nothing \
                 was changed",
                self.count(|o| matches!(o, Outcome::WouldCreate { .. })),
                self.count(|o| matches!(o, Outcome::WouldReplace { .. })),
            );
        }
        writeln!(
            out,
            "restore: {} created, {} replaced, {skipped} skipped, {} failed",
            self.count(|o| matches!(o, Outcome::Created { .. })),
            self.count(|o| matches!(o, Outcome::Replaced { .. })),
            self.failed(),
        )
    }
}

pub fn run(options: &RestoreOptions) -> Result<ExitCode> {
    let config = Config::load()?;
    let state = StateDir::from_env()?;
    let lock = RestoreLock::acquire(&state)?;
    let rex = Rex::from_env()?;
    let server = rex.server_status()?;
    let options = RestoreOptions {
        own_block: settings::env_non_empty(BLOCK_VAR).map(|id| id.to_string_lossy().into_owned()),
        ..options.clone()
    };
    let report = restore(&rex, &state, &lock, &server, &config, &options)?;
    let mut printed = Vec::new();
    let status = report.finish(&state, "restore", &mut printed);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&printed).and_then(|()| out.flush());
    if let Some(session_id) = report.close_last() {
        // Closing it closes the terminal this run prints to, and may end
        // the run: everything is recorded by now, and the log keeps the
        // report.
        Log::new(&state).append(&format!(
            "restore --force, before closing {session_id}, the session it ran in: {}",
            String::from_utf8_lossy(&printed)
        ))?;
        if let Err(e) = rex.destroy_session(session_id) {
            let message = format!("couldn't close {session_id}, the session it ran in: {e}");
            eprintln!("rexurrect: {message}");
            Log::new(&state).append(&format!("restore --force {message}"))?;
        }
    }
    Ok(status)
}

/// Restores into the server's current instance, the one `server` (its
/// `server.status`, read under `lock`) names, unless rexurrect already has:
/// `None` then. `save` runs this first; auto-restore, once the server is
/// ready, needs nothing else.
pub fn restore_unrestored_instance(
    rex: &Rex,
    state: &StateDir,
    lock: &RestoreLock,
    server: &ServerStatus,
    config: &Config,
    options: &RestoreOptions,
) -> Result<Option<RestoreReport>> {
    if lock.is_restored(&server.instance_id)? {
        return Ok(None);
    }
    restore(rex, state, lock, server, config, options).map(Some)
}

/// Restores a snapshot and, once that has finished, records the server
/// instance it restored into (`server`'s), unless `options` restore less
/// than a whole snapshot, every session it tried to create failed, or it
/// skipped a session Rex carried over. Sessions that failed don't stop it
/// from finishing.
pub fn restore(
    rex: &Rex,
    state: &StateDir,
    lock: &RestoreLock,
    server: &ServerStatus,
    config: &Config,
    options: &RestoreOptions,
) -> Result<RestoreReport> {
    let report = restore_sessions(rex, state, server, config, options)?;
    if options.restores_whole_snapshot()
        && !report.every_attempt_failed()
        && !report.skipped_a_carried_over_session()
    {
        lock.record_restored(&server.instance_id)?;
    }
    Ok(report)
}

/// Whether the server instance `server` describes holds a session Rex
/// carried over from the latest snapshot (see [`carried_over_sessions`]).
pub fn holds_carried_over_session(
    rex: &Rex,
    state: &StateDir,
    server: &ServerStatus,
) -> Result<bool> {
    let Some((_, snapshot)) = SnapshotStore::new(state).read_latest()? else {
        return Ok(false);
    };
    let live = rex.list_sessions()?;
    let carried_over = carried_over_sessions(
        rex,
        &snapshot,
        server,
        live.iter().map(|s| s.session_id.as_str()),
    )?;
    Ok(!carried_over.is_empty())
}

/// Whether `snapshot` was taken before the server instance `server`
/// describes started: in an earlier one.
fn taken_in_an_earlier_instance(snapshot: &Snapshot, server: &ServerStatus) -> bool {
    snapshot.taken_at < server.started_at
}

/// Those of the live sessions `session_ids` that Rex carried over into the
/// server instance `server` describes (ADR-0008): each holds a block that's
/// also in `snapshot`, taken before the instance started. Block IDs are
/// never reused, so only Rex's own restore brings one into a later
/// instance. Looks at none of them when `snapshot` was taken in this one.
fn carried_over_sessions<'a>(
    rex: &Rex,
    snapshot: &Snapshot,
    server: &ServerStatus,
    session_ids: impl IntoIterator<Item = &'a str>,
) -> Result<HashSet<String>> {
    if !taken_in_an_earlier_instance(snapshot, server) {
        return Ok(HashSet::new());
    }
    let saved: HashSet<&str> = snapshot
        .sessions
        .iter()
        .flat_map(SavedSession::blocks)
        .filter_map(|block| block.block_id.as_deref())
        .collect();
    let mut carried_over = HashSet::new();
    if saved.is_empty() {
        return Ok(carried_over);
    }
    for session_id in session_ids {
        let view = rex
            .view_session(session_id)
            .with_context(|| format!("looking for sessions Rex carried over: {session_id}"))?;
        if view.block_ids().any(|block_id| saved.contains(block_id)) {
            carried_over.insert(session_id.to_string());
        }
    }
    Ok(carried_over)
}

/// The one of the live sessions `session_ids` that holds the block `block_id`.
fn session_holding<'a>(
    rex: &Rex,
    block_id: &str,
    session_ids: impl IntoIterator<Item = &'a str>,
) -> Result<Option<String>> {
    for session_id in session_ids {
        let view = rex.view_session(session_id).with_context(|| {
            format!("looking for the session this restore runs in: {session_id}")
        })?;
        if view.block_ids().any(|id| id == block_id) {
            return Ok(Some(session_id.to_string()));
        }
    }
    Ok(None)
}

/// Creates each saved session whose lineage has no live session (ADR-0001),
/// or, with `force`, replaces the live one; see [`Creating::create`]. A dry
/// run only reports what it would create and replace. A session that fails,
/// to create or to have its blocks told apart, doesn't stop the others;
/// errors that stop the whole restore (no server, unreadable or unwritable
/// state) are returned.
fn restore_sessions(
    rex: &Rex,
    state: &StateDir,
    server: &ServerStatus,
    config: &Config,
    options: &RestoreOptions,
) -> Result<RestoreReport> {
    let Some((snapshot_id, snapshot)) = chosen_snapshot(state, options)? else {
        return Ok(RestoreReport::default());
    };
    let selected = selected_sessions(&snapshot, options)?;
    let live_sessions = rex.list_sessions()?;
    let mut lineages = LineageMap::load(state)?;
    let mut live: HashMap<Lineage, String> = live_sessions
        .iter()
        .map(|s| (lineages.lineage_of(&s.session_id), s.session_id.clone()))
        .collect();
    // The live sessions of the selected saved sessions' lineages: the ones
    // restore skips, or with `force` replaces.
    let matched: Vec<String> = selected
        .iter()
        .filter_map(|saved| live.get(&saved.lineage).cloned())
        .collect();
    let matched = || matched.iter().map(String::as_str);
    let (carried_over, own_session) = if options.force {
        let own_session = match &options.own_block {
            Some(block_id) if !options.dry_run => session_holding(rex, block_id, matched())?,
            _ => None,
        };
        (HashSet::new(), own_session)
    } else {
        (
            carried_over_sessions(rex, &snapshot, server, matched())?,
            None,
        )
    };
    let is_live = |id: &str| live_sessions.iter().any(|s| s.session_id == id);
    lineages.retain_live(is_live);
    let mut starting = StartingSessions::load(state)?;
    starting.retain(Timestamp::now(), is_live);
    let mut creating = Creating {
        rex,
        home: account::home()?,
        snapshot_id: &snapshot_id,
        // A block of an earlier instance's snapshot that's live in this one
        // is carried over, and its record rewritten by Rex's new shell.
        exited_before: taken_in_an_earlier_instance(&snapshot, server).then_some(server.started_at),
        lineages,
        starting,
        defaults: DefaultCommands::load(state)?,
        replay: Replay::new(state),
        resuming: Resuming::new(&config.programs, &config.auto_resume),
        recorder: RecorderFiles::new(state),
    };

    let mut report = RestoreReport {
        snapshot: Some(snapshot_id.clone()),
        dry_run: options.dry_run,
        ..RestoreReport::default()
    };
    for saved in selected {
        let label = saved.label.clone();
        let replacing = match live.get(&saved.lineage) {
            None => None,
            Some(live_session_id) if options.force => Some(live_session_id.clone()),
            Some(live_session_id) => {
                report.outcomes.push(Outcome::Skipped {
                    label,
                    carried_over: carried_over.contains(live_session_id),
                    live_session_id: live_session_id.clone(),
                });
                continue;
            }
        };
        if options.dry_run {
            report.outcomes.push(match replacing {
                Some(live_session_id) => Outcome::WouldReplace {
                    label,
                    live_session_id,
                },
                None => Outcome::WouldCreate { label },
            });
            continue;
        }
        let (outcome, created) = creating.create(saved)?;
        if let Some(session_id) = created {
            live.insert(saved.lineage.clone(), session_id);
        }
        report.outcomes.push(match replacing {
            Some(live_session_id) => {
                let last = own_session.as_deref() == Some(live_session_id.as_str());
                replaced(rex, outcome, live_session_id, last)
            }
            None => outcome,
        });
    }
    Ok(report)
}

/// `outcome`, the creation of a saved session in place of the live session
/// `live_session_id` of its lineage, as a replacement once that's closed,
/// unless it's to be closed `last`. If creating it failed, the live session
/// is left as it is, and so is `outcome`.
fn replaced(rex: &Rex, outcome: Outcome, live_session_id: String, last: bool) -> Outcome {
    let Outcome::Created {
        label,
        session_id,
        note,
    } = outcome
    else {
        return outcome;
    };
    let closed = if last {
        Closed::Last
    } else {
        match rex.destroy_session(&live_session_id) {
            Ok(()) => Closed::Yes,
            Err(e) => Closed::Failed(e.to_string()),
        }
    };
    Outcome::Replaced {
        label,
        session_id,
        closed_session_id: live_session_id,
        closed,
        note,
    }
}

/// What creating saved sessions needs, for one restore.
struct Creating<'a> {
    rex: &'a Rex,
    home: PathBuf,
    snapshot_id: &'a str,
    /// Records written from then on say nothing about a program exiting
    /// (see `exited`).
    exited_before: Option<Timestamp>,
    lineages: LineageMap,
    starting: StartingSessions,
    defaults: DefaultCommands,
    replay: Replay,
    resuming: Resuming<'a>,
    recorder: RecorderFiles,
}

impl Creating<'_> {
    /// Creates `saved` and records the new session's lineage, what it was
    /// created with for saves in its startup window (see `startup`), and
    /// what it typed at its blocks' prompts (see `resume`) as their default
    /// last commands for saves after that (see `defaults`). Returns its
    /// outcome, [`Outcome::Created`] or [`Outcome::Failed`], and the new
    /// session's ID if it got that far.
    fn create(&mut self, saved: &SavedSession) -> Result<(Outcome, Option<String>)> {
        let rex = self.rex;
        let label = saved.label.clone();
        let saved = exited::without_exited_programs(saved, &self.recorder, self.exited_before);
        let mut as_created = create::as_created(&saved, &self.home, &self.replay);
        let mut inputs: Vec<Option<Input>> = as_created
            .blocks()
            .map(|b| self.resuming.input(b))
            .collect();
        let keystrokes: Vec<Option<String>> = inputs
            .iter()
            .map(|input| input.as_ref().map(Input::keystrokes))
            .collect();
        let requests = create::create_requests(&as_created, &keystrokes, &self.replay);
        let created = match rex.create_session(&requests.session) {
            Ok(created) => created,
            Err(e) => {
                let failed = Outcome::Failed {
                    label,
                    error: e.to_string(),
                    session_id: None,
                };
                return Ok((failed, None));
            }
        };
        // The session exists from here on, whatever else fails: record its
        // lineage first, so no restore creates it again.
        self.lineages
            .record(&created.session_id, saved.lineage.clone());
        self.lineages.save().with_context(|| {
            format!(
                "recording the lineage of restored session {}",
                created.session_id
            )
        })?;
        // A layer that can't be created leaves the session restored without
        // it: a note, as below.
        let mut notes = Vec::new();
        let layer_ids = new_layers(rex, &created, &requests.floating_layers, &mut notes);
        without_missing_layers(&mut as_created, &mut inputs, &layer_ids);
        // session.create leaves the last window active: open on the first.
        if let [first, _, ..] = created.initial_windows.as_slice()
            && let Err(e) = rex.focus_window(&created.session_id, &first.window_id)
        {
            notes.push(format!("couldn't focus its first window: {e}"));
        }
        let note = (!notes.is_empty()).then(|| notes.join("; "));
        // Restore no longer writes recorder files (#6 had it seed each new
        // block's with what it typed there, which kept the foreground
        // fallback from ever applying to a restored block): what it typed
        // is the block's default, which a program seen there replaces.
        let block_ids = new_block_ids(&as_created, &created, &layer_ids);
        let recording = || {
            format!(
                "recording what restored session {} was created with",
                created.session_id
            )
        };
        if let Ok(block_ids) = &block_ids {
            let mut changed = false;
            for (block_id, input) in block_ids.iter().zip(&inputs) {
                if let Some(input) = input {
                    changed |= self.defaults.set(block_id, input.command());
                }
            }
            if changed {
                self.defaults.save().with_context(recording)?;
            }
        }
        self.starting.record(
            &created.session_id,
            self.snapshot_id,
            as_created,
            block_ids.as_ref().ok().cloned(),
        );
        self.starting.save().with_context(recording)?;
        let session_id = created.session_id.clone();
        let outcome = match block_ids {
            Ok(_) => Outcome::Created {
                label,
                session_id: created.session_id,
                note,
            },
            Err(e) => Outcome::Failed {
                label,
                error: format!(
                    "created as {}, but its new blocks couldn't be told apart, so their last \
                     commands won't carry forward: {e}{}",
                    created.session_id,
                    note.map(|note| format!("; it also {note}"))
                        .unwrap_or_default()
                ),
                session_id: Some(created.session_id),
            },
        };
        Ok((outcome, Some(session_id)))
    }
}

/// The snapshot to restore from, with its identifier: the one asked for,
/// else the latest (`None` before the first save).
fn chosen_snapshot(
    state: &StateDir,
    options: &RestoreOptions,
) -> Result<Option<(String, Snapshot)>> {
    let store = SnapshotStore::new(state);
    let Some(id) = &options.snapshot else {
        return store.read_latest();
    };
    match store.read(id)? {
        Some(snapshot) => Ok(Some((id.clone(), snapshot))),
        None => Err(Error::new(format!(
            "no snapshot {id:?}: `rexurrect list` shows the snapshots"
        ))),
    }
}

/// The saved sessions to restore: every one, or every one with the label
/// asked for. Asking for a label the snapshot doesn't hold is an error.
fn selected_sessions<'a>(
    snapshot: &'a Snapshot,
    options: &RestoreOptions,
) -> Result<Vec<&'a SavedSession>> {
    let Some(label) = &options.session else {
        return Ok(snapshot.sessions.iter().collect());
    };
    let selected: Vec<_> = snapshot
        .sessions
        .iter()
        .filter(|s| s.label == *label)
        .collect();
    if selected.is_empty() {
        let which = match &options.snapshot {
            Some(id) => format!("snapshot {id}"),
            None => "the latest snapshot".to_string(),
        };
        let labels: Vec<&str> = snapshot.sessions.iter().map(|s| s.label.as_str()).collect();
        return Err(Error::new(format!(
            "no saved session labelled {label:?} in {which}, which holds: {}",
            labels.join(", ")
        )));
    }
    Ok(selected)
}

/// Adds each window of `created` its floating layers, `layers` (in window
/// order, each window's back to front), without focusing any. Returns the
/// new blocks' IDs, layer by layer the same way, `None` for a layer that
/// couldn't be created; each of those is a note in `notes`.
///
/// Each layer goes in front of the window's others, so adding them back to
/// front stacks them as they were.
fn new_layers(
    rex: &Rex,
    created: &CreatedSession,
    layers: &[Vec<CreateFloatingLayer>],
    notes: &mut Vec<String>,
) -> Vec<Vec<Option<Vec<String>>>> {
    let mut ids = Vec::new();
    for (w, (window, layers)) in created.initial_windows.iter().zip(layers).enumerate() {
        let mut window_ids = Vec::new();
        for (l, layer) in layers.iter().enumerate() {
            let request = NewLayer {
                session_id: &created.session_id,
                window_id: &window.window_id,
                bounds: layer.bounds,
                layout: &layer.layout,
                focus: false,
            };
            window_ids.push(match rex.new_layer(&request) {
                Ok(new) => Some(new.block_ids),
                Err(e) => {
                    notes.push(format!(
                        "couldn't restore floating layer {} of window {}: {e}",
                        l + 1,
                        w + 1
                    ));
                    None
                }
            });
        }
        ids.push(window_ids);
    }
    ids
}

/// Leaves out of `as_created`, and of `inputs` (one per block of it, in
/// [`SavedSession::blocks`] order), the floating layers that
/// [`new_layers`] didn't create, `None` in `layer_ids`, so both describe
/// the session as it is.
fn without_missing_layers(
    as_created: &mut SavedSession,
    inputs: &mut Vec<Option<Input>>,
    layer_ids: &[Vec<Option<Vec<String>>>],
) {
    // Whether each block of `as_created` was created, in `inputs`' order.
    let mut kept = Vec::new();
    for (window, layer_ids) in as_created.windows.iter_mut().zip(layer_ids) {
        kept.extend(window.layout.blocks().iter().map(|_| true));
        let mut layer_ids = layer_ids.iter();
        window.floating.retain(|layer| {
            let created = layer_ids.next().is_some_and(Option::is_some);
            kept.extend(layer.layout.blocks().iter().map(|_| created));
            created
        });
    }
    let mut kept = kept.into_iter();
    inputs.retain(|_| kept.next().unwrap_or(true));
}

/// The IDs of the blocks `session.create` and then [`new_layers`] created
/// for `as_created`, in [`SavedSession::blocks`] order. `as_created` holds
/// only the floating layers that were created ([`without_missing_layers`]).
///
/// Both return a layout's block IDs in layout order (depth-first, `before`
/// then `after`), the order `SavedLayout::blocks` walks the saved layout in.
fn new_block_ids(
    as_created: &SavedSession,
    created: &CreatedSession,
    layer_ids: &[Vec<Option<Vec<String>>>],
) -> Result<Vec<String>> {
    if created.initial_windows.len() != as_created.windows.len() {
        return Err(Error::new(format!(
            "session.create returned {} windows for {} saved ones",
            created.initial_windows.len(),
            as_created.windows.len()
        )));
    }
    let mut ids = Vec::new();
    for (i, ((window, created_window), layer_ids)) in as_created
        .windows
        .iter()
        .zip(&created.initial_windows)
        .zip(layer_ids)
        .enumerate()
    {
        let blocks = window.layout.blocks().len();
        if blocks != created_window.block_ids.len() {
            return Err(Error::new(format!(
                "session.create returned {} block IDs for window {i}, which has {blocks} saved blocks",
                created_window.block_ids.len(),
            )));
        }
        ids.extend(created_window.block_ids.iter().cloned());
        for (layer, layer_ids) in window.floating.iter().zip(layer_ids.iter().flatten()) {
            let blocks = layer.layout.blocks().len();
            if blocks != layer_ids.len() {
                return Err(Error::new(format!(
                    "session.new_layer returned {} block IDs for a floating layer of window \
                     {i}, which has {blocks} saved blocks",
                    layer_ids.len(),
                )));
            }
            ids.extend(layer_ids.iter().cloned());
        }
    }
    Ok(ids)
}
