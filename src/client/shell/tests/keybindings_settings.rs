use super::*;

#[test]
fn shell_new_controls_use_the_same_client_action_routes_as_keybinds() {
    let mut config = Config::default();
    config.ui.prompt_new_workspace_name = false;
    config.ui.prompt_new_tab_name = true;
    let mut state = ClientShellState::new(test_shell_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 20).expect("composed frame");

    let new_workspace = state.hits.new_workspace;
    let create_workspace =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: new_workspace.x + 1,
            row: new_workspace.y,
            modifiers: KeyModifiers::empty(),
        })]);
    let [ClientShellAction::Endpoint { request, .. }] = &create_workspace.actions[..] else {
        panic!("new workspace click should use the endpoint API");
    };
    assert!(matches!(
        request.method,
        crate::api::schema::Method::WorkspaceCreate(_)
    ));

    let new_tab = state.hits.new_tab;
    let open_new_tab =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: new_tab.x + 1,
            row: new_tab.y,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(open_new_tab.actions.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Rename(ClientRenameOverlay {
            target: ClientRenameTarget::NewTab { .. },
            ..
        }))
    ));
}

#[test]
fn manual_client_chrome_preferences_round_trip_per_endpoint() {
    let path = std::env::temp_dir().join(format!(
        "herdr-client-shell-prefs-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let config = test_shell_config(&Config::default()).with_preferences_path(path.clone());
    let mut state = ClientShellState::new(config);
    state.sidebar_width = 31;
    state.sidebar_width_manual = true;
    state.sidebar_section_split = 0.7;
    state.sidebar_section_split_manual = true;
    state.sidebar_collapsed = true;
    state.sidebar_collapsed_manual = true;
    state.collapsed_groups.insert("repo-two".into());
    state.collapsed_groups.insert("repo-one".into());
    let profile =
        SavedSshEndpoint::new("Build", "dev@build.example", "agents").expect("saved SSH profile");
    let remote_id = ClientEndpointId::Ssh(profile.id.clone());
    state
        .remote_collapsed_groups
        .insert(remote_id.clone(), HashSet::from(["/repo".to_owned()]));
    state.persist_chrome_preferences(&mut ClientShellInput::default());
    let stored = std::fs::read_to_string(&path).expect("stored client chrome preferences");
    assert!(stored.contains(profile.id.as_str()));
    assert!(stored.contains("repo-one"));
    assert!(!stored.contains(&profile.label));
    assert!(!stored.contains(&profile.target));

    let reloaded_config = test_shell_config(&Config::default()).with_preferences_path(path.clone());
    let reloaded = ClientShellState::new(reloaded_config);
    assert_eq!(reloaded.sidebar_width, 31);
    assert!(reloaded.sidebar_width_manual);
    assert_eq!(reloaded.sidebar_section_split, 0.7);
    assert!(reloaded.sidebar_section_split_manual);
    assert!(reloaded.sidebar_collapsed);
    assert!(reloaded.sidebar_collapsed_manual);
    assert_eq!(
        reloaded.collapsed_groups,
        HashSet::from(["repo-one".to_string(), "repo-two".to_string()])
    );
    assert_eq!(
        reloaded.remote_collapsed_groups.get(&remote_id),
        Some(&HashSet::from(["/repo".to_owned()]))
    );
    let mut reloaded = reloaded;
    reloaded.persist_chrome_preferences(&mut ClientShellInput::default());
    let stored_again = std::fs::read_to_string(&path).expect("restored client chrome preferences");
    assert!(stored_again.contains(profile.id.as_str()));
    assert!(stored_again.contains("/repo"));
    std::fs::remove_file(path).expect("remove client chrome preferences");
}

#[test]
fn tab_bar_renders_endpoint_status_ellipses_and_clamps_to_useful_scroll() {
    let mut projected = snapshot();
    projected.tab_bar_right = vec![
        crate::protocol::ClientShellTabStatusSegment {
            text: "ZOOM".into(),
            accent: true,
        },
        crate::protocol::ClientShellTabStatusSegment {
            text: "host".into(),
            accent: false,
        },
    ];
    projected.tab_bar_right_separator = " · ".into();
    for number in 2..=8 {
        projected.tabs.push(ClientShellTab {
            tab_id: format!("tab_{number}"),
            workspace_id: "ws_1".into(),
            number,
            label: number.to_string(),
            custom_label: false,
            zoomed: false,
            focused: false,
            agent_status: AgentStatus::Idle,
        });
    }
    let mut config = test_shell_config(&Config::default());
    config.mobile_width_threshold = 0;
    let mut state = ClientShellState::new(config);
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 20).expect("status and overflow tabs");
    let top = frame.cells[..frame.width as usize]
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect::<String>();
    assert!(top.contains("ZOOM · host"));
    assert!(top.contains('…'));

    state.tab_scroll = usize::MAX;
    state.reveal_focused_tab = false;
    state.compose(106, 20).expect("clamped tab scroll");
    assert!(state.tab_scroll < 7);
    let manual_scroll = state.tab_scroll;
    let mut replacement = (**state.snapshot.as_ref().expect("snapshot")).clone();
    replacement.revision = 2;
    replacement.tab_bar_right[1].text = "tick".into();
    let mut replacement_surface = surface();
    replacement_surface.projection_revision = 2;
    state.set_snapshot(Box::new(replacement));
    state.set_pane_surface(replacement_surface);
    assert!(!state.reveal_focused_tab);
    state.compose(106, 20).expect("same-width status update");
    assert_eq!(state.tab_scroll, manual_scroll);

    state.compose(45, 20).expect("narrow tabs win over status");
    let narrow = state.compose(45, 20).expect("narrow tab frame");
    let top = narrow.cells[..narrow.width as usize]
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect::<String>();
    assert!(!top.contains("ZOOM · host"));
}

#[test]
fn inactive_auto_named_tab_label_does_not_stack_terminal_faint() {
    let mut projected = snapshot();
    projected.tabs.push(ClientShellTab {
        tab_id: "tab_2".into(),
        workspace_id: "ws_1".into(),
        number: 2,
        label: "beta".into(),
        custom_label: false,
        zoomed: false,
        focused: false,
        agent_status: AgentStatus::Idle,
    });
    let mut state = ClientShellState::new(test_shell_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 20).expect("tab bar frame");
    let rect = state
        .hits
        .tabs
        .iter()
        .find(|(_, tab_id)| tab_id == "tab_2")
        .expect("inactive tab hit")
        .0;
    let buffer = frame.to_ratatui_buffer().expect("tab bar buffer");
    let (x, y) = cell_symbol_position(&frame, rect, "beta");
    let cell = buffer.cell((x, y)).expect("inactive tab cell");
    assert!(
        !cell.modifier.contains(Modifier::DIM),
        "inactive tab label at ({x},{y}) should not stack terminal faint: {cell:?}"
    );
}

#[test]
fn configured_prefix_is_client_owned_and_renders_its_bar() {
    let config = config_with_keymap(
        r#"
base classic prefix=ctrl+a
prefix { x app.detach }
"#,
    );
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());

    let old_default = state.handle_input_bytes(&[0x02]);
    assert_eq!(
        old_default.requests.len(),
        1,
        "ctrl-b should reach the pane"
    );

    let prefix = state.handle_input_bytes(&[0x01]);
    assert!(prefix.requests.is_empty());
    assert!(prefix.repaint);
    let frame = state.compose(106, 20).expect("prefix frame");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("PREFIX"), "frame: {text:?}");
    assert!(text.contains("ctrl+a"), "frame: {text:?}");

    let detach = state.handle_input_bytes(b"x");
    assert!(detach.detach);
    assert!(detach.requests.is_empty());
}

