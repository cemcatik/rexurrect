//! rexurrect's settings: one TOML file (ADR-0007), which save, restore,
//! autosave and auto-restore all read, at every run.
//!
//! Location, first match wins:
//! 1. `$REXURRECT_CONFIG`
//! 2. `$XDG_CONFIG_HOME/rexurrect/config.toml` (only if `XDG_CONFIG_HOME` is
//!    absolute)
//! 3. `$HOME/.config/rexurrect/config.toml`
//!
//! Autosave runs under a scheduler and auto-restore inside the Rex server,
//! and neither sees the user's shell environment, so `autosave install` and
//! `auto-restore --print-hook` pin the file's path as they find it, as they
//! pin the state directory. They pin the path, never the values, so an edit
//! takes effect at the next run.
//!
//! No file means every default. A file that can't be read or parsed, a key
//! rexurrect doesn't know, or a value of the wrong kind is an error naming
//! the file and the key.
//!
//! The settings replaced environment variables, which are no longer read: a
//! run that finds one set warns, naming its replacement (see
//! [`warn_about_moved_variables`]).

use std::ffi::OsString;
use std::io::IsTerminal;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Context, Error, Result};
use crate::log;
use crate::programs::Programs;
use crate::retention::Retention;
use crate::scrollback::NoScrollbackPrograms;
use crate::settings;
use crate::state::{self, StateDir};

/// Names another config file instead.
pub const CONFIG_VAR: &str = "REXURRECT_CONFIG";

/// Every setting, as the file sets it or by default.
#[derive(Debug, Clone)]
pub struct Config {
    /// `keep`: how many snapshots save keeps, newest first (10).
    pub retention: Retention,
    /// `no_scrollback`: the programs whose blocks save keeps no scrollback
    /// for (`["claude"]`).
    pub no_scrollback: NoScrollbackPrograms,
    /// `ready_timeout`: how long auto-restore waits for the Rex server to be
    /// ready (60 seconds).
    pub ready_timeout: Duration,
    /// `app_session_watch`: how long auto-restore looks for the app session
    /// after restoring (30 seconds).
    pub app_session_watch: Duration,
    /// The program definitions blocks resume by: the built-in ones.
    pub programs: Programs,
    /// `auto_resume`: the program definitions whose programs restore runs
    /// rather than types (ADR-0006); none by default.
    pub auto_resume: Vec<String>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            retention: Retention::new(NonZeroUsize::new(10).expect("10 is at least 1")),
            no_scrollback: NoScrollbackPrograms::new(vec!["claude".to_string()]),
            ready_timeout: Duration::from_secs(60),
            app_session_watch: Duration::from_secs(30),
            programs: Programs::default(),
            auto_resume: Vec::new(),
        }
    }
}

impl Config {
    /// The settings in the config file (see [`ConfigFile::from_env`]).
    pub fn load() -> Result<Config> {
        ConfigFile::from_env()?.load()
    }

    /// The settings `text`, the contents of the file at `path`, holds.
    fn parse(path: &Path, text: &str) -> Result<Config> {
        let table: toml::Table = text
            .parse()
            .with_context(|| format!("reading {}", path.display()))?;
        let mut config = Config::default();
        for (key, value) in &table {
            let wrong = |what: &str| {
                Error::new(format!(
                    "`{key}` in {} must be {what}, not {value}",
                    path.display()
                ))
            };
            match key.as_str() {
                "keep" => {
                    let keep = whole_number(value)
                        .and_then(|keep| NonZeroUsize::try_from(keep).ok())
                        .ok_or_else(|| wrong("a whole number of snapshots to keep, at least 1"))?;
                    config.retention = Retention::new(keep);
                }
                "no_scrollback" => {
                    let names = program_names(value)
                        .ok_or_else(|| wrong("a list of program names, like [\"claude\"]"))?;
                    config.no_scrollback = NoScrollbackPrograms::new(names);
                }
                "ready_timeout" => {
                    config.ready_timeout = seconds(value).ok_or_else(|| wrong(SECONDS_WANTED))?;
                }
                "app_session_watch" => {
                    config.app_session_watch =
                        seconds(value).ok_or_else(|| wrong(SECONDS_WANTED))?;
                }
                "auto_resume" => {
                    config.auto_resume = program_names(value)
                        .ok_or_else(|| wrong("a list of program names, like [\"nvim\"]"))?;
                }
                _ => {
                    return Err(Error::new(format!(
                        "`{key}` in {} isn't a setting rexurrect knows; it knows {}",
                        path.display(),
                        KEYS.join(", ")
                    )));
                }
            }
        }
        if let Some(unknown) = config
            .auto_resume
            .iter()
            .find(|name| config.programs.get(name).is_none())
        {
            return Err(Error::new(format!(
                "`auto_resume` in {} names {unknown:?}, which no program definition has; \
                 they are {}",
                path.display(),
                config.programs.names().collect::<Vec<_>>().join(", ")
            )));
        }
        Ok(config)
    }
}

