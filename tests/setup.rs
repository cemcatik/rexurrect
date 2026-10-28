//! Setup: `rexurrect setup` turns autosave on, the way `autosave install`
//! does, then prints the two lines left for the user to add: the
//! auto-restore line for Rex's init.lua, and the line that loads the zsh
//! recorder installed beside the binary, if there is one.
//!
//! Like the autosave tests, these install into a stand-in launchd
//! ([`FakeLaunchd`]), never the real one, so they're macOS only too. Setup
//! with the systemd timer has tests of its own, in `setup_systemd.rs`.

#![cfg(target_os = "macos")]

mod common;

use std::os::unix::fs::symlink;
use std::path::PathBuf;

use common::launchd::FakeLaunchd;
use common::*;

#[test]
fn setup_installs_the_same_agent_as_autosave_install() {
    let env = TestEnv::new();
    let by_setup = FakeLaunchd::new(&env);
    let by_install = FakeLaunchd::new_named(&env, "launchd-install");

    by_setup
        .aim(env.rexurrect_at(NO_SERVER))
        .arg("setup")
        .run()
        .assert_success();
    by_install
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    assert_eq!(by_setup.the_agent().plist, by_install.the_agent().plist);
}

#[test]
fn setup_prints_the_auto_restore_line_and_how_to_append_it_to_init_lua() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);

    let out = launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .arg("setup")
        .run()
        .assert_success();

    let hook = printed_hook(&env);
    assert!(
        out.stdout.lines().any(|line| line == hook.trim_end()),
        "no line equal to the hook {hook:?}: {out}"
    );
    let append = format!(
        "mkdir -p ~/.config/rex && {} auto-restore --print-hook >> ~/.config/rex/init.lua",
        common::rexurrect::bin()
    );
    assert!(
        out.stdout.lines().any(|line| line == append),
        "no line {append:?}: {out}"
    );
    assert!(out.stdout.contains("rex config check"), "{out}");
}

/// A Homebrew-shaped prefix: `bin/rexurrect`, a symlink to the built binary
/// as Homebrew links one to its Cellar, and the recorder in
/// `share/rexurrect/`, where Homebrew links the formula's extra files.
struct Prefix {
    dir: PathBuf,
}

impl Prefix {
    fn new(env: &TestEnv) -> Prefix {
        let dir = env.mkdir("prefix");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        symlink(common::rexurrect::bin(), dir.join("bin/rexurrect")).unwrap();
        std::fs::create_dir_all(dir.join("share/rexurrect")).unwrap();
        std::fs::copy(
            common::zsh::recorder_plugin(),
            dir.join("share/rexurrect/rexurrect.plugin.zsh"),
        )
        .unwrap();
        Prefix { dir }
    }

    fn bin(&self) -> PathBuf {
        self.dir.join("bin/rexurrect")
    }

    /// Where Homebrew links the formula's extra files.
    fn share(&self) -> PathBuf {
        self.dir.join("share/rexurrect")
    }

    fn recorder(&self) -> PathBuf {
        self.share().join("rexurrect.plugin.zsh")
    }
}

#[test]
fn run_from_a_homebrew_prefix_setup_prints_the_line_that_sources_its_recorder() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let prefix = Prefix::new(&env);

    let out = launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .via(prefix.bin())
        .arg("setup")
        .run()
        .assert_success();

    let source = format!("source {}", prefix.recorder().display());
    assert!(
        out.stdout.lines().any(|line| line == source),
        "no line {source:?}: {out}"
    );
    // The oh-my-zsh way: the recorder's directory as a custom plugin.
    assert!(
        out.stdout
            .contains(&format!("ln -s {}", prefix.share().display())),
        "{out}"
    );
    assert!(!out.stdout.contains(RECORDER_DOCS), "{out}");
}

