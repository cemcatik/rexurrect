//! Resuming: a block whose foreground program at the save matches a program
//! definition comes back with the program's resume command typed at its
//! prompt, in place of its last command, never run.
//!
//! Programs are stand-ins ([`stand_in_bin`]): a test-built `nvim` that
//! records how it was run and waits. None of these blocks load the zsh
//! recorder; their last commands come from the foreground fallback or from
//! recorder files written by hand.

mod common;

use std::path::Path;

use common::*;

/// vim-obsession's Neovim session, in `dir`.
fn write_session_vim(dir: &Path) {
    std::fs::write(dir.join("Session.vim"), "\" a Neovim session\n").unwrap();
}

/// A session labelled `label` with one shell block in `dir`, running
/// `command` (a stand-in, by name) when this returns. Returns the session
/// and its block.
fn running(server: &Server, label: &str, dir: &Path, command: &str) -> (String, String) {
    let fixture = server.create_session(label, &[window(shell(dir))]);
    server.wait_for_pwds(&fixture);
    let block = fixture.block_ids()[0].to_string();
    let program = command.split_whitespace().next().unwrap();
    let ran = runs(dir, program).len();
    server.run(&fixture.id, &block, command);
    common::server::wait_for(&format!("{command} to start"), || {
        (runs(dir, program).len() > ran).then_some(())
    });
    (fixture.id, block)
}

/// Saves, kills the session `session_id`, restores it, and returns the
/// restored session and its one block, once `typed` sits at its prompt.
fn restored_with_typed(
    env: &TestEnv,
    server: &Server,
    session_id: &str,
    label: &str,
    typed: &str,
) -> (String, String) {
    env.save(server);
    server.kill_session(session_id);
    env.restore(server);
    let restored = server.the_session_labelled(label);
    let block = server.block_ids(&restored).remove(0);
    server.wait_for_typed(&restored, &block, typed);
    (restored, block)
}

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
