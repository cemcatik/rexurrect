//! Autosave on Linux: `rexurrect autosave install` puts a systemd user timer
//! in place (a `.service` and a `.timer` unit) that runs `save` on a timer,
//! and `autosave uninstall` removes it.
//!
//! systemd itself isn't exercised, so these run on macOS too, with the
//! systemd scheduler picked by `REXURRECT_SCHEDULER`. Install and uninstall
//! run with their unit directory pointed at a temp dir and a stand-in
//! `systemctl` first on `PATH`
//! ([`FakeSystemd`](common::systemd::FakeSystemd)) that keeps track of which
//! units are loaded, enabled and started. The installed service is then run
//! the way systemd would run it
//! ([`Units::run_as_systemd`](common::systemd::Units::run_as_systemd)): its
//! command line and environment only, so no `REX_SERVER` from the test leaks
//! in.

mod common;

use common::systemd::*;
use common::*;

#[test]
fn install_starts_a_timer_that_saves_every_15_minutes_by_default() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    let out = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let units = systemd.the_units();
    assert_eq!(units.exec_start(), [common::rexurrect::bin(), "save"]);
    assert_eq!(units.period(), 15 * 60);
    assert_eq!(units.timer.get("Install", "WantedBy"), "timers.target");
    // Install says what it installed, and where.
    assert!(out.stdout.contains("systemd user timer"), "{out}");
    assert!(out.stdout.contains(&format!("{NAME}.timer")), "{out}");
    assert!(
        out.stdout.contains(systemd.unit_dir().to_str().unwrap()),
        "{out}"
    );
}

#[test]
fn install_reloads_units_before_enabling_and_starting_the_timer() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    assert_eq!(
        systemd.calls(),
        [
            "--user daemon-reload".to_string(),
            format!("--user enable {NAME}.timer"),
            format!("--user restart {NAME}.timer"),
        ]
    );
}

#[test]
fn the_interval_can_be_changed_at_install() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install", "--interval", "5"])
        .run()
        .assert_success();

    assert_eq!(systemd.the_units().period(), 5 * 60);
}

#[test]
fn installing_twice_leaves_exactly_one_timer_running_with_the_latest_settings() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let install = |args: &[&str]| {
        systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .args(["autosave", "install"])
            .args(args)
            .run()
            .assert_success()
    };

    install(&["--interval", "5"]);
    install(&[]);

    // the_units checks there's one of each installed and that the timer
    // started is the one installed: the second install restarted it.
    assert_eq!(systemd.the_units().period(), 15 * 60);
}

#[test]
fn an_autosave_saves_the_server_and_state_install_ran_with_though_systemd_gives_neither() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let server = env.start_server();
    let fixture = server.create_session("zz-autosaved", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    systemd
        .aim(env.rexurrect(&server))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    systemd.the_units().run_as_systemd(&env).assert_success();

    // The snapshot went to the state directory install ran with.
    server.kill_session(&fixture.id);
    env.restore(&server);
    assert_eq!(server.session_ids_labelled("zz-autosaved").len(), 1);
}

/// On Linux, with no `REX_SERVER` at install (outside Rex), autosave saves
/// the server `rex` itself defaults to: `$XDG_RUNTIME_DIR/rex/server.sock`,
/// where the login server's per-account server listens.
#[cfg(target_os = "linux")]
#[test]
fn installed_outside_rex_autosave_saves_the_server_at_rexs_default_endpoint() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let runtime = env.mkdir("run user");

    let install = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .env_remove("REX_SERVER")
        .env("XDG_RUNTIME_DIR", &runtime)
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let endpoint = format!(
        "unix://{}/rex/server.sock",
        runtime.to_str().unwrap().replace(' ', "%20")
    );
    assert_eq!(
        systemd.the_units().env_var("REX_SERVER"),
        Some(endpoint.clone())
    );
    assert!(install.stdout.contains(&endpoint), "{install}");
}

/// Without `XDG_RUNTIME_DIR` (under `su`, say), `rex` has no default
/// endpoint where a Linux server listens, so install pins nothing and says
/// to name the server.
#[cfg(target_os = "linux")]
#[test]
fn installed_with_neither_rex_server_nor_xdg_runtime_dir_installs_nothing() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    let out = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .env_remove("REX_SERVER")
        .env_remove("XDG_RUNTIME_DIR")
        .args(["autosave", "install"])
        .run()
        .assert_failure();

    assert!(out.stderr.contains("XDG_RUNTIME_DIR"), "{out}");
    assert!(out.stderr.contains("REX_SERVER"), "{out}");
    assert!(names_in(&systemd.unit_dir()).is_empty());
}

#[test]
fn a_successful_autosave_is_silent() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let server = env.start_server();
    systemd
        .aim(env.rexurrect(&server))
        .args(["autosave", "install"])
        .run()
        .assert_success();
    let units = systemd.the_units();

    units.run_as_systemd(&env).assert_success();

    // Nothing goes to the journal every interval; failures go to the log.
    assert_eq!(units.service.get("Service", "StandardOutput"), "null");
    assert_eq!(units.service.get("Service", "StandardError"), "null");
    assert_eq!(env.log_entries(), Vec::<String>::new());
}

#[test]
fn values_systemd_would_expand_reach_the_job_as_they_were() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let state = env.mkdir("state 100%h $HOME \"q\" \\b");
    let config = env.mkdir("config %n $USER \"x\" \\y").join("config.toml");

    systemd
        .aim(env.rexurrect_at("unix:///tmp/rxt%20none/s.sock"))
        .env(STATE_DIR_VAR, &state)
        .env(CONFIG_VAR, &config)
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let units = systemd.the_units();
    assert_eq!(
        units.env_var("REX_SERVER").as_deref(),
        Some("unix:///tmp/rxt%20none/s.sock")
    );
    assert_eq!(
        units.env_var(STATE_DIR_VAR).as_deref(),
        Some(state.to_str().unwrap())
    );
    assert_eq!(
        units.env_var(CONFIG_VAR).as_deref(),
        Some(config.to_str().unwrap())
    );
}

