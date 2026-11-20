//! Each block's default last command: what a save saves as the last command
//! of a block whose recorder has recorded nothing (it has no recorder file)
//! while its shell sits at a prompt.
//!
//! It's the last program a save saw in the block's foreground, else the
//! command restore typed at the block's prompt when it created the block.
//! So without a recorder, a block keeps the program it was last seen
//! running after that program exits, and a restored block keeps the command
//! restore typed there, through any number of restores, until a save sees
//! a program in it. A block restore types nothing at has no default.
//!
//! A program a save saw keeps the directory it ran in beside it, if Rex
//! reported one: a command block that stays open once its program exits
//! (`--keep-open`) has no shell to report a working directory, and Rex
//! reports no foreground for it (#104). What restore typed has none: the
//! block it created is a shell block, and its shell reports one.
//!
//! Kept in rexurrect's own state, never in recorder files: those are the
//! recorders', so a recorder file always means a recorder wrote it, and
//! rexurrect can't race one. Only restore and save touch it, both under the
//! restore lock. A save that writes a snapshot forgets the blocks that
//! aren't live, as it does their recorder files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::state::{self, StateDir};

const FILE: &str = "default-commands.json";

/// Block ID -> default last command.
#[derive(Debug)]
pub struct DefaultCommands {
    path: PathBuf,
    commands: BTreeMap<String, Entry>,
}

/// A block's default last command, and the directory it ran in, if it's a
/// program a save saw and Rex reported one. Without a directory it's stored
/// as the command alone, as every entry was before #104.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Entry {
    Command(String),
    WithDirectory { command: String, directory: PathBuf },
}

impl Entry {
    fn new(command: &str, directory: Option<&Path>) -> Entry {
        match directory {
            Some(directory) => Entry::WithDirectory {
                command: command.to_string(),
                directory: directory.to_path_buf(),
            },
            None => Entry::Command(command.to_string()),
        }
    }

    fn command(&self) -> &str {
        match self {
            Entry::Command(command) | Entry::WithDirectory { command, .. } => command,
        }
    }

    fn directory(&self) -> Option<&Path> {
        match self {
            Entry::Command(_) => None,
            Entry::WithDirectory { directory, .. } => Some(directory),
        }
    }
}

impl DefaultCommands {
    pub fn load(state: &StateDir) -> Result<DefaultCommands> {
        let path = state.join(FILE);
        Ok(DefaultCommands {
            commands: state::read_json(&path)?,
            path,
        })
    }

    /// The block's default last command. Never one an earlier version saved
    /// from `ps`, with its control characters escaped: that isn't what ran
    /// (see [`is_escaped_by_ps`]).
    pub fn get(&self, block_id: &str) -> Option<&str> {
        self.entry(block_id).map(Entry::command)
    }

    /// The directory the block's default last command ran in, if it's a
    /// program a save saw there and Rex reported one.
    pub fn directory(&self, block_id: &str) -> Option<&Path> {
        self.entry(block_id).and_then(Entry::directory)
    }

    fn entry(&self, block_id: &str) -> Option<&Entry> {
        self.commands
            .get(block_id)
            .filter(|entry| !is_escaped_by_ps(entry.command()))
    }

    /// Makes `command`, run in `directory` if that's known, the block's
    /// default last command, and says whether that changed anything.
    pub fn set(&mut self, block_id: &str, command: &str, directory: Option<&Path>) -> bool {
        let entry = Entry::new(command, directory);
        if self.entry(block_id) == Some(&entry) {
            return false;
        }
        self.commands.insert(block_id.to_string(), entry);
        true
    }

    /// Forgets the blocks `is_live` says aren't live, and says whether there
    /// were any. Block IDs are never reused, so their entries can't matter
    /// again.
    pub fn retain_live(&mut self, is_live: impl Fn(&str) -> bool) -> bool {
        let before = self.commands.len();
        self.commands.retain(|block_id, _| is_live(block_id));
        self.commands.len() != before
    }

    pub fn save(&self) -> Result<()> {
        state::write_json(&self.path, &self.commands)
    }
}

/// Whether `command` holds `\` and three octal digits: how `ps`, which
/// earlier versions read the foreground program from, prints a control
/// character (a line break as `\012`). Such a default last command isn't what
/// ran, so it's treated as absent. A program seen with that text really in its
/// arguments is rarely dropped with it: the cost is an empty prompt.
/// Recorded commands aren't checked: `printf '\033[0m'` is typed by hand.
fn is_escaped_by_ps(command: &str) -> bool {
    command.as_bytes().windows(4).any(|window| {
        window[0] == b'\\'
            && window[1..]
                .iter()
                .all(|digit| (b'0'..=b'7').contains(digit))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state directory of its own, removed when dropped.
    struct TempState(StateDir);

    impl TempState {
        fn new(name: &str) -> TempState {
            let root = std::env::temp_dir()
                .join(format!("rexurrect-defaults-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            TempState(StateDir::at(root))
        }
    }

    impl Drop for TempState {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.path());
        }
    }

    /// #104: entries written before it are the command alone, and keep
    /// loading beside entries with a directory. One without a directory is
    /// still written as the command alone.
    #[test]
    fn commands_with_and_without_a_directory_load_and_save_together() {
        let state = TempState::new("mixed");
        let file = state.0.join(FILE);
        std::fs::write(
            &file,
            r#"{"block:a": "make test", "block:b": {"command": "cat", "directory": "/src/app"}}"#,
        )
        .unwrap();

        let mut defaults = DefaultCommands::load(&state.0).unwrap();
        assert_eq!(defaults.get("block:a"), Some("make test"));
        assert_eq!(defaults.directory("block:a"), None);
        assert_eq!(defaults.get("block:b"), Some("cat"));
        assert_eq!(defaults.directory("block:b"), Some(Path::new("/src/app")));

        assert!(defaults.set("block:c", "make watch", Some(Path::new("/src/web"))));
        assert!(!defaults.set("block:b", "cat", Some(Path::new("/src/app"))));
        assert!(defaults.set("block:b", "cat", None));
        defaults.save().unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({
                "block:a": "make test",
                "block:b": "cat",
                "block:c": { "command": "make watch", "directory": "/src/web" },
            })
        );
    }

    #[test]
    fn ps_escapes_are_recognised() {
        assert!(is_escaped_by_ps(r"sh -c tail -f /dev/null\012true"));
        assert!(is_escaped_by_ps(r"printf a\011b"));
        assert!(!is_escaped_by_ps(r"printf '\n'"));
        assert!(!is_escaped_by_ps(r"grep \089"));
        assert!(!is_escaped_by_ps("vim notes.txt"));
    }
}
