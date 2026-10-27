//! Running a job on a timer: the one interface autosave installs through.
//!
//! launchd on macOS ([`launchd::Launchd`]), a systemd user timer everywhere
//! else ([`systemd::Systemd`]), picked in [`for_this_platform`].

mod launchd;
mod systemd;

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

use crate::error::{Error, Result};
use crate::settings;

/// What a scheduler runs: a program with its arguments, every [`Interval`].
#[derive(Debug)]
pub struct Job {
    /// An absolute path: schedulers don't search `PATH` for it.
    pub program: String,
    pub args: Vec<String>,
    /// Variables set for the job, on top of the scheduler's own.
    pub env: BTreeMap<String, String>,
    pub interval: Interval,
}

/// Installs one job, the autosave, and removes it again.
pub trait Scheduler {
    /// What the job is installed as, for messages: "launchd agent …".
    fn describe(&self) -> String;

    /// Installs `job` and starts its timer, replacing the installed job if
    /// there is one, so installing twice leaves exactly one.
    fn install(&self, job: &Job) -> Result<()>;

    /// Stops the job and removes it. `false` if there was nothing to remove.
    fn uninstall(&self) -> Result<bool>;
}

/// Replaces the job's name in the scheduler (launchd's label, systemd's
/// unit name), so a scratch job tried out on the real scheduler stays apart
/// from the user's own.
const LABEL_VAR: &str = "REXURRECT_AUTOSAVE_LABEL";

/// Picks the scheduler (`launchd` or `systemd`) instead of the platform, so
/// the systemd scheduler's tests run on macOS too.
const SCHEDULER_VAR: &str = "REXURRECT_SCHEDULER";

/// The scheduler for the platform rexurrect runs on.
pub fn for_this_platform() -> Result<Box<dyn Scheduler>> {
    let chosen = settings::env_non_empty(SCHEDULER_VAR);
    let name = match chosen.as_ref().map(|name| name.to_str()) {
        Some(Some(name)) => name,
        Some(None) => return Err(Error::new(format!("{SCHEDULER_VAR} isn't valid UTF-8"))),
        None if cfg!(target_os = "macos") => "launchd",
        None => "systemd",
    };
    match name {
        "launchd" => Ok(Box::new(launchd::Launchd::from_env()?)),
        "systemd" => Ok(Box::new(systemd::Systemd::from_env()?)),
        other => Err(Error::new(format!(
            "{SCHEDULER_VAR} must be `launchd` or `systemd`, not {other:?}"
        ))),
    }
}

/// How often a job runs: a whole number of minutes, at least 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    minutes: NonZeroU32,
}

impl Interval {
    /// Every 15 minutes, like tmux-continuum.
    pub const DEFAULT: Interval = Interval {
        minutes: NonZeroU32::new(15).unwrap(),
    };

    /// Parses a whole number of minutes, at least 1 (`"15"`).
    pub fn parse_minutes(minutes: &str) -> Option<Interval> {
        settings::whole_number_at_least_1(minutes)
            .and_then(|minutes| NonZeroU32::try_from(minutes).ok())
            .map(|minutes| Interval { minutes })
    }

    pub fn seconds(&self) -> u64 {
        u64::from(self.minutes.get()) * 60
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.minutes.get() {
            1 => f.write_str("minute"),
            n => write!(f, "{n} minutes"),
        }
    }
}
