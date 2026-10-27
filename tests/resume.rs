//! Resuming: a block whose foreground program at the save matches a program
//! definition comes back with the program's resume command typed at its
//! prompt, in place of its last command, never run.
//!
//! Programs are stand-ins ([`stand_in_bin`]): a test-built `nvim` or
//! `claude` that records how it was run and waits. A stand-in Claude Code
//! hook is `rexurrect report claude` run with the block's `REX_BLOCK`, fed
//! a recorded `SessionStart` payload. None of these blocks load the zsh
//! recorder; their last commands come from the foreground fallback or from
//! recorder files written by hand.

mod common;

use common::*;
use serde_json::json;

#[test]
fn a_block_running_neovim_where_a_session_vim_is_comes_back_with_nvim_dash_s_typed_not_run() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-nvim", &dir, "nvim init.lua");

    let (restored, block) = restored_with_typed(&env, &server, &session, "zz-nvim", "nvim -S");

    assert_eq!(server.foreground(&restored, &block).as_deref(), Some("zsh"));
    assert_eq!(runs(&dir, "nvim"), ["init.lua"], "nothing ran at restore");
}

#[test]
fn a_block_running_neovim_where_there_is_no_session_vim_comes_back_with_its_last_command() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    let (session, _) = running(&server, "zz-nvim", &dir, "nvim init.lua");

    restored_with_typed(&env, &server, &session, "zz-nvim", "nvim init.lua");
}

/// Only the program in the foreground at the save resumes: one the user had
/// quit doesn't, even when it's still the block's last command.
#[test]
fn a_block_whose_neovim_had_exited_before_the_save_comes_back_with_its_last_command() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, block) = running(&server, "zz-quit", &dir, "nvim init.lua");
    server.write(&session, &block, "\u{3}");
    server.wait_for_foreground(&session, &block, "zsh");
    // A recorder in the block's shell, played by hand.
    env.write_recorder_file(&block, "nvim init.lua");

    restored_with_typed(&env, &server, &session, "zz-quit", "nvim init.lua");
}

/// Matched by any name it goes by, as no-scrollback programs are: here it's
/// run through a `nvim` symlink to an executable named for its version.
#[test]
fn neovim_is_matched_by_the_name_it_was_run_by_as_well_as_its_executables() {
    let env = TestEnv::new();
    let versions = env.mkdir("versions");
    build_stand_in(&env, &versions.join("0.12.6"));
    let bin = env.root().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(versions.join("0.12.6"), bin.join("nvim")).unwrap();
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let fixture = server.create_session("zz-symlink", &[window(shell(&dir))]);
    server.wait_for_pwds(&fixture);
    server.run(&fixture.id, fixture.block_ids()[0], "nvim");
    server.wait_for_foreground(&fixture.id, fixture.block_ids()[0], "0.12.6");

    restored_with_typed(&env, &server, &fixture.id, "zz-symlink", "nvim -S");
}

/// A save in the startup window saves a restored block as restore created
/// it: resumable, with the last command it had before, not the resume
/// command restore typed.
#[test]
fn a_save_in_the_startup_window_keeps_the_resume_of_a_block_restore_created() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-window", &dir, "nvim init.lua");
    let (restored, _) = restored_with_typed(&env, &server, &session, "zz-window", "nvim -S");

    restored_with_typed(&env, &server, &restored, "zz-window", "nvim -S");
    std::fs::remove_file(dir.join("Session.vim")).unwrap();
    let restored = server.the_session_labelled("zz-window");
    restored_with_typed(&env, &server, &restored, "zz-window", "nvim init.lua");
}

/// Once the window has closed, a typed `nvim -S` nobody ran is like any
/// command restore typed: the block's last command, which carries forward.
/// Its shell is in its foreground, so it no longer resumes.
#[test]
fn a_typed_nvim_dash_s_nobody_ran_carries_forward_as_the_last_command() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-carried", &dir, "nvim init.lua");
    let (restored, _) = restored_with_typed(&env, &server, &session, "zz-carried", "nvim -S");
    env.end_startup_windows();

    std::fs::remove_file(dir.join("Session.vim")).unwrap();
    let (restored, _) = restored_with_typed(&env, &server, &restored, "zz-carried", "nvim -S");
    env.end_startup_windows();
    restored_with_typed(&env, &server, &restored, "zz-carried", "nvim -S");
}

/// A Claude Code `SessionStart` hook's payload, as Claude Code 2.1 sends it,
/// for the conversation `session_id`.
fn session_start(session_id: &str, source: &str) -> String {
    json!({
        "session_id": session_id,
        "transcript_path": format!("/Users/u/.claude/projects/-w-a/{session_id}.jsonl"),
        "cwd": "/w/a",
        "hook_event_name": "SessionStart",
        "source": source,
    })
    .to_string()
}