#[test]
fn prefix_endpoint_action_uses_public_api_with_stable_ids() {
    let mut config = Config::default();
    config.ui.prompt_new_tab_name = false;
    let mut state = ClientShellState::new(test_shell_config(&config));
    state.set_snapshot(Box::new(snapshot()));

    assert!(state.handle_input_bytes(&[0x02]).actions.is_empty());
    let create = state.handle_input_bytes(b"c");
    let [ClientShellAction::Endpoint {
        boot_id, request, ..
    }] = &create.actions[..]
    else {
        panic!("expected one endpoint action: {:?}", create.actions);
    };
    assert_eq!(boot_id, "boot-1");
    match &request.method {
        crate::api::schema::Method::TabCreate(params) => {
            assert_eq!(params.workspace_id.as_deref(), Some("ws_1"));
            assert!(params.focus);
        }
        other => panic!("expected tab.create, got {other:?}"),
    }
    assert!(state.pending_requests.contains_key(&request.id));
}

fn command_invocation(outcome: &ClientShellInput) -> Option<String> {
    outcome.actions.iter().find_map(|action| match action {
        ClientShellAction::Endpoint { request, .. } => match &request.method {
            crate::api::schema::Method::CommandInvoke(params) => Some(params.command_id.clone()),
            _ => None,
        },
        _ => None,
    })
}

