//! The snapshot model: what `save` writes and `restore` reads.
//!
//! Only rexurrect reads this format (tests never look inside), so it can
//! change freely. Optional fields are skipped when empty so that fields added
//! later don't break older snapshots.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::lineage::Lineage;
use crate::rex::api::{Bounds, Direction};
use crate::timestamp::Timestamp;

/// Bumped on incompatible changes to this format.
pub const FORMAT: u32 = 1;

/// Every saved session at one point in time.
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub format: u32,
    /// Stored as RFC 3339, UTC.
    pub taken_at: Timestamp,
    pub sessions: Vec<SavedSession>,
}

impl Snapshot {
    pub fn new(taken_at: Timestamp, sessions: Vec<SavedSession>) -> Snapshot {
        Snapshot {
            format: FORMAT,
            taken_at,
            sessions,
        }
    }

    /// Checks what restore relies on: every session has at least one window
    /// and every split ratio, tiled or floating, is in (0, 1). That every
    /// layout node is exactly one of a block or a split holds by
    /// construction ([`SavedLayout`]).
    pub fn validate(&self) -> Result<()> {
        for session in &self.sessions {
            if session.windows.is_empty() {
                return Err(Error::new(format!(
                    "session {:?} has no windows",
                    session.label
                )));
            }
            for (i, window) in session.windows.iter().enumerate() {
                window
                    .layouts()
                    .try_for_each(validate_layout)
                    .map_err(|e| {
                        Error::new(format!(
                            "session {:?}, window {}: {e}",
                            session.label,
                            i + 1
                        ))
                    })?;
            }
        }
        Ok(())
    }
}

fn validate_layout(layout: &SavedLayout) -> Result<(), String> {
    match layout {
        SavedLayout::Block(_) => Ok(()),
        SavedLayout::Split(split) => {
            // Written so that NaN fails too.
            if !(split.ratio > 0.0 && split.ratio < 1.0) {
                return Err(format!("split ratio {} isn't between 0 and 1", split.ratio));
            }
            validate_layout(&split.before)?;
            validate_layout(&split.after)
        }
    }
}

/// A session as recorded in a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    /// Which lineage this session belongs to (ADR-0001). Restore skips it
    /// while a session of the same lineage is live.
    pub lineage: Lineage,
    /// Always saved: Rex's generated session labels can't be told apart
    /// from user-set ones.
    pub label: String,
    /// In window order.
    pub windows: Vec<SavedWindow>,
}

impl SavedSession {
    /// Every block, window by window, in [`SavedWindow::blocks`] order.
    pub fn blocks(&self) -> impl Iterator<Item = &SavedBlock> {
        self.windows.iter().flat_map(SavedWindow::blocks)
    }

    /// [`SavedSession::blocks`], to change.
    pub fn blocks_mut(&mut self) -> impl Iterator<Item = &mut SavedBlock> {
        self.windows.iter_mut().flat_map(SavedWindow::blocks_mut)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedWindow {
    /// Only a user-set label; omitted so Rex generates a fresh default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The window's tiled layout.
    pub layout: SavedLayout,
    /// The window's floating layers, back to front. Absent from snapshots
    /// taken before #84, and for windows with none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub floating: Vec<SavedFloatingLayer>,
}

impl SavedWindow {
    /// Its tiled layout, then each floating layer's, back to front.
    pub fn layouts(&self) -> impl Iterator<Item = &SavedLayout> {
        std::iter::once(&self.layout).chain(self.floating.iter().map(|layer| &layer.layout))
    }

    /// The window's blocks: its tiled layout's, then each floating layer's,
    /// back to front, each layout's in [`SavedLayout::blocks`] order. The
    /// order Rex returns new blocks' IDs in, window by window and layer by
    /// layer.
    pub fn blocks(&self) -> Vec<&SavedBlock> {
        self.layouts().flat_map(SavedLayout::blocks).collect()
    }

    /// [`SavedWindow::blocks`], to change.
    pub fn blocks_mut(&mut self) -> Vec<&mut SavedBlock> {
        std::iter::once(&mut self.layout)
            .chain(self.floating.iter_mut().map(|layer| &mut layer.layout))
            .flat_map(SavedLayout::blocks_mut)
            .collect()
    }
}

/// A floating layer: a layout over the window's tiled one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedFloatingLayer {
    /// Where it sits in its window, as Rex reported it.
    pub bounds: Bounds,
    pub layout: SavedLayout,
}

/// A layout node: exactly one of a block or a split, by construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedLayout {
    Block(SavedBlock),
    Split(SavedSplit),
}

impl SavedLayout {
    /// The layout's blocks in depth-first, `before`-then-`after` order.
    pub fn blocks(&self) -> Vec<&SavedBlock> {
        match self {
            SavedLayout::Block(block) => vec![block],
            SavedLayout::Split(split) => {
                let mut blocks = split.before.blocks();
                blocks.extend(split.after.blocks());
                blocks
            }
        }
    }

