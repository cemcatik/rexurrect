//! A window's floating layers come back over its tiled layout, back to front,
//! each where it was and holding the layout it held. Their blocks come back
//! like tiled ones: in their directories, with their scrollback, and with
//! their programs resumed.

mod common;

use common::server::above_prompt;
use common::*;

/// The IDs of the blocks in each floating layer of the session's window
/// `window`, back to front, in layout order.
fn floating_block_ids(server: &Server, session_id: &str, window: usize) -> Vec<Vec<String>> {
    floating_layouts(&server.view(session_id))[window]
        .iter()
        .map(|layer| layer.layout.leaves())
        .collect()
}

#[test]
fn a_floating_split_comes_back_with_its_direction_and_ratio() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, top, bottom] = ["a", "top", "bottom"].map(|name| env.mkdir(name));
    let fixture = server.create_session("zz-float-split", &[window(shell(&a))]);
    server.wait_for_pwds(&fixture);
    let split = vsplit(0.3, shell(&top), shell(&bottom));
    server.new_layer(
        &fixture.id,
        &fixture.windows[0].id,
        [0.2, 0.25, 0.6, 0.5],
        &split,
    );

    env.save(&server);
    server.kill_session(&fixture.id);
    env.restore(&server);

    let restored = server.the_session_labelled("zz-float-split");
    assert_eq!(
        server.window_shapes(&restored),
        [shell(&a).expected_shape()]
    );
    let floating = server.floating_shapes(&restored);
    let [layer] = floating[0].as_slice() else {
        panic!("one floating layer: {floating:?}");
    };
    layer.assert_is([0.2, 0.25, 0.6, 0.5], &split.expected_shape());
}

#[test]
fn a_windows_floating_layers_come_back_in_their_back_to_front_order() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, back, front, other] = ["a", "back", "front", "other"].map(|name| env.mkdir(name));
    let fixture = server.create_session("zz-float-order", &[window(shell(&a)), window(shell(&a))]);
    server.wait_for_pwds(&fixture);
    let [first, second] = [0, 1].map(|i| fixture.windows[i].id.as_str());
    server.new_layer(&fixture.id, second, [0.0, 0.0, 0.5, 0.5], &shell(&other));
    // Overlapping, so the order is what decides which one shows.
    server.new_layer(&fixture.id, first, [0.1, 0.1, 0.5, 0.5], &shell(&back));
    server.new_layer(&fixture.id, first, [0.3, 0.3, 0.5, 0.5], &shell(&front));

    env.save(&server);
    server.kill_session(&fixture.id);
    env.restore(&server);

    let restored = server.the_session_labelled("zz-float-order");
    let floating = server.floating_shapes(&restored);
    let [behind, in_front] = floating[0].as_slice() else {
        panic!("two floating layers in the first window: {floating:?}");
    };
    behind.assert_is([0.1, 0.1, 0.5, 0.5], &shell(&back).expected_shape());
    in_front.assert_is([0.3, 0.3, 0.5, 0.5], &shell(&front).expected_shape());
    let [only] = floating[1].as_slice() else {
        panic!("one floating layer in the second window: {floating:?}");
    };
    only.assert_is([0.0, 0.0, 0.5, 0.5], &shell(&other).expected_shape());
}

/// Restoring a floating layer neither switches windows nor moves the focus
/// out of the tiled layout: the session opens on its first window (#80).
#[test]
fn restoring_floating_layers_moves_no_focus() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, floated] = ["a", "floated"].map(|name| env.mkdir(name));
    let fixture = server.create_session("zz-float-focus", &[window(shell(&a)), window(shell(&a))]);
    server.wait_for_pwds(&fixture);
    for window in &fixture.windows {
        server.new_layer(
            &fixture.id,
            &window.id,
            [0.5, 0.5, 0.5, 0.5],
            &shell(&floated),
        );
    }

    env.save(&server);
    server.kill_session(&fixture.id);
    env.restore(&server);

    let restored = server.the_session_labelled("zz-float-focus");
    let view = server.view(&restored);
    let windows = labels(&view);
    assert_eq!(server.active_window(&restored), windows[0].window_id);
    let tiled = server.block_ids(&restored);
    for window in view["windows"].as_array().unwrap() {
        let focused = window["focused_block_id"].as_str().unwrap();
        assert!(
            tiled.iter().any(|block| block == focused),
            "the focused block {focused} is a tiled one: {view:#}"
        );
    }
    assert_eq!(
        server
            .floating_shapes(&restored)
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        [1, 1]
    );
}

