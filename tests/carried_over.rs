//! Sessions Rex carried over (ADR-0008): Rex builds with API revision 9
//! bring a server's sessions back by themselves when it starts, keeping
//! their session and block IDs but not their scrollback or programs.
//! rexurrect replaces them: auto-restore at server start, `restore --force`
//! on request. A plain restore skips them without counting the server
//! instance as restored, and save backs off.
//!
//! The throwaway servers' Rex doesn't carry sessions over, so a stub
//! ([`restarted_at`]) plays the restart: it reports a new server instance,
//! started after the snapshot was taken, while the snapshot's sessions are
//! still live with the IDs they were saved with. That's what Rex's copies
//! look like.

mod common;

use common::stub::sh_quote;
use common::*;

/// A start time later than any snapshot a test takes.
const LATER: &str = "2099-01-01T00:00:00.000000Z";

/// A stub that makes the server look restarted at `started_at` (a new
/// server instance), with init.lua run `generation` times in it: 1 at the
/// start, more after each `rex config reload`.
fn restarted_at(env: &TestEnv, started_at: &str, generation: u32) -> RexStub {
    RexStub::new(env).rewrite_when("server.status", "{", &restart_sed(started_at, generation))
}

fn restart_sed(started_at: &str, generation: u32) -> String {
    format!(
        r#"s/"instance_id":"[^"]*"/"instance_id":"zz-carried-over-instance"/;s/"started_at":"[^"]*"/"started_at":"{started_at}"/;s/"generation":[0-9]*/"generation":{generation}/"#
    )
}

/// Two sessions, each a shell in its own directory that has printed
/// something, saved: through [`restarted_at`]'s stub, Rex's copies.
fn two_saved_sessions(env: &TestEnv, server: &Server) -> [FixtureSession; 2] {
    let sessions = ["zz-a", "zz-b"].map(|label| {
        let session = server.create_session(label, &[window(shell(env.mkdir(label)))]);
        server.wait_for_pwds(&session);
        let block = session.block_ids()[0];
        server.run(&session.id, block, &format!("echo {label}-earlier"));
        server.wait_for_capture(&session.id, block, "the echo", |c| {
            c.contains(&format!("\n{label}-earlier"))
        });
        session
    });
    env.save(server);
    sessions
}

/// Runs rexurrect with `args` through `stub`.
fn run(env: &TestEnv, server: &Server, stub: &RexStub, args: &[&str]) -> RunOutput {
    env.rexurrect(server)
        .args(args)
        .path_prepend(stub.dir())
        .run()
}

/// Whether the (stubbed) server instance counts as restored: a save then
/// doesn't restore first.
fn assert_instance_restored(env: &TestEnv, server: &Server, stub: &RexStub) {
    let saved = run(env, server, stub, &["save"]).assert_success();
    assert!(!saved.stdout.contains("restored first"), "{saved}");
}

/// Waits until the block of `label`'s only live session shows the scrollback
/// its saved session printed.
fn assert_scrollback_came_back(server: &Server, label: &str) {
    let session = server.the_session_labelled(label);
    let block = server.block_ids(&session).remove(0);
    server.wait_for_capture(&session, &block, "the replayed scrollback", |c| {
        c.contains(&format!("\n{label}-earlier"))
    });
}

#[test]
fn restore_force_replaces_each_carried_over_session_with_its_saved_one() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, b] = two_saved_sessions(&env, &server);
    let stub = restarted_at(&env, LATER, 1);

    let out = run(&env, &server, &stub, &["restore", "--force"]).assert_success();

    assert_eq!(server.sorted_labels(), ["zz-a", "zz-b"]);
    let new_a = server.the_session_labelled("zz-a");
    let new_b = server.the_session_labelled("zz-b");
    assert_ne!(new_a, a.id);
    assert_ne!(new_b, b.id);
    assert_eq!(
        out.lines_starting("replaced"),
        [
            format!("replaced  zz-a  {new_a}  (closed {})", a.id),
            format!("replaced  zz-b  {new_b}  (closed {})", b.id),
        ]
    );
    assert!(
        out.stdout
            .contains("restore: 0 created, 2 replaced, 0 skipped, 0 failed"),
        "{out}"
    );
    assert_scrollback_came_back(&server, "zz-a");
    assert_scrollback_came_back(&server, "zz-b");
    assert_instance_restored(&env, &server, &stub);
}

