use crossterm::event::{KeyCode, KeyModifiers};

use super::*;
use crate::input::TerminalKey;

fn key(code: KeyCode) -> TerminalKey {
    TerminalKey::new(code, KeyModifiers::empty())
}

fn ch(c: char) -> TerminalKey {
    key(KeyCode::Char(c))
}

fn ctrl(c: char) -> TerminalKey {
    TerminalKey::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn user(text: &str) -> KeymapText {
    KeymapText {
        source: "keymap.kdl".to_owned(),
        text: text.to_owned(),
    }
}

fn build(text: &str) -> CompiledKeymap {
    CompiledKeymap::build(Some(&user(text)), &[])
}

/// Press keys from the terminal level and return the final open menus.
fn press(keymap: &CompiledKeymap, keys: &[TerminalKey]) -> (Option<MenuStack>, Vec<String>) {
    let mut stack = None;
    let mut ran = Vec::new();
    for key in keys {
        match resolve(keymap, stack, key) {
            Step::Forward => ran.push("forward".to_owned()),
            Step::Ignore => ran.push("ignore".to_owned()),
            Step::Apply { next, effect } => {
                stack = next;
                match effect {
                    Effect::None => {}
                    Effect::ForwardKey => ran.push("forward".to_owned()),
                    Effect::Literal { .. } => ran.push("literal".to_owned()),
                    Effect::Run { binding, index } => ran.push(run_label(keymap, binding, index)),
                }
            }
        }
    }
    (stack, ran)
}

fn run_label(keymap: &CompiledKeymap, binding: &CompiledBinding, index: Option<usize>) -> String {
    match &binding.target {
        CompiledTarget::Action(_) => {
            let id = binding_action_id(binding).unwrap_or("?");
            match binding.digit_index(index) {
                Some(index) => format!("{id}:{index}"),
                None => id.to_owned(),
            }
        }
        CompiledTarget::Command(index) => {
            let command = &keymap.commands[*index];
            format!("{} {}", command.spec.kind.word(), command.spec.command)
        }
        other => format!("{other:?}"),
    }
}

fn titles(keymap: &CompiledKeymap, stack: Option<MenuStack>) -> Vec<String> {
    stack
        .map(|stack| {
            stack
                .frames()
                .iter()
                .map(|id| keymap.menu(*id).title.clone())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn shipped_keymaps_compile_without_diagnostics() {
    let default = CompiledKeymap::build(None, &[]);
    assert!(default.diagnostics.is_empty(), "{:?}", default.diagnostics);
    assert!(default.conflicts.is_empty(), "{:?}", default.conflicts);
    let classic = build("base classic");
    assert!(classic.diagnostics.is_empty(), "{:?}", classic.diagnostics);
    assert!(classic.conflicts.is_empty(), "{:?}", classic.conflicts);
    assert_eq!(classic.base, Base::Classic);
}

#[test]
fn default_tree_groups_actions_under_the_prefix() {
    let keymap = CompiledKeymap::default();
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('t'), ch('n')]);
    assert_eq!(stack, None);
    assert_eq!(ran, ["tab.new"]);

    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('w')]);
    assert_eq!(titles(&keymap, stack), ["herdra", "workspace"]);
    assert!(ran.is_empty());

    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('w'), ch('j'), ch('j'), ch('n')]);
    assert_eq!(stack, None);
    assert_eq!(
        ran,
        [
            "workspace.list.down",
            "workspace.list.down",
            "workspace.new"
        ]
    );

    let (_, ran) = press(&keymap, &[ctrl('b'), ch('t'), ch('3')]);
    assert_eq!(ran, ["tab.switch:2"]);
}

