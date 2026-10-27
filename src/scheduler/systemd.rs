//! The systemd scheduler: a user timer, a `.service` and a `.timer` unit in
//! the user's unit directory, enabled and started with `systemctl --user`.
//!
//! It only writes unit files and runs the `systemctl` it finds on `PATH`, so
//! it builds and is tested on every platform; only [`super::for_this_platform`]
//! picks it on Linux.
//!
//! For testing:
//! - `systemctl` is found on `PATH`, and `$REXURRECT_SYSTEMD_USER_DIR`
//!   replaces the user's unit directory, so tests can install and uninstall
//!   with a stand-in `systemctl` and never touch the real systemd;
//! - `$REXURRECT_AUTOSAVE_LABEL` replaces the units' name, so a scratch
//!   timer tried out on the real systemd stays apart from the user's own.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::{Job, LABEL_VAR, Scheduler};
use crate::account;
use crate::error::{Context, Error, Result};
use crate::settings;
use crate::state;

/// The autosave units' name, before `.service` and `.timer`.
const NAME: &str = "rexurrect-autosave";
const UNIT_DIR_VAR: &str = "REXURRECT_SYSTEMD_USER_DIR";

#[derive(Debug)]
pub struct Systemd {
    name: String,
    unit_dir: PathBuf,
}

impl Systemd {
    /// The unit directory is the one systemd reads the user's own units
    /// from: `$XDG_CONFIG_HOME/systemd/user`, else `~/.config/systemd/user`.
    pub fn from_env() -> Result<Systemd> {
        let unit_dir = match settings::env_non_empty(UNIT_DIR_VAR) {
            Some(dir) => PathBuf::from(dir),
            None => match settings::env_non_empty("XDG_CONFIG_HOME") {
                Some(config) => PathBuf::from(config).join("systemd/user"),
                None => account::home()?.join(".config/systemd/user"),
            },
        };
        let name = super::label_from_env(NAME)?;
        if !is_unit_name(&name) {
            return Err(Error::new(format!(
                "{LABEL_VAR} must be a systemd unit name (letters, digits, `-`, `_`, `.` \
                 and `:`), not {name:?}"
            )));
        }
        Ok(Systemd { name, unit_dir })
    }

    fn service(&self) -> String {
        format!("{}.service", self.name)
    }

    fn timer(&self) -> String {
        format!("{}.timer", self.name)
    }

    fn service_path(&self) -> PathBuf {
        self.unit_dir.join(self.service())
    }

    fn timer_path(&self) -> PathBuf {
        self.unit_dir.join(self.timer())
    }

    /// Stops and disables the timer, if its unit is there to name, and
    /// removes both units. Whether there was anything to remove.
    fn remove(&self) -> Result<bool> {
        let timer = self.timer_path();
        let had_timer = timer.exists();
        if had_timer {
            systemctl_ok(&["disable", "--now", &self.timer()])?;
        }
        let removed_timer = state::remove_if_exists(&timer)?;
        let removed_service = state::remove_if_exists(&self.service_path())?;
        Ok(had_timer || removed_timer || removed_service)
    }
}

impl Scheduler for Systemd {
    fn describe(&self) -> String {
        format!(
            "systemd user timer {} ({})",
            self.timer(),
            self.unit_dir.display()
        )
    }

    /// An enabled timer starts at every login, so units systemd refused to
    /// enable or start now are disabled and removed again.
    fn install(&self, job: &Job) -> Result<()> {
        fs::create_dir_all(&self.unit_dir)
            .with_context(|| format!("creating {}", self.unit_dir.display()))?;
        state::write_atomically(&self.service_path(), service_unit(job).as_bytes(), 0o644)?;
        state::write_atomically(
            &self.timer_path(),
            timer_unit(&self.service(), job).as_bytes(),
            0o644,
        )?;
        // `restart`, not `start`: a timer already running from an earlier
        // install keeps its old period until it's restarted.
        let started = systemctl_ok(&["daemon-reload"])
            .and_then(|()| systemctl_ok(&["enable", &self.timer()]))
            .and_then(|()| systemctl_ok(&["restart", &self.timer()]));
        if let Err(e) = started {
            // A timer that didn't get as far as being enabled can't be
            // disabled, and that mustn't keep its units from being removed.
            let _ = systemctl_ok(&["disable", "--now", &self.timer()]);
            let cleaned = state::remove_if_exists(&self.timer_path())
                .and_then(|_| state::remove_if_exists(&self.service_path()))
                .and_then(|_| systemctl_ok(&["daemon-reload"]));
            return Err(match cleaned {
                Ok(()) => Error::new(format!("{e}; removed its units again")),
                Err(removing) => Error::new(format!("{e}; and then {removing}")),
            });
        }
        Ok(())
    }