    /// [`SavedLayout::blocks`], to change.
    pub fn blocks_mut(&mut self) -> Vec<&mut SavedBlock> {
        match self {
            SavedLayout::Block(block) => vec![block],
            SavedLayout::Split(split) => {
                let mut blocks = split.before.blocks_mut();
                blocks.extend(split.after.blocks_mut());
                blocks
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSplit {
    pub direction: Direction,
    /// `before`'s share, in (0, 1).
    pub ratio: f64,
    pub before: Box<SavedLayout>,
    pub after: Box<SavedLayout>,
}

/// A shell block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedBlock {
    /// Only a user-set label; omitted so Rex uses its default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub working_directory: PathBuf,
    /// The block's last command, as found: it may not be typeable (see
    /// `create::typed_command`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_command: Option<String>,
    /// The block's saved scrollback: a file's path relative to the state
    /// directory (see `scrollback`). Absent for a block that was showing a
    /// full-screen program, or had nothing to save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollback: Option<PathBuf>,
    /// The program in the block's foreground at the save, if it matched a
    /// program definition: restore resumes it if it can (see `resume`).
    /// Absent from snapshots taken before rexurrect resumed programs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<SavedResume>,
    /// The live block's ID at the save, which names its recorder file: a
    /// restore before that file is pruned can tell whether the block's
    /// shell got its prompt back after the save (see `exited`). Absent from
    /// snapshots taken before #60, and for blocks that aren't local shells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
}

/// A block's resumable program, as save found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedResume {
    /// The name of the program definition it matched. The definition itself
    /// is read at restore, so a change to it applies to existing snapshots.
    pub program: String,
    /// The resume state the program had reported for the block, if any, and
    /// if it was of the kind its definition expected (see `resume_state`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// The program's own command line, as the foreground fallback reads it
    /// (see `foreground`); absent if it couldn't be read or typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_line: Option<String>,
    /// When the block's recorder file was last written, as save read it
    /// just after finding the program: a record written since means the
    /// block's shell got its prompt back, so the program had exited (see
    /// `exited`). Absent if the block had no record then, and from
    /// snapshots taken before #60.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_written_at: Option<Timestamp>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Snapshots taken before rexurrect resumed programs restore as before.
    #[test]
    fn a_block_saved_before_resuming_existed_has_nothing_to_resume() {
        let block: SavedBlock = serde_json::from_value(json!({
            "working_directory": "/w/a",
            "last_command": "nvim init.lua",
        }))
        .unwrap();
        assert_eq!(block.resume, None);
    }

    #[test]
    fn a_resumable_block_records_its_program_and_command_line() {
        let block: SavedBlock = serde_json::from_value(json!({
            "working_directory": "/w/a",
            "resume": { "program": "nvim", "command_line": "nvim init.lua" },
        }))
        .unwrap();
        let resume = block.resume.unwrap();
        assert_eq!(resume.program, "nvim");
        assert_eq!(resume.command_line.as_deref(), Some("nvim init.lua"));
    }

    /// Snapshots taken before #84 have no floating layers, and a window
    /// with none is written as before.
    #[test]
    fn a_window_saved_before_floating_layers_were_has_none_and_writes_none() {
        let window: SavedWindow = serde_json::from_value(json!({
            "layout": { "block": { "working_directory": "/w/a" } },
        }))
        .unwrap();
        assert!(window.floating.is_empty());
        let written = serde_json::to_value(&window).unwrap();
        assert_eq!(written.get("floating"), None, "{written}");
    }

    #[test]
    fn a_floating_layers_blocks_come_after_the_tiled_layouts_back_to_front() {
        let block = |dir: &str| json!({ "block": { "working_directory": dir } });
        let window: SavedWindow = serde_json::from_value(json!({
            "layout": block("/tiled"),
            "floating": [
                { "bounds": [0.1, 0.1, 0.5, 0.5], "layout": { "split": {
                    "direction": "vertical", "ratio": 0.3,
                    "before": block("/back/top"), "after": block("/back/bottom"),
                } } },
                { "bounds": [0.2, 0.2, 0.5, 0.5], "layout": block("/front") },
            ],
        }))
        .unwrap();
        let dirs: Vec<_> = window
            .blocks()
            .iter()
            .map(|b| b.working_directory.to_str().unwrap())
            .collect();
        assert_eq!(dirs, ["/tiled", "/back/top", "/back/bottom", "/front"]);
    }

    /// Snapshots taken before #60 name no block, so restore can't tell
    /// whether a resumable block's program exited after them: it resumes.
    #[test]
    fn a_resumable_block_saved_before_block_ids_were_kept_names_no_block() {
        let block: SavedBlock = serde_json::from_value(json!({
            "working_directory": "/w/a",
            "resume": { "program": "nvim" },
        }))
        .unwrap();
        assert_eq!(block.block_id, None);
        assert_eq!(block.resume.unwrap().record_written_at, None);
    }
}