#[test]
fn unmatched_keys_follow_each_level() {
    let keymap = CompiledKeymap::default();
    let (stack, ran) = press(&keymap, &[ch('x')]);
    assert_eq!(stack, None);
    assert_eq!(ran, ["forward"]);

    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('%')]);
    assert_eq!(stack, None, "one-shot menus close on an unknown key");
    assert!(ran.is_empty());

    let (stack, _) = press(&keymap, &[ctrl('b'), ch('w'), ch('%')]);
    assert_eq!(
        titles(&keymap, stack),
        ["herdra", "workspace"],
        "unmatched=ignore keeps the list open"
    );
}

#[test]
fn prefix_twice_sends_the_literal_prefix() {
    let keymap = CompiledKeymap::default();
    let (stack, ran) = press(&keymap, &[ctrl('b'), ctrl('b')]);
    assert_eq!(stack, None);
    assert_eq!(ran, ["literal"]);
}

#[test]
fn stay_leaves_and_sticky_menus_keep_menus_open() {
    let keymap = CompiledKeymap::default();
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('p'), ch('h'), ch('l')]);
    assert_eq!(titles(&keymap, stack), ["herdra", "pane"]);
    assert_eq!(ran, ["pane.focus.left", "pane.focus.right"]);

    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('p'), ch('r'), ch('h'), ch('h')]);
    assert_eq!(titles(&keymap, stack), ["herdra", "pane", "resize"]);
    assert_eq!(ran, ["pane.resize.left", "pane.resize.left"]);

    let (stack, _) = press(&keymap, &[ctrl('b'), ch('p'), ch('r'), key(KeyCode::Enter)]);
    assert_eq!(stack, None);
}

#[test]
fn esc_cancels_and_backspace_goes_back() {
    let keymap = build(
        r#"
        ctrl+b {
            p { r { x pane.close } }
            q pane { z pane.zoom }
        }
        ctrl+g outer sticky {
            i inner { z pane.zoom }
        }
        "#,
    );
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    let (stack, _) = press(&keymap, &[ctrl('g'), ch('i')]);
    assert_eq!(titles(&keymap, stack), ["outer", "inner"]);
    let (stack, _) = press(&keymap, &[ctrl('g'), ch('i'), key(KeyCode::Backspace)]);
    assert_eq!(titles(&keymap, stack), ["outer"]);
    let (stack, _) = press(&keymap, &[ctrl('g'), ch('i'), key(KeyCode::Esc)]);
    assert_eq!(stack, None);
}

#[test]
fn copy_mode_is_a_mode_that_menus_return_to() {
    let keymap = CompiledKeymap::default();
    let (stack, _) = press(&keymap, &[ctrl('b'), ch('p'), ch('y')]);
    assert_eq!(titles(&keymap, stack), ["copy"]);
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('p'), ch('y'), ch('j'), ch('k')]);
    assert_eq!(titles(&keymap, stack), ["copy"]);
    assert_eq!(ran, ["copy.move.down", "copy.move.up"]);

    let (stack, _) = press(&keymap, &[ctrl('b'), ch('p'), ch('y'), ctrl('b')]);
    assert_eq!(titles(&keymap, stack), ["copy", "herdra"]);
    let (stack, _) = press(
        &keymap,
        &[ctrl('b'), ch('p'), ch('y'), ctrl('b'), key(KeyCode::Esc)],
    );
    assert_eq!(titles(&keymap, stack), ["copy"]);
    let (stack, _) = press(&keymap, &[ctrl('b'), ch('p'), ch('y'), ctrl('b'), ch('t')]);
    assert_eq!(titles(&keymap, stack), ["copy", "herdra", "tab"]);
    let (stack, ran) = press(
        &keymap,
        &[ctrl('b'), ch('p'), ch('y'), ctrl('b'), ch('t'), ch('n')],
    );
    assert_eq!(titles(&keymap, stack), ["copy"]);
    assert_eq!(ran, ["tab.new"]);

    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('p'), ch('y'), ch('y')]);
    assert_eq!(titles(&keymap, stack), ["copy"], "yank closes copy itself");
    assert_eq!(ran, ["copy.yank"]);
}

