//! Auto-resume: programs listed in `auto_resume` in the config file come
//! back running. Restore types their resume command followed by Enter, so
//! the block's shell runs it; a listed program that can't resume runs its
//! own command line from the save instead. Nothing runs by default
//! (ADR-0006).
//!
//! Programs are stand-ins ([`stand_in_bin`]), first on every block's `PATH`.

mod common;

use common::*;

#[test]
fn a_listed_neovim_where_a_session_vim_is_comes_back_running_nvim_dash_s_in_its_shell() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\nauto_resume = [\"nvim\"]\n");
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-auto", &dir, "nvim init.lua");

    let (restored, block) = restarted(&env, &server, &session, "zz-auto");

    assert_eq!(wait_for_runs(&dir, "nvim", 2), ["init.lua", "-S"]);
    server.wait_for_foreground(&restored, &block, "nvim");
    server.write(&restored, &block, "\u{3}");
    server.wait_for_foreground(&restored, &block, "zsh");
    server.wait_for_empty_prompt(&restored, &block);
    server.wait_for_pwd_eq(&restored, &block, &dir);
}

/// The block's shell runs it, so its recorder records it: it's the block's
/// last command at the next save, like anything the user ran there.
#[test]
fn an_auto_resumed_command_is_the_blocks_last_command_at_the_next_save() {
    let env = TestEnv::with_recorder();
    env.write_config("app_session_watch = 1\nauto_resume = [\"nvim\"]\n");
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-recorded", &dir, "nvim init.lua");
    let (restored, block) = restarted(&env, &server, &session, "zz-recorded");
    server.wait_for_foreground(&restored, &block, "nvim");
    server.write(&restored, &block, "\u{3}");
    server.wait_for_empty_prompt(&restored, &block);
    let recorded = std::fs::read_to_string(env.state_dir().join("recorder").join(&block));
    assert_eq!(
        recorded.unwrap(),
        "nvim -S\n",
        "the shell's recorder recorded it"
    );
    env.end_startup_windows();

    env.write_config("app_session_watch = 1\n");
    let (restored, block) = restarted(&env, &server, &restored, "zz-recorded");
    server.wait_for_typed(&restored, &block, "nvim -S");
}

/// It still comes back running, without its Neovim session: `nvim` as it
/// was started, never the block's last command, which only started it.
#[test]
fn a_listed_neovim_with_no_session_vim_comes_back_running_its_command_line_not_its_last_command() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\nauto_resume = [\"nvim\"]\n");
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    let (session, block) = running(&server, "zz-make", &dir, "nvim src/main.rs");
    // A recorder in the block's shell, played by hand: `make edit` started
    // the Neovim in its foreground.
    env.write_recorder_file(&block, "make edit");

    let (restored, block) = restarted(&env, &server, &session, "zz-make");

    assert_eq!(
        wait_for_runs(&dir, "nvim", 2),
        ["src/main.rs", "src/main.rs"]
    );
    server.wait_for_foreground(&restored, &block, "nvim");
}

#[test]
fn a_listed_claude_that_reported_nothing_comes_back_running_as_it_was_started() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\nauto_resume = [\"claude\"]\n");
    stand_in_bin(&env, "claude");
    let server = env.start_server();
    let dir = env.mkdir("a");
    let (session, _) = running(&server, "zz-claude", &dir, "claude --model opus");

    let (restored, block) = restarted(&env, &server, &session, "zz-claude");

    assert_eq!(
        wait_for_runs(&dir, "claude", 2),
        ["--model opus", "--model opus"]
    );
    server.wait_for_foreground(&restored, &block, "claude");
}

#[test]
fn with_an_empty_auto_resume_nothing_runs() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\nauto_resume = []\n");
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-empty", &dir, "nvim init.lua");

    let (restored, block) = restarted(&env, &server, &session, "zz-empty");

    server.wait_for_typed(&restored, &block, "nvim -S");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(runs(&dir, "nvim"), ["init.lua"], "nothing ran at restore");
}

#[test]
fn a_program_not_listed_is_typed_not_run() {
    let env = TestEnv::new();
    env.write_config("app_session_watch = 1\nauto_resume = [\"claude\"]\n");
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let dir = env.mkdir("a");
    write_session_vim(&dir);
    let (session, _) = running(&server, "zz-typed", &dir, "nvim init.lua");

    let (restored, block) = restarted(&env, &server, &session, "zz-typed");

    server.wait_for_typed(&restored, &block, "nvim -S");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(runs(&dir, "nvim"), ["init.lua"], "nothing ran at restore");
}

#[test]
fn an_unknown_name_in_auto_resume_fails_restore_naming_it() {
    let env = TestEnv::new();
    env.saved_on_an_earlier_server(&["zz-proj"]);
    let server = env.start_server();
    env.write_config("auto_resume = [\"emacs\"]\n");

    let out = env.rexurrect(&server).arg("restore").run().assert_failure();

    assert!(
        out.stderr.contains("`auto_resume`") && out.stderr.contains("\"emacs\""),
        "{out}"
    );
    assert_eq!(server.sessions(), []);
}
