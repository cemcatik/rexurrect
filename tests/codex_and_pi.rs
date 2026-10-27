//! Codex and pi, built in: a block running either comes back with its
//! agent's conversation typed, once the agent's hook has reported it.
//!
//! The agents are stand-ins ([`stand_in_bin`]). A stand-in Codex hook is
//! `rexurrect report codex` fed a recorded `SessionStart` payload; a
//! stand-in pi extension is `rexurrect report pi <session file>`, run with
//! the block's `REX_BLOCK`, as each would be.

mod common;

use common::*;
use serde_json::json;

const THREAD: &str = "019a3c2b-1d4e-7f60-9a0f-3c2b1d4e5f60";

/// A Codex `SessionStart` hook's payload, as Codex 0.165 sends it.
fn codex_session_start(session_id: &str) -> String {
    json!({
        "session_id": session_id,
        "transcript_path": null,
        "cwd": "/w/a",
        "hook_event_name": "SessionStart",
        "model": "gpt-5.5",
        "permission_mode": "default",
        "source": "startup",
    })
    .to_string()
}

fn report(env: &TestEnv, server: &Server, block_id: &str, args: &[&str], stdin: &str) {
    let out = env
        .rexurrect(server)
        .env("REX_BLOCK", block_id)
        .arg("report")
        .args(args)
        .stdin(stdin)
        .run();
    assert!(
        out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
        "{out}"
    );
}

#[test]
fn a_block_running_codex_that_reported_its_thread_comes_back_with_codex_resume_typed() {
    let env = TestEnv::new();
    stand_in_bin(&env, "codex");
    let server = env.start_server();
    let dir = env.mkdir("a");
    let (session, block) = running(&server, "zz-codex", &dir, "codex");

    report(
        &env,
        &server,
        &block,
        &["codex"],
        &codex_session_start(THREAD),
    );

    restored_with_typed(
        &env,
        &server,
        &session,
        "zz-codex",
        &format!("codex resume {THREAD}"),
    );
    assert_eq!(runs(&dir, "codex"), [""], "nothing ran at restore");
}

/// npm installs Codex as a script `node` runs: the block's foreground
/// program is `node`, and only its script's name is `codex`.
#[test]
fn a_program_an_interpreter_runs_is_matched_by_its_scripts_name() {
    let env = TestEnv::new();
    let bin = stand_in_bin(&env, "node");
    let script = bin.join("codex");
    std::fs::write(&script, format!("#!{}\n", bin.join("node").display())).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let server = env.start_server();
    let dir = env.mkdir("a");
    let fixture = server.create_session("zz-npm", &[window(shell(&dir))]);
    server.wait_for_pwds(&fixture);
    let block = fixture.block_ids()[0].to_string();
    server.run(&fixture.id, &block, "codex");
    assert_eq!(
        wait_for_runs(&dir, "node", 1),
        [script.display().to_string()]
    );

    report(
        &env,
        &server,
        &block,
        &["codex"],
        &codex_session_start(THREAD),
    );

    restored_with_typed(
        &env,
        &server,
        &fixture.id,
        "zz-npm",
        &format!("codex resume {THREAD}"),
    );
}

#[test]
fn a_block_running_pi_that_reported_its_session_file_comes_back_with_it_typed() {
    let env = TestEnv::new();
    stand_in_bin(&env, "pi");
    let server = env.start_server();
    let dir = env.mkdir("a");
    let (session, block) = running(&server, "zz-pi", &dir, "pi");
    let file = "/Users/u/.pi/agent/sessions/--w-a--/2026-10-27T16-05-00-000Z_s.jsonl";

    report(&env, &server, &block, &["pi", file], "");

    restored_with_typed(
        &env,
        &server,
        &session,
        "zz-pi",
        &format!("pi --session {file}"),
    );
}

#[test]
fn print_hook_prints_codexs_hook_and_feature_flag_and_changes_no_file() {
    let env = TestEnv::builder().own_home().build();
    let server = env.start_server();
    let before = files::contents(env.root());

    let out = env
        .rexurrect(&server)
        .args(["report", "codex", "--print-hook"])
        .run()
        .assert_success();

    let (hooks, config) = out
        .stdout
        .strip_prefix("# In ~/.codex/hooks.json:\n")
        .and_then(|rest| rest.split_once("# In ~/.codex/config.toml:\n"))
        .unwrap_or_else(|| panic!("{}", out.stdout));
    let hooks: serde_json::Value = serde_json::from_str(hooks).unwrap();
    let command = hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(
        command.ends_with(&format!("{} report codex", common::rexurrect::bin())),
        "{command}"
    );
    let config: toml::Table = config.parse().unwrap();
    assert_eq!(config["features"]["hooks"].as_bool(), Some(true));
    assert_eq!(files::contents(env.root()), before);
}

#[test]
fn print_hook_prints_pis_extension_and_changes_no_file() {
    let env = TestEnv::builder().own_home().build();
    let server = env.start_server();
    let before = files::contents(env.root());

    let out = env
        .rexurrect(&server)
        .args(["report", "pi", "--print-hook"])
        .run()
        .assert_success();

    for expected in [
        format!("const REXURRECT = {:?};", common::rexurrect::bin()),
        format!(
            "REXURRECT_STATE_DIR: {:?}",
            env.state_dir().display().to_string()
        ),
        r#"["report", "pi", file]"#.to_string(),
        r#"pi.on("session_start""#.to_string(),
    ] {
        assert!(out.stdout.contains(&expected), "{expected}: {}", out.stdout);
    }
    assert_eq!(files::contents(env.root()), before);
}