#[test]
fn top_level_chords_work_inside_menus() {
    let keymap = build("alt+h pane.focus.left");
    let (stack, ran) = press(
        &keymap,
        &[
            ctrl('b'),
            ch('p'),
            ch('y'),
            TerminalKey::new(KeyCode::Char('h'), KeyModifiers::ALT),
        ],
    );
    assert_eq!(titles(&keymap, stack), ["copy"]);
    assert_eq!(ran, ["pane.focus.left"]);

    let (stack, ran) = press(
        &keymap,
        &[TerminalKey::new(KeyCode::Char('h'), KeyModifiers::ALT)],
    );
    assert_eq!(stack, None);
    assert_eq!(ran, ["pane.focus.left"]);

    let (stack, _) = press(&keymap, &[ctrl('b'), ch('w'), ctrl('b')]);
    assert_eq!(
        titles(&keymap, stack),
        ["herdra"],
        "the prefix reopens the main menu"
    );
}

#[test]
fn user_layers_extend_override_and_unbind() {
    let keymap = build(
        r#"
        prefix {
            t {
                c tab.new "create"
                n none
            }
            g shell "git pull" pull
            x "extra" {
                z pane.zoom
            }
        }
        "#,
    );
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('t'), ch('c')]);
    assert_eq!(ran, ["tab.new"]);
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('t'), ch('n')]);
    assert_eq!(stack, None);
    assert!(
        ran.is_empty(),
        "unbound key closes the one-shot menu: {ran:?}"
    );
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('g')]);
    assert_eq!(ran, ["shell git pull"]);
    assert_eq!(keymap.commands[0].path_label, "ctrl+b g");
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('x'), ch('z')]);
    assert_eq!(ran, ["pane.zoom"]);
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('w'), ch('r')]);
    assert_eq!(ran, ["workspace.rename"], "untouched menus keep defaults");
}

#[test]
fn replace_and_base_none_start_fresh() {
    let keymap = build(
        r#"
        prefix herdra replace {
            n tab.new
        }
        "#,
    );
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('t')]);
    assert!(ran.is_empty());
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('n')]);
    assert_eq!(ran, ["tab.new"]);

    let keymap = build(
        r#"
        base none
        ctrl+space leader {
            v pane.split.right
        }
        "#,
    );
    assert_eq!(
        keymap
            .menus
            .iter()
            .filter(|menu| menu.parent.is_some())
            .count(),
        1,
        "only the leader menu is reachable by keys"
    );
    let (stack, ran) = press(&keymap, &[ctrl('b')]);
    assert_eq!(stack, None);
    assert_eq!(ran, ["forward"]);
    let (_, ran) = press(&keymap, &[ctrl(' '), ch('v')]);
    assert_eq!(ran, ["pane.split.right"]);
}

#[test]
fn prefix_setting_moves_the_prefix_everywhere() {
    let keymap = build("base prefix=ctrl+a");
    assert_eq!(keymap.prefix, (KeyCode::Char('a'), KeyModifiers::CONTROL));
    let (stack, ran) = press(&keymap, &[ctrl('b')]);
    assert_eq!(stack, None);
    assert_eq!(ran, ["forward"]);
    let (_, ran) = press(&keymap, &[ctrl('a'), ctrl('a')]);
    assert_eq!(ran, ["literal"]);
    let (_, ran) = press(&keymap, &[ctrl('a'), ch('t'), ch('n')]);
    assert_eq!(ran, ["tab.new"]);
}

#[test]
fn named_menus_can_be_opened_from_anywhere() {
    let keymap = build(
        r#"
        ctrl+space menu.open herdra
        prefix {
            R menu.open resize
        }
        "#,
    );
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    let (stack, _) = press(&keymap, &[ctrl(' ')]);
    assert_eq!(titles(&keymap, stack), ["herdra"]);
    let (stack, ran) = press(
        &keymap,
        &[
            ctrl('b'),
            TerminalKey::new(KeyCode::Char('r'), KeyModifiers::SHIFT),
            ch('h'),
        ],
    );
    assert_eq!(titles(&keymap, stack), ["herdra", "resize"]);
    assert_eq!(ran, ["pane.resize.left"]);

    let keymap = build("prefix { m menu.open nowhere }");
    assert_eq!(
        keymap.diagnostics,
        ["keymap: ctrl+b m opens unknown menu id \"nowhere\""]
    );
}