    fn uninstall(&self) -> Result<bool> {
        let removed = self.remove()?;
        if removed {
            systemctl_ok(&["daemon-reload"])?;
        }
        Ok(removed)
    }
}

/// Whether systemd takes `name` as a unit's name, before its suffix: the
/// characters it allows in a plain (not template) unit's name.
fn is_unit_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// The service: runs the job once, in the background, with its output
/// discarded, as launchd's agent does. Failures go to rexurrect's log.
fn service_unit(job: &Job) -> String {
    let command: Vec<String> = std::iter::once(&job.program)
        .chain(&job.args)
        .map(|arg| quoted(arg, true))
        .collect();
    let mut environment = String::new();
    for (name, value) in &job.env {
        environment.push_str(&format!(
            "Environment={}\n",
            quoted(&format!("{name}={value}"), false)
        ));
    }
    format!(
        "\
[Unit]
Description=rexurrect autosave: save Rex sessions

[Service]
Type=oneshot
ExecStart={command}
{environment}StandardOutput=null
StandardError=null
",
        command = command.join(" "),
    )
}

/// The timer: runs the service one interval after it's started, then one
/// interval after each run starts.
fn timer_unit(service: &str, job: &Job) -> String {
    format!(
        "\
[Unit]
Description=Run rexurrect autosave every {interval}

[Timer]
Unit={service}
OnActiveSec={seconds}s
OnUnitActiveSec={seconds}s

[Install]
WantedBy=timers.target
",
        interval = job.interval,
        seconds = job.interval.seconds(),
    )
}

/// `word` as one double-quoted word of a unit setting, read back by systemd
/// as it is: C escapes for `\`, `"` and control characters, `%%` for `%`
/// (every setting expands `%` specifiers) and, in a command line (`exec`),
/// `$$` for `$` (`ExecStart=` expands variables; `Environment=` doesn't).
fn quoted(word: &str, exec: bool) -> String {
    let mut quoted = String::from("\"");
    for c in word.chars() {
        match c {
            '\\' => quoted.push_str(r"\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str(r"\n"),
            '\t' => quoted.push_str(r"\t"),
            '%' => quoted.push_str("%%"),
            '$' if exec => quoted.push_str("$$"),
            c if c.is_control() => quoted.push_str(&format!("\\x{:02x}", u32::from(c))),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// Runs `systemctl --user` (from `PATH`), failing if it does.
fn systemctl_ok(args: &[&str]) -> Result<()> {
    let shown = format!("systemctl --user {}", args.join(" "));
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running `{shown}`"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(Error::new(format!(
        "`{shown}` failed ({}): {}",
        out.status,
        stderr.trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_quoted_so_systemd_expands_nothing_in_it() {
        assert_eq!(
            quoted("/usr/bin/rexurrect", true),
            r#""/usr/bin/rexurrect""#
        );
        assert_eq!(
            quoted(r#"a %h $HOME "q" \b"#, true),
            r#""a %%h $$HOME \"q\" \\b""#
        );
        assert_eq!(quoted("V=$HOME %n", false), r#""V=$HOME %%n""#);
        assert_eq!(quoted("a\nb\u{1}", false), r#""a\nb\x01""#);
    }

    #[test]
    fn unit_names_are_what_systemd_accepts() {
        for good in [
            "rexurrect-autosave",
            "io.github.cemcatik.rexurrect.autosave",
            "a_b:c",
        ] {
            assert!(is_unit_name(good), "{good}");
        }
        for bad in ["", "a/b", "a b", "a@b", "a\nb", "é"] {
            assert!(!is_unit_name(bad), "{bad:?}");
        }
    }
}