/// Every key the file can set.
const KEYS: [&str; 5] = [
    "keep",
    "no_scrollback",
    "ready_timeout",
    "app_session_watch",
    "auto_resume",
];

const SECONDS_WANTED: &str = "a whole number of seconds, at least 1";

/// A TOML integer of at least 1.
fn whole_number(value: &toml::Value) -> Option<NonZeroU64> {
    value
        .as_integer()
        .and_then(|n| u64::try_from(n).ok())
        .and_then(NonZeroU64::new)
}

fn seconds(value: &toml::Value) -> Option<Duration> {
    whole_number(value).map(|seconds| Duration::from_secs(seconds.get()))
}

/// A TOML array of strings.
fn program_names(value: &toml::Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|name| name.as_str().map(str::to_string))
        .collect()
}

/// Where the config file is, whether or not it exists.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    path: PathBuf,
}

impl ConfigFile {
    pub fn from_env() -> Result<ConfigFile> {
        let var = settings::env_non_empty;
        resolve(var(CONFIG_VAR), var("XDG_CONFIG_HOME"), var("HOME"))
            .map(|path| ConfigFile { path })
            .ok_or_else(|| {
                Error::new(format!(
                    "can't find the config file: set {CONFIG_VAR}, XDG_CONFIG_HOME or HOME"
                ))
            })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The settings the file holds; every default if there's no file.
    pub fn load(&self) -> Result<Config> {
        match state::read_file(&self.path)? {
            None => Ok(Config::default()),
            Some(bytes) => {
                let text = String::from_utf8(bytes)
                    .with_context(|| format!("reading {}", self.path.display()))?;
                Config::parse(&self.path, &text)
            }
        }
    }
}

fn resolve(
    override_path: Option<OsString>,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(path) = override_path {
        return Some(PathBuf::from(path));
    }
    if let Some(xdg) = xdg_config_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Some(xdg.join(PATH_IN_CONFIG_DIR));
    }
    home.map(|home| PathBuf::from(home).join(".config").join(PATH_IN_CONFIG_DIR))
}

/// The file's path in a config directory.
const PATH_IN_CONFIG_DIR: &str = "rexurrect/config.toml";

/// A setting that was an environment variable before it moved into the
/// file.
struct Moved {
    var: &'static str,
    key: &'static str,
    old_value: OldValue,
}

/// What a moved variable held, and so what it becomes in the file.
enum OldValue {
    /// A whole number; empty counted as unset.
    Number,
    /// Program names separated by spaces; empty meant none.
    Programs,
}

const MOVED: [Moved; 4] = [
    Moved {
        var: "REXURRECT_KEEP",
        key: "keep",
        old_value: OldValue::Number,
    },
    Moved {
        var: "REXURRECT_NO_SCROLLBACK",
        key: "no_scrollback",
        old_value: OldValue::Programs,
    },
    Moved {
        var: "REXURRECT_READY_TIMEOUT",
        key: "ready_timeout",
        old_value: OldValue::Number,
    },
    Moved {
        var: "REXURRECT_APP_SESSION_WATCH",
        key: "app_session_watch",
        old_value: OldValue::Number,
    },
];

/// The environment variables rexurrect no longer reads because their
/// settings moved into the file.
pub fn moved_variables() -> impl Iterator<Item = &'static str> {
    MOVED.iter().map(|moved| moved.var)
}

