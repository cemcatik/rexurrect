//! The Rex API shapes rexurrect reads and sends, as verified against
//! Rex 0.0.1791570264-g7b5bf3 (floating layers and `session.new_layer`:
//! 0.0.1793131181-gdd9a20; the terminal `process` method's `last_exit`, and
//! its leaving out `foreground` once `last_exit` is set: 0.1.0-preview.21).
//!
//! Parsing is strict about what rexurrect relies on (a changed shape is an
//! error, not an empty snapshot) and lenient about the rest: unknown keys are
//! ignored, and keys Rex omits instead of sending `false`/`null` (`windows`
//! on an empty session, `layout` on a tiled layer whose blocks all floated
//! away) default.
//!
//! The model covers only the fields rexurrect reads.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::timestamp::Timestamp;

/// The terminal shell block flavor, today the only one Rex has.
pub const SHELL_FLAVOR: &str = "com.superlogical.terminal.shell";

/// Where a floating layer sits in its window: `[x, y, w, h]`, fractions of
/// the window's width and height, measured from its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds(pub [f64; 4]);

impl Bounds {
    /// The bounds made to fit inside the window, as `session.new_layer`
    /// requires: `x` and `y` within `[0, 1]`, and `w` and `h` shrunk so that
    /// `x + w` and `y + h` are at most 1. Bounds Rex reported already fit;
    /// this keeps a rounding error in a snapshot from failing the call.
    pub fn clamped(self) -> Bounds {
        // Shrunk rather than moved, so the layer keeps its top-left corner.
        // In f64, `x + (1 - x)` doesn't round past 1.
        let Bounds([x, y, w, h]) = self;
        let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        Bounds([x, y, w.min(1.0 - x), h.min(1.0 - y)])
    }
}

/// Split direction. `horizontal`: side by side, `before` on the left.
/// `vertical`: stacked, `before` on top. `ratio` is `before`'s share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Horizontal,
    Vertical,
}

// ---- server.status ---------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ServerStatus {
    /// The server's process.
    pub pid: u32,
    /// New for every run of the server (a server instance).
    pub instance_id: String,
    /// The local server's host; equals the `host_id` of every local block.
    pub host_id: String,
    /// When this server instance started.
    pub started_at: Timestamp,
    pub config: ServerConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// How many times the server has run init.lua: 1 from its start, one
    /// more for each `rex config reload`.
    pub generation: u64,
}

// ---- session.list ----------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SessionList {
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub label: String,
}

// ---- session.view ----------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SessionView {
    pub session_id: String,
    pub label: String,
    /// Absent for a session with no windows.
    #[serde(default)]
    pub active_window_id: Option<String>,
    /// In window order. Absent for a session with no windows (Rex
    /// keeps a session live after its last shell exits).
    #[serde(default)]
    pub windows: Vec<WindowView>,
}

impl SessionView {
    /// The ID of every block in the session, in every layer of every window.
    pub fn block_ids(&self) -> impl Iterator<Item = &str> {
        self.windows
            .iter()
            .flat_map(|window| &window.layers)
            .flat_map(|layer| &layer.blocks)
            .map(|block| block.block_id.as_str())
    }

