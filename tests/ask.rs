//! With `REXURRECT_ASK` set, nothing restores into a restarted server until
//! someone chooses: save neither restores first nor replaces the snapshot on
//! offer, `fresh` declines it, and `ask` only asks at a terminal.

mod common;

use common::*;

const ASK: &str = "REXURRECT_ASK";

fn save_asking(env: &TestEnv, server: &Server) -> String {
    env.rexurrect(server)
        .env(ASK, "1")
        .arg("save")
        .run()
        .assert_success()
        .stdout
}

#[test]
fn save_on_a_restarted_server_waits_for_a_choice_and_keeps_the_latest_snapshot() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-proj"]);
    let listed = env.listed(&env.start_server());

    let after = env.start_server();
    let early = after.create_session("zz-early", &[window(shell(env.mkdir("early")))]);
    after.wait_for_pwds(&early);
    let out = save_asking(&env, &after);

    assert!(out.contains("nothing saved"), "save printed {out:?}");
    assert_eq!(after.sorted_labels(), ["zz-early"]);
    assert_eq!(env.listed(&after), listed);
}

#[test]
fn ask_without_a_terminal_chooses_nothing() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-proj"]);
    let after = env.start_server();

    env.rexurrect(&after)
        .env(ASK, "1")
        .arg("ask")
        .run()
        .assert_success();

    assert!(after.sorted_labels().is_empty());
    assert!(save_asking(&env, &after).contains("nothing saved"));
}

#[test]
fn ask_never_restores_without_asking_even_with_asking_off() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-proj"]);
    let after = env.start_server();

    env.rexurrect(&after)
        .env_remove(ASK)
        .arg("ask")
        .run()
        .assert_success();

    assert!(after.sorted_labels().is_empty());
}

#[test]
fn fresh_declines_the_restore_so_saves_save_what_is_live() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-proj"]);
    let after = env.start_server();
    let early = after.create_session("zz-early", &[window(shell(env.mkdir("early")))]);
    after.wait_for_pwds(&early);

    env.rexurrect(&after)
        .env(ASK, "1")
        .arg("fresh")
        .run()
        .assert_success();
    let out = save_asking(&env, &after);

    assert!(out.contains("saved 1 session(s)"), "save printed {out:?}");
    assert_eq!(after.sorted_labels(), ["zz-early"]);
    // The declined snapshot is still there to restore by hand.
    let listed = env.listed(&after);
    assert_eq!(listed.lines().count(), 2, "list printed {listed:?}");
}

#[test]
fn with_nothing_to_restore_save_saves_as_usual() {
    let env = TestEnv::new();
    let server = env.start_server();
    let fixture = server.create_session("zz-proj", &[window(shell(env.mkdir("proj")))]);
    server.wait_for_pwds(&fixture);

    let out = save_asking(&env, &server);

    assert!(out.contains("saved 1 session(s)"), "save printed {out:?}");
}

#[test]
fn without_asking_save_still_restores_first() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-proj"]);
    let after = env.start_server();

    env.rexurrect(&after)
        .env(ASK, "0")
        .arg("save")
        .run()
        .assert_success();

    assert_eq!(after.sorted_labels(), ["zz-proj"]);
}