#[test]
fn keymap_commands_find_endpoint_ids_by_chord_path() {
    let local = config_with_keymap(
        r#"
base classic prefix=ctrl+a
prefix { y shell "local-only" }
"#,
    );
    let manifest = |command_id: &str| crate::protocol::ClientShellCommand {
        command_id: command_id.into(),
        binding_label: "ctrl+a y".into(),
        binding_labels: vec!["ctrl+a y".into()],
        action: crate::protocol::ClientShellCommandAction::Shell,
        description: Some("local-only".into()),
    };

    let mut remote_state = ClientShellState::new(
        ClientShellConfig::from_config(&local)
            .with_keybinding_source(ClientShellKeybindingSource::RemoteLocal),
    );
    let mut remote_projection = snapshot();
    remote_projection.commands.push(manifest("cmd_remote"));
    remote_state.set_snapshot(Box::new(remote_projection));
    assert_eq!(remote_state.config.keymap.prefix.0, KeyCode::Char('a'));
    remote_state.handle_input_bytes(&[0x01]);
    let outcome = remote_state.handle_input_bytes(b"y");
    assert_eq!(
        command_invocation(&outcome),
        None,
        "local commands never run on a remote server"
    );

    let mut local_state = ClientShellState::new(ClientShellConfig::from_config(&local));
    let mut local_projection = snapshot();
    local_projection
        .commands
        .push(manifest("cmd_loaded_endpoint"));
    local_state.set_snapshot(Box::new(local_projection));
    local_state.handle_input_bytes(&[0x01]);
    let outcome = local_state.handle_input_bytes(b"y");
    assert_eq!(
        command_invocation(&outcome).as_deref(),
        Some("cmd_loaded_endpoint")
    );

    let mut reloaded = snapshot();
    reloaded.revision = 2;
    reloaded.commands.push(manifest("cmd_reloaded_endpoint"));
    local_state.handle_input_bytes(&[0x01]);
    assert_eq!(local_state.mode_name(), "prefix");
    local_state.set_snapshot(Box::new(reloaded));
    assert_eq!(
        local_state.mode_name(),
        "prefix",
        "new command ids do not close open menus"
    );
    let outcome = local_state.handle_input_bytes(b"y");
    assert_eq!(
        command_invocation(&outcome).as_deref(),
        Some("cmd_reloaded_endpoint")
    );
}

#[test]
fn custom_binding_invokes_only_the_endpoint_manifest_id() {
    let mut state = ClientShellState::new(test_shell_config(&Config::default()));
    let binding = command_leaf(
        "ctrl+b z",
        crate::input::keymap::CommandKind::Shell,
        "secret-command --token hidden",
    );
    let mut projection = snapshot();
    projection
        .commands
        .push(crate::protocol::ClientShellCommand {
            command_id: "cmd_0123456789abcdef0123456789abcdef".into(),
            binding_label: binding.path_label.clone(),
            binding_labels: vec![binding.path_label.clone()],
            action: crate::protocol::ClientShellCommandAction::Shell,
            description: None,
        });
    state.set_snapshot(Box::new(projection));

    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Command(binding), &mut outcome);

    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!("expected endpoint command invocation");
    };
    let crate::api::schema::Method::CommandInvoke(params) = &request.method else {
        panic!("expected command.invoke");
    };
    assert_eq!(params.command_id, "cmd_0123456789abcdef0123456789abcdef");
    assert_eq!(params.workspace_id.as_deref(), Some("ws_1"));
    assert_eq!(params.tab_id.as_deref(), Some("tab_1"));
    assert_eq!(params.pane_id.as_deref(), Some("pane_1"));
    assert_eq!(params.selection, None);
    assert!(!serde_json::to_string(request)
        .unwrap()
        .contains("secret-command"));
}