#[test]
fn diagnostics_name_the_line_and_keep_the_rest() {
    let keymap = build(
        "prefix {\n    t {\n        n tab.nwe\n        c tab.new\n        c tab.close\n    }\n}\na pane.zoom\nctrl+; pane.zoom\n",
    );
    assert_eq!(
        keymap.diagnostics,
        [
            "keymap keymap.kdl:3: unknown action \"tab.nwe\"; did you mean tab.new?",
            "keymap keymap.kdl:5: c is already bound on line 4; keeping the first binding",
            "keymap keymap.kdl:8: a at the top level would intercept typing in panes; add a modifier or bind it inside a menu",
            "keymap keymap.kdl:9: incomplete key chord \"ctrl+\"; quote chords that contain ; # = [ ] { } / or \\, for example \"ctrl+;\"",
        ]
    );
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('t'), ch('c')]);
    assert_eq!(ran, ["tab.new"]);

    let broken = build("prefix {");
    assert_eq!(broken.diagnostics.len(), 1, "{:?}", broken.diagnostics);
    assert!(broken.diagnostics[0].contains("ignoring this file"));
    let (_, ran) = press(&broken, &[ctrl('b'), ch('t'), ch('n')]);
    assert_eq!(ran, ["tab.new"], "a broken file keeps the defaults");
}

#[test]
fn indexed_targets_need_digit_chords() {
    let keymap = build("prefix { x tab.switch\n \"1..9\" tab.new }");
    assert_eq!(
        keymap.diagnostics,
        [
            "keymap keymap.kdl:1: tab.switch needs a 1..9 chord or a single digit",
            "keymap keymap.kdl:2: 1..9 can only run workspace.switch, tab.switch, or agent.focus",
        ]
    );
}

#[test]
fn single_digit_chords_target_one_index() {
    let keymap = build("ctrl+alt+3 agent.focus");
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    let (_, ran) = press(
        &keymap,
        &[TerminalKey::new(
            KeyCode::Char('3'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        )],
    );
    assert_eq!(ran, ["agent.focus:2"]);
}

#[test]
fn view_actions_outside_their_view_are_reported() {
    let keymap = build("prefix { x copy.yank }");
    assert_eq!(
        keymap.diagnostics,
        ["keymap: ctrl+b x runs copy.yank, which only works inside a menu with view=copy"]
    );
}

#[test]
fn plugins_extend_menus_but_never_take_existing_keys() {
    let plugin = KeymapText {
        source: "example.layout/keymap.kdl".to_owned(),
        text: r#"
        prefix {
            p {
                g plugin apply layout
                v plugin steal
            }
            L layout {
                a plugin apply
            }
        }
        "#
        .to_owned(),
    };
    let keymap = CompiledKeymap::build(None, &[("example.layout".to_owned(), plugin)]);
    assert_eq!(
        keymap.conflicts,
        ["keymap example.layout/keymap.kdl:5: v is already bound by builtin; keeping that binding"]
    );
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('p'), ch('g')]);
    assert_eq!(ran, ["plugin example.layout.apply"]);
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('p'), ch('v')]);
    assert_eq!(ran, ["pane.split.right"]);
    let shift_l = TerminalKey::new(KeyCode::Char('l'), KeyModifiers::SHIFT);
    let (_, ran) = press(&keymap, &[ctrl('b'), shift_l, ch('a')]);
    assert_eq!(ran, ["plugin example.layout.apply"]);

    let user_override = user("prefix { p { v plugin example.layout.apply } }");
    let keymap = CompiledKeymap::build(
        Some(&user_override),
        &[(
            "example.layout".to_owned(),
            KeymapText {
                source: "p".to_owned(),
                text: String::new(),
            },
        )],
    );
    let (_, ran) = press(&keymap, &[ctrl('b'), ch('p'), ch('v')]);
    assert_eq!(
        ran,
        ["plugin example.layout.apply"],
        "users may take any key"
    );
}

