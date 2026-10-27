//! A program that exited after the last save doesn't resume (#60). The zsh
//! recorder rewrites the block's record each time the shell's prompt comes
//! back, so a record changed after the snapshot was taken means the shell
//! got control back: its program had exited. The block then comes back
//! with its record typed, as if the save had seen its shell.
//!
//! Programs are stand-ins ([`stand_in_bin`]). Each block quits its program
//! with Ctrl-C, which ends a stand-in.

mod common;

use common::*;

/// Saves, then runs `after_the_save` (quitting the program, say), then kills
/// the session labelled `label` and restores it, and returns the restored
/// session and its first block. Nothing saves in between, as when the
/// server stops before the next autosave.
fn restarted_after(
    env: &TestEnv,
    server: &Server,
    session: &str,
    label: &str,
    after_the_save: impl FnOnce(),
) -> (String, String) {
    env.save(server);
    after_the_save();
    server.kill_session(session);
    env.restore(server);
    let restored = server.the_session_labelled(label);
    let block = server.block_ids(&restored).remove(0);
    (restored, block)
}

/// Quits the stand-in in the block, and waits for the shell's prompt.
fn quit(server: &Server, session: &str, block: &str) {
    server.write(session, block, "\u{3}");
    server.wait_for_foreground(session, block, "zsh");
    server.wait_for_empty_prompt(session, block);
}

#[test]
fn a_neovim_quit_after_the_save_comes_back_with_its_last_command_not_resumed() {
    let env = TestEnv::with_recorder();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, block) = running(&server, "zz-quit", &dir, "nvim init.lua");

    let (restored, block) = restarted_after(&env, &server, &session, "zz-quit", || {
        quit(&server, &session, &block)
    });

    server.wait_for_typed(&restored, &block, "nvim init.lua");
}

/// The record as the server stopped, not as the save found it: here the
/// user ran something else after quitting.
#[test]
fn a_block_comes_back_with_the_command_run_after_quitting_the_program() {
    let env = TestEnv::with_recorder();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, block) = running(&server, "zz-after", &dir, "nvim init.lua");

    let (restored, block) = restarted_after(&env, &server, &session, "zz-after", || {
        quit(&server, &session, &block);
        server.run(&session, &block, "true zz-after");
        server.wait_for_empty_prompt(&session, &block);
    });

    server.wait_for_typed(&restored, &block, "true zz-after");
}

/// Auto-resume only runs programs that resume: one that exited comes back
/// typed, never run, as if the save had seen the block's shell.
#[test]
fn a_listed_neovim_quit_after_the_save_comes_back_typed_not_running() {
    let env = TestEnv::with_recorder();
    env.write_config("app_session_watch = 1\nauto_resume = [\"nvim\"]\n");
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, block) = running(&server, "zz-listed", &dir, "nvim init.lua");

    let (restored, block) = restarted_after(&env, &server, &session, "zz-listed", || {
        quit(&server, &session, &block)
    });

    server.wait_for_typed(&restored, &block, "nvim init.lua");
    assert_eq!(runs(&dir, "nvim"), ["init.lua"], "nothing ran at restore");
}

/// The record the recorder wrote as the program started is older than the
/// save: it says nothing about the program exiting.
#[test]
fn a_neovim_still_running_at_the_restart_resumes() {
    let env = TestEnv::with_recorder();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-running", &dir, "nvim init.lua");

    let (restored, block) = restarted_after(&env, &server, &session, "zz-running", || {});

    server.wait_for_typed(&restored, &block, "nvim -S");
}

/// A recorder for another shell, played by hand, records after the save
/// that the block has no last command.
#[test]
fn a_record_of_no_last_command_written_after_the_save_leaves_the_prompt_empty() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, block) = running(&server, "zz-none", &dir, "nvim init.lua");

    let (restored, block) = restarted_after(&env, &server, &session, "zz-none", || {
        env.write_recorder_file(&block, "")
    });

    server.assert_empty_prompt(&restored, &block);
}

/// The shell got control back, but its record can't be used: the block
/// gets the last command the save found, still not resuming.
#[test]
fn a_record_that_is_not_text_written_after_the_save_leaves_the_saved_last_command() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, block) = running(&server, "zz-bytes", &dir, "nvim init.lua");
    env.write_recorder_file(&block, "nvim init.lua");

    let (restored, block) = restarted_after(&env, &server, &session, "zz-bytes", || {
        let file = env.state_dir().join("recorder").join(&block);
        std::fs::write(file, b"nvim \xff\n").unwrap();
    });

    server.wait_for_typed(&restored, &block, "nvim init.lua");
}

/// A save in the startup window keeps a restored block as restore created
/// it: not resumable, once restore found its program had exited.
#[test]
fn a_save_in_the_startup_window_keeps_a_block_whose_program_exited_from_resuming() {
    let env = TestEnv::with_recorder();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, block) = running(&server, "zz-window", &dir, "nvim init.lua");
    let (restored, block) = restarted_after(&env, &server, &session, "zz-window", || {
        quit(&server, &session, &block)
    });
    server.wait_for_typed(&restored, &block, "nvim init.lua");

    restored_with_typed(&env, &server, &restored, "zz-window", "nvim init.lua");
}

/// The recorder rewrites only a record that exists: a restored block's
/// shell reaching its first prompt records nothing, so a save in the
/// startup window still keeps the block as restore created it.
#[test]
fn a_restored_blocks_first_prompt_records_nothing_so_the_startup_window_keeps_its_resume() {
    let env = TestEnv::with_recorder();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-first", &dir, "nvim init.lua");
    let (restored, block) = restored_with_typed(&env, &server, &session, "zz-first", "nvim -S");
    assert!(
        !env.state_dir().join("recorder").join(&block).exists(),
        "the restored block's prompt created a record"
    );

    restored_with_typed(&env, &server, &restored, "zz-first", "nvim -S");
}
