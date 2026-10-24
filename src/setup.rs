//! `rexurrect setup`: turn autosave on, then print the lines left for the
//! user to add. Setup never edits `init.lua` or `.zshrc` itself: they're the
//! user's to change. Everything after the install is output only, so running
//! setup again is as safe as running `autosave install` again.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::auto_restore;
use crate::autosave::{self, InstallOptions};
use crate::error::{Context, Error, Result};

pub fn run(options: &InstallOptions) -> Result<ExitCode> {
    // Fails before printing anything else, so a failed setup doesn't ask the
    // user to turn on the rest.
    autosave::install(options)?;
    let program = std::env::current_exe().with_context(|| "finding rexurrect's own path")?;
    println!(
        "
Two steps are left for you, each a line to add:

1. Auto-restore: add this line to Rex's init.lua.

{hook}

   This command appends it to ~/.config/rex/init.lua, where Rex looks by default
   (`rex config check` shows where yours is if you've moved it):

mkdir -p ~/.config/rex && {program} auto-restore --print-hook >> ~/.config/rex/init.lua",
        hook = auto_restore::hook()?,
        program = shell_quote(utf8(&program)?),
    );
    match installed_recorder(&program) {
        Some(recorder) => {
            let dir = recorder.parent().expect("the recorder is in a directory");
            println!(
                "
2. The zsh recorder: add this line to ~/.zshrc, then open new shells.

source {recorder}

   Or, with oh-my-zsh, make it a custom plugin, then add `rexurrect` to
   plugins=(...) in ~/.zshrc:

ln -s {dir} \"${{ZSH_CUSTOM:-$HOME/.oh-my-zsh/custom}}/plugins/rexurrect\"",
                recorder = shell_quote(utf8(&recorder)?),
                dir = shell_quote(utf8(dir)?),
            );
        }
        None => println!(
            "
2. The zsh recorder: there's no copy installed beside rexurrect. See
   {RECORDER_DOCS}
   for how to get it and load it from ~/.zshrc."
        ),
    }
    Ok(ExitCode::SUCCESS)
}

/// Where the README says how to get the recorder and load it.
const RECORDER_DOCS: &str = "https://github.com/cemcatik/rexurrect#the-zsh-recorder";

/// The recorder installed beside `program`, if there is one:
/// `share/rexurrect/` next to the `bin` directory `program` is in, where
/// Homebrew links the files the formula installs besides the binary.
///
/// `program` is the path rexurrect was run by. Under Homebrew that's the
/// symlink in the prefix's `bin`, not the binary in the versioned Cellar, so
/// the recorder's path stays the same across upgrades too.
fn installed_recorder(program: &Path) -> Option<PathBuf> {
    let prefix = program.parent()?.parent()?;
    let recorder = prefix.join("share/rexurrect/rexurrect.plugin.zsh");
    recorder.is_file().then_some(recorder)
}

fn utf8(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| Error::new(format!("{} isn't valid UTF-8", path.display())))
}

/// `word` as one word on a shell command line: as it is if no shell would
/// read anything in it specially, else in single quotes.
fn shell_quote(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+:@%=,".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}
