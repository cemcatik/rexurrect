//! A program that exited after the last save doesn't resume (#60). The zsh
//! recorder rewrites the block's record each time the shell's prompt comes
//! back, so a record written since the save read it means the shell got
//! control back: its program had exited. The block then comes back
//! with its record typed, as if the save had seen its shell.
//!
//! Programs are stand-ins ([`stand_in_bin`]). Each block quits its program
//! with Ctrl-C, which ends a stand-in, between the save and the restart
//! ([`restarted_after`]).

mod common;

use common::*;

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

    let (restored, block) = restarted(&env, &server, &session, "zz-running");

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
    env.write_recorder_file(&block, "nvim init.lua");

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

/// A save in the startup window found no record for a restored block, so
/// a record written after it says nothing about the program: here the user
/// ran the typed `nvim -S`, which is still running at the restart.
#[test]
fn a_resume_command_run_after_a_save_in_the_startup_window_still_resumes() {
    let env = TestEnv::with_recorder();
    env.write_config("app_session_watch = 1\n");
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-ran", &dir, "nvim init.lua");
    let (restored, block) = restored_with_typed(&env, &server, &session, "zz-ran", "nvim -S");
    env.save(&server);
    server.write(&restored, &block, "\n");
    wait_for_runs(&dir, "nvim", 2);

    env.write_config("app_session_watch = 1\nauto_resume = [\"nvim\"]\n");
    server.kill_session(&restored);
    env.restore(&server);

    assert_eq!(wait_for_runs(&dir, "nvim", 3), ["init.lua", "-S", "-S"]);
}

/// Restore can't read the record it would type: the block gets the last
/// command the save found, and the restore carries on.
#[test]
fn a_record_restore_cannot_read_leaves_the_saved_last_command() {
    use std::os::unix::fs::PermissionsExt;
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, block) = running(&server, "zz-unreadable", &dir, "nvim init.lua");
    env.write_recorder_file(&block, "nvim init.lua");

    let (restored, block) = restarted_after(&env, &server, &session, "zz-unreadable", || {
        env.write_recorder_file(&block, "true zz-after");
        let file = env.state_dir().join("recorder").join(&block);
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o000)).unwrap();
    });

    server.wait_for_typed(&restored, &block, "nvim init.lua");
}
