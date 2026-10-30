//! Command-line parsing.
//!
//! Each subcommand with options parses them into the options struct its module
//! defines. To add a subcommand: a `Command` variant, a line in `parse`, a
//! `parse_<name>` function and a line in `USAGE`. A subcommand that takes
//! another's options reuses its struct and parser: `setup` takes
//! `autosave install`'s.

use std::ffi::OsString;
use std::fmt;

use crate::auto_restore::AutoRestoreOptions;
use crate::autosave::{AutosaveCommand, InstallOptions};
use crate::report::ReportOptions;
use crate::restore::RestoreOptions;
use crate::scheduler::Interval;

pub enum Command {
    Save,
    Restore(RestoreOptions),
    List,
    Autosave(AutosaveCommand),
    AutoRestore(AutoRestoreOptions),
    Report(ReportOptions),
    Setup(InstallOptions),
    Help,
    Version,
}

impl Command {
    /// The command's name, for warnings and the log; `None` for help and
    /// the version, which read nothing, and for `report`, which never
    /// prints (a hook's output reaches its agent).
    pub fn name(&self) -> Option<&'static str> {
        match self {
            Command::Save => Some("save"),
            Command::Restore(_) => Some("restore"),
            Command::List => Some("list"),
            Command::Autosave(AutosaveCommand::Install(_)) => Some("autosave install"),
            Command::Autosave(AutosaveCommand::Uninstall) => Some("autosave uninstall"),
            Command::AutoRestore(_) => Some("auto-restore"),
            Command::Setup(_) => Some("setup"),
            Command::Report(_) | Command::Help | Command::Version => None,
        }
    }
}

pub const USAGE: &str = "\
Save Rex sessions and recreate them after the Rex server stops.

Usage: rexurrect <command> [options]

Commands:
  save      Save every session on the Rex server to a new snapshot
  restore   Recreate the latest snapshot's sessions that aren't live
  list      List the snapshots, newest first, with their session labels
  autosave install
            Run `save` every 15 minutes in the background (a launchd agent)
  autosave uninstall
            Stop autosave, and say how to remove the rest of rexurrect
  auto-restore
            For Rex's init.lua: once the server running it is ready, restore
            into it in the background, once per server instance
  report <program> [value]
            For a program's hook: store the program's resume state for the
            block it runs in (the hook's payload on stdin, without a value).
            Never prints, always exits 0
  setup     Turn autosave on, then print the lines that turn auto-restore
            and the zsh recorder on, for you to add
  help      Show this help

Restore options:
  --snapshot <id>    Restore from this snapshot (an ID from `list`) instead
                     of the latest
  --session <label>  Restore only the saved sessions with this label
  -n, --dry-run      Print what would be created and skipped; create nothing
  --force            Replace the saved sessions that are live (the ones
                     Rex carries over, without scrollback or programs)
                     instead of skipping them: create each, then close the
                     live one

Autosave install and setup options:
  --interval <minutes>  Save this often instead (default: 15)

Auto-restore options:
  --print-hook        Print the line that turns auto-restore on, to append to
                      Rex's init.lua; it pins this state directory and
                      config file
  --server-pid <pid>  Wait for the Rex server with this pid and restore into
                      it, in the foreground (what the background run does)

Report options:
  --print-hook  Print what to add to the program's own config (Claude Code's
                settings.json) to report its resume state; changes no file

Options:
  -h, --help     Show this help
  -V, --version  Show the version

