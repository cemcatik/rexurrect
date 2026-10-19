//! Without `rex` on `PATH` (autosave's agent never has it), save and
//! restore run the `rex` a running Rex server runs, still naming the server
//! to call with `-S` (ADR-0003).
//!
//! Every throwaway server runs the same `rex`, so which server's executable
//! gets picked can't be told apart here; that choice has unit tests of its
//! own.

mod common;

use common::*;

/// The system directories, and no `rex`: launchd's `PATH` for its agents.
const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

#[test]
fn save_and_restore_reach_a_running_server_without_rex_on_path() {
    let env = TestEnv::new();
    let server = env.start_server();
    let fixture = server.create_session("zz-off-path", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    let run = |command: &str| {
        env.rexurrect(&server)
            .env("PATH", SYSTEM_PATH)
            .arg(command)
            .run()
            .assert_success()
    };

    run("save");
    server.kill_session(&fixture.id);
    run("restore");

    assert_eq!(server.sorted_labels(), ["zz-off-path"]);
}

#[test]
fn a_save_with_no_server_at_the_endpoint_logs_the_failure_naming_it_and_starts_no_server() {
    let env = TestEnv::new();
    let endpoint = env.start_server().endpoint().to_string(); // It stops here.
    // A temp HOME too: if autostart were ever on, rex's default server would
    // start in there rather than anywhere real.
    let home = env.mkdir("home");

    let out = env
        .rexurrect_at(&endpoint)
        .env("PATH", SYSTEM_PATH)
        .env("HOME", &home)
        .arg("save")
        .run();

    if let Some(pid) = env.server_pid_at(&endpoint) {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
        panic!("the save started a Rex server (pid {pid}) at {endpoint}");
    }
    out.assert_failure();
    let entries = env.log_entries();
    assert_eq!(entries.len(), 1, "{entries:#?}");
    assert!(entries[0].contains("save failed"), "{entries:#?}");
    assert!(entries[0].contains(&endpoint), "{entries:#?}");
}
