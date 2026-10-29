//! On Debian and Ubuntu, an unrelated package also named `rex`, (R)?ex (a
//! Perl automation framework), can replace Rex's: unattended-upgrade
//! installs it over Rex's in spite of Rex's apt pin (#79). Every call
//! through it then fails with its Rexfile warnings. rexurrect says instead
//! which `rex` it ran and that it isn't Rex's CLI, judged the way Rex's
//! docs do: `rex -v` prints `(R)?ex`. It checks only once a call has
//! failed, and still runs the `rex` it always would (ADR-0003).

mod common;

use std::path::Path;

use common::*;

/// Rex's Linux installation docs, which cover (R)?ex.
const INSTALL_DOCS: &str = "https://www.superlogical.com/rex/docs/linux/installation";

/// `message` names `rex` as (R)?ex rather than Rex's CLI and points to Rex's
/// installation docs, without (R)?ex's own output.
fn assert_names_perl_rex(message: &str, rex: &Path) {
    assert!(message.contains(rex.to_str().unwrap()), "{message}");
    assert!(message.contains("(R)?ex"), "{message}");
    assert!(message.contains("not Rex's CLI"), "{message}");
    assert!(message.contains(INSTALL_DOCS), "{message}");
    assert!(!message.contains("Rexfile"), "{message}");
}

#[test]
fn a_save_through_perl_rex_names_it_and_points_to_rexs_install_docs() {
    let env = TestEnv::new();
    let server = env.start_server();
    let perl = perl_rex(&env);

    let run = env
        .rexurrect(&server)
        .path_prepend(&perl)
        .arg("save")
        .run()
        .assert_failure();

    let entries = env.log_entries();
    assert_eq!(entries.len(), 1, "{entries:#?}");
    for message in [&run.stderr, &entries[0]] {
        assert_names_perl_rex(message, &perl.join("rex"));
    }
}

#[test]
fn a_failure_from_rexs_own_cli_is_reported_as_it_is() {
    let env = TestEnv::new();
    let server = env.start_server();
    let failure = "stream response \"control-1\" failed: boom (status=500 code=internal)";
    let stub = RexStub::new(&env).fail_when_all("server.status", &[], 1, failure);

    let run = env
        .rexurrect(&server)
        .path_prepend(stub.dir())
        .arg("save")
        .run()
        .assert_failure();

    assert!(run.stderr.contains(failure), "{run}");
    assert!(!run.stderr.contains("(R)?ex"), "{run}");
}

#[test]
fn a_save_and_restore_that_succeed_never_check_for_perl_rex() {
    let env = TestEnv::new();
    let server = env.start_server();
    let fixture = server.create_session("zz-kept", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    let (recording, runs) = recording_rex(&env);
    let run = |command: &str| {
        env.rexurrect(&server)
            .path_prepend(&recording)
            .arg(command)
            .run()
            .assert_success()
    };

    run("save");
    server.kill_session(&fixture.id);
    run("restore");

    let runs = std::fs::read_to_string(runs).unwrap();
    assert!(runs.contains("api call"), "the recording rex ran: {runs}");
    assert!(!runs.lines().any(|args| args == "-v"), "{runs}");
}

#[test]
fn a_restore_through_perl_rex_names_it_and_points_to_rexs_install_docs() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-saved"]);
    let server = env.start_server();
    let perl = perl_rex(&env);

    let run = env
        .rexurrect(&server)
        .path_prepend(&perl)
        .arg("restore")
        .run()
        .assert_failure();

    let entries = env.log_entries();
    assert_eq!(entries.len(), 1, "{entries:#?}");
    for message in [&run.stderr, &entries[0]] {
        assert_names_perl_rex(message, &perl.join("rex"));
    }
}
