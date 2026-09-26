//! The navigator, keybind list, and settings popups take their keys from
//! menus in the keymap, and each popup keeps its menu open.

use super::*;

fn two_pane_snapshot() -> ClientShellSnapshot {
    let mut projected = snapshot();
    let mut sibling = projected.panes[0].clone();
    sibling.pane_id = "pane_2".into();
    sibling.label = Some("sibling".into());
    sibling.focused = false;
    projected.panes.push(sibling);
    projected
}

fn shell(config: &Config) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(config));
    state.set_snapshot(Box::new(two_pane_snapshot()));
    state.set_pane_surface(surface());
    state
}

fn navigator(state: &ClientShellState) -> &ClientNavigatorOverlay {
    match &state.overlay {
        Some(ClientShellOverlay::Navigator(navigator)) => navigator,
        other => panic!("expected the navigator, found {other:?}"),
    }
}

fn help(state: &ClientShellState) -> &ClientHelpOverlay {
    match &state.overlay {
        Some(ClientShellOverlay::Help(help)) => help,
        other => panic!("expected the keybind list, found {other:?}"),
    }
}

/// Titles of the open menus, bottom first.
fn menu_titles(state: &ClientShellState) -> Vec<String> {
    state
        .mode
        .stack()
        .map(|stack| {
            stack
                .frames()
                .iter()
                .map(|id| state.config.keymap.menu(*id).title.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn press(state: &mut ClientShellState, code: KeyCode, modifiers: KeyModifiers) {
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        code, modifiers,
    ))]);
}

fn click(state: &mut ClientShellState, column: u16, row: u16) {
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })]);
}

fn composed_text(state: &mut ClientShellState) -> String {
    let frame = state.compose(106, 30).expect("frame");
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
fn popup_keys_rebound_in_keymap_kdl_drive_the_popups() {
    let config = config_with_keymap(
        r#"
prefix {
    g {
        x navigator.move.down
        j none
    }
    "?" {
        q help.close
    }
}
"#,
    );
    let mut state = shell(&config);
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"g");
    assert_eq!(menu_titles(&state), ["herdra", "navigator"]);
    let start = navigator(&state).selected.clone();
    assert!(start.is_some());

    state.handle_input_bytes(b"j");
    assert_eq!(navigator(&state).selected, start, "j is unbound now");
    state.handle_input_bytes(b"x");
    assert_ne!(navigator(&state).selected, start);
    assert_eq!(menu_titles(&state), ["herdra", "navigator"]);
    let hints = composed_text(&mut state);
    assert!(
        hints.contains("↑↓/k rows"),
        "the popup lists the menu's keys"
    );
    assert!(hints.contains("x down"));

    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);

    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"?");
    assert_eq!(menu_titles(&state), ["herdra", "keybinds"]);
    state.handle_input_bytes(b"j");
    assert!(state.overlay.is_some(), "j scrolls the list");
    state.handle_input_bytes(b"q");
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn question_mark_opens_the_keybind_list_at_the_menu_it_was_pressed_in() {
    let mut state = shell(&Config::default());
    state.compose(106, 30).expect("frame");
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"p");
    state.handle_input_bytes(b"?");
    assert_eq!(menu_titles(&state), ["herdra", "pane", "keybinds"]);
    let pane = state
        .config
        .keymap
        .menu_by_path("ctrl+b p")
        .expect("pane menu");
    let expected = render::help_scroll_to_menu(&state.config.keymap, pane, Some((106, 30)));
    assert!(expected > 0);
    assert_eq!(help(&state).scroll, expected);
    let text = composed_text(&mut state);
    assert!(text.contains(" herdra › pane"));
    assert!(
        !text.contains(" top level"),
        "the list starts at the pane menu"
    );

    state.handle_input_bytes(b"\x7f");
    assert!(
        state.overlay.is_none(),
        "backspace goes back to the pane menu"
    );
    assert_eq!(menu_titles(&state), ["herdra", "pane"]);
    state.handle_input_bytes(b"?");
    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);

    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"?");
    assert_eq!(
        help(&state).scroll,
        0,
        "the main menu's list starts at the top"
    );
    state.handle_input_bytes(b"\x1b");

    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"g");
    state.handle_input_bytes(b"?");
    assert_eq!(
        menu_titles(&state),
        ["keybinds"],
        "the keybind list replaces the navigator and its menu"
    );
    let navigator_menu = state
        .config
        .keymap
        .menu_by_path("ctrl+b g")
        .expect("navigator menu");
    assert_eq!(
        help(&state).scroll,
        render::help_scroll_to_menu(&state.config.keymap, navigator_menu, Some((106, 30)))
    );
}

#[test]
fn keybind_list_shows_keymap_problems_first() {
    let mut state = shell(&config_with_keymap("prefix { t { n tab.nwe } }"));
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::Help),
        &mut ClientShellInput::default(),
    );
    let text = composed_text(&mut state);
    assert!(text.contains("keymap problems"), "{text}");
    assert!(text.contains("unknown action \"tab.nwe\""), "{text}");
}