/// Runs `rexurrect report` with `args` in the block `block_id` (its
/// `REX_BLOCK`), fed `stdin`, as a hook in it would.
fn report(env: &TestEnv, server: &Server, block_id: &str, args: &[&str], stdin: &str) -> RunOutput {
    env.rexurrect(server)
        .env("REX_BLOCK", block_id)
        .arg("report")
        .args(args)
        .stdin(stdin)
        .run()
}

const CONVERSATION: &str = "0b9e1f2a-61c4-4d1e-9a0f-3c2b1d4e5f60";

#[test]
fn a_block_running_claude_that_reported_its_conversation_comes_back_with_it_resumable() {
    let env = TestEnv::new();
    stand_in_bin(&env, "claude");
    let server = env.start_server();
    let dir = env.mkdir("a");
    let (session, block) = running(&server, "zz-claude", &dir, "claude");

    let out = report(
        &env,
        &server,
        &block,
        &["claude"],
        &session_start(CONVERSATION, "startup"),
    );
    assert!(
        out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
        "{out}"
    );

    let (restored, block) = restored_with_typed(
        &env,
        &server,
        &session,
        "zz-claude",
        &format!("claude --resume {CONVERSATION}"),
    );
    assert_eq!(server.foreground(&restored, &block).as_deref(), Some("zsh"));
    assert_eq!(runs(&dir, "claude"), [""], "nothing ran at restore");
}

/// `SessionStart` fires again on `/clear`, `/resume` and compaction: the
/// block resumes the conversation on screen at the save.
#[test]
fn a_later_report_for_the_same_block_replaces_the_earlier_one() {
    let env = TestEnv::new();
    stand_in_bin(&env, "claude");
    let server = env.start_server();
    let (session, block) = running(&server, "zz-clear", &env.mkdir("a"), "claude");
    report(
        &env,
        &server,
        &block,
        &["claude"],
        &session_start("first", "startup"),
    );
    report(
        &env,
        &server,
        &block,
        &["claude"],
        &session_start("second", "clear"),
    );

    restored_with_typed(
        &env,
        &server,
        &session,
        "zz-clear",
        "claude --resume second",
    );
}

/// A subagent's hooks carry an `agent_id`: they never change which
/// conversation the block resumes.
#[test]
fn a_subagents_report_changes_nothing() {
    let env = TestEnv::new();
    stand_in_bin(&env, "claude");
    let server = env.start_server();
    let (session, block) = running(&server, "zz-agent", &env.mkdir("a"), "claude");
    report(
        &env,
        &server,
        &block,
        &["claude"],
        &session_start(CONVERSATION, "startup"),
    );
    let mut subagent: serde_json::Value =
        serde_json::from_str(&session_start("subagent", "startup")).unwrap();
    subagent["agent_id"] = "agent-1".into();
    report(&env, &server, &block, &["claude"], &subagent.to_string());

    restored_with_typed(
        &env,
        &server,
        &session,
        "zz-agent",
        &format!("claude --resume {CONVERSATION}"),
    );
}

/// Whatever goes wrong, a hook's output reaches its agent: `report` stores
/// nothing, prints nothing and exits 0. It logs why instead, once per cause.
#[test]
fn a_report_rexurrect_cannot_use_stores_nothing_prints_nothing_and_exits_0() {
    let env = TestEnv::new();
    stand_in_bin(&env, "claude");
    let server = env.start_server();
    let (session, block) = running(&server, "zz-bad", &env.mkdir("a"), "claude");
    let payload = session_start(CONVERSATION, "startup");
    let reports = [
        (Some(block.as_str()), vec!["claude"], "not json".to_string()),
        (Some(block.as_str()), vec!["claude"], "not json".to_string()),
        (
            Some(block.as_str()),
            vec!["claude"],
            session_start("a b", "startup"),
        ),
        (
            Some(block.as_str()),
            vec!["claude", "--dangerously-skip-permissions"],
            String::new(),
        ),
        (
            Some(block.as_str()),
            vec!["claude", "$(touch zz)"],
            String::new(),
        ),
        (Some(block.as_str()), vec!["nvim", "x"], String::new()),
        (Some(block.as_str()), vec!["zz-unknown", "x"], String::new()),
        (Some(block.as_str()), vec![], String::new()),
        (Some("../escaped"), vec!["claude"], payload.clone()),
        (Some("block:a/b"), vec!["claude"], payload.clone()),
        (None, vec!["claude"], payload.clone()),
    ];
    for (rex_block, args, stdin) in reports {
        let mut cmd = env
            .rexurrect(&server)
            .arg("report")
            .args(&args)
            .stdin(stdin);
        if let Some(rex_block) = rex_block {
            cmd = cmd.env("REX_BLOCK", rex_block);
        }
        let out = cmd.run();
        assert!(
            out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
            "{rex_block:?} {args:?}: {out}"
        );
    }

    assert!(!files::has_entries(&env.state_dir().join("resume")));
    let logged = env.log_entries();
    assert_eq!(
        logged
            .iter()
            .filter(|e| e.contains("isn't a Claude Code hook's JSON"))
            .count(),
        1,
        "once per cause: {logged:#?}"
    );
    restored_with_typed(&env, &server, &session, "zz-bad", "claude");
}

