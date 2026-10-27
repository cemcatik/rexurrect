//! The config file: `$REXURRECT_CONFIG`, else
//! `$XDG_CONFIG_HOME/rexurrect/config.toml`, else
//! `~/.config/rexurrect/config.toml`. No file means every default; a file
//! rexurrect can't use fails the run, naming the file and the key.
//!
//! Each setting's effect has tests where the setting matters (`keep` in
//! `snapshot_files.rs`, `no_scrollback` in `scrollback.rs`, the auto-restore
//! ones in `auto_restore.rs`); these use `keep`, the quickest to see.

mod common;

use std::path::Path;

use common::*;

/// A server with one session worth saving.
fn server_with_a_session(env: &TestEnv) -> Server {
    let server = env.start_server();
    let fixture = server.create_session("zz-kept", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    server
}

/// How many snapshots there are after two saves of `cmd`: 1 if the config
/// file it reads keeps one.
fn snapshots_after_two_saves(
    env: &TestEnv,
    server: &Server,
    cmd: impl Fn() -> RexurrectCmd,
) -> usize {
    for _ in 0..2 {
        cmd().arg("save").run().assert_success();
    }
    env.listed(server).lines().count()
}

fn write(path: &Path, toml: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, toml).unwrap();
}

#[test]
fn rexurrect_config_names_the_file() {
    let env = TestEnv::new();
    let server = server_with_a_session(&env);
    let elsewhere = env.root().join("elsewhere/rx.toml");
    write(&elsewhere, "keep = 1\n");

    let snapshots = snapshots_after_two_saves(&env, &server, || {
        env.rexurrect(&server).env(CONFIG_VAR, &elsewhere)
    });

    assert_eq!(snapshots, 1);
}

#[test]
fn without_rexurrect_config_the_file_is_under_xdg_config_home() {
    let env = TestEnv::new();
    let server = server_with_a_session(&env);
    let xdg_config_home = env.root().join("xdg-config");
    write(&xdg_config_home.join("rexurrect/config.toml"), "keep = 1\n");

    let snapshots = snapshots_after_two_saves(&env, &server, || {
        env.rexurrect(&server)
            .env_remove(CONFIG_VAR)
            .env("XDG_CONFIG_HOME", &xdg_config_home)
    });

    assert_eq!(snapshots, 1);
}

#[test]
fn without_xdg_config_home_the_file_is_under_home() {
    let env = TestEnv::builder().own_home().build();
    let server = server_with_a_session(&env);
    write(
        &env.home().join(".config/rexurrect/config.toml"),
        "keep = 1\n",
    );

    let snapshots = snapshots_after_two_saves(&env, &server, || {
        env.rexurrect(&server)
            .env_remove(CONFIG_VAR)
            .env_remove("XDG_CONFIG_HOME")
    });

    assert_eq!(snapshots, 1);
}

/// The variables are a clean break (ADR-0007): ignored, and each run says
/// once what to put in the file instead.
#[test]
fn a_moved_variable_is_ignored_and_each_run_warns_once_naming_the_key_and_the_file() {
    let env = TestEnv::new();
    let server = server_with_a_session(&env);
    let warning = format!(
        "rexurrect: REXURRECT_KEEP is no longer read: set keep = 1 in {}",
        env.config_path().display()
    );

    for _ in 0..2 {
        let out = env
            .rexurrect(&server)
            .env("REXURRECT_KEEP", "1")
            .arg("save")
            .run()
            .assert_success();
        assert_eq!(out.stderr.matches(&warning).count(), 1, "{out}");
    }

    assert_eq!(env.listed(&server).lines().count(), 2);
}

#[test]
fn a_moved_variable_that_is_set_but_empty_warns_only_if_empty_meant_something() {
    let env = TestEnv::new();
    let server = server_with_a_session(&env);

    let out = env
        .rexurrect(&server)
        .env("REXURRECT_KEEP", "")
        .env("REXURRECT_NO_SCROLLBACK", "")
        .arg("save")
        .run()
        .assert_success();

    assert!(!out.stderr.contains("REXURRECT_KEEP"), "{out}");
    assert!(
        out.stderr
            .contains("REXURRECT_NO_SCROLLBACK is no longer read: set no_scrollback = []"),
        "{out}"
    );
}

#[test]
fn a_file_rexurrect_cannot_use_fails_save_naming_the_file_and_the_key_and_saves_nothing() {
    let env = TestEnv::new();
    let server = server_with_a_session(&env);
    let file = env.config_path().display().to_string();

    for (toml, names) in [
        ("keep = \n", vec![file.as_str()]),
        ("kep = 3\n", vec![file.as_str(), "`kep`"]),
        ("keep = \"3\"\n", vec![file.as_str(), "`keep`"]),
        (
            "no_scrollback = \"claude\"\n",
            vec![file.as_str(), "`no_scrollback`"],
        ),
    ] {
        env.write_config(toml);
        let out = env.rexurrect(&server).arg("save").run().assert_failure();
        for name in names {
            assert!(out.stderr.contains(name), "{toml}: {out}");
        }
    }

    assert_eq!(env.listed(&server), "no snapshots yet\n");
}

#[test]
fn a_config_file_that_cannot_be_read_fails_save_naming_it() {
    let env = TestEnv::new();
    let server = server_with_a_session(&env);
    std::fs::remove_file(env.config_path()).unwrap();
    std::fs::create_dir(env.config_path()).unwrap();

    let out = env.rexurrect(&server).arg("save").run().assert_failure();

    assert!(
        out.stderr
            .contains(&env.config_path().display().to_string()),
        "{out}"
    );
    assert_eq!(env.listed(&server), "no snapshots yet\n");
}

/// Restore reads the file too, though no setting changes it yet: a file
/// rexurrect can't use fails it before anything is restored.
#[test]
fn a_file_rexurrect_cannot_use_fails_restore_before_anything_is_restored() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-proj"]);
    let server = env.start_server();
    env.write_config("kep = 3\n");

    let out = env.rexurrect(&server).arg("restore").run().assert_failure();

    assert!(
        out.stderr.contains("`kep`")
            && out
                .stderr
                .contains(&env.config_path().display().to_string()),
        "{out}"
    );
    assert_eq!(server.sessions(), []);
}