#[test]
fn bar_plans_show_exit_first_submenus_and_help_last() {
    let keymap = CompiledKeymap::default();
    let herdra = keymap.menu(keymap.menu_by_path("ctrl+b").expect("main menu"));
    let segments = herdra
        .bar_plan
        .segments
        .iter()
        .map(|segment| format!("{} {}", segment.keys, segment.label))
        .collect::<Vec<_>>();
    assert_eq!(
        segments,
        [
            "esc cancel",
            "w +workspace",
            "t +tab",
            "p +pane",
            "a +agent",
            "s +session",
            "ctrl+b send prefix",
            "g navigator",
            "b sidebar",
            "? keybinds",
        ]
    );
    assert_eq!(herdra.badge, "HERDRA");

    let pane = keymap.menu(keymap.menu_by_path("ctrl+b p").expect("pane menu"));
    let segments = pane
        .bar_plan
        .segments
        .iter()
        .take(5)
        .map(|segment| format!("{} {}", segment.keys, segment.label))
        .collect::<Vec<_>>();
    assert_eq!(
        segments,
        [
            "esc cancel",
            "r +resize",
            "y +copy",
            "h/j/k/l focus",
            "H/J/K/L swap"
        ]
    );

    let resize = keymap.menu(keymap.menu_by_path("ctrl+b p r").expect("resize"));
    assert_eq!(resize.badge, "HERDRA › PANE › RESIZE");
    assert!(resize
        .bar_plan
        .segments
        .iter()
        .skip(1)
        .all(|segment| segment.kind == SegmentKind::Sticky || segment.kind == SegmentKind::Action));

    let copy = keymap.menu(keymap.menu_with_view(ViewKind::Copy).expect("copy"));
    assert_eq!(copy.badge, "COPY");

    let classic = build("base classic");
    let prefix = classic.menu(classic.menu_by_path("ctrl+b").expect("prefix"));
    let segments = prefix
        .bar_plan
        .segments
        .iter()
        .map(|segment| format!("{} {}", segment.keys, segment.label))
        .collect::<Vec<_>>();
    assert_eq!(
        segments,
        [
            "esc cancel",
            "w +workspace nav",
            "ctrl+b send prefix",
            "? keybinds"
        ]
    );
    let resize = classic.menu(classic.menu_by_path("ctrl+b r").expect("resize"));
    let segments = resize
        .bar_plan
        .segments
        .iter()
        .map(|segment| format!("{} {}", segment.keys, segment.label))
        .collect::<Vec<_>>();
    assert_eq!(segments, ["esc done", "h/l width", "j/k height"]);
}

#[test]
fn keymap_docs_list_every_action_and_the_default_tree() {
    let docs = include_str!("../../../docs/next/website/src/content/docs/keymap.mdx");
    for entry in CATALOG {
        assert!(
            docs.contains(&format!("`{}`", entry.id)),
            "{} is missing from docs/next/website/src/content/docs/keymap.mdx",
            entry.id
        );
    }
    assert!(
        docs.contains(DEFAULT_KEYMAP.trim_end()),
        "keymap.mdx must include default.kdl verbatim"
    );
}

#[test]
fn shared_keymaps_hide_command_text() {
    let redacted = redact_commands(
        "prefix {\n    g popup \"lazygit --token secret\" git width=\"80%\"\n    r plugin example.run\n}\n",
    )
    .expect("valid KDL");
    assert!(!redacted.contains("secret"), "{redacted}");
    assert!(redacted.contains("example.run"), "{redacted}");
    let keymap = build(&redacted);
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    assert_eq!(keymap.commands[0].path_label, "ctrl+b g");
    assert_eq!(keymap.commands[0].hint.as_deref(), Some("git"));
    assert_eq!(redact_commands("prefix {"), None);
}