#[test]
fn a_session_whose_create_fails_under_force_keeps_its_live_one() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, b] = two_saved_sessions(&env, &server);
    let stub = restarted_at(&env, LATER, 1).fail_when(
        "session.create",
        "zz-a",
        1,
        "Error: refused (status=500 code=internal)",
    );

    let out = run(&env, &server, &stub, &["restore", "--force"]).assert_failure();

    assert_eq!(server.session_ids_labelled("zz-a"), [a.id]);
    let new_b = server.the_session_labelled("zz-b");
    assert_ne!(new_b, b.id);
    assert_eq!(out.lines_starting("failed").len(), 1, "{out}");
    assert!(out.stdout.contains("failed   zz-a"), "{out}");
    assert!(
        out.stdout
            .contains("restore: 0 created, 1 replaced, 0 skipped, 1 failed"),
        "{out}"
    );
}

#[test]
fn a_dry_run_with_force_says_what_it_would_replace_and_changes_nothing() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, b] = two_saved_sessions(&env, &server);
    let stub = restarted_at(&env, LATER, 1);

    let out = run(&env, &server, &stub, &["restore", "--force", "--dry-run"]).assert_success();

    assert_eq!(
        out.lines_starting("would"),
        [
            format!("would replace  zz-a  (live as {})", a.id),
            format!("would replace  zz-b  (live as {})", b.id),
        ]
    );
    assert_eq!(server.session_ids_labelled("zz-a"), [a.id]);
    assert_eq!(server.session_ids_labelled("zz-b"), [b.id]);
    // Still unrestored, and still holding Rex's copies: save backs off.
    run(&env, &server, &stub, &["save"]).assert_failure();
}

#[test]
fn force_with_a_session_label_replaces_only_that_labels_sessions_and_does_not_count_as_restoring() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, b] = two_saved_sessions(&env, &server);
    let stub = restarted_at(&env, LATER, 1);

    let out = run(
        &env,
        &server,
        &stub,
        &["restore", "--force", "--session", "zz-a"],
    )
    .assert_success();

    assert_eq!(out.lines_starting("replaced").len(), 1, "{out}");
    assert_ne!(server.the_session_labelled("zz-a"), a.id);
    assert_eq!(server.session_ids_labelled("zz-b"), [b.id]);
    // zz-b is still Rex's copy, and the instance is still unrestored.
    run(&env, &server, &stub, &["save"]).assert_failure();
}

/// The session `restore --force` runs in disappears, with the terminal it
/// prints to, as soon as it's closed: so it's closed last, once everything
/// is recorded and the report is in the log.
#[test]
fn force_run_inside_a_session_it_replaces_closes_that_session_last() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, b] = two_saved_sessions(&env, &server);
    // What the log holds as Rex closes the session the run is in.
    let log_then = env.root().join("log-when-closing-own-session");
    let stub = restarted_at(&env, LATER, 1).after(
        "session.destroy",
        &format!(
            r#"case "$input" in *{}*) cp {} {} ;; esac"#,
            a.id,
            sh_quote(&env.state_dir().join("rexurrect.log")),
            sh_quote(&log_then)
        ),
    );

    let out = env
        .rexurrect(&server)
        .args(["restore", "--force"])
        .path_prepend(stub.dir())
        .env("REX_BLOCK", a.block_ids()[0])
        .run()
        .assert_success();

    assert_eq!(server.sorted_labels(), ["zz-a", "zz-b"]);
    let new_a = server.the_session_labelled("zz-a");
    let new_b = server.the_session_labelled("zz-b");
    assert_ne!(new_a, a.id);
    // zz-b was replaced first, though it comes after zz-a.
    let lines = out.lines_starting("replaced");
    assert_eq!(
        lines[1],
        format!("replaced  zz-b  {new_b}  (closed {})", b.id)
    );
    assert!(lines[0].contains(&a.id), "{out}");
    let logged = std::fs::read_to_string(&log_then)
        .unwrap_or_else(|e| panic!("the stub saw no close of {}: {e}\n{out}", a.id));
    assert!(
        logged.contains(&format!("replaced  zz-b  {new_b}")),
        "{logged}"
    );
    assert!(
        logged.contains("restore: 0 created, 2 replaced, 0 skipped, 0 failed"),
        "{logged}"
    );
    assert_instance_restored(&env, &server, &stub);
}

