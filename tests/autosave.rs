//! Autosave: `rexurrect autosave install` puts a launchd user agent in place
//! that runs `save` on a timer, and `autosave uninstall` removes it.
//!
//! launchd itself isn't exercised. Install and uninstall run with their
//! LaunchAgents directory pointed at a temp dir and a stand-in `launchctl`
//! first on `PATH` ([`FakeLaunchd`]) that keeps track of which agents are
//! loaded. The installed agent's job is then run the way launchd would run
//! it ([`Agent::run_as_launchd`](common::launchd::Agent::run_as_launchd)):
//! its program, arguments and environment only, so no `REX_SERVER` from the
//! test leaks in.

mod common;

use common::launchd::{FakeLaunchd, LABEL, NO_SERVER};
use common::*;

#[test]
fn install_loads_an_agent_that_saves_every_15_minutes_by_default() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);

    launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let agent = launchd.the_agent();
    assert_eq!(agent.label(), LABEL);
    assert_eq!(
        agent.program_arguments(),
        [common::rexurrect::bin(), "save"]
    );
    assert_eq!(agent.start_interval(), 15 * 60);
}

#[test]
fn the_interval_can_be_changed_at_install() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);

    launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install", "--interval", "5"])
        .run()
        .assert_success();

    assert_eq!(launchd.the_agent().start_interval(), 5 * 60);
}

#[test]
fn installing_twice_leaves_exactly_one_agent_running_with_the_latest_settings() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let install = |args: &[&str]| {
        launchd
            .aim(env.rexurrect_at(NO_SERVER))
            .args(["autosave", "install"])
            .args(args)
            .run()
            .assert_success()
    };

    install(&["--interval", "5"]);
    install(&[]);

    // the_agent checks there's one installed and one loaded, and that
    // they're the same: the second install reloaded the agent.
    assert_eq!(launchd.the_agent().start_interval(), 15 * 60);
}

#[test]
fn an_autosave_saves_the_server_and_state_install_ran_with_though_launchd_gives_neither() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let server = env.start_server();
    let fixture = server.create_session("zz-autosaved", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    launchd
        .aim(env.rexurrect(&server))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    launchd.the_agent().run_as_launchd(&env).assert_success();

    // The snapshot went to the state directory install ran with.
    server.kill_session(&fixture.id);
    env.restore(&server);
    assert_eq!(server.session_ids_labelled("zz-autosaved").len(), 1);
}

#[test]
fn installed_outside_rex_autosave_saves_the_rex_apps_server() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let server = env.start_server();
    let fixture = server.create_session("zz-app", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    // The throwaway server plays the Rex app's: it answers at the app's
    // socket path under this HOME.
    let home = env.mkdir("home");
    let app_socket = home.join("Library/Application Support/rex/server.sock");
    std::fs::create_dir_all(app_socket.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(server.dir().join("s.sock"), &app_socket).unwrap();

    // No REX_SERVER, as outside Rex. The harness sets XDG_RUNTIME_DIR, which
    // moves rex's own default endpoint away from the app's server.
    let install = launchd
        .aim(env.rexurrect(&server))
        .env_remove("REX_SERVER")
        .env("HOME", &home)
        .args(["autosave", "install"])
        .run()
        .assert_success();
    launchd.the_agent().run_as_launchd(&env).assert_success();

    server.kill_session(&fixture.id);
    env.restore(&server);
    assert_eq!(server.session_ids_labelled("zz-app").len(), 1);
    // Install says which server autosave saves, so the user can check.
    let endpoint = format!(
        "unix://{}",
        app_socket.to_str().unwrap().replace(' ', "%20")
    );
    assert!(install.stdout.contains(&endpoint), "{install}");
}

#[test]
fn a_successful_autosave_is_silent() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let server = env.start_server();
    launchd
        .aim(env.rexurrect(&server))
        .args(["autosave", "install"])
        .run()
        .assert_success();
    let agent = launchd.the_agent();

    agent.run_as_launchd(&env).assert_success();

    // launchd's default for output isn't documented, so the agent says.
    assert_eq!(agent.output_paths(), [Some("/dev/null"); 2]);
    assert_eq!(env.log_entries(), Vec::<String>::new());
}

#[test]
fn an_autosave_while_rex_is_not_running_logs_the_failure_and_starts_no_server() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let endpoint = {
        let server = env.start_server();
        launchd
            .aim(env.rexurrect(&server))
            .args(["autosave", "install"])
            .run()
            .assert_success();
        server.endpoint().to_string()
    }; // Rex quits.

    let job = launchd.the_agent().run_as_launchd(&env);

    env.assert_no_server_started_at(&endpoint, "the autosave");
    job.assert_failure();
    let entries = env.log_entries();
    assert_eq!(entries.len(), 1, "{entries:#?}");
    assert!(entries[0].contains("save failed"), "{entries:#?}");
    assert!(entries[0].contains("isn't running"), "{entries:#?}");
}