Settings, in the config file $XDG_CONFIG_HOME/rexurrect/config.toml, else
~/.config/rexurrect/config.toml (TOML; no file means every default):
  keep = 10                   How many snapshots save keeps, newest first
  no_scrollback = [\"claude\"]  Programs whose blocks save keeps no scrollback
                              for
  ready_timeout = 60          How long auto-restore waits for the Rex server
                              to be ready, in seconds
  app_session_watch = 30      How long auto-restore looks for the app session
                              after restoring, in seconds
  auto_resume = []            Programs restore resumes running, not typed
                              (\"nvim\", \"claude\")
  [programs.<name>]           A program definition: match, state, resume,
                              requires_file, fallback, enabled (see README)

Environment:
  REX_SERVER           The Rex server to talk to (default: rex's own default)
  REXURRECT_STATE_DIR  Where snapshots and state live (default:
                       $XDG_STATE_HOME/rexurrect, else
                       ~/.local/state/rexurrect)
  REXURRECT_CONFIG     The config file to read instead
";

#[derive(Debug)]
pub struct UsageError(String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, UsageError> {
    let args: Vec<OsString> = args.into_iter().collect();
    if args.first().is_some_and(|command| command == "report") {
        return Ok(Command::Report(parse_report(&args[1..])));
    }
    let mut args = Args::new(args)?;
    if args.wants_help() {
        return Ok(Command::Help);
    }
    let Some(command) = args.next() else {
        return Err(UsageError("missing command".into()));
    };
    match command.as_str() {
        "save" => no_options(&mut args, "save").map(|()| Command::Save),
        "restore" => parse_restore(&mut args).map(Command::Restore),
        "list" => no_options(&mut args, "list").map(|()| Command::List),
        "autosave" => parse_autosave(&mut args).map(Command::Autosave),
        "auto-restore" => parse_auto_restore(&mut args).map(Command::AutoRestore),
        "setup" => parse_install(&mut args, "setup").map(Command::Setup),
        "help" => Ok(Command::Help),
        "-V" | "--version" | "version" => Ok(Command::Version),
        other => Err(UsageError(format!("unknown command {other:?}"))),
    }
}

/// For a command that takes no options or arguments.
fn no_options(args: &mut Args, command: &str) -> Result<(), UsageError> {
    match args.next() {
        Some(arg) => Err(args.unexpected(&arg, command)),
        None => Ok(()),
    }
}

fn parse_restore(args: &mut Args) -> Result<RestoreOptions, UsageError> {
    let mut options = RestoreOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--snapshot" => options.snapshot = Some(args.value(&arg)?),
            "--session" => options.session = Some(args.value(&arg)?),
            "-n" | "--dry-run" => options.dry_run = true,
            "--force" => options.force = true,
            _ => return Err(args.unexpected(&arg, "restore")),
        }
    }
    Ok(options)
}

fn parse_autosave(args: &mut Args) -> Result<AutosaveCommand, UsageError> {
    let Some(action) = args.next() else {
        return Err(UsageError("autosave needs `install` or `uninstall`".into()));
    };
    match action.as_str() {
        "install" => parse_install(args, "autosave install").map(AutosaveCommand::Install),
        "uninstall" => match args.next() {
            Some(arg) => Err(args.unexpected(&arg, "autosave uninstall")),
            None => Ok(AutosaveCommand::Uninstall),
        },
        _ => Err(args.unexpected(&action, "autosave")),
    }
}

/// The options of `autosave install`, which `setup` shares.
fn parse_install(args: &mut Args, command: &str) -> Result<InstallOptions, UsageError> {
    let mut options = InstallOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--interval" => {
                let minutes = args.value(&arg)?;
                options.interval = Interval::parse_minutes(&minutes).ok_or_else(|| {
                    UsageError(format!(
                        "--interval takes a whole number of minutes, at least 1, not {minutes:?}"
                    ))
                })?;
            }
            _ => return Err(args.unexpected(&arg, command)),
        }
    }
    Ok(options)
}

fn parse_auto_restore(args: &mut Args) -> Result<AutoRestoreOptions, UsageError> {
    let mut options = AutoRestoreOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server-pid" => {
                let pid = args.value(&arg)?;
                options.server_pid = Some(pid.parse().map_err(|_| {
                    UsageError(format!("--server-pid takes a process ID, not {pid:?}"))
                })?);
            }
            "--print-hook" => options.print_hook = true,
            _ => return Err(args.unexpected(&arg, "auto-restore")),
        }
    }
    if options.print_hook && options.server_pid.is_some() {
        return Err(UsageError(
            "--print-hook and --server-pid can't be combined".into(),
        ));
    }
    Ok(options)
}

/// `report`'s arguments. Never a usage error, nor help: `report` runs from
/// hooks, whose output reaches their agent, so arguments it can't use are
/// logged, not printed (see `report`). One that isn't UTF-8 is read with
/// its bad bytes replaced, which no program definition's state kind
/// accepts.
fn parse_report(args: &[OsString]) -> ReportOptions {
    let mut args = args.iter().map(|arg| arg.to_string_lossy().into_owned());
    let mut options = ReportOptions {
        program: args.next(),
        ..ReportOptions::default()
    };
    for arg in args {
        if arg == "--print-hook" {
            options.print_hook = true;
        } else if options.value.is_none() {
            options.value = Some(arg);
        } else {
            options.extra_arguments = true;
        }
    }
    options
}

/// The remaining arguments, as UTF-8 strings.
struct Args {
    rest: std::vec::IntoIter<String>,
}

impl Args {
    fn new(args: impl IntoIterator<Item = OsString>) -> Result<Args, UsageError> {
        let args = args
            .into_iter()
            .map(|a| {
                a.into_string()
                    .map_err(|a| UsageError(format!("argument {a:?} isn't valid UTF-8")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Args {
            rest: args.into_iter(),
        })
    }

    fn wants_help(&self) -> bool {
        self.rest
            .as_slice()
            .iter()
            .any(|a| a == "-h" || a == "--help")
    }

    fn next(&mut self) -> Option<String> {
        self.rest.next()
    }

    /// The value following `option`.
    fn value(&mut self, option: &str) -> Result<String, UsageError> {
        self.next()
            .ok_or_else(|| UsageError(format!("{option} needs a value")))
    }

    fn unexpected(&self, arg: &str, command: &str) -> UsageError {
        if arg.starts_with('-') {
            UsageError(format!("unknown option {arg:?} for {command}"))
        } else {
            UsageError(format!("unexpected argument {arg:?} for {command}"))
        }
    }
}
