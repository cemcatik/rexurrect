//! A stand-in for a program rexurrect resumes (`nvim`, `claude`), built by
//! the test itself as [`super::build_sleeper`] is, so no test runs a real
//! editor or agent.
//!
//! It takes any arguments. As it starts, it appends them, joined by spaces,
//! as one line to `.<its name>.ran` in its working directory, so a test can
//! tell whether it ran and how. Then it waits ten minutes, or until Ctrl-C.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::env::TestEnv;
use super::fixture::{shell, window};
use super::rexurrect::RunOutput;
use super::server::{Server, wait_for};

const SOURCE: &str = r#"fn main() {
    let mut args = std::env::args();
    let name = std::path::Path::new(&args.next().expect("argv0"))
        .file_name()
        .expect("a file name")
        .to_string_lossy()
        .into_owned();
    let line = args.collect::<Vec<_>>().join(" ");
    let mut ran = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!(".{name}.ran"))
        .expect("opening the record of runs");
    std::io::Write::write_all(&mut ran, format!("{line}\n").as_bytes()).expect("recording the run");
    drop(ran);
    std::thread::sleep(std::time::Duration::from_secs(600));
}
"#;

/// Builds the stand-in as the executable `path` (its parent must exist).
/// Its source goes in `env`'s root.
pub fn build_stand_in(env: &TestEnv, path: &Path) {
    let source = env.root().join("stand_in.rs");
    if !source.exists() {
        std::fs::write(&source, SOURCE).unwrap();
    }
    let output = Command::new("rustc")
        .args(["--edition", "2024", "-o"])
        .arg(path)
        .arg(&source)
        .output()
        .expect("rustc runs");
    assert!(
        output.status.success(),
        "building the stand-in failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Builds a stand-in named `name` in a `bin` directory of its own in `env`,
/// and returns that directory, to put first on a server's `PATH`.
pub fn stand_in_bin(env: &TestEnv, name: &str) -> PathBuf {
    let bin = env.root().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    build_stand_in(env, &bin.join(name));
    bin
}

/// Each run of the stand-in `name` in `dir`, oldest first: its arguments,
/// joined by spaces.
pub fn runs(dir: &Path, name: &str) -> Vec<String> {
    match std::fs::read_to_string(dir.join(format!(".{name}.ran"))) {
        Ok(text) => text.lines().map(str::to_string).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => panic!("reading the stand-in's runs: {e}"),
    }
}

/// Waits until the stand-in `program` in `dir` has run `count` times, and
/// returns how each run was started.
pub fn wait_for_runs(dir: &Path, program: &str, count: usize) -> Vec<String> {
    wait_for(&format!("{program} to have run {count} times"), || {
        let runs = runs(dir, program);
        (runs.len() >= count).then_some(runs)
    })
}

/// vim-obsession's Neovim session, in `dir`.
pub fn write_session_vim(dir: &Path) {
    std::fs::write(dir.join("Session.vim"), "\" a Neovim session\n").unwrap();
}

/// Creates a session labelled `label` with one shell block in `dir`, and
/// runs `command` there (a stand-in, by name, its first word), waiting
/// until it has started. Returns the session and its block.
pub fn running(server: &Server, label: &str, dir: &Path, command: &str) -> (String, String) {
    let fixture = server.create_session(label, &[window(shell(dir))]);
    server.wait_for_pwds(&fixture);
    let block = fixture.block_ids()[0].to_string();
    run_and_wait(server, &fixture.id, &block, dir, command);
    (fixture.id, block)
}

/// Runs `command` (a stand-in, by name, its first word) in the block, whose
/// working directory is `dir`, and waits until it has started.
pub fn run_and_wait(server: &Server, session: &str, block: &str, dir: &Path, command: &str) {
    let program = command.split_whitespace().next().unwrap();
    let ran = runs(dir, program).len();
    server.run(session, block, command);
    wait_for_runs(dir, program, ran + 1);
}

/// Saves, kills the session `session`, restores it, and returns the
/// restored session and its first block.
pub fn restarted(env: &TestEnv, server: &Server, session: &str, label: &str) -> (String, String) {
    restarted_after(env, server, session, label, || {})
}

/// [`restarted`], running `after_the_save` (quitting a program, say)
/// between the save and the kill. Nothing saves in between, as when the
/// server stops before the next autosave.
pub fn restarted_after(
    env: &TestEnv,
    server: &Server,
    session: &str,
    label: &str,
    after_the_save: impl FnOnce(),
) -> (String, String) {
    env.save(server);
    after_the_save();
    server.kill_session(session);
    env.restore(server);
    let restored = server.the_session_labelled(label);
    let block = server.block_ids(&restored).remove(0);
    (restored, block)
}

/// [`restarted`], once `typed` sits at the restored block's prompt.
pub fn restored_with_typed(
    env: &TestEnv,
    server: &Server,
    session: &str,
    label: &str,
    typed: &str,
) -> (String, String) {
    let (restored, block) = restarted(env, server, session, label);
    server.wait_for_typed(&restored, &block, typed);
    (restored, block)
}

/// Runs `rexurrect report` with `args` in the block `block_id` (its
/// `REX_BLOCK`), fed `stdin`, as a hook in it would.
pub fn report(
    env: &TestEnv,
    server: &Server,
    block_id: &str,
    args: &[&str],
    stdin: &str,
) -> RunOutput {
    env.rexurrect(server)
        .env("REX_BLOCK", block_id)
        .arg("report")
        .args(args)
        .stdin(stdin)
        .run()
}