    /// Cross-checks the keys Rex omits instead of sending them empty, so a
    /// renamed key can't pass for an empty value (and the session for one
    /// with nothing to save):
    /// - `windows` is absent only when there's no `active_window_id` either;
    /// - a tiled layer's `layout` is null only when its `blocks` are empty
    ///   (every block floated away);
    /// - a floating layer always has its `bounds` and a `layout`, which
    ///   save relies on.
    pub fn check_consistency(&self) -> Result<(), String> {
        if self.windows.is_empty() && self.active_window_id.is_some() {
            return Err("it names an active window but has no `windows`".into());
        }
        for window in &self.windows {
            if let Some(tiled) = window.tiled_layer()
                && tiled.layout.is_none()
                && !tiled.blocks.is_empty()
            {
                return Err(format!(
                    "window {}: its tiled layer has blocks but no `layout`",
                    window.window_id
                ));
            }
            for floating in window.floating_layers() {
                let missing = match (&floating.bounds, &floating.layout) {
                    (Some(_), Some(_)) => continue,
                    (None, _) => "bounds",
                    (Some(_), None) => "layout",
                };
                return Err(format!(
                    "window {}: a floating layer has no `{missing}`",
                    window.window_id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct WindowView {
    pub window_id: String,
    pub label: String,
    pub layers: Vec<LayerView>,
}

impl WindowView {
    /// The window's tiled layer: its layout. Floating layers aren't part of it.
    pub fn tiled_layer(&self) -> Option<&LayerView> {
        self.layers
            .iter()
            .find(|layer| layer.kind == LayerKind::Tiled)
    }

    /// The window's floating layers, back to front: `layers` lists them in
    /// that order.
    pub fn floating_layers(&self) -> impl Iterator<Item = &LayerView> {
        self.layers
            .iter()
            .filter(|layer| layer.kind == LayerKind::Floating)
    }
}

#[derive(Debug, Deserialize)]
pub struct LayerView {
    pub kind: LayerKind,
    /// Where the layer sits in its window. Always there for a floating
    /// layer ([`SessionView::check_consistency`]); a tiled layer fills its
    /// window.
    #[serde(default)]
    pub bounds: Option<Bounds>,
    /// `null` on a tiled layer whose blocks have all been floated.
    #[serde(default)]
    pub layout: Option<LayoutView>,
    /// The layer's blocks, joined to layout leaves by `block_id`.
    #[serde(default)]
    pub blocks: Vec<BlockView>,
}

impl LayerView {
    pub fn block(&self, block_id: &str) -> Option<&BlockView> {
        self.blocks.iter().find(|block| block.block_id == block_id)
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerKind {
    Tiled,
    Floating,
    #[serde(other)]
    Other,
}

/// A layout node: exactly one of a block leaf or a split. View-only fields
/// (`rect`, `split_id`, `focused`) are ignored.
#[derive(Debug)]
pub enum LayoutView {
    Block { block_id: String },
    Split(SplitView),
}

#[derive(Debug, Deserialize)]
pub struct SplitView {
    pub direction: Direction,
    pub ratio: f64,
    pub before: Box<LayoutView>,
    pub after: Box<LayoutView>,
}

impl<'de> Deserialize<'de> for LayoutView {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Node {
            #[serde(default)]
            block_id: Option<String>,
            #[serde(default)]
            split: Option<SplitView>,
        }
        match Node::deserialize(deserializer)? {
            Node {
                block_id: Some(block_id),
                split: None,
            } => Ok(LayoutView::Block { block_id }),
            Node {
                block_id: None,
                split: Some(split),
            } => Ok(LayoutView::Split(split)),
            Node {
                block_id: None,
                split: None,
            } => Err(serde::de::Error::custom(
                "layout node has neither `block_id` nor `split`",
            )),
            Node { .. } => Err(serde::de::Error::custom(
                "layout node has both `block_id` and `split`",
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BlockView {
    pub block_id: String,
    pub label: String,
    pub flavor: String,
    #[serde(default)]
    pub host_id: Option<String>,
}

// ---- shell blocks' terminal methods (through block.method) -----------------

/// The terminal `pwd` method. `{}` until the shell has reported a directory
/// (through OSC 7), and forever for shells that never do.
#[derive(Debug, Deserialize)]
pub struct TerminalPwd {
    /// The shell's report verbatim: `file://<host>/<path>`, percent-encoded
    /// (or not), any or empty host.
    #[serde(default)]
    pub pwd: Option<String>,
}

/// The terminal `process` method. Rex always reports `home`, and reports
/// `foreground` until the block's program exits, so both are required
/// otherwise: without them a save would quietly lose last commands and
/// working directories.
#[derive(Debug, Deserialize)]
#[serde(try_from = "ProcessAnswer")]
pub struct TerminalProcess {
    /// The process in the terminal's foreground: the block's shell while it's
    /// idle, otherwise the program it's running. `None` once the block's
    /// program has exited and the block stays open (`--keep-open`).
    pub foreground: Option<ProcessInfo>,
    /// The foreground process's ancestors, nearest first, ending with `login`.
    /// Rex leaves it out when the foreground process is the block's first
    /// (a command it started without `login`).
    pub ancestors: Vec<ProcessInfo>,
    /// The user's home directory.
    pub home: PathBuf,
}

/// The `process` answer as Rex sends it.
#[derive(Deserialize)]
struct ProcessAnswer {
    #[serde(default)]
    foreground: Option<ProcessInfo>,
    #[serde(default)]
    ancestors: Vec<ProcessInfo>,
    home: PathBuf,
    /// How the block's program exited: `null` while it runs.
    #[serde(default)]
    last_exit: Option<LastExit>,
}

/// How a block's program exited, as the terminal `process` method reports
/// it. Parsed only so that a changed shape fails rather than passing for
/// "exited".
#[derive(Deserialize)]
struct LastExit {
    /// The program's exit code, or -1 when a signal ended it.
    #[expect(dead_code, reason = "parsed to check the shape, not read")]
    exit_code: i64,
}

impl TryFrom<ProcessAnswer> for TerminalProcess {
    type Error = &'static str;

    /// Only a block whose program has exited may leave out `foreground`.
    fn try_from(answer: ProcessAnswer) -> Result<Self, Self::Error> {
        if answer.foreground.is_none() && answer.last_exit.is_none() {
            return Err("no `foreground` while the block's program is running");
        }
        Ok(TerminalProcess {
            foreground: answer.foreground,
            ancestors: answer.ancestors,
            home: answer.home,
        })
    }
}

impl TerminalProcess {
    /// The process in the terminal's foreground, unless it's the block's
    /// own shell: a program the shell runs, a shell nested in it, or a
    /// command block's command.
    ///
    /// The block's shell is its outermost process other than `login`, the
    /// wrapper Rex starts login shells through. `ancestors` reach up to the
    /// block's first process (`login`, or the shell itself when Rex starts it
    /// directly), so a foreground process with an ancestor other than `login`
    /// isn't the block's shell. Without `ancestors`, the foreground process
    /// is the block's first, and that's its shell only if it is one (see
    /// [`ProcessInfo::is_shell`]): a command block has no shell, and its
    /// command is its foreground program.
    pub fn foreground_program(&self) -> Option<&ProcessInfo> {
        let foreground = self.foreground.as_ref()?;
        let is_program = if self.ancestors.is_empty() {
            !foreground.is_shell()
        } else {
            self.ancestors.iter().any(|ancestor| ancestor.name != LOGIN)
        };
        is_program.then_some(foreground)
    }
}

/// The process Rex starts a block's login shell through.
const LOGIN: &str = "login";

/// The shells recognized by name. A shell missing here, as a block's first
/// process, counts as that block's foreground program.
const SHELL_NAMES: &[&str] = &[
    "zsh", "bash", "fish", "sh", "dash", "ksh", "tcsh", "csh", "nu",
];

#[derive(Debug, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    /// The name of the executable it runs, which isn't always the name it
    /// was started by: Claude Code's `claude` is a symlink to an executable
    /// named for its version.
    pub name: String,
    /// The path it was started by (`~/.local/bin/claude`), before symlinks.
    #[serde(default)]
    pub invoked_path: Option<PathBuf>,
    /// Absent for root processes.
    #[serde(default)]
    pub argv0: Option<String>,
    /// Physical path (`/private/tmp/...`). Absent for root processes.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

impl ProcessInfo {
    /// Every name the process goes by: its executable's, and the file names
    /// of the path and `argv0` it was started by (the command as typed).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        fn file_name(path: &Path) -> Option<&str> {
            path.file_name().and_then(|name| name.to_str())
        }
        let invoked = self.invoked_path.as_deref().and_then(file_name);
        let argv0 = self.argv0.as_deref().map(Path::new).and_then(file_name);
        std::iter::once(self.name.as_str())
            .chain(invoked)
            .chain(argv0)
    }

    /// Whether it's a shell, by any name it goes by. A login shell's `argv0`
    /// (`-zsh`) counts without its `-`. Matching shells rather than programs
    /// is what makes this reliable: Claude Code's executable is named for its
    /// version (`2.1.309`), and only its `argv0` is `claude`.
    pub fn is_shell(&self) -> bool {
        self.names()
            .map(|name| name.strip_prefix('-').unwrap_or(name))
            .any(|name| SHELL_NAMES.contains(&name))
    }
}

/// Arguments of the terminal `format` method, which renders the block's
/// scrollback and screen (not just the screen, whatever its description
/// says). Only `format: "vt"` is used: text plus SGR styles, no cursor
/// movement, modes or OSC sequences, so it's safe to print into a new block.
#[derive(Debug, Default, Serialize)]
pub struct FormatArgs {
    pub format: &'static str,
    /// Joins soft-wrapped rows into one line, so the text reflows at any
    /// width instead of breaking where the block's width was.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unwrap: bool,
    /// Prefixes the terminal's modes (`ESC[?1049h` while on the alternate
    /// screen). Only has an effect with `format: "vt"`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub extra_modes: bool,
}

/// The terminal `format` method. CRLF line endings, no final newline.
#[derive(Debug, Deserialize)]
pub struct TerminalFormat {
    pub content: String,
}

// ---- session.create --------------------------------------------------------

/// `session.create` takes only `label` and `initial_windows`, and silently
/// ignores unknown keys (a typo creates an empty session), so this type is
/// the single source of the request's key names.
#[derive(Debug, Serialize)]
pub struct CreateSession {
    pub label: String,
    pub initial_windows: Vec<CreateWindow>,
}

#[derive(Debug, Serialize)]
pub struct CreateWindow {
    /// Omitted: Rex generates a default label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_label: Option<String>,
    pub layout: CreateLayout,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CreateLayout {
    Block(CreateBlock),
    Split(CreateSplit),
}

#[derive(Debug, Serialize)]
pub struct CreateSplit {
    pub direction: Direction,
    /// Must be in (0, 1); Rex turns a missing or zero ratio into 0.5.
    pub ratio: f64,
    pub before: Box<CreateLayout>,
    pub after: Box<CreateLayout>,
}

#[derive(Debug, Serialize)]
pub struct CreateBlock {
    pub flavor: String,
    /// Omitted: Rex uses the default label (`shell`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub options: TerminalOptions,
}

#[derive(Debug, Default, Serialize)]
pub struct TerminalOptions {
    /// A missing directory isn't an error: the shell starts in `$HOME`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Written to the terminal as if typed. A newline acts as Enter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_input: Option<String>,
    /// What the block runs instead of the user's shell. Rex wraps it in a
    /// login shell unless `shell` is `"none"` and `login` is `false`; with
    /// both, Rex runs this argv as it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<bool>,
}

// ---- session.new_layer -----------------------------------------------------

/// A floating layer to create, before the window it goes in is known.
#[derive(Debug)]
pub struct CreateFloatingLayer {
    pub bounds: Bounds,
    pub layout: CreateLayout,
}

/// `session.new_layer`: adds a floating layer at the front of a window,
/// creating its blocks. All of them or none.
#[derive(Debug, Serialize)]
pub struct NewLayer<'a> {
    pub session_id: &'a str,
    pub window_id: &'a str,
    pub bounds: Bounds,
    pub layout: &'a CreateLayout,
    /// Whether to make the window active and focus the layer's first block.
    pub focus: bool,
}

#[derive(Debug, Deserialize)]
pub struct NewLayerResult {
    /// In depth-first, before-then-after order of the layer's layout.
    pub block_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatedSession {
    pub session_id: String,
    /// In request order. Absent when no windows were requested.
    #[serde(default)]
    pub initial_windows: Vec<CreatedWindow>,
}

#[derive(Debug, Deserialize)]
pub struct CreatedWindow {
    pub window_id: String,
    /// In depth-first, before-then-after order of the window's layout.
    pub block_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process running the executable `name`, started as `argv0`.
    fn process_info(name: &str, argv0: &str) -> ProcessInfo {
        ProcessInfo {
            pid: 1,
            name: name.to_string(),
            invoked_path: None,
            argv0: Some(argv0.to_string()),
            cwd: None,
        }
    }

    /// A block whose first process, `foreground`, is in its foreground.
    fn first_process(foreground: ProcessInfo) -> TerminalProcess {
        TerminalProcess {
            foreground: Some(foreground),
            ancestors: Vec::new(),
            home: PathBuf::from("/home/u"),
        }
    }

    fn is_program(foreground: ProcessInfo) -> bool {
        first_process(foreground).foreground_program().is_some()
    }

    #[test]
    fn bounds_that_fit_are_sent_as_they_are() {
        for bounds in [
            [0.1, 0.2, 0.5, 0.3],
            [0.0, 0.0, 1.0, 1.0],
            [0.5, 0.5, 0.5, 0.5],
        ] {
            assert_eq!(Bounds(bounds).clamped(), Bounds(bounds));
        }
    }

    #[test]
    fn bounds_a_rounding_error_took_past_the_window_are_brought_inside_it() {
        for bounds in [
            [0.7, 0.1, 0.30000000000000004, 0.2],
            [0.3, 0.0, 0.7000000000000001, 1.0000000000000002],
            [-1e-12, 0.5, 0.5, 0.5000000000000002],
        ] {
            let Bounds([x, y, w, h]) = Bounds(bounds).clamped();
            assert!(x >= 0.0 && y >= 0.0, "{bounds:?} -> {x}, {y}");
            assert!(w > 0.0 && h > 0.0, "{bounds:?} -> {w}, {h}");
            assert!(
                x + w <= 1.0 && y + h <= 1.0,
                "{bounds:?} -> {x}+{w}, {y}+{h}"
            );
            for (clamped, saved) in [x, y, w, h].into_iter().zip(bounds) {
                assert!((clamped - saved).abs() < 1e-9, "{bounds:?} moved too far");
            }
        }
    }

    #[test]
    fn a_first_process_that_is_a_known_shell_is_the_blocks_own_shell() {
        assert!(!is_program(process_info("zsh", "zsh")));
        assert!(!is_program(process_info("bash", "/bin/bash")));
    }

    #[test]
    fn a_login_shells_dash_prefixed_argv0_still_names_a_shell() {
        // The executable's name differs, so only `argv0` can match.
        assert!(!is_program(process_info("zsh-5.9", "-zsh")));
    }

    #[test]
    fn a_first_process_that_is_not_a_shell_is_the_blocks_foreground_program() {
        assert!(is_program(process_info("tail", "tail")));
        // Claude Code: its executable is named for its version.
        assert!(is_program(process_info("2.1.309", "claude")));
    }

    #[test]
    fn with_ancestors_only_one_other_than_login_makes_the_foreground_a_program() {
        let mut process = first_process(process_info("zsh", "-zsh"));
        process.ancestors = vec![process_info(LOGIN, LOGIN)];
        assert!(process.foreground_program().is_none());
        process.ancestors.insert(0, process_info("zsh", "-zsh"));
        assert!(process.foreground_program().is_some());
    }

    /// A block whose program exited and that stays open (`--keep-open`):
    /// Rex reports no foreground process, and a save must not fail on it.
    #[test]
    fn a_block_whose_program_exited_has_no_foreground_program() {
        for last_exit in [
            r#"{"exit_code":0}"#,
            // A signal ended it.
            r#"{"exit_code":-1,"error":"signal: killed"}"#,
        ] {
            let answer = format!(r#"{{"last_exit":{last_exit},"home":"/Users/u"}}"#);
            let process: TerminalProcess = serde_json::from_str(&answer).unwrap();
            assert!(process.foreground.is_none(), "{answer}");
            assert!(process.foreground_program().is_none(), "{answer}");
        }
    }

    /// Should Rex report a foreground process once the block's program has
    /// exited, it's kept.
    #[test]
    fn a_block_whose_program_exited_keeps_a_foreground_rex_reports() {
        let process: TerminalProcess = serde_json::from_str(
            r#"{"last_exit":{"exit_code":0},"foreground":{"pid":7,"name":"zsh"},"home":"/Users/u"}"#,
        )
        .unwrap();
        assert_eq!(process.foreground.map(|foreground| foreground.pid), Some(7));
    }

    /// While the block's program runs (`last_exit` is null or absent), a
    /// missing `foreground` is still an answer rexurrect doesn't understand.
    #[test]
    fn a_running_block_without_a_foreground_is_unexpected_output() {
        for answer in [
            r#"{"last_exit":null,"home":"/Users/u"}"#,
            r#"{"home":"/Users/u"}"#,
        ] {
            let error = serde_json::from_str::<TerminalProcess>(answer)
                .unwrap_err()
                .to_string();
            // Named for what's wrong, not as serde's missing field.
            assert!(
                error.contains("foreground") && error.contains("running"),
                "{answer}: {error}"
            );
        }
    }

    /// Only the shape Rex documents (`{"exit_code": <n>}`) says the program
    /// exited, so a changed shape can't pass a block without a foreground.
    #[test]
    fn a_last_exit_rexurrect_does_not_understand_is_unexpected_output() {
        for last_exit in ["{}", "false", "0", r#""""#, r#"{"exit_code":"0"}"#] {
            let answer = format!(r#"{{"last_exit":{last_exit},"home":"/Users/u"}}"#);
            assert!(
                serde_json::from_str::<TerminalProcess>(&answer).is_err(),
                "{answer}"
            );
        }
    }
}
