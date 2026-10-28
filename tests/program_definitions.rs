//! Program definitions in the config file: `[programs.<name>]` tables that
//! change a built-in definition field by field, turn one off, or add a
//! program rexurrect doesn't know.
//!
//! Programs are stand-ins ([`stand_in_bin`]), first on every block's `PATH`.

mod common;

use common::server::wait_for;
use common::*;

/// The README's persistence.nvim (LazyVim) example.
const PERSISTENCE: &str = r#"nvim -c 'lua require("persistence").load()'"#;

#[test]
fn neovim_can_resume_through_persistence_with_or_without_a_session_vim() {
    let env = TestEnv::new();
    env.write_config(&format!(
        "app_session_watch = 1\n\
         [programs.nvim]\n\
         resume = '''{PERSISTENCE}'''\n\
         requires_file = \"\"\n"
    ));
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    let (session, _) = running(&server, "zz-lazy", &dir, "nvim init.lua");

    let (restored, _) = restored_with_typed(&env, &server, &session, "zz-lazy", PERSISTENCE);
    std::fs::write(dir.join("Session.vim"), "").unwrap();
    env.end_startup_windows();
    let block = server.block_ids(&restored).remove(0);
    server.write(&restored, &block, "\n");
    wait_for("nvim to run again", || {
        (runs(&dir, "nvim").len() == 2).then_some(())
    });
    restored_with_typed(&env, &server, &restored, "zz-lazy", PERSISTENCE);
}

/// Fields an override leaves out keep the built-in's: here, its
/// `requires_file`, `Session.vim`.
#[test]
fn a_field_an_override_leaves_out_keeps_the_built_ins_value() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\n[programs.nvim]\nresume = \"nvim -S Session.vim\"\n");
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let [with, without] = ["with", "without"].map(|name| env.mkdir(name));
    std::fs::write(with.join("Session.vim"), "").unwrap();
    let (a, _) = running(&server, "zz-with", &with, "nvim init.lua");
    let (b, _) = running(&server, "zz-without", &without, "nvim init.lua");

    env.save(&server);
    server.kill_session(&a);
    server.kill_session(&b);
    env.restore(&server);

    for (label, typed) in [
        ("zz-with", "nvim -S Session.vim"),
        ("zz-without", "nvim init.lua"),
    ] {
        let restored = server.the_session_labelled(label);
        server.wait_for_typed(&restored, &server.block_ids(&restored)[0], typed);
    }
}

#[test]
fn a_built_in_turned_off_restores_its_blocks_with_their_last_command() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\n[programs.nvim]\nenabled = false\n");
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    std::fs::write(dir.join("Session.vim"), "").unwrap();
    let (session, _) = running(&server, "zz-off", &dir, "nvim init.lua");

    restored_with_typed(&env, &server, &session, "zz-off", "nvim init.lua");
}

/// A program rexurrect doesn't know, whose own hook or plugin reports an
/// ID, the way the README's bespoke example does.
const MYREPL: &str = "app_session_watch = 1\n\
    [programs.myrepl]\n\
    state = \"id\"\n\
    resume = \"myrepl --attach {state}\"\n";

#[test]
fn a_new_program_resumes_with_its_template_and_the_id_it_reported() {
    let env = TestEnv::new();
    env.write_config(MYREPL);
    stand_in_bin(&env, "myrepl");
    let server = env.start_server();
    let (session, block) = running(&server, "zz-repl", &env.mkdir("a"), "myrepl");

    let out = env
        .rexurrect(&server)
        .env("REX_BLOCK", &block)
        .args(["report", "myrepl", "repl:42"])
        .run();
    assert!(
        out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
        "{out}"
    );

    restored_with_typed(
        &env,
        &server,
        &session,
        "zz-repl",
        "myrepl --attach repl:42",
    );
}

#[test]
fn a_reported_path_is_quoted_where_zsh_needs_it() {
    let env = TestEnv::new();
    env.write_config(
        "app_session_watch = 1\n\
         [programs.journal]\n\
         state = \"path\"\n\
         resume = \"journal --open {state}\"\n",
    );
    stand_in_bin(&env, "journal");
    let server = env.start_server();
    let (session, block) = running(&server, "zz-path", &env.mkdir("a"), "journal");

    env.rexurrect(&server)
        .env("REX_BLOCK", &block)
        .args(["report", "journal", "/notes/it's today.md"])
        .run()
        .assert_success();

    restored_with_typed(
        &env,
        &server,
        &session,
        "zz-path",
        r"journal --open '/notes/it'\''s today.md'",
    );
}

#[test]
fn report_drops_a_value_that_doesnt_fit_the_programs_state_kind() {
    let env = TestEnv::new();
    env.write_config(MYREPL);
    stand_in_bin(&env, "myrepl");
    let server = env.start_server();
    let (session, block) = running(&server, "zz-unfit", &env.mkdir("a"), "myrepl");

    let out = env
        .rexurrect(&server)
        .env("REX_BLOCK", &block)
        .args(["report", "myrepl", "/not/an/id"])
        .run();
    assert!(
        out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
        "{out}"
    );

    restored_with_typed(&env, &server, &session, "zz-unfit", "myrepl");
    // The log names the value, so a hook sending the wrong one can be told
    // from one sending nothing.
    let logged = env.log_entries();
    assert!(
        logged
            .iter()
            .any(|e| e.contains(r#"the value "/not/an/id" isn't an ID"#)),
        "{logged:#?}"
    );
}

#[test]
fn a_new_program_can_auto_resume() {
    let env = TestEnv::new();
    env.write_config(&format!("auto_resume = [\"myrepl\"]\n{MYREPL}"));
    stand_in_bin(&env, "myrepl");
    let server = env.start_server();
    let dir = env.mkdir("a");
    let (session, block) = running(&server, "zz-auto", &dir, "myrepl");
    env.rexurrect(&server)
        .env("REX_BLOCK", &block)
        .args(["report", "myrepl", "repl:42"])
        .run()
        .assert_success();

    env.save(&server);
    server.kill_session(&session);
    env.restore(&server);

    let runs = wait_for("myrepl to run again", || {
        let runs = runs(&dir, "myrepl");
        (runs.len() == 2).then_some(runs)
    });
    assert_eq!(runs, ["", "--attach repl:42"]);
}

#[test]
fn a_definition_rexurrect_cannot_use_fails_save_naming_the_table_and_the_field() {
    let env = TestEnv::new();
    let server = env.start_server();
    let fixture = server.create_session("zz-kept", &[window(shell(env.mkdir("a")))]);
    server.wait_for_pwds(&fixture);

    for (toml, names) in [
        (
            "[programs.nvim]\nresmue = \"nvim -S\"\n",
            ["programs.nvim.resmue"],
        ),
        (
            "[programs.x]\nstate = \"uuid\"\nresume = \"x {state}\"\n",
            ["programs.x.state"],
        ),
        (
            "[programs.x]\nresume = \"x {state}\"\n",
            ["programs.x.resume"],
        ),
        (
            "[programs.x]\nstate = \"id\"\nresume = \"x\"\n",
            ["programs.x.resume"],
        ),
        ("[programs.x]\nmatch = [\"x\"]\n", ["programs.x"]),
    ] {
        env.write_config(toml);
        let out = env.rexurrect(&server).arg("save").run().assert_failure();
        for name in names {
            assert!(
                out.stderr.contains(&format!("`{name}`"))
                    && out
                        .stderr
                        .contains(&env.config_path().display().to_string()),
                "{toml}: {out}"
            );
        }
    }
    assert_eq!(env.listed(&server), "no snapshots yet\n");
}