#[test]
fn uninstall_stops_and_removes_the_timer_and_says_how_to_back_out_of_the_rest() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let out = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "uninstall"])
        .run()
        .assert_success();

    assert!(
        systemd.names_in("enabled").is_empty(),
        "the timer was disabled"
    );
    assert!(
        systemd.names_in("active").is_empty(),
        "the timer was stopped"
    );
    assert!(
        names_in(&systemd.unit_dir()).is_empty(),
        "its units were removed"
    );
    assert!(systemd.names_in("loaded").is_empty(), "and unloaded");
    assert!(out.stdout.contains("autosave uninstalled"), "{out}");
    assert_explains_backing_out(&out);
}

#[test]
fn uninstall_without_a_timer_changes_nothing_and_still_says_how_to_back_out() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let uninstall = || {
        systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .args(["autosave", "uninstall"])
            .run()
            .assert_success()
    };

    let out = uninstall();

    assert!(out.stdout.contains("wasn't installed"), "{out}");
    assert_explains_backing_out(&out);
    assert_eq!(systemd.calls(), Vec::<String>::new());
}

#[test]
fn a_second_uninstall_says_there_was_nothing_to_remove() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let autosave = |action: &str| {
        systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .args(["autosave", action])
            .run()
            .assert_success()
    };

    autosave("install");
    autosave("uninstall");
    let out = autosave("uninstall");

    assert!(out.stdout.contains("wasn't installed"), "{out}");
}

#[test]
fn a_test_label_keeps_a_scratch_timer_apart_from_the_real_one() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let scratch = "rexurrect-autosave-test-1234";
    let autosave = |action: &str| {
        systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .env("REXURRECT_AUTOSAVE_LABEL", scratch)
            .args(["autosave", action])
            .run()
            .assert_success()
    };

    autosave("install");
    systemd.the_units_in(&systemd.unit_dir(), scratch);

    autosave("uninstall");
    assert!(names_in(&systemd.unit_dir()).is_empty());
    assert!(systemd.names_in("enabled").is_empty());
}

#[test]
fn a_label_that_is_not_a_unit_name_installs_nothing() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    for bad in ["a/b", "a b", "a@b", "x.timer\nExecStart=/bin/sh"] {
        let out = systemd
            .aim(env.rexurrect_at(NO_SERVER))
            .env("REXURRECT_AUTOSAVE_LABEL", bad)
            .args(["autosave", "install"])
            .run()
            .assert_failure();
        assert!(out.stderr.contains("REXURRECT_AUTOSAVE_LABEL"), "{out}");
    }
    assert!(names_in(&systemd.unit_dir()).is_empty());
}

#[test]
fn units_go_in_xdg_config_home_else_dot_config() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let home = env.mkdir("home");
    let config = env.mkdir("config");

    // The fake systemctl loads units from where install put them.
    systemd
        .aim_at_systemctl_only(env.rexurrect_at(NO_SERVER))
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config)
        .env("UNIT_DIR", config.join("systemd/user"))
        .args(["autosave", "install"])
        .run()
        .assert_success();
    systemd.the_units_in(&config.join("systemd/user"), NAME);

    systemd
        .aim_at_systemctl_only(env.rexurrect_at(NO_SERVER))
        .env("HOME", &home)
        .env("UNIT_DIR", config.join("systemd/user"))
        .args(["autosave", "uninstall"])
        .run()
        .assert_success();

    systemd
        .aim_at_systemctl_only(env.rexurrect_at(NO_SERVER))
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .env("UNIT_DIR", home.join(".config/systemd/user"))
        .args(["autosave", "install"])
        .run()
        .assert_success();
    systemd.the_units_in(&home.join(".config/systemd/user"), NAME);
}

/// An enabled timer starts at every login, so units systemd refused to
/// enable mustn't stay installed.
#[test]
fn a_timer_systemd_fails_to_enable_is_not_left_installed() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    systemd.refuse_to_enable();

    let out = systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_failure();

    assert!(out.stderr.contains("masked"), "{out}");
    assert!(
        names_in(&systemd.unit_dir()).is_empty(),
        "its units were removed"
    );
    assert!(systemd.names_in("loaded").is_empty(), "and unloaded");
}

/// Rex moves its CLI on updates, so the service doesn't pin one: with no
/// `rex` on its `PATH`, save runs the one a running Rex server runs
/// (ADR-0003).
#[test]
fn the_service_gets_only_the_system_path() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);

    systemd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    assert_eq!(
        systemd.the_units().env_var("PATH").as_deref(),
        Some(SYSTEM_PATH)
    );
}

/// The config file's path is pinned, never its settings, and the moved
/// variables aren't handed on.
#[test]
fn an_edit_to_the_config_file_after_install_reaches_the_next_autosave() {
    let env = TestEnv::new();
    let systemd = FakeSystemd::new(&env);
    let server = env.start_server();
    let fixture = server.create_session("zz-kept", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    systemd
        .aim(env.rexurrect(&server))
        .env("REXURRECT_KEEP", "5")
        .args(["autosave", "install"])
        .run()
        .assert_success();
    let units = systemd.the_units();
    assert_eq!(units.env_var("REXURRECT_KEEP"), None);

    env.write_config("keep = 2\n");
    for _ in 0..3 {
        units.run_as_systemd(&env).assert_success();
    }

    let list = env.rexurrect(&server).arg("list").run().assert_success();
    assert_eq!(list.stdout.lines().count(), 2, "{list}");
}