#[test]
fn plugin_command_carries_client_owned_selection_coordinates() {
    let mut state = ClientShellState::new(test_shell_config(&Config::default()));
    let binding = command_leaf(
        "ctrl+b p",
        crate::input::keymap::CommandKind::Plugin,
        "plugin.action",
    );
    let mut projection = snapshot();
    projection
        .commands
        .push(crate::protocol::ClientShellCommand {
            command_id: "cmd_plugin".into(),
            binding_label: binding.path_label.clone(),
            binding_labels: vec![binding.path_label.clone()],
            action: crate::protocol::ClientShellCommandAction::PluginAction,
            description: None,
        });
    state.set_snapshot(Box::new(projection));
    let mut pane_surface = surface();
    pane_surface.panes[0].content_revision = 42;
    state.set_pane_surface(pane_surface);
    let mut selection =
        crate::selection::Selection::absolute_range("pane_1".to_owned(), (2, 3), (4, 5));
    assert!(selection.finish());
    state.selection = Some(selection);

    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Command(binding), &mut outcome);

    let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
        panic!("expected endpoint command invocation");
    };
    let crate::api::schema::Method::CommandInvoke(params) = &request.method else {
        panic!("expected command.invoke");
    };
    assert_eq!(
        params.selection,
        Some(crate::api::schema::PaneSelectionReadParams {
            pane_id: "pane_1".into(),
            anchor: crate::api::schema::PaneTextPoint { row: 2, col: 3 },
            cursor: crate::api::schema::PaneTextPoint { row: 4, col: 5 },
            content_revision: Some(42),
        })
    );
}

#[test]
fn unavailable_endpoint_method_is_disabled_without_disconnect() {
    let mut state = ClientShellState::new(test_shell_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_endpoint_methods(Some(vec!["pane.focus".into()]));
    let mut outcome = ClientShellInput::default();

    state.push_endpoint_method(
        crate::api::schema::Method::WorkspaceFocus(crate::api::schema::WorkspaceTarget {
            workspace_id: "missing".into(),
        }),
        &mut outcome,
    );

    assert!(outcome.actions.is_empty());
    assert!(outcome.repaint);
    let notice = state
        .visible_endpoint_notice
        .as_ref()
        .expect("unsupported action notice");
    assert_eq!(notice.key.kind, ClientEndpointNoticeKind::Unsupported);
    assert_eq!(notice.key.code, "workspace.focus");
    assert!(notice.body.contains("This server"));
    assert!(state.endpoint_error.is_none());

    let mut repeated = ClientShellInput::default();
    state.push_endpoint_method(
        crate::api::schema::Method::WorkspaceFocus(crate::api::schema::WorkspaceTarget {
            workspace_id: "missing".into(),
        }),
        &mut repeated,
    );
    assert!(repeated.actions.is_empty());
    assert!(!repeated.repaint);

    state.compose(106, 20).expect("endpoint notice frame");
    assert!(!state.hits.notification_toast.is_empty());

    state.handle_input_bytes(b"x");
    assert!(state.visible_endpoint_notice.is_some());

    let hit = state.hits.notification_toast;
    let dismissed =
        state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::empty(),
        })]);
    assert!(dismissed.repaint);
    assert!(state.visible_endpoint_notice.is_none());
}