#[test]
fn plain_restore_skips_carried_over_sessions_says_so_and_leaves_the_instance_unrestored() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, b] = two_saved_sessions(&env, &server);
    let stub = restarted_at(&env, LATER, 1);

    let out = run(&env, &server, &stub, &["restore"]).assert_success();

    assert_eq!(
        out.lines_starting("skipped"),
        [
            "skipped  zz-a  (carried over by Rex; --force replaces it)",
            "skipped  zz-b  (carried over by Rex; --force replaces it)",
        ]
    );
    assert_eq!(server.session_ids_labelled("zz-a"), [a.id]);
    assert_eq!(server.session_ids_labelled("zz-b"), [b.id]);
    run(&env, &server, &stub, &["save"]).assert_failure();
}

/// A session restore created in this server instance holds none of the
/// snapshot's blocks: it's skipped as it always was.
#[test]
fn plain_restore_skips_a_session_restored_into_this_instance_without_the_hint() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-a", "zz-b"]);
    let server = env.start_server();
    env.rexurrect(&server)
        .args(["restore", "--session", "zz-a"])
        .run()
        .assert_success();
    let a = server.the_session_labelled("zz-a");

    let out = env.restore(&server);

    assert_eq!(
        out.lines_starting("skipped"),
        [format!("skipped  zz-a  (live as {a})")]
    );
    assert_eq!(out.lines_starting("created").len(), 1, "{out}");
    let saved = env.save(&server);
    assert!(!saved.stdout.contains("restored first"), "{saved}");
}

#[test]
fn save_backs_off_on_an_unrestored_instance_holding_a_carried_over_session() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, b] = two_saved_sessions(&env, &server);
    let listed = env.listed(&server);
    let stub = restarted_at(&env, LATER, 1);

    let out = run(&env, &server, &stub, &["save"]).assert_failure();

    assert!(out.stderr.contains("rexurrect restore --force"), "{out}");
    assert!(!out.stdout.contains("restored first"), "{out}");
    assert_eq!(env.listed(&server), listed, "no snapshot was written");
    assert_eq!(server.session_ids_labelled("zz-a"), [a.id]);
    assert_eq!(server.session_ids_labelled("zz-b"), [b.id]);
    let entries = env.log_entries();
    assert_eq!(entries.len(), 1, "{entries:#?}");
    assert!(entries[0].contains("save failed"), "{entries:#?}");
    assert!(entries[0].contains("restore --force"), "{entries:#?}");
    // Nothing was recorded either: it backs off again.
    run(&env, &server, &stub, &["save"]).assert_failure();
}

/// Auto-restore as the hook starts it in the background, run in the
/// foreground against a server whose `rex` is `stub`. Returns its log entry.
fn auto_restore(env: &TestEnv, server: &Server) -> String {
    env.rexurrect(server)
        .args(["auto-restore", "--server-pid", &server.pid().to_string()])
        .run()
        .assert_success();
    let entries = env.log_entries();
    let entry = entries.last().expect("auto-restore logs its outcome");
    assert!(entry.contains(" auto-restore into "), "{entries:#?}");
    entry.clone()
}

