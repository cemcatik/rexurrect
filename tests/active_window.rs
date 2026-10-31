//! A restored session with more than one window opens on its first window.
//! `session.create` leaves the last window it creates active, so restore
//! focuses the first one afterwards (#80).

mod common;

use common::*;

/// Each of the session's window IDs, in window order.
fn window_ids(server: &Server, session_id: &str) -> Vec<String> {
    labels(&server.view(session_id))
        .into_iter()
        .map(|w| w.window_id)
        .collect()
}

/// Saves `zz-wide`, a session of three one-shell windows, on a server that
/// then stops. The next server started from `env` is a restart.
fn saved_three_window_session(env: &TestEnv) {
    let a = env.mkdir("a");
    let before = env.start_server();
    let wide = before.create_session(
        "zz-wide",
        &[window(shell(&a)), window(shell(&a)), window(shell(&a))],
    );
    before.wait_for_pwds(&wide);
    env.save(&before);
}

#[test]
fn a_restored_session_with_several_windows_opens_on_its_first_window() {
    let env = TestEnv::new();
    saved_three_window_session(&env);
    let after = env.start_server();

    env.restore(&after);

    let session_id = after.the_session_labelled("zz-wide");
    let windows = window_ids(&after, &session_id);
    assert_eq!(windows.len(), 3);
    assert_eq!(after.active_window(&session_id), windows[0]);
}

/// The session exists once `session.create` answers, so a window that
/// couldn't be focused is a note on its `created` line, not a failure.
#[test]
fn a_first_window_that_cant_be_focused_leaves_the_session_created_with_a_note() {
    let env = TestEnv::new();
    saved_three_window_session(&env);
    let after = env.start_server();
    let stub = RexStub::new(&env).fail_when("session.focus_window", "window:", 1, "Error: boom");

    let out = env
        .rexurrect(&after)
        .arg("restore")
        .path_prepend(stub.dir())
        .run()
        .assert_success();

    let created = out.lines_starting("created");
    assert_eq!(created.len(), 1, "{out}");
    let session_id = after.the_session_labelled("zz-wide");
    assert!(created[0].contains(&session_id), "{out}");
    assert!(
        created[0].ends_with(
            "(couldn't focus its first window: \
             rex api call session.focus_window failed (exit 1): boom)"
        ),
        "{out}"
    );
    assert!(
        out.stdout
            .contains("1 created, 0 replaced, 0 skipped, 0 failed"),
        "{out}"
    );
    assert!(env.log_entries().is_empty(), "{:#?}", env.log_entries());
    // Its lineage is recorded all the same: the next restore skips it.
    let again = env.restore(&after);
    assert_eq!(again.lines_starting("skipped").len(), 1, "{again}");
    assert_eq!(after.session_ids_labelled("zz-wide"), [session_id]);
}

/// How many times `recording` was run to call `session.focus_window`.
fn focus_calls(recording: &RecordingRex) -> usize {
    recording
        .runs()
        .iter()
        .filter(|run| run.contains("session.focus_window"))
        .count()
}

#[test]
fn restore_focuses_no_window_of_a_one_window_session() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-narrow"]);
    let after = env.start_server();
    let recording = RecordingRex::new(&env);

    env.rexurrect(&after)
        .arg("restore")
        .path_prepend(recording.dir())
        .run()
        .assert_success();

    assert_eq!(after.sorted_labels(), ["zz-narrow"]);
    assert_eq!(focus_calls(&recording), 0, "{:#?}", recording.runs());
}

#[test]
fn a_dry_run_focuses_no_window() {
    let env = TestEnv::new();
    saved_three_window_session(&env);
    let after = env.start_server();
    let recording = RecordingRex::new(&env);

    let out = env
        .rexurrect(&after)
        .args(["restore", "--dry-run"])
        .path_prepend(recording.dir())
        .run()
        .assert_success();

    assert!(out.stdout.contains("would create  zz-wide"), "{out}");
    assert!(!recording.runs().is_empty(), "the recording rex wasn't run");
    assert_eq!(focus_calls(&recording), 0, "{:#?}", recording.runs());
}