/// One warning for each moved variable `var` finds set, naming the line to
/// put in `file` instead: `REXURRECT_KEEP is no longer read: set keep = 30
/// in <file>`.
fn moved_variable_warnings(file: &Path, var: impl Fn(&str) -> Option<OsString>) -> Vec<String> {
    MOVED
        .iter()
        .filter_map(|moved| {
            let value = var(moved.var)?;
            let value = value.to_string_lossy();
            let line = match moved.old_value {
                OldValue::Number if value.is_empty() => return None,
                // As the file would have it, even a number the file would
                // refuse: loading it then says what's wrong with it.
                OldValue::Number => match value.trim().parse::<i64>() {
                    Ok(number) => toml::Value::Integer(number),
                    Err(_) => toml::Value::String(value.into_owned()),
                },
                OldValue::Programs => toml::Value::Array(
                    value
                        .split_whitespace()
                        .map(|name| toml::Value::String(name.to_string()))
                        .collect(),
                ),
            };
            Some(format!(
                "{} is no longer read: set {} = {line} in {}",
                moved.var,
                moved.key,
                file.display()
            ))
        })
        .collect()
}

/// Warns about each moved variable that's set, once per run: on stderr,
/// and, unless that's a terminal, in the log too, since autosave and
/// auto-restore run where nobody sees their stderr. Never fails: a warning
/// that can't be logged is still on stderr.
pub fn warn_about_moved_variables(command: &str) {
    let file = ConfigFile::from_env().map_or_else(
        |_| PathBuf::from("the config file"),
        |file| file.path.clone(),
    );
    let warnings = moved_variable_warnings(&file, |name| std::env::var_os(name));
    if warnings.is_empty() {
        return;
    }
    let state = (!std::io::stderr().is_terminal())
        .then(StateDir::from_env)
        .and_then(Result::ok);
    for warning in &warnings {
        eprintln!("rexurrect: {warning}");
        if let Some(state) = &state {
            log::log_warning(state, command, warning);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Config> {
        Config::parse(Path::new("/c/config.toml"), text)
    }

    fn error(text: &str) -> String {
        parse(text).expect_err(text).to_string()
    }

    #[test]
    fn an_empty_file_means_every_default() {
        let config = parse("").unwrap();
        let default = Config::default();
        assert_eq!(config.retention.keep(), default.retention.keep());
        assert_eq!(config.no_scrollback.names(), ["claude"]);
        assert_eq!(config.ready_timeout, Duration::from_secs(60));
        assert_eq!(config.app_session_watch, Duration::from_secs(30));
        assert!(config.auto_resume.is_empty());
    }

    #[test]
    fn each_key_sets_its_setting() {
        let config = parse(
            "keep = 3\n\
             no_scrollback = [\"claude\", \"aider\"]\n\
             ready_timeout = 5\n\
             app_session_watch = 7\n\
             auto_resume = [\"nvim\", \"claude\"]\n",
        )
        .unwrap();
        assert_eq!(config.retention.keep(), 3);
        assert_eq!(config.no_scrollback.names(), ["claude", "aider"]);
        assert_eq!(config.ready_timeout, Duration::from_secs(5));
        assert_eq!(config.app_session_watch, Duration::from_secs(7));
        assert_eq!(config.auto_resume, ["nvim", "claude"]);
    }

    #[test]
    fn an_unknown_program_in_auto_resume_is_an_error_naming_it() {
        let error = error("auto_resume = [\"nvim\", \"emacs\"]");
        assert!(
            error.contains("/c/config.toml")
                && error.contains("`auto_resume`")
                && error.contains("\"emacs\""),
            "{error}"
        );
    }

    #[test]
    fn an_empty_no_scrollback_list_means_none() {
        let config = parse("no_scrollback = []").unwrap();
        assert!(config.no_scrollback.names().is_empty());
    }

    #[test]
    fn a_value_of_the_wrong_kind_is_an_error_naming_the_file_and_the_key() {
        for (text, key) in [
            ("keep = 0", "keep"),
            ("keep = -5", "keep"),
            ("keep = 1.5", "keep"),
            ("keep = \"10\"", "keep"),
            ("no_scrollback = \"claude aider\"", "no_scrollback"),
            ("no_scrollback = [\"claude\", 3]", "no_scrollback"),
            ("ready_timeout = 0", "ready_timeout"),
            ("ready_timeout = \"soon\"", "ready_timeout"),
            ("app_session_watch = 1.5", "app_session_watch"),
            ("auto_resume = \"nvim\"", "auto_resume"),
        ] {
            let error = error(text);
            assert!(
                error.contains("/c/config.toml") && error.contains(&format!("`{key}`")),
                "{text}: {error}"
            );
        }
    }

    #[test]
    fn a_bad_value_is_quoted_as_the_file_has_it() {
        let error = error("ready_timeout = \"soon\"");
        assert!(error.contains("\"soon\""), "{error}");
    }

    #[test]
    fn an_unknown_key_is_an_error_naming_the_file_and_the_key() {
        let error = error("kep = 3");
        assert!(
            error.contains("/c/config.toml") && error.contains("`kep`"),
            "{error}"
        );
    }

    #[test]
    fn a_file_that_isnt_toml_is_an_error_naming_the_file() {
        let error = error("keep = ");
        assert!(error.contains("/c/config.toml"), "{error}");
    }

    #[test]
    fn the_file_is_rexurrect_config_first() {
        let found = resolve(
            Some("/elsewhere/rx.toml".into()),
            Some("/xdg".into()),
            Some("/home/u".into()),
        );
        assert_eq!(found, Some(PathBuf::from("/elsewhere/rx.toml")));
    }

    #[test]
    fn the_file_is_under_an_absolute_xdg_config_home_next() {
        let found = resolve(None, Some("/xdg".into()), Some("/home/u".into()));
        assert_eq!(found, Some(PathBuf::from("/xdg/rexurrect/config.toml")));
    }

    #[test]
    fn a_relative_xdg_config_home_is_ignored() {
        let found = resolve(None, Some("xdg".into()), Some("/home/u".into()));
        assert_eq!(
            found,
            Some(PathBuf::from("/home/u/.config/rexurrect/config.toml"))
        );
    }

    #[test]
    fn without_home_there_is_no_file() {
        assert_eq!(resolve(None, None, None), None);
    }

    fn warnings(set: &[(&str, &str)]) -> Vec<String> {
        moved_variable_warnings(Path::new("/c/config.toml"), |name| {
            set.iter()
                .find(|(var, _)| *var == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn no_moved_variable_set_means_no_warning() {
        assert_eq!(
            warnings(&[("REXURRECT_STATE_DIR", "/s")]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn each_moved_variable_set_warns_with_the_line_to_put_in_the_file() {
        assert_eq!(
            warnings(&[
                ("REXURRECT_KEEP", "30"),
                ("REXURRECT_NO_SCROLLBACK", "claude aider"),
                ("REXURRECT_READY_TIMEOUT", " 5 "),
                ("REXURRECT_APP_SESSION_WATCH", "2"),
            ]),
            [
                "REXURRECT_KEEP is no longer read: set keep = 30 in /c/config.toml",
                "REXURRECT_NO_SCROLLBACK is no longer read: \
                 set no_scrollback = [\"claude\", \"aider\"] in /c/config.toml",
                "REXURRECT_READY_TIMEOUT is no longer read: set ready_timeout = 5 in /c/config.toml",
                "REXURRECT_APP_SESSION_WATCH is no longer read: \
                 set app_session_watch = 2 in /c/config.toml",
            ]
        );
    }

    /// An empty `REXURRECT_NO_SCROLLBACK` meant no programs, so it's set;
    /// the others counted as unset when empty.
    #[test]
    fn only_an_empty_no_scrollback_still_warns() {
        assert_eq!(
            warnings(&[("REXURRECT_KEEP", ""), ("REXURRECT_NO_SCROLLBACK", "")]),
            ["REXURRECT_NO_SCROLLBACK is no longer read: set no_scrollback = [] in /c/config.toml"]
        );
    }

    /// The line it suggests is valid TOML, so loading it names what's wrong.
    #[test]
    fn a_value_that_isnt_a_number_is_suggested_as_a_string() {
        assert_eq!(
            warnings(&[("REXURRECT_READY_TIMEOUT", "soon")]),
            ["REXURRECT_READY_TIMEOUT is no longer read: \
              set ready_timeout = \"soon\" in /c/config.toml"]
        );
    }
}