/// Asserts `out` tells the user how to remove the zsh recorder and the
/// auto-restore line in Rex's init.lua.
fn assert_explains_backing_out(out: &RunOutput) {
    for what in [
        "rexurrect.plugin.zsh",
        ".zshrc",
        "init.lua",
        "https://github.com/cemcatik/rexurrect#auto-restore",
    ] {
        assert!(out.stdout.contains(what), "no mention of {what}: {out}");
    }
}

#[test]
fn uninstall_unloads_and_removes_the_agent_and_says_how_to_back_out_of_the_rest() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let out = launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "uninstall"])
        .run()
        .assert_success();

    assert!(launchd.loaded().is_empty(), "the agent was unloaded");
    assert!(launchd.installed().is_empty(), "its plist was removed");
    assert_explains_backing_out(&out);
}

#[test]
fn uninstall_without_an_agent_changes_nothing_and_still_says_how_to_back_out() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);

    let out = launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "uninstall"])
        .run()
        .assert_success();

    assert!(out.stdout.contains("wasn't installed"), "{out}");
    assert_explains_backing_out(&out);
}

#[test]
fn an_interval_that_is_not_a_whole_number_of_minutes_installs_nothing() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);

    for bad in ["0", "-5", "1.5", "15m", ""] {
        let out = launchd
            .aim(env.rexurrect_at(NO_SERVER))
            .args(["autosave", "install", "--interval", bad])
            .run()
            .assert_failure();
        assert!(out.stderr.contains("--interval"), "{out}");
    }
    assert!(launchd.installed().is_empty());
    assert!(launchd.loaded().is_empty());
}

#[test]
fn a_test_label_keeps_a_scratch_agent_apart_from_the_real_one() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let scratch = "io.github.cemcatik.rexurrect.autosave.test-1234";
    let autosave = |action: &str| {
        launchd
            .aim(env.rexurrect_at(NO_SERVER))
            .env("REXURRECT_AUTOSAVE_LABEL", scratch)
            .args(["autosave", action])
            .run()
            .assert_success()
    };

    autosave("install");
    assert_eq!(launchd.the_agent().label(), scratch);
    assert!(
        launchd
            .agents_dir()
            .join(format!("{scratch}.plist"))
            .is_file()
    );

    autosave("uninstall");
    assert!(launchd.installed().is_empty());
    assert!(launchd.loaded().is_empty());
}

/// A plist left in LaunchAgents is loaded at the next login, so an agent
/// launchd refused to load now mustn't stay there.
#[test]
fn an_agent_launchd_fails_to_load_is_not_left_installed() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    launchd.refuse_to_load();

    let out = launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .args(["autosave", "install"])
        .run()
        .assert_failure();

    assert!(out.stderr.contains("Bootstrap failed"), "{out}");
    assert!(launchd.installed().is_empty(), "its plist was removed");
    assert!(launchd.loaded().is_empty());
}

/// Rex moves its CLI on updates, so the agent doesn't pin one: with no `rex`
/// on its `PATH`, save runs the one a running Rex server runs (ADR-0003).
#[test]
fn the_agent_gets_only_the_system_path_whether_or_not_rex_is_on_path_at_install() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let with_rex = env.rexurrect_at(NO_SERVER);
    let without_rex = env.rexurrect_at(NO_SERVER).env("PATH", "/usr/bin:/bin");

    for install in [with_rex, without_rex] {
        launchd
            .aim(install)
            .args(["autosave", "install"])
            .run()
            .assert_success();

        let agent = launchd.the_agent();
        assert!(
            agent.environment().contains(&("PATH", SYSTEM_PATH)),
            "{agent:?}"
        );
    }
}

#[test]
fn install_hands_the_programs_whose_blocks_get_no_scrollback_on_to_autosave() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);

    launchd
        .aim(env.rexurrect_at(NO_SERVER))
        .env("REXURRECT_NO_SCROLLBACK", "claude aider")
        .args(["autosave", "install"])
        .run()
        .assert_success();

    let agent = launchd.the_agent();
    assert!(
        agent
            .environment()
            .contains(&("REXURRECT_NO_SCROLLBACK", "claude aider")),
        "{agent:?}"
    );
}

#[test]
fn autosaves_keep_as_many_snapshots_as_install_was_told_to() {
    let env = TestEnv::new();
    let launchd = FakeLaunchd::new(&env);
    let server = env.start_server();
    let fixture = server.create_session("zz-kept", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    launchd
        .aim(env.rexurrect(&server))
        .env("REXURRECT_KEEP", "2")
        .args(["autosave", "install"])
        .run()
        .assert_success();
    let agent = launchd.the_agent();

    for _ in 0..3 {
        agent.run_as_launchd(&env).assert_success();
    }

    let list = env.rexurrect(&server).arg("list").run().assert_success();
    assert_eq!(list.stdout.lines().count(), 2, "{list}");
}