#[test]
fn a_floating_blocks_scrollback_comes_back_with_its_last_command_typed() {
    let env = TestEnv::with_recorder();
    let server = env.start_server();
    let [a, floated] = ["a", "floated"].map(|name| env.mkdir(name));
    let fixture = server.create_session("zz-float-scrollback", &[window(shell(&a))]);
    server.wait_for_pwds(&fixture);
    let [block] = server
        .new_layer(
            &fixture.id,
            &fixture.windows[0].id,
            [0.1, 0.1, 0.8, 0.8],
            &shell(&floated),
        )
        .try_into()
        .unwrap();
    server.wait_for_pwd(&fixture.id, &block);
    server.run(&fixture.id, &block, "echo zz-floating-earlier");
    server.wait_for_empty_prompt(&fixture.id, &block);
    server.run(&fixture.id, &block, "echo zz-floating-last");
    server.wait_for_empty_prompt(&fixture.id, &block);

    env.save(&server);
    server.kill_session(&fixture.id);
    env.restore(&server);

    let restored = server.the_session_labelled("zz-float-scrollback");
    let [layer] = floating_block_ids(&server, &restored, 0)
        .try_into()
        .unwrap();
    let [block] = layer.try_into().unwrap();
    server.wait_for_typed(&restored, &block, "echo zz-floating-last");
    let capture = server.capture(&restored, &block);
    assert!(
        above_prompt(&capture).contains(&"zz-floating-earlier"),
        "earlier output is above the prompt:\n{capture}"
    );
}

#[test]
fn a_floating_block_running_neovim_where_a_session_vim_is_comes_back_with_nvim_dash_s_typed() {
    let env = TestEnv::new();
    stand_in_bin(&env, "nvim");
    let server = env.start_server();
    let [a, edited] = ["a", "edited"].map(|name| env.mkdir(name));
    write_session_vim(&edited);
    let fixture = server.create_session("zz-float-nvim", &[window(shell(&a))]);
    server.wait_for_pwds(&fixture);
    let [block] = server
        .new_layer(
            &fixture.id,
            &fixture.windows[0].id,
            [0.1, 0.1, 0.8, 0.8],
            &shell(&edited),
        )
        .try_into()
        .unwrap();
    server.wait_for_pwd(&fixture.id, &block);
    run_and_wait(&server, &fixture.id, &block, &edited, "nvim init.lua");

    env.save(&server);
    server.kill_session(&fixture.id);
    env.restore(&server);

    let restored = server.the_session_labelled("zz-float-nvim");
    let [layer] = floating_block_ids(&server, &restored, 0)
        .try_into()
        .unwrap();
    let [block] = layer.try_into().unwrap();
    server.wait_for_typed(&restored, &block, "nvim -S");
    assert_eq!(
        runs(&edited, "nvim"),
        ["init.lua"],
        "nothing ran at restore"
    );
}

/// The session exists once `session.create` answers, so a floating layer
/// that couldn't be created is a note on its `created` line, not a failure,
/// and the session's other layers still come back. Their blocks are still
/// told apart: a save in the startup window saves the layer in front as
/// restore created it, not as the missing one behind it.
#[test]
fn a_floating_layer_that_cant_be_created_leaves_the_session_created_with_a_note() {
    let env = TestEnv::new();
    let server = env.start_server();
    let [a, doomed, kept] = ["a", "doomed", "kept"].map(|name| env.mkdir(name));
    let fixture = server.create_session("zz-float-fails", &[window(shell(&a))]);
    server.wait_for_pwds(&fixture);
    let window = &fixture.windows[0].id;
    for (dir, bounds) in [
        (&doomed, [0.1, 0.1, 0.4, 0.4]),
        (&kept, [0.5, 0.5, 0.4, 0.4]),
    ] {
        let [block] = server
            .new_layer(&fixture.id, window, bounds, &shell(dir))
            .try_into()
            .unwrap();
        server.wait_for_pwd(&fixture.id, &block);
        let name = dir.file_name().unwrap().to_str().unwrap();
        env.write_recorder_file(&block, &format!("echo zz-{name}"));
    }
    env.save(&server);
    server.kill_session(&fixture.id);
    let stub = RexStub::new(&env).fail_when(
        "session.new_layer",
        doomed.to_str().unwrap(),
        1,
        "Error: boom",
    );

    let out = env
        .rexurrect(&server)
        .arg("restore")
        .path_prepend(stub.dir())
        .run()
        .assert_success();

    let restored = server.the_session_labelled("zz-float-fails");
    let created = out.lines_starting("created");
    assert_eq!(created.len(), 1, "{out}");
    assert!(created[0].contains(&restored), "{out}");
    assert!(
        created[0].ends_with(
            "(couldn't restore floating layer 1 of window 1: \
             rex api call session.new_layer failed (exit 1): boom)"
        ),
        "{out}"
    );
    assert!(
        out.stdout
            .contains("1 created, 0 replaced, 0 skipped, 0 failed"),
        "{out}"
    );
    let assert_only_the_kept_layer = |restored: &str| {
        let floating = server.floating_shapes(restored);
        let [layer] = floating[0].as_slice() else {
            panic!("only the other floating layer: {floating:?}");
        };
        layer.assert_is([0.5, 0.5, 0.4, 0.4], &shell(&kept).expected_shape());
        let [layer] = floating_block_ids(&server, restored, 0).try_into().unwrap();
        let [block] = layer.try_into().unwrap();
        server.wait_for_typed(restored, &block, "echo zz-kept");
    };
    assert_only_the_kept_layer(&restored);

    env.save(&server);
    server.kill_session(&restored);
    env.restore(&server);
    assert_only_the_kept_layer(&server.the_session_labelled("zz-float-fails"));
}