#[test]
fn popup_menus_follow_popups_opened_and_closed_outside_their_menus() {
    let mut state = shell(&config_with_keymap("ctrl+alt+g app.navigator"));
    press(
        &mut state,
        KeyCode::Char('g'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    );
    assert_eq!(
        menu_titles(&state),
        ["navigator"],
        "a top-level leaf opens the navigator with its menu"
    );
    let start = navigator(&state).selected.clone();
    state.handle_input_bytes(b"j");
    assert_ne!(navigator(&state).selected, start);
    state.compose(106, 30).expect("navigator");
    click(&mut state, 0, 0);
    assert!(state.overlay.is_none());
    assert_eq!(
        state.mode,
        ClientShellMode::Terminal,
        "closing the navigator by mouse closes its menu"
    );

    state.overlay = Some(ClientShellOverlay::GlobalMenu(ClientGlobalMenuOverlay {
        highlighted: 0,
    }));
    state.handle_input_bytes(b"\r");
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Settings(_))
    ));
    assert_eq!(menu_titles(&state), ["settings"]);
    state.handle_input_bytes(b"\t");
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
            section: ClientSettingsSection::Indicators,
            ..
        }))
    ));
    state.compose(106, 30).expect("settings");
    let cancel = state.hits.overlay_cancel;
    click(&mut state, cancel.x, cancel.y);
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);

    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"t");
    state.handle_input_bytes(b"?");
    assert_eq!(menu_titles(&state), ["herdra", "tab", "keybinds"]);
    state.compose(106, 30).expect("keybind list");
    click(&mut state, 0, 0);
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);

    state.open_navigator_overlay();
    assert_eq!(menu_titles(&state), ["navigator"]);
    state.overlay = None;
    state.handle_input_bytes(b"j");
    assert_eq!(
        state.mode,
        ClientShellMode::Terminal,
        "a popup closed by any path takes its menu with it"
    );
}

#[test]
fn another_popup_in_place_of_settings_undoes_its_theme_preview() {
    let mut state = shell(&config_with_keymap("ctrl+alt+g app.navigator"));
    let original_theme = state.config.theme_name.clone();
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"s");
    state.handle_input_bytes(b"s");
    assert_eq!(menu_titles(&state), ["herdra", "session", "settings"]);
    state.handle_input_bytes(b"j");
    assert_ne!(state.config.theme_name, original_theme);
    state.handle_input_bytes(b"?");
    assert!(matches!(state.overlay, Some(ClientShellOverlay::Help(_))));
    assert_eq!(menu_titles(&state), ["keybinds"]);
    assert_eq!(state.config.theme_name, original_theme);

    state.handle_input_bytes(b"\x1b");
    state.open_settings_overlay();
    state.handle_input_bytes(b"j");
    assert_ne!(state.config.theme_name, original_theme);
    press(
        &mut state,
        KeyCode::Char('g'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    );
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));
    assert_eq!(menu_titles(&state), ["navigator"]);
    assert_eq!(state.config.theme_name, original_theme);
}

#[test]
fn popups_list_the_keys_of_their_menus() {
    let mut state = shell(&Config::default());
    state.open_navigator_overlay();
    assert!(composed_text(&mut state).contains(
        " ↑↓/j/k rows · ←→ workspace · / search · a/b/w/i/d filter · enter open · esc close"
    ));
    state.open_help_overlay();
    assert!(composed_text(&mut state)
        .contains(" / filter · j/k/↑↓/pageup/pagedown scroll · enter/esc close"));
    state.open_settings_overlay();
    assert!(composed_text(&mut state).contains(" ↑↓ select · tab section"));
}

#[test]
fn search_fields_take_typing_until_enter_or_esc() {
    let mut state = shell(&Config::default());
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"g");
    state.handle_input_bytes(b"/");
    for byte in b"sib?q" {
        state.handle_input_bytes(&[*byte]);
    }
    assert!(navigator(&state).search_focused);
    assert_eq!(navigator(&state).query.as_str(), "sib?q");
    assert_eq!(menu_titles(&state), ["herdra", "navigator"]);
    state.handle_input_bytes(b"\x1b");
    assert!(!navigator(&state).search_focused, "esc ends the search");
    assert_eq!(navigator(&state).query.as_str(), "sib?q");
    assert_eq!(menu_titles(&state), ["herdra", "navigator"]);
    state.handle_input_bytes(b"a");
    assert!(navigator(&state).query.as_str().is_empty());
    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);

    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"?");
    state.handle_input_bytes(b"/");
    for byte in b"q?j" {
        state.handle_input_bytes(&[*byte]);
    }
    assert!(help(&state).search_focused);
    assert_eq!(help(&state).query.as_str(), "q?j");
    assert_eq!(menu_titles(&state), ["herdra", "keybinds"]);
    state.handle_input_bytes(b"\r");
    assert!(state.overlay.is_none(), "enter closes the list");
    assert_eq!(state.mode, ClientShellMode::Terminal);
}
