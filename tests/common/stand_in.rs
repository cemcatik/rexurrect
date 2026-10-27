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