#[test]
fn auto_restore_at_server_start_replaces_carried_over_sessions() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\n");
    let stub = RexStub::new(&env);
    let server = env.server().shown_as(stub.dir().join("rex")).start();
    let [a, b] = two_saved_sessions(&env, &server);
    // Only now does the server look restarted.
    let stub = stub.rewrite_when("server.status", "{", &restart_sed(LATER, 1));

    let entry = auto_restore(&env, &server);

    assert!(
        entry.contains("restore: 0 created, 2 replaced, 0 skipped, 0 failed"),
        "{entry}"
    );
    assert_eq!(server.sorted_labels(), ["zz-a", "zz-b"]);
    assert_ne!(server.the_session_labelled("zz-a"), a.id);
    assert_ne!(server.the_session_labelled("zz-b"), b.id);
    assert_scrollback_came_back(&server, "zz-a");
    assert_instance_restored(&env, &server, &stub);
}

/// A `rex config reload` runs init.lua again, in a server the user may
/// have been working in for hours: auto-restore never replaces then.
#[test]
fn auto_restore_after_a_config_reload_only_skips_carried_over_sessions() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\n");
    let stub = RexStub::new(&env);
    let server = env.server().shown_as(stub.dir().join("rex")).start();
    let [a, b] = two_saved_sessions(&env, &server);
    let stub = stub.rewrite_when("server.status", "{", &restart_sed(LATER, 2));

    let entry = auto_restore(&env, &server);

    assert!(
        entry.contains("skipped  zz-a  (carried over by Rex; --force replaces it)"),
        "{entry}"
    );
    assert!(
        entry.contains("restore: 0 created, 0 replaced, 2 skipped, 0 failed"),
        "{entry}"
    );
    assert_eq!(server.session_ids_labelled("zz-a"), [a.id]);
    assert_eq!(server.session_ids_labelled("zz-b"), [b.id]);
    // Not counted as restoring the instance.
    run(&env, &server, &stub, &["save"]).assert_failure();
}

/// The current UTC time to the second, as Rex's `started_at` writes it.
fn now_to_the_second() -> String {
    let out = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%S.000000Z"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Rex starts a new shell in each block it carries over, and the zsh
/// recorder rewrites the block's record at that shell's first prompt. That
/// isn't the program the save found exiting: the block still resumes.
#[test]
fn a_replaced_block_resumes_although_its_carried_over_shell_rewrote_its_record() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, block) = running(&server, "zz-nvim", &dir, "nvim init.lua");
    env.write_recorder_file(&block, "nvim init.lua");
    env.save(&server);
    // The server restarts a second later, and the carried-over shell's
    // prompt rewrites the record.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let started_at = now_to_the_second();
    std::thread::sleep(std::time::Duration::from_millis(50));
    env.write_recorder_file(&block, "nvim init.lua");
    let stub = restarted_at(&env, &started_at, 1);

    run(&env, &server, &stub, &["restore", "--force"]).assert_success();

    let restored = server.the_session_labelled("zz-nvim");
    assert_ne!(restored, session);
    let block = server.block_ids(&restored).remove(0);
    server.wait_for_typed(&restored, &block, "nvim -S");
}

/// Without the restart, a record rewritten since the save still means the
/// program exited (#60).
#[test]
fn a_record_rewritten_before_the_restart_still_means_the_program_exited() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (_, block) = running(&server, "zz-nvim", &dir, "nvim init.lua");
    env.write_recorder_file(&block, "nvim init.lua");
    env.save(&server);
    std::thread::sleep(std::time::Duration::from_millis(50));
    env.write_recorder_file(&block, "nvim init.lua");
    let stub = restarted_at(&env, LATER, 1);

    run(&env, &server, &stub, &["restore", "--force"]).assert_success();

    let restored = server.the_session_labelled("zz-nvim");
    let block = server.block_ids(&restored).remove(0);
    server.wait_for_typed(&restored, &block, "nvim init.lua");
}
