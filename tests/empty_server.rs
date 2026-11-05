//! An empty server: one with no sessions worth saving.
//!
//! On an emptied server instance, one rexurrect restored into, the user
//! closed its sessions, and a save records that with a snapshot with no
//! sessions (ADR-0009). On a fresh one, a save restores first, so an
//! autosave between a restart and a restore can't destroy the latest
//! snapshot.

mod common;

use common::server::wait_for;
use common::*;

/// Types `exit` in the session's only shell and waits until Rex reports the
/// session with no windows. Rex keeps such a session live.
fn exit_last_shell(server: &Server, session: &FixtureSession) {
    // Rex keeps a block open when its process exits within 250 ms of
    // starting (the terminal option `exit.quick_exit_threshold_ms`), and the
    // harness shell reports its `pwd` sooner than that.
    server.wait_for_pwds(session);
    std::thread::sleep(std::time::Duration::from_millis(500));
    server.write(&session.id, session.block_ids()[0], "exit\n");
    wait_for("the session to have no windows", || {
        server
            .view(&session.id)
            .get("windows")
            .is_none()
            .then_some(())
    });
}

#[test]
fn save_on_a_fresh_server_with_no_sessions_keeps_the_latest_snapshot() {
    let env = TestEnv::new();
    let a = env.mkdir("a");
    let layout = || hsplit(0.6, shell(&a), shell(&a));
    {
        let before = env.start_server();
        let fixture = before.create_session("zz-kept", &[window(layout())]);
        before.wait_for_pwds(&fixture);
        env.save(&before);
    } // The server stops, as in a restart.

    let after = env.start_server();
    assert!(
        after.sessions().is_empty(),
        "a fresh server has no sessions"
    );
    env.save(&after);

    env.restore(&after);
    let restored = after.session_ids_labelled("zz-kept");
    assert_eq!(
        restored.len(),
        1,
        "the earlier snapshot is still the latest"
    );
    assert_eq!(
        after.window_shapes(&restored[0]),
        [layout().expected_shape()]
    );
}

/// On an emptied server instance, one rexurrect has restored into (here, the
/// save with no snapshot yet), the user closed every session: a save records
/// that, so neither a restore nor a restart brings them back.
#[test]
fn save_after_every_session_was_killed_saves_a_snapshot_with_no_sessions() {
    let env = TestEnv::new();
    let server = env.start_server();
    let fixture = server.create_session("zz-closed", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    env.save(&server);
    server.kill_session(&fixture.id);

    let out = env.save(&server);
    assert!(
        out.stdout.contains("saved a snapshot with no sessions"),
        "{out}"
    );
    assert!(!out.stdout.contains("nothing to save"), "{out}");
    let listed = env.listed(&server);
    let newest = listed.lines().next().unwrap();
    assert!(newest.ends_with("(no sessions)"), "{listed}");

    let restored = env.restore(&server);
    assert!(restored.lines_starting("created").is_empty(), "{restored}");
    assert_eq!(server.sessions(), []);

    drop(server);
    let fresh = env.start_server();
    let saved = env.save(&fresh);
    assert!(saved.stdout.contains("restored first"), "{saved}");
    assert_eq!(fresh.sessions(), [], "a session came back: {saved}");
}

/// A session with no windows has nothing to restore: it's left out instead
/// of failing the save, and a server holding only such sessions is empty,
/// here on an emptied server instance.
#[test]
fn a_session_whose_last_shell_exited_is_left_out_of_the_snapshot() {
    let env = TestEnv::new();
    let server = env.start_server();
    let a = env.mkdir("a");
    let kept = server.create_session("zz-kept", &[window(shell(&a))]);
    let emptied = server.create_session("zz-emptied", &[window(shell(&a))]);
    server.wait_for_pwds(&kept);
    exit_last_shell(&server, &emptied);

    env.save(&server);
    let listed = env.listed(&server);
    assert!(
        listed.lines().next().unwrap().ends_with("  zz-kept"),
        "{listed}"
    );
    server.kill_session(&kept.id);
    let out = env.save(&server); // Only zz-emptied is live: an empty server.
    assert!(
        out.stdout.contains("saved a snapshot with no sessions"),
        "{out}"
    );
    server.kill_session(&emptied.id);

    env.restore(&server);
    assert_eq!(server.sessions(), []);
}

/// Autosave keeps saving an emptied server instance: one snapshot with no
/// sessions records it, and the next saves change nothing, so they don't
/// push earlier snapshots out of retention. Those stay listed and
/// restorable.
#[test]
fn repeated_saves_on_an_emptied_server_instance_write_one_snapshot_with_no_sessions() {
    let env = TestEnv::new();
    let server = env.start_server();
    let fixture = server.create_session("zz-closed", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);
    env.save(&server);
    server.kill_session(&fixture.id);
    env.save(&server);
    let listed = env.listed(&server);

    let out = env.save(&server);
    assert!(out.stdout.contains("nothing changed"), "{out}");
    assert_eq!(env.listed(&server), listed, "a new snapshot: {out}");

    let earlier = listed.lines().nth(1).unwrap();
    assert!(earlier.ends_with("zz-closed"), "{listed}");
    let id = earlier.split("  ").next().unwrap();
    env.rexurrect(&server)
        .args(["restore", "--snapshot", id])
        .run()
        .assert_success();
    assert_eq!(server.sorted_labels(), ["zz-closed"]);
}