fn shift(c: char) -> TerminalKey {
    TerminalKey::new(KeyCode::Char(c), KeyModifiers::SHIFT)
}

#[test]
fn backspace_returns_to_the_menu_a_submenu_opened_from() {
    let keymap = CompiledKeymap::default();
    let (stack, _) = press(&keymap, &[ctrl('b'), ch('t'), key(KeyCode::Backspace)]);
    assert_eq!(titles(&keymap, stack), ["herdra"]);
    let (stack, _) = press(
        &keymap,
        &[ctrl('b'), ch('w'), ch('t'), key(KeyCode::Backspace)],
    );
    assert_eq!(titles(&keymap, stack), ["herdra", "workspace"]);
    let (stack, _) = press(&keymap, &[ctrl('b'), ch('p'), ch('r'), key(KeyCode::Esc)]);
    assert_eq!(stack, None, "esc still closes every menu");
}

#[test]
fn submenus_of_the_workspace_list_keep_the_list_open() {
    let keymap = CompiledKeymap::default();
    let (stack, _) = press(&keymap, &[ctrl('b'), ch('w'), ch('j'), ch('t')]);
    assert_eq!(titles(&keymap, stack), ["herdra", "workspace", "worktree"]);
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('w'), ch('j'), ch('t'), ch('n')]);
    assert_eq!(stack, None, "a leaf closes the menus it passed through");
    assert_eq!(ran, ["workspace.list.down", "worktree.new"]);
}

#[test]
fn one_digit_changes_leave_the_rest_of_the_range() {
    let keymap = build("prefix { t { \"5\" tab.close } }");
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    assert_eq!(
        press(&keymap, &[ctrl('b'), ch('t'), ch('5')]).1,
        ["tab.close:4"]
    );
    assert_eq!(
        press(&keymap, &[ctrl('b'), ch('t'), ch('3')]).1,
        ["tab.switch:2"]
    );

    let keymap = build("prefix { t { \"9\" none } }");
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    assert!(press(&keymap, &[ctrl('b'), ch('t'), ch('9')]).1.is_empty());
    assert_eq!(
        press(&keymap, &[ctrl('b'), ch('t'), ch('2')]).1,
        ["tab.switch:1"]
    );

    let keymap = build("prefix { g { \"1..9\" tab.switch; \"5\" tab.close } }");
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    assert_eq!(
        press(&keymap, &[ctrl('b'), ch('g'), ch('5')]).1,
        ["tab.close:4"]
    );
    assert_eq!(
        press(&keymap, &[ctrl('b'), ch('g'), ch('4')]).1,
        ["tab.switch:3"]
    );
}

#[test]
fn overlays_can_name_and_replace_default_menus() {
    let keymap = build(
        r#"
        prefix { p id=panes {} }
        ctrl+alt+p menu.open panes
        "#,
    );
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    let ctrl_alt_p = TerminalKey::new(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    );
    assert_eq!(
        press(&keymap, &[ctrl_alt_p, ch('h')]).1,
        ["pane.focus.left"]
    );

    let keymap = build(
        r#"
        prefix { p { r replace { h pane.resize.left } } }
        alt+r menu.open resize
        "#,
    );
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    let (stack, ran) = press(
        &keymap,
        &[ctrl('b'), ch('p'), ch('r'), ch('h'), ch('l'), ch('h')],
    );
    assert_eq!(
        titles(&keymap, stack),
        ["herdra", "pane", "resize"],
        "replace keeps the menu sticky"
    );
    assert_eq!(ran, ["pane.resize.left", "ignore", "pane.resize.left"]);
    let alt_r = TerminalKey::new(KeyCode::Char('r'), KeyModifiers::ALT);
    assert_eq!(press(&keymap, &[alt_r, ch('h')]).1, ["pane.resize.left"]);
}