/// What Homebrew upgrades is the Cellar the symlink points into, so the
/// agent and the auto-restore line keep working only if they name the
/// symlink.
#[test]
fn run_from_a_homebrew_prefix_autosave_and_auto_restore_name_its_bin_symlink() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let prefix = Prefix::new(&env);

    let out = launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .via(prefix.bin())
        .arg("setup")
        .run()
        .assert_success();

    let bin = prefix.bin().to_str().unwrap().to_string();
    assert_eq!(
        launchd.the_agent().program_arguments(),
        [bin.as_str(), "save"]
    );
    assert!(out.stdout.contains(&format!("[[{bin}]]")), "{out}");
}

#[test]
fn without_a_recorder_beside_the_binary_setup_points_to_the_readme_and_sources_nothing() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);

    let out = launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .arg("setup")
        .run()
        .assert_success();

    assert!(out.stdout.contains(RECORDER_DOCS), "{out}");
    assert!(
        !out.stdout.lines().any(|line| line.starts_with("source ")),
        "{out}"
    );
}

#[test]
fn the_interval_passes_through_to_the_agent() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);

    launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["setup", "--interval", "5"])
        .run()
        .assert_success();

    assert_eq!(launchd.the_agent().start_interval(), 5 * 60);
}

#[test]
fn an_interval_that_is_not_a_whole_number_of_minutes_installs_and_prints_nothing() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);

    for bad in ["0", "1.5", "15m", ""] {
        let out = launchd
            .aim(env.rexurrect_at(NO_SERVER))
            .args(["setup", "--interval", bad])
            .run()
            .assert_failure();
        assert!(out.stderr.contains("--interval"), "{out}");
        assert_eq!(out.stdout, "", "{out}");
    }
    assert!(launchd.installed().is_empty());
    assert!(launchd.loaded().is_empty());
}

#[test]
fn running_setup_twice_leaves_exactly_one_agent_with_the_latest_settings() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let setup = |args: &[&str]| {
        launchd
            .aim(env.rexurrect_at(NO_SERVER))
            .arg("setup")
            .args(args)
            .run()
            .assert_success()
    };

    setup(&["--interval", "5"]);
    setup(&[]);

    assert_eq!(launchd.the_agent().start_interval(), 15 * 60);
}

/// Autosave failing to install is setup failing: it doesn't go on to ask
/// for the rest.
#[test]
fn when_autosave_fails_to_install_setup_fails_and_prints_no_further_steps() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    launchd.refuse_to_load();

    let out = launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .arg("setup")
        .run()
        .assert_failure();

    assert!(out.stderr.contains("Bootstrap failed"), "{out}");
    assert_eq!(out.stdout, "", "{out}");
    assert!(launchd.installed().is_empty());
}

/// `autosave install` doesn't need `rex` on `PATH` (ADR-0003), so neither
/// does setup.
#[test]
fn without_rex_on_path_setup_installs_what_autosave_install_does() {
    let env = TestEnv::new();
    let by_setup = FakeLaunchd::new(&env);
    let by_install = FakeLaunchd::new_named(&env, "launchd-install");
    let without_rex = || env.rexurrect_at(NO_SERVER).env("PATH", SYSTEM_PATH);

    by_setup
        .aim(without_rex())
        .arg("setup")
        .run()
        .assert_success();
    by_install
        .aim(without_rex())
        .args(["autosave", "install"])
        .run()
        .assert_success();

    assert_eq!(by_setup.the_agent().plist, by_install.the_agent().plist);
}

/// `.zshrc` and init.lua are the user's: setup prints lines to add to
/// them and changes neither.
#[test]
fn setup_writes_nothing_in_the_home_directory() {
    let env = TestEnv::builder().own_home().build();
    let launchd = FakeLaunchd::new(&env);
    let prefix = Prefix::new(&env);

    launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .via(prefix.bin())
        .arg("setup")
        .run()
        .assert_success();

    let left: Vec<_> = std::fs::read_dir(env.home()).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
}
