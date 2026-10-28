//! Setup on Linux: `rexurrect setup` turns autosave on, the way `autosave
//! install` does, with the systemd user timer, then prints the two lines
//! left for the user to add.
//!
//! Like the systemd autosave tests, these install into a stand-in systemd
//! ([`FakeSystemd`](common::systemd::FakeSystemd)), never the real one, so
//! they run on macOS too, with the systemd scheduler picked by
//! `REXURRECT_SCHEDULER`. What setup prints doesn't depend on the scheduler,
//! so these check it too: on Linux, `setup.rs`'s launchd tests don't run.

mod common;

use common::systemd::*;
use common::*;

#[test]
fn setup_installs_the_same_timer_as_autosave_install() {
    let env = TestEnv::new();
    let by_setup = FakeSystemd::new(&env);
    let by_install = FakeSystemd::new_named(&env, "systemd-install");

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

    assert_eq!(by_setup.the_units(), by_install.the_units());
}

#[test]
fn setup_prints_the_auto_restore_line_and_how_to_append_it_to_init_lua() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    let out = systemd
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
}

/// No Linux install puts the recorder beside the binary: `cargo install`
/// doesn't, and there are no Linux release archives.
#[test]
fn without_a_recorder_beside_the_binary_setup_points_to_the_readme_and_sources_nothing() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    let out = systemd
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
fn the_interval_passes_through_to_the_timer() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["setup", "--interval", "5"])
        .run()
        .assert_success();

    assert_eq!(systemd.the_units().period(), 5 * 60);
}

#[test]
fn running_setup_twice_leaves_exactly_one_timer_running_with_the_latest_settings() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let setup = |args: &[&str]| {
        systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .arg("setup")
            .args(args)
            .run()
            .assert_success()
    };

    setup(&["--interval", "5"]);
    setup(&[]);

    // the_units checks there's one of each installed and that the timer
    // started is the one installed: the second setup restarted it.
    assert_eq!(systemd.the_units().period(), 15 * 60);
}

/// Autosave failing to install is setup failing: it doesn't go on to ask
/// for the rest.
#[test]
fn when_systemd_fails_to_enable_the_timer_setup_fails_and_prints_no_further_steps() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    systemd.refuse_to_enable();

    let out = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .arg("setup")
        .run()
        .assert_failure();

    assert!(out.stderr.contains("masked"), "{out}");
    assert_eq!(out.stdout, "", "{out}");
    assert!(names_in(&systemd.unit_dir()).is_empty(), "{out}");
}

/// `.zshrc` and init.lua are the user's: setup prints lines to add to
/// them and changes neither. The units go in their own directory.
#[test]
fn setup_writes_nothing_in_the_home_directory() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .arg("setup")
        .run()
        .assert_success();

    let left: Vec<_> = std::fs::read_dir(systemd.home()).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
}
