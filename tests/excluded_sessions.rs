//! Excluded sessions (`exclude_sessions`, ADR-0010): sessions another tool
//! makes and keeps up itself, such as tr3x's `T3 Code`, which rexurrect
//! leaves to that tool. Save doesn't save them, and restore neither creates
//! nor replaces them, auto-restore at server start included, even from a
//! snapshot taken before they were excluded.
//!
//! As in `carried_over.rs`, a stub plays Rex's restart: a new server
//! instance, started after the snapshot was taken, with the snapshot's
//! sessions still live under the IDs they were saved with.

mod common;

use common::*;

/// A start time later than any snapshot a test takes.
const LATER: &str = "2099-01-01T00:00:00.000000Z";

fn restart_sed(started_at: &str, generation: u32) -> String {
    format!(
        r#"s/"instance_id":"[^"]*"/"instance_id":"zz-carried-over-instance"/;s/"started_at":"[^"]*"/"started_at":"{started_at}"/;s/"generation":[0-9]*/"generation":{generation}/"#
    )
}

/// A session labelled `label`: a shell in a directory of its own, so it's
/// worth saving.
fn session(env: &TestEnv, server: &Server, label: &str) -> FixtureSession {
    let session = server.create_session(label, &[window(shell(env.mkdir(label)))]);
    server.wait_for_pwds(&session);
    session
}

fn kill_every_session(server: &Server) {
    for session in server.sessions() {
        server.kill_session(&session.id);
    }
}

#[test]
fn an_excluded_session_is_neither_saved_nor_restored() {
    let env = TestEnv::new();
    env.write_config("exclude_sessions = [\"zz-tool\", \"zz-tool · *\"]\n");
    let server = env.start_server();
    session(&env, &server, "zz-mine");
    session(&env, &server, "zz-tool");
    session(&env, &server, "zz-tool · studio");

    let saved = env.save(&server).assert_success();
    assert!(saved.stdout.contains("saved 1 session(s)"), "{saved}");
    kill_every_session(&server);
    env.restore(&server).assert_success();

    assert_eq!(server.sorted_labels(), ["zz-mine"]);
}

/// The sessions were saved before they were excluded, so the latest
/// snapshot still holds them. Auto-restore at server start replaces the
/// others Rex carried over, leaves the excluded one live as it is, and
/// counts the instance as restored, so saves carry on.
#[test]
fn auto_restore_leaves_an_excluded_carried_over_session_alone() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\n");
    let stub = RexStub::new(&env);
    let server = env.server().shown_as(stub.dir().join("rex")).start();
    let mine = session(&env, &server, "zz-mine");
    let tool = session(&env, &server, "zz-tool");
    env.save(&server).assert_success();
    env.write_config("app_session_watch = 1\nexclude_sessions = [\"zz-tool\"]\n");
    let stub = stub.rewrite_when("server.status", "{", &restart_sed(LATER, 1));

    env.rexurrect(&server)
        .args(["auto-restore", "--server-pid", &server.pid().to_string()])
        .run()
        .assert_success();

    let entries = env.log_entries();
    let entry = entries.last().expect("auto-restore logs its outcome");
    assert!(
        entry.contains("excluded  zz-tool  (exclude_sessions)"),
        "{entry}"
    );
    assert!(
        entry.contains("restore: 0 created, 1 replaced, 1 skipped, 0 failed"),
        "{entry}"
    );
    assert_eq!(server.session_ids_labelled("zz-tool"), [tool.id]);
    assert_ne!(server.the_session_labelled("zz-mine"), mine.id);
    let saved = env
        .rexurrect(&server)
        .arg("save")
        .path_prepend(stub.dir())
        .run()
        .assert_success();
    assert!(!saved.stdout.contains("restored first"), "{saved}");
}

/// Save backs off on an unrestored instance holding a session Rex carried
/// over, but not for an excluded one: nothing will ever replace it.
#[test]
fn save_doesnt_back_off_for_an_excluded_carried_over_session() {
    let env = TestEnv::new();
    let server = env.start_server();
    let tool = session(&env, &server, "zz-tool");
    env.save(&server).assert_success();
    env.write_config("exclude_sessions = [\"zz-*\"]\n");
    let stub = RexStub::new(&env).rewrite_when("server.status", "{", &restart_sed(LATER, 1));

    let out = env
        .rexurrect(&server)
        .arg("save")
        .path_prepend(stub.dir())
        .run()
        .assert_success();

    assert!(
        out.stdout.contains("excluded  zz-tool  (exclude_sessions)"),
        "{out}"
    );
    assert_eq!(server.session_ids_labelled("zz-tool"), [tool.id]);
}