#[test]
fn generic_endpoint_failures_and_control_errors_are_visible() {
    let mut state = ClientShellState::new(test_shell_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let mut outcome = ClientShellInput::default();
    state.push_endpoint_method(
        crate::api::schema::Method::WorkspaceFocus(crate::api::schema::WorkspaceTarget {
            workspace_id: "missing".into(),
        }),
        &mut outcome,
    );
    let request_id = match &outcome.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => request.id.clone(),
        other => panic!("expected generic endpoint request, got {other:?}"),
    };
    let rejected_at = std::time::Instant::now();
    let (repaint, actions) = state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Err(ClientShellEndpointError {
            code: Some("not_found".into()),
            message: "workspace no longer exists".into(),
        }),
    );
    assert!(repaint);
    assert!(actions.is_empty());
    let notice = state
        .visible_endpoint_notice
        .as_ref()
        .expect("rejected action notice");
    assert_eq!(notice.key.kind, ClientEndpointNoticeKind::Rejected);
    assert_eq!(notice.key.code, "workspace.focus:not_found");
    assert_eq!(notice.body, "workspace no longer exists");
    let observed_at = std::time::Instant::now();
    assert!(notice.deadline >= rejected_at + std::time::Duration::from_secs(3));
    assert!(notice.deadline <= observed_at + std::time::Duration::from_secs(3));
    assert!(state.endpoint_error.is_none());

    assert!(state.receive_endpoint_error("Paste rejected: too large".into()));
    let notice = state
        .visible_endpoint_notice
        .as_ref()
        .expect("paste rejection notice");
    assert_eq!(notice.key.kind, ClientEndpointNoticeKind::Rejected);
    assert_eq!(notice.key.code, "paste_rejected");
    assert_eq!(notice.body, "Paste rejected: too large");
    assert!(state.receive_endpoint_error("Paste rejected again".into()));
    assert_eq!(
        state
            .visible_endpoint_notice
            .as_ref()
            .map(|notice| notice.body.as_str()),
        Some("Paste rejected again")
    );
}

#[test]
fn endpoint_timeout_is_a_deduplicated_server_notice() {
    let mut state = ClientShellState::new(test_shell_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let mut outcome = ClientShellInput::default();
    state.push_endpoint_method(
        crate::api::schema::Method::WorkspaceFocus(crate::api::schema::WorkspaceTarget {
            workspace_id: "work".into(),
        }),
        &mut outcome,
    );
    let request_id = match &outcome.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => request.id.clone(),
        other => panic!("expected endpoint request, got {other:?}"),
    };

    let (repaint, actions) = state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Err(ClientShellEndpointError {
            code: Some("endpoint_timeout".into()),
            message: "transport timeout".into(),
        }),
    );

    assert!(repaint);
    assert!(actions.is_empty());
    let notice = state
        .visible_endpoint_notice
        .as_ref()
        .expect("timeout notice");
    assert_eq!(notice.key.kind, ClientEndpointNoticeKind::Timeout);
    assert_eq!(notice.key.code, "workspace.focus");
    assert!(notice.body.contains("workspace.focus"));
    assert!(state.endpoint_error.is_none());
}

#[test]
fn endpoint_notice_dedupe_resets_for_a_new_server_boot() {
    let mut state = ClientShellState::new(test_shell_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_endpoint_methods(Some(Vec::new()));
    let method = || {
        crate::api::schema::Method::WorkspaceFocus(crate::api::schema::WorkspaceTarget {
            workspace_id: "work".into(),
        })
    };
    let mut first = ClientShellInput::default();
    state.push_endpoint_method(method(), &mut first);
    assert!(first.repaint);

    let mut next_snapshot = snapshot();
    next_snapshot.boot_id = "boot-2".into();
    state.set_snapshot(Box::new(next_snapshot));
    let mut second = ClientShellInput::default();
    state.push_endpoint_method(method(), &mut second);

    assert!(second.repaint);
    assert_eq!(
        state
            .visible_endpoint_notice
            .as_ref()
            .map(|notice| notice.key.boot_id.as_str()),
        Some("boot-2")
    );
}

#[test]
fn custom_binding_missing_from_endpoint_manifest_is_not_forwarded() {
    let mut state = ClientShellState::new(test_shell_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let binding = command_leaf(
        "ctrl+b z",
        crate::input::keymap::CommandKind::Shell,
        "secret-command",
    );

    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Command(binding), &mut outcome);

    assert!(outcome.actions.is_empty());
    assert!(outcome.repaint);
    assert!(state
        .endpoint_error
        .as_deref()
        .is_some_and(|error| error.contains("not available")));
}

#[test]
fn help_overlay_restores_released_search_scroll_and_custom_binding_behavior() {
    let config = config_with_keymap(
        r#"
base classic
prefix { z plugin "example.run" "run plugin action" }
"#,
    );
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let mut projection = snapshot();
    projection
        .commands
        .push(crate::protocol::ClientShellCommand {
            command_id: "plugin-action".into(),
            binding_label: "ctrl+b z".into(),
            binding_labels: vec!["ctrl+b z".into()],
            action: crate::protocol::ClientShellCommandAction::PluginAction,
            description: Some("run plugin action".into()),
        });
    state.set_snapshot(Box::new(projection));
    state.set_pane_surface(surface());
    let mut open = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::Help),
        &mut open,
    );
    let initial = state.compose(106, 30).expect("help overlay");
    let text = initial
        .cells
        .chunks(initial.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("prefix"));
    assert!(state.hits.help_max_scroll > 0);
    assert_ne!(state.hits.help_scrollbar, Rect::default());

    state.handle_input_bytes(b"/");
    state.handle_input_bytes(b"plugin");
    let custom = state.compose(106, 30).expect("custom help search");
    let text = custom
        .cells
        .chunks(custom.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("ctrl+b z"));
    assert!(text.contains("run plugin action"));
    state.handle_input_bytes(b"\x1b");

    state.handle_input_bytes(b"/");
    state.handle_input_bytes(b"does-not-exist");
    let empty = state.compose(106, 30).expect("empty help search");
    let text = empty
        .cells
        .chunks(empty.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("no matching keybinds"));

    state.handle_input_bytes(b"\x1b");
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Help(ClientHelpOverlay {
            search_focused: false,
            ref query,
            scroll: 0,
        })) if query.is_empty()
    ));
    state.compose(106, 30).expect("restored help");
    state.handle_input_bytes(b"\x1b[F");
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Help(ClientHelpOverlay { scroll, .. }))
            if scroll == state.hits.help_max_scroll
    ));
    state.handle_input_bytes(b"\x1b[H");
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Help(ClientHelpOverlay {
            scroll: 0,
            ..
        }))
    ));
    state.handle_input_bytes(b"?");
    assert!(state.overlay.is_none());
}