#[test]
fn named_menus_stay_reachable_after_their_key_is_rebound() {
    let keymap = build("base classic\nprefix { r app.reload; R menu.open resize }");
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    assert_eq!(press(&keymap, &[ctrl('b'), ch('r')]).1, ["app.reload"]);
    let (stack, ran) = press(&keymap, &[ctrl('b'), shift('r'), ch('h')]);
    assert_eq!(titles(&keymap, stack), ["prefix", "resize"]);
    assert_eq!(ran, ["pane.resize.left"]);
}

#[test]
fn fallthrough_menus_use_the_keys_of_the_menu_below() {
    let keymap = build("base classic\nprefix { c none; t tab.new }");
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('w'), ch('t')]);
    assert_eq!(
        stack, None,
        "a prefix leaf leaves navigate like it did before"
    );
    assert_eq!(ran, ["tab.new"]);
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('w'), ch('c')]);
    assert_eq!(titles(&keymap, stack), ["prefix", "navigate"]);
    assert_eq!(
        ran,
        ["ignore"],
        "unbound prefix keys stay unbound in navigate"
    );
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('w'), ch('j')]);
    assert_eq!(titles(&keymap, stack), ["prefix", "navigate"]);
    assert_eq!(ran, ["pane.focus.down"], "navigate's own keys win");
    let (stack, _) = press(&keymap, &[ctrl('b'), ch('w'), ch('r')]);
    assert_eq!(titles(&keymap, stack), ["prefix", "resize"]);
}

#[test]
fn ctrl_bracket_is_escape_inside_menus_only() {
    let keymap = CompiledKeymap::default();
    assert_eq!(press(&keymap, &[ctrl('b'), ctrl('[')]).0, None);
    let (stack, _) = press(&keymap, &[ctrl('b'), ch('p'), ch('r'), ctrl('[')]);
    assert_eq!(stack, None, "sticky menus close too");
    assert_eq!(press(&keymap, &[ctrl('[')]).1, ["forward"]);
}

#[test]
fn a_prefix_without_modifiers_keeps_the_whole_tree() {
    let keymap = build("base classic prefix=\"`\"");
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    assert!(keymap.conflicts.is_empty(), "{:?}", keymap.conflicts);
    assert_eq!(press(&keymap, &[ch('`'), ch('c')]).1, ["tab.new"]);
    assert_eq!(press(&keymap, &[ch('`'), ch('`')]).1, ["literal"]);
    assert_eq!(press(&keymap, &[ch('a')]).1, ["forward"]);
}

#[test]
fn built_in_keys_give_way_to_a_custom_prefix() {
    let keymap = build("base prefix=ctrl+a");
    let (stack, ran) = press(&keymap, &[ctrl('a'), ch('p'), ch('y'), ctrl('b')]);
    assert_eq!(titles(&keymap, stack), ["copy"]);
    assert_eq!(ran, ["copy.page.up"]);

    let keymap = CompiledKeymap::default();
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('p'), ch('y'), ctrl('b')]);
    assert_eq!(titles(&keymap, stack), ["copy", "herdra"]);
    assert!(
        ran.is_empty(),
        "with the default prefix, ctrl+b opens the menu"
    );
}

#[test]
fn keys_match_by_the_character_they_type_when_the_code_differs() {
    // A layout where shift+7 types "/": kitty reports the physical key and
    // the generated text, and the copy menu binds "/".
    let keymap = CompiledKeymap::default();
    let mut slash = TerminalKey::new(KeyCode::Char('7'), KeyModifiers::SHIFT);
    slash.generated_text = Some("/".to_owned());
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('p'), ch('y'), slash]);
    assert_eq!(titles(&keymap, stack), ["copy"]);
    assert_eq!(ran, ["copy.search.forward"]);
}