#[test]
fn a_block_running_claude_that_reported_nothing_comes_back_with_its_last_command() {
    let env = TestEnv::new();
    stand_in_bin(&env, "claude");
    let server = env.start_server();
    let (session, _) = running(&server, "zz-silent", &env.mkdir("a"), "claude");

    restored_with_typed(&env, &server, &session, "zz-silent", "claude");
}

#[test]
fn a_block_whose_claude_had_exited_before_the_save_comes_back_with_its_last_command() {
    let env = TestEnv::new();
    stand_in_bin(&env, "claude");
    let server = env.start_server();
    let (session, block) = running(&server, "zz-exited", &env.mkdir("a"), "claude");
    report(
        &env,
        &server,
        &block,
        &["claude"],
        &session_start(CONVERSATION, "startup"),
    );
    server.write(&session, &block, "\u{3}");
    server.wait_for_foreground(&session, &block, "zsh");
    // A recorder in the block's shell, played by hand.
    env.write_recorder_file(&block, "claude");

    restored_with_typed(&env, &server, &session, "zz-exited", "claude");
}

/// Two blocks resuming one conversation would fight over it.
#[test]
fn of_two_blocks_with_one_conversation_only_the_first_in_layout_order_resumes_it() {
    let env = TestEnv::new();
    stand_in_bin(&env, "claude");
    let server = env.start_server();
    let [a, b] = ["a", "b"].map(|name| env.mkdir(name));
    let fixture = server.create_session("zz-twice", &[window(hsplit(0.5, shell(&a), shell(&b)))]);
    server.wait_for_pwds(&fixture);
    for (block, dir) in fixture.block_ids().into_iter().zip([&a, &b]) {
        server.run(&fixture.id, block, "claude");
        common::server::wait_for("claude to start", || {
            (!runs(dir, "claude").is_empty()).then_some(())
        });
        report(
            &env,
            &server,
            block,
            &["claude"],
            &session_start(CONVERSATION, "startup"),
        );
    }

    env.save(&server);
    server.kill_session(&fixture.id);
    env.restore(&server);

    let restored = server.the_session_labelled("zz-twice");
    let blocks = server.block_ids(&restored);
    server.wait_for_typed(
        &restored,
        &blocks[0],
        &format!("claude --resume {CONVERSATION}"),
    );
    server.wait_for_typed(&restored, &blocks[1], "claude");
}

#[test]
fn the_resume_state_of_blocks_that_are_gone_is_removed_by_the_next_save() {
    let env = TestEnv::new();
    stand_in_bin(&env, "claude");
    let server = env.start_server();
    let (gone, block) = running(&server, "zz-gone", &env.mkdir("a"), "claude");
    report(
        &env,
        &server,
        &block,
        &["claude"],
        &session_start(CONVERSATION, "startup"),
    );
    let kept = server.create_session("zz-kept", &[window(shell(env.mkdir("b")))]);
    server.wait_for_pwds(&kept);
    assert!(files::has_entries(&env.state_dir().join("resume")));

    server.kill_session(&gone);
    env.save(&server);

    assert!(!files::has_entries(&env.state_dir().join("resume")));
}

#[test]
fn print_hook_prints_claude_codes_session_start_hook_with_rexurrects_full_path_and_changes_no_file()
{
    let env = TestEnv::builder().own_home().build();
    let server = env.start_server();
    let before = files::contents(env.root());

    let out = env
        .rexurrect(&server)
        .args(["report", "claude", "--print-hook"])
        .run()
        .assert_success();

    let settings: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    let hooks = &settings["hooks"]["SessionStart"][0]["hooks"];
    assert_eq!(hooks[0]["type"], "command");
    let command = hooks[0]["command"].as_str().unwrap();
    assert!(
        command.ends_with(&format!("{} report claude", common::rexurrect::bin())),
        "{command}"
    );
    assert!(
        command.contains(env.state_dir().to_str().unwrap()),
        "{command}"
    );
    assert_eq!(files::contents(env.root()), before);
}

/// Not even `--help`, or an argument that isn't UTF-8: a hook passing an
/// odd value never puts rexurrect's usage into its agent.
#[test]
fn report_prints_nothing_whatever_its_arguments() {
    use std::os::unix::ffi::OsStrExt;
    let env = TestEnv::new();
    let server = env.start_server();
    let not_utf8 = std::ffi::OsStr::from_bytes(b"ab\xffcd");
    for args in [
        vec![
            std::ffi::OsStr::new("claude"),
            std::ffi::OsStr::new("--help"),
        ],
        vec![std::ffi::OsStr::new("-h")],
        vec![std::ffi::OsStr::new("claude"), not_utf8],
    ] {
        let out = env
            .rexurrect(&server)
            .env("REX_BLOCK", "block:0b9e1f2a")
            .arg("report")
            .args(&args)
            .run();
        assert!(
            out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
            "{args:?}: {out}"
        );
    }
    assert!(!files::has_entries(&env.state_dir().join("resume")));
}
