//! Turning a saved session into the requests that create it: a
//! `session.create`, then a `session.new_layer` per floating layer.

use std::path::Path;

use crate::rex::api::{
    CreateBlock, CreateFloatingLayer, CreateLayout, CreateSession, CreateSplit, CreateWindow,
    SHELL_FLAVOR, TerminalOptions,
};
use crate::scrollback::Replay;
use crate::snapshot::{SavedBlock, SavedLayout, SavedSession, SavedWindow};

/// `saved` as restore creates it. Every block comes back as a plain shell
/// block in its saved directory, or in `home` if that directory no longer
/// exists, with its last command typed at its prompt if it can be typed
/// ([`typed_command`]), and its scrollback printed above that prompt if
/// `replay` can print it. Rex would quietly start the shell in its own home
/// anyway, but rexurrect doesn't rely on that.
///
/// What restore types at each block's prompt is decided apart from this
/// (see `resume`), so the session as created keeps what a later restore
/// decides from: its blocks' last commands and resumable programs.
pub fn as_created(saved: &SavedSession, home: &Path, replay: &Replay) -> SavedSession {
    let mut created = saved.clone();
    for block in created.blocks_mut() {
        if !replay.prints(block) {
            block.scrollback = None;
        }
        block.last_command = typed_command(block).map(str::to_string);
        if !block.working_directory.is_dir() {
            block.working_directory = home.to_path_buf();
        }
    }
    created
}

/// What creates a session: `session.create` makes its windows with their
/// tiled layouts, then each window gets its floating layers.
#[derive(Debug)]
pub struct CreateRequests {
    pub session: CreateSession,
    /// For each of `session`'s windows, its floating layers, back to front:
    /// the order to add them in, as each goes in front of the others.
    pub floating_layers: Vec<Vec<CreateFloatingLayer>>,
}

/// The requests that create `created`, a session [`as_created`] made.
/// `inputs` holds what to write to each block's terminal as if typed, in
/// [`SavedSession::blocks`] order (see `resume`). `replay` prints each
/// block's scrollback above its new prompt.
pub fn create_requests(
    created: &SavedSession,
    inputs: &[Option<String>],
    replay: &Replay,
) -> CreateRequests {
    let mut inputs = inputs.iter().cloned();
    let (initial_windows, floating_layers) = created
        .windows
        .iter()
        .map(|window| create_window(window, &mut inputs, replay))
        .unzip();
    CreateRequests {
        session: CreateSession {
            label: created.label.clone(),
            initial_windows,
        },
        floating_layers,
    }
}

/// A window and its floating layers, taking their blocks' inputs in
/// [`SavedWindow::blocks`] order.
fn create_window(
    window: &SavedWindow,
    inputs: &mut impl Iterator<Item = Option<String>>,
    replay: &Replay,
) -> (CreateWindow, Vec<CreateFloatingLayer>) {
    let tiled = CreateWindow {
        window_label: window.label.clone(),
        layout: create_layout(&window.layout, inputs, replay),
    };
    let floating = window
        .floating
        .iter()
        .map(|layer| CreateFloatingLayer {
            bounds: layer.bounds.clamped(),
            layout: create_layout(&layer.layout, inputs, replay),
        })
        .collect();
    (tiled, floating)
}

fn create_layout(
    layout: &SavedLayout,
    inputs: &mut impl Iterator<Item = Option<String>>,
    replay: &Replay,
) -> CreateLayout {
    match layout {
        SavedLayout::Block(block) => {
            CreateLayout::Block(create_block(block, inputs.next().flatten(), replay))
        }
        SavedLayout::Split(split) => CreateLayout::Split(CreateSplit {
            direction: split.direction,
            ratio: split.ratio,
            before: Box::new(create_layout(&split.before, inputs, replay)),
            after: Box::new(create_layout(&split.after, inputs, replay)),
        }),
    }
}

fn create_block(block: &SavedBlock, input: Option<String>, replay: &Replay) -> CreateBlock {
    let mut options = TerminalOptions {
        cwd: Some(block.working_directory.clone()),
        initial_input: input,
        ..TerminalOptions::default()
    };
    replay.apply(block, &mut options);
    CreateBlock {
        flavor: SHELL_FLAVOR.to_string(),
        label: block.label.clone(),
        options,
    }
}

/// The command restore types at a block's prompt: its last command, only if
/// it's a single line without control characters. Rex writes `initial_input`
/// to the terminal as if typed, so a line break would act as Enter and run
/// what came before it (other control characters can be editor keys too).
/// Recorders already ignore such commands; this check doesn't trust them, or
/// the snapshot.
pub fn typed_command(block: &SavedBlock) -> Option<&str> {
    block
        .last_command
        .as_deref()
        .filter(|command| is_typeable(command))
}

/// Whether restore can type `command` at a prompt: not empty, and no
/// control character, tab included (typed, a tab triggers completion).
pub fn is_typeable(command: &str) -> bool {
    !command.is_empty() && !command.chars().any(char::is_control)
}