#[test]
fn resize_mode_reuses_endpoint_resize_and_stays_active_until_done() {
    let mut state = ClientShellState::new(test_shell_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());

    assert!(state.handle_input_bytes(&[0x02]).actions.is_empty());
    assert!(state.handle_input_bytes(b"r").actions.is_empty());
    assert_eq!(state.mode_name(), "resize");

    let modified = state.handle_input_bytes(b"\x1b[1;2D");
    assert!(matches!(
        &modified.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(
                &request.method,
                crate::api::schema::Method::PaneResize(params)
                    if params.direction == crate::api::schema::PaneDirection::Left
            )
    ));
    assert_eq!(state.mode_name(), "resize");

    let resize = state.handle_input_bytes(b"h");
    let [ClientShellAction::Endpoint { request, .. }] = &resize.actions[..] else {
        panic!("resize should use endpoint API");
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneResize(params)
            if params.pane_id.as_deref() == Some("pane_1")
                && params.direction == crate::api::schema::PaneDirection::Left
    ));
    assert_eq!(state.mode_name(), "resize");

    assert!(state.handle_input_bytes(b"\r").actions.is_empty());
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

fn keymap_projection(
    revision: u64,
    plugins: &[(&str, &str)],
    server_keymap: Option<&str>,
) -> crate::protocol::endpoint::EndpointKeymapProjection {
    crate::protocol::endpoint::EndpointKeymapProjection {
        boot_id: "boot-1".into(),
        revision,
        plugins: plugins
            .iter()
            .map(
                |(plugin_id, text)| crate::protocol::endpoint::EndpointKeymapLayer {
                    plugin_id: (*plugin_id).into(),
                    source: format!("{plugin_id}/keymap.kdl"),
                    text: (*text).into(),
                },
            )
            .collect(),
        server_keymap: server_keymap.map(str::to_owned),
    }
}

fn tab_created(outcome: &ClientShellInput) -> bool {
    outcome.actions.iter().any(|action| {
        matches!(action, ClientShellAction::Endpoint { request, .. }
            if matches!(request.method, crate::api::schema::Method::TabCreate(_)))
    })
}

#[test]
fn server_plugin_menus_join_the_local_keymap() {
    let mut config = Config::default();
    config.ui.prompt_new_tab_name = false;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let mut projection_snapshot = snapshot();
    projection_snapshot
        .commands
        .push(crate::protocol::ClientShellCommand {
            command_id: "cmd_layout".into(),
            binding_label: "ctrl+b p g".into(),
            binding_labels: vec!["ctrl+b p g".into()],
            action: crate::protocol::ClientShellCommandAction::PluginAction,
            description: Some("layout".into()),
        });
    state.set_snapshot(Box::new(projection_snapshot));
    state.set_endpoint_keymap_projection(
        &ClientEndpointId::Local,
        keymap_projection(
            2,
            &[("example.layout", "prefix { p { g plugin apply layout } }")],
            Some("prefix { t { c tab.new } }"),
        ),
    );

    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"p");
    let outcome = state.handle_input_bytes(b"g");
    assert_eq!(command_invocation(&outcome).as_deref(), Some("cmd_layout"));

    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"t");
    let outcome = state.handle_input_bytes(b"c");
    assert!(
        !tab_created(&outcome),
        "the local keymap applies, not the server's"
    );

    state.handle_input_bytes(&[0x02]);
    assert_eq!(state.mode_name(), "herdra");
    state.set_endpoint_keymap_projection(&ClientEndpointId::Local, keymap_projection(1, &[], None));
    assert_eq!(state.mode_name(), "herdra", "stale projections are ignored");
    state.set_endpoint_keymap_projection(&ClientEndpointId::Local, keymap_projection(3, &[], None));
    assert_eq!(
        state.mode,
        ClientShellMode::Terminal,
        "a new keymap closes menus"
    );
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"p");
    let outcome = state.handle_input_bytes(b"g");
    assert_eq!(command_invocation(&outcome), None);
}

#[test]
fn server_keybinding_source_uses_the_server_keymap() {
    let mut config = config_with_keymap("base classic");
    config.ui.prompt_new_tab_name = false;
    let mut state = ClientShellState::new(
        ClientShellConfig::from_config(&config)
            .with_keybinding_source(ClientShellKeybindingSource::Endpoint),
    );
    state.set_snapshot(Box::new(snapshot()));
    state.handle_input_bytes(&[0x02]);
    assert!(
        tab_created(&state.handle_input_bytes(b"c")),
        "the local keymap applies until the server sends its own"
    );

    state.set_endpoint_keymap_projection(
        &ClientEndpointId::Local,
        keymap_projection(2, &[], Some("prefix { t { c tab.new } }")),
    );
    state.handle_input_bytes(&[0x02]);
    assert!(!tab_created(&state.handle_input_bytes(b"c")));
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"t");
    assert!(tab_created(&state.handle_input_bytes(b"c")));
}

fn frame_text(frame: &crate::client::frame_output::ComposedFrame) -> String {
    frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn mode_hint_bar_setting_hides_menu_bars_unless_a_menu_asks() {
    let mut config = config_with_keymap("prefix { t tab bar=full {} }\n");
    config.ui.mode_hint_bar = crate::config::ModeHintBarConfig::Hidden;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    let mut pane_surface = surface();
    pane_surface.panes[0].scroll = Some(crate::protocol::PaneSurfaceScrollMetrics {
        offset_from_bottom: 0,
        max_offset_from_bottom: 20,
        viewport_rows: 2,
    });
    state.set_pane_surface(pane_surface);
    let terminal = frame_text(&state.compose(106, 20).expect("terminal frame"));

    state.handle_input_bytes(&[0x02]);
    assert_eq!(state.mode_name(), "herdra");
    let hidden = frame_text(&state.compose(106, 20).expect("hidden bar frame"));
    assert_eq!(hidden, terminal, "the main menu draws no bar");

    state.handle_input_bytes(b"t");
    assert_eq!(state.mode_name(), "tab");
    let shown = frame_text(&state.compose(106, 20).expect("tab bar frame"));
    assert!(shown.contains("esc cancel"), "frame: {shown:?}");

    state.handle_input_bytes(&[0x1b]);
    state.handle_input_bytes(b"\x02py");
    assert!(state.copy_mode_focused());
    let copy = frame_text(&state.compose(106, 20).expect("copy frame"));
    assert!(
        !copy.contains("esc cancel") && !copy.contains("yank"),
        "frame: {copy:?}"
    );

    state.handle_input_bytes(b"/");
    let search = frame_text(&state.compose(106, 20).expect("copy search frame"));
    assert!(
        search.contains("enter search"),
        "the search prompt shows without a bar: {search:?}"
    );
}
