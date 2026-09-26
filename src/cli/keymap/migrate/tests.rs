use crossterm::event::{KeyCode, KeyModifiers};

use super::*;
use crate::input::keymap::{
    binding_action_id, resolve, CompiledBinding, CompiledTarget, Effect, MenuStack, Step, ViewKind,
};
use crate::input::TerminalKey;

fn legacy(text: &str) -> toml::Table {
    toml::from_str(text).expect("legacy [keys] table")
}

fn build(kdl: &str) -> CompiledKeymap {
    CompiledKeymap::build(
        Some(&KeymapText {
            source: "keymap.kdl".to_owned(),
            text: kdl.to_owned(),
        }),
        &[],
    )
}

/// Convert `text`, build the result, and require that Herdr loads it cleanly.
fn migrated(text: &str) -> (LegacyMigration, CompiledKeymap) {
    let migration = convert_legacy_keys(&legacy(text));
    let keymap = build(&migration.kdl);
    assert!(
        keymap.diagnostics.is_empty(),
        "{}\n{:?}",
        migration.kdl,
        keymap.diagnostics
    );
    assert!(
        keymap.conflicts.is_empty(),
        "{}\n{:?}",
        migration.kdl,
        keymap.conflicts
    );
    (migration, keymap)
}

fn key(combo: KeyCombo) -> TerminalKey {
    TerminalKey::new(combo.0, combo.1)
}

fn ch(c: char) -> TerminalKey {
    TerminalKey::new(KeyCode::Char(c), KeyModifiers::empty())
}

fn shift(c: char) -> TerminalKey {
    TerminalKey::new(KeyCode::Char(c), KeyModifiers::SHIFT)
}

fn ctrl(c: char) -> TerminalKey {
    TerminalKey::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn alt(c: char) -> TerminalKey {
    TerminalKey::new(KeyCode::Char(c), KeyModifiers::ALT)
}

/// Press keys from a pane; return the open menus and what ran.
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
        CompiledTarget::Action(action) => {
            let id = binding_action_id(binding).unwrap_or("?");
            let indexed = matches!(action, CatalogAction::Indexed(_));
            match binding.digit_index(index).filter(|_| indexed) {
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

#[derive(Debug)]
enum Expected {
    /// The last key runs this (`tab.switch:2` for indexed actions).
    Runs(String),
    /// The last key opens the menu with this title.
    Opens(String),
    /// The last key runs this and leaves navigate mode open.
    RunsInNavigate(String),
}

struct Expectation {
    what: String,
    keys: Vec<TerminalKey>,
    expected: Expected,
}

/// How old Herdr resolved a `[keys]` table, written independently of the
/// converter from the `LEGACY_FIELDS` tables: fields the user set take
/// their keys first, then `[keys.indexed]`, then commands, then the defaults
/// of unset fields on the keys that are left. Test configs avoid the
/// reserved keys and invalid values that the converter reports as notes.
struct LegacyModel {
    prefix: KeyCombo,
    taken: Vec<(bool, KeyCombo)>,
    expectations: Vec<Expectation>,
}

impl LegacyModel {
    fn claim(&mut self, what: &str, value: &str, target: &str) {
        let (in_prefix, text) = match value.strip_prefix("prefix+") {
            Some(rest) => (true, rest),
            None => (false, value),
        };
        let chord = Chord::parse(text).expect("test binding parses");
        for combo in combos(chord) {
            if combo == self.prefix || self.taken.contains(&(in_prefix, combo)) {
                continue;
            }
            self.taken.push((in_prefix, combo));
            let mut keys = Vec::new();
            if in_prefix {
                keys.push(key(self.prefix));
            }
            keys.push(key(combo));
            let expected = if let Some(menu) = target.strip_prefix('@') {
                Expected::Opens(menu.to_owned())
            } else if Target::field(static_target(target)).is_indexed() {
                let KeyCode::Char(digit) = combo.0 else {
                    panic!("indexed binding on a non-digit")
                };
                Expected::Runs(format!("{target}:{}", digit as usize - '1' as usize))
            } else {
                Expected::Runs(target.to_owned())
            };
            self.expectations.push(Expectation {
                what: format!("{what} = {value:?}"),
                keys,
                expected,
            });
        }
    }
}

fn static_target(target: &str) -> &'static str {
    LEGACY_FIELDS
        .iter()
        .map(|(_, target, _)| *target)
        .chain(LEGACY_INDEXED.iter().map(|(_, _, action)| *action))
        .find(|known| *known == target)
        .unwrap_or("?")
}

fn strings(value: Option<&toml::Value>) -> Option<Vec<String>> {
    value.map(|value| match value {
        toml::Value::String(value) => vec![value.clone()],
        toml::Value::Array(values) => values
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect(),
        other => panic!("unexpected binding value {other:?}"),
    })
}

fn legacy_expectations(keys: &toml::Table, prefix: KeyCombo) -> Vec<Expectation> {
    let mut model = LegacyModel {
        prefix,
        taken: Vec::new(),
        expectations: Vec::new(),
    };
    let configured = |field: &str| {
        strings(
            keys.get(field)
                .or_else(|| (field == "zoom").then(|| keys.get("fullscreen")).flatten()),
        )
    };
    for (field, target, _) in LEGACY_FIELDS {
        for value in configured(field).unwrap_or_default() {
            model.claim(field, &value, target);
        }
    }
    let indexed = keys.get("indexed").and_then(toml::Value::as_table);
    let mut displaced = Vec::new();
    for (name, field, action) in LEGACY_INDEXED {
        if let Some(modifiers) = indexed.and_then(|indexed| indexed.get(*name)) {
            let modifiers = modifiers.as_str().expect("modifiers");
            displaced.push(*field);
            model.claim(
                &format!("indexed.{name}"),
                &format!("{modifiers}+1..9"),
                action,
            );
        }
    }
    let commands = keys
        .get("command")
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut command_claims = Vec::new();
    for command in &commands {
        let command = command.as_table().expect("command table");
        let text = command["command"].as_str().expect("command text");
        let kind = match command.get("type").and_then(toml::Value::as_str) {
            None | Some("shell") => "shell",
            Some("plugin_action") => "plugin",
            Some(other) => other,
        };
        for value in strings(command.get("key")).unwrap_or_default() {
            command_claims.push((value, format!("{kind} {text}")));
        }
    }
    for (value, runs) in command_claims {
        let before = model.expectations.len();
        model.claim("command", &value, "tab.new");
        for expectation in &mut model.expectations[before..] {
            expectation.expected = Expected::Runs(runs.clone());
        }
    }
    for (field, target, defaults) in LEGACY_FIELDS {
        if configured(field).is_some() || displaced.contains(field) {
            continue;
        }
        for value in *defaults {
            model.claim(field, value, target);
        }
    }

    // Navigate mode, entered through the first key that opens it.
    let navigate_entry = model.expectations.iter().find_map(|expectation| {
        matches!(&expectation.expected, Expected::Opens(menu) if menu == "navigate")
            .then(|| expectation.keys.clone())
    });
    if let Some(entry) = navigate_entry {
        let mut taken: Vec<KeyCombo> = Vec::new();
        let mut navigate = Vec::new();
        for pass_configured in [true, false] {
            for (field, target, default, _) in LEGACY_NAVIGATE_FIELDS {
                let values = match (configured(field), pass_configured) {
                    (Some(values), true) => values,
                    (None, false) => vec![(*default).to_owned()],
                    _ => continue,
                };
                for value in values {
                    let Ok(Chord::Key(combo)) = Chord::parse(&value) else {
                        panic!("navigate key {value:?}")
                    };
                    if combo == prefix || taken.contains(&combo) {
                        continue;
                    }
                    taken.push(combo);
                    let mut keys = entry.clone();
                    keys.push(key(combo));
                    navigate.push(Expectation {
                        what: format!("{field} = {value:?}"),
                        keys,
                        expected: Expected::RunsInNavigate((*target).to_owned()),
                    });
                }
            }
        }
        model.expectations.extend(navigate);
    }
    model.expectations
}

/// Migrate `text` and check that every field, command, and navigate key
/// resolves where old Herdr put it.
fn assert_matches_legacy(case: &str, text: &str) -> (LegacyMigration, CompiledKeymap) {
    let (migration, keymap) = migrated(text);
    let expectations = legacy_expectations(&legacy(text), keymap.prefix);
    assert!(!expectations.is_empty(), "{case}: no expectations");
    for expectation in expectations {
        let (stack, ran) = press(&keymap, &expectation.keys);
        let titles = titles(&keymap, stack);
        let context = format!(
            "{case}: {} pressed {:?}, ran {ran:?}, open {titles:?}\n{}",
            expectation.what,
            expectation
                .keys
                .iter()
                .map(|key| (key.code, key.modifiers))
                .collect::<Vec<_>>(),
            migration.kdl
        );
        match &expectation.expected {
            Expected::Runs(runs) => {
                assert_eq!(ran.last(), Some(runs), "{context}");
            }
            Expected::Opens(menu) => {
                assert_eq!(titles.last(), Some(menu), "{context}");
            }
            Expected::RunsInNavigate(runs) => {
                assert_eq!(ran.last(), Some(runs), "{context}");
                assert_eq!(
                    titles.last().map(String::as_str),
                    Some("navigate"),
                    "{context}"
                );
            }
        }
    }
    (migration, keymap)
}

const HEADER: &str = "// Converted from [keys] in config.toml by `herdr keymap migrate`.\n// It starts from Herdr's classic layout and applies your changes.\n";

#[test]
fn migrated_keymaps_resolve_every_legacy_binding_where_herdr_did() {
    let cases: &[(&str, &str)] = &[
        ("defaults", ""),
        ("rebound prefix", r#"prefix = "ctrl+a""#),
        (
            "moved action frees its key for a command",
            r#"
next_tab = "alt+n"
[[command]]
key = "prefix+n"
command = "notes"
"#,
        ),
        (
            "moved range frees a digit for a command",
            r#"
switch_tab = "alt+1..9"
[[command]]
key = "prefix+1"
type = "pane"
command = "htop"
"#,
        ),
        (
            "equivalent spellings",
            r#"
split_horizontal = "prefix+-"
new_workspace = "prefix+N"
reload_config = "prefix+R"
close_tab = ["prefix+X"]
cycle_pane_previous = "prefix+shift+tab"
"#,
        ),
        (
            "digits over the classic range",
            r#"
help = "prefix+5"
[[command]]
key = "prefix+1"
command = "one"
"#,
        ),
        (
            "moved menus give their keys to actions",
            r#"
resize_mode = "prefix+shift+r"
reload_config = "prefix+r"
copy_mode = "prefix+v"
split_vertical = "prefix+["
workspace_picker = "prefix+space"
close_pane = "prefix+w"
navigate_pane_left = "ctrl+h"
navigate_workspace_down = ["down", "n"]
"#,
        ),
        (
            "navigate on a direct chord",
            r#"
workspace_picker = "alt+w"
navigate_workspace_down = "j"
"#,
        ),
        (
            "navigate keeps its key and gains another",
            r#"
workspace_picker = ["prefix+w", "alt+w"]
navigate_pane_up = "u"
"#,
        ),
        (
            "prefix actions used from navigate mode",
            r#"
new_tab = "prefix+t"
close_tab = "prefix+x"
close_pane = "prefix+shift+x"
[[command]]
key = "prefix+g"
type = "popup"
command = "lazygit"
"#,
        ),
        ("backtick prefix", "prefix = \"`\""),
        ("minus prefix", r#"prefix = "-""#),
        ("esc prefix", r#"prefix = "esc""#),
        (
            "rich config",
            r#"
prefix = "ctrl+space"
help = ["prefix+?", "f1"]
new_tab = "prefix+t"
next_tab = ["prefix+n", "alt+shift+l"]
previous_tab = ["prefix+p", "alt+shift+h"]
split_horizontal = ["prefix+minus", "prefix+_"]
split_vertical = ["prefix+v", "prefix+|"]
focus_agent = "prefix+alt+1..9"
last_pane = "alt+tab"
fullscreen = "prefix+f"
remote_image_paste = "ctrl+shift+v"
navigate_workspace_up = ["up", "shift+k"]

[indexed]
tabs = "ctrl"
workspaces = "alt"

[[command]]
key = "prefix+g"
type = "popup"
command = "lazygit"
description = "git"
width = "80%"
height = 30

[[command]]
key = ["alt+shift+t", "prefix+shift+y"]
type = "pane"
command = "htop"
"#,
        ),
    ];
    for (case, text) in cases {
        assert_matches_legacy(case, text);
    }
}

#[test]
fn unchanged_keys_migrate_to_the_bare_classic_base() {
    for text in [
        "",
        r#"prefix = "ctrl+b""#,
        r#"
split_horizontal = "prefix+-"
new_workspace = "prefix+N"
reload_config = "prefix+R"
switch_tab = "prefix+1..9"
cycle_pane_previous = "prefix+shift+tab"
workspace_picker = "prefix+w"
navigate_pane_left = "h"
"#,
    ] {
        let (migration, _) = migrated(text);
        assert_eq!(migration.kdl, format!("{HEADER}base classic\n"), "{text}");
        assert!(migration.notes.is_empty(), "{:?}", migration.notes);
    }
}

#[test]
fn migration_rebuilds_old_keys_on_the_classic_base() {
    let (migration, keymap) = migrated(
        r#"
prefix = "ctrl+a"
new_tab = "prefix+t"
next_tab = ["prefix+n", "alt+shift+l"]
split_horizontal = ["prefix+minus", "prefix+_"]
navigate_pane_left = "ctrl+h"
remote_image_paste = ""

[indexed]
tabs = "ctrl"

[[command]]
key = "prefix+g"
type = "popup"
command = "lazygit"
description = "lazygit"
width = "80%"
"#,
    );
    assert_eq!(
        migration.kdl,
        format!(
            r#"{HEADER}base classic prefix="ctrl+a"

prefix {{
    "1..9" none
    "c" none
    "t" tab.new
    "_" pane.split.down
    "g" popup "lazygit" hint="lazygit" width="80%"
    "w" {{
        "h" none
        "ctrl+h" pane.focus.left hidden stay
    }}
}}

"ctrl+1..9" tab.switch
"alt+shift+l" tab.next
"#
        )
    );
    assert_eq!(migration.image_paste_key.as_deref(), Some(""));
    assert_eq!(keymap.prefix, (KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert_eq!(keymap.commands[0].path_label, "ctrl+a g");
}

#[test]
fn a_rebound_default_key_is_bound_once_and_never_unbound() {
    // next_tab moved away and a command took prefix+n.
    let (migration, keymap) = migrated(
        r#"
next_tab = "alt+n"
[[command]]
key = "prefix+n"
command = "notes"
"#,
    );
    assert!(!migration.kdl.contains("\"n\" none"), "{}", migration.kdl);
    assert_eq!(press(&keymap, &[ctrl('b'), ch('n')]).1, ["shell notes"]);
    assert_eq!(press(&keymap, &[alt('n')]).1, ["tab.next"]);

    // switch_tab moved to alt and a command took prefix+1.
    let (migration, keymap) = migrated(
        r#"
switch_tab = "alt+1..9"
[[command]]
key = "prefix+1"
command = "one"
"#,
    );
    assert!(
        migration
            .kdl
            .contains("prefix {\n    \"1..9\" none\n    \"1\" shell \"one\"\n}"),
        "{}",
        migration.kdl
    );
    assert_eq!(press(&keymap, &[ctrl('b'), ch('1')]).1, ["shell one"]);
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('2')]);
    assert!(stack.is_none() && ran.is_empty(), "{ran:?}");
    assert_eq!(press(&keymap, &[alt('9')]).1, ["tab.switch:8"]);

    // A single digit over the classic range: only that digit is written.
    let (migration, keymap) = migrated(
        r#"
[[command]]
key = "prefix+1"
command = "one"
"#,
    );
    assert_eq!(
        migration.kdl,
        format!("{HEADER}base classic\n\nprefix {{\n    \"1\" shell \"one\"\n}}\n")
    );
    assert_eq!(press(&keymap, &[ctrl('b'), ch('2')]).1, ["tab.switch:1"]);
}

#[test]
fn later_claims_on_a_taken_key_are_reported_not_written() {
    let (migration, keymap) = migrated(
        r#"
help = "prefix+c"
new_tab = "prefix+c"
[[command]]
key = "prefix+?"
command = "ask"
"#,
    );
    assert!(
        migration
            .notes
            .iter()
            .any(|note| note.contains("keys.new_tab: prefix+c is already taken by keys.help")),
        "{:?}",
        migration.notes
    );
    assert_eq!(press(&keymap, &[ctrl('b'), ch('c')]).1, ["app.help"]);
    // help left `?`, so the command takes it; new_tab has no key left.
    assert_eq!(press(&keymap, &[ctrl('b'), ch('?')]).1, ["shell ask"]);
}

#[test]
fn keys_old_herdr_refused_are_reported_not_written() {
    let (migration, _) = migrated(
        r#"
prefix = "ctrl+a"
new_tab = ["prefix+ctrl+a", "t", "prefix+esc", "prefix+bogus+key"]
split_vertical = "prefix+1..9"
navigate_pane_left = ["prefix+h", "enter", "esc"]
"#,
    );
    for expected in [
        "prefix+ctrl+a after the prefix sends the prefix key to the pane",
        "would intercept typing in panes",
        "prefix+esc after the prefix cancels",
        "invalid key chord",
        "only switch_tab, switch_workspace, and focus_agent take 1..9",
        "navigate keys are pressed without the prefix",
        "enter is reserved in navigate mode",
        "esc is reserved in navigate mode",
    ] {
        assert!(
            migration.notes.iter().any(|note| note.contains(expected)),
            "missing {expected:?} in {:?}",
            migration.notes
        );
    }
}

#[test]
fn moved_menus_stay_reachable_through_their_ids() {
    let (migration, keymap) = assert_matches_legacy(
        "swap",
        r#"
resize_mode = "prefix+shift+r"
reload_config = "prefix+r"
copy_mode = "prefix+v"
split_vertical = "prefix+["
"#,
    );
    assert!(
        migration.kdl.contains("\"R\" menu.open resize"),
        "{}",
        migration.kdl
    );
    assert!(
        migration.kdl.contains("\"v\" menu.open copy"),
        "{}",
        migration.kdl
    );
    let (stack, _) = press(&keymap, &[ctrl('b'), shift('r')]);
    assert_eq!(titles(&keymap, stack), ["prefix", "resize"]);
    let (stack, ran) = press(&keymap, &[ctrl('b'), shift('r'), ch('h'), ch('h')]);
    assert_eq!(
        titles(&keymap, stack),
        ["prefix", "resize"],
        "resize is sticky"
    );
    assert_eq!(ran, ["pane.resize.left", "pane.resize.left"]);
    let (stack, _) = press(
        &keymap,
        &[
            ctrl('b'),
            shift('r'),
            key((KeyCode::Esc, KeyModifiers::empty())),
        ],
    );
    assert_eq!(stack, None);
    assert_eq!(press(&keymap, &[ctrl('b'), ch('r')]).1, ["app.reload"]);

    let (stack, _) = press(&keymap, &[ctrl('b'), ch('v')]);
    assert_eq!(titles(&keymap, stack), ["copy"], "copy mode is a mode");
    assert_eq!(
        press(&keymap, &[ctrl('b'), ch('[')]).1,
        ["pane.split.right"]
    );
}

#[test]
fn moved_navigate_is_one_menu_at_its_new_key() {
    let (migration, keymap) = assert_matches_legacy(
        "navigate moved",
        r#"
workspace_picker = "prefix+space"
close_pane = "prefix+w"
navigate_pane_left = "ctrl+h"
"#,
    );
    let navigate_menus = keymap
        .menus
        .iter()
        .filter(|menu| menu.view == Some(ViewKind::WorkspaceList))
        .count();
    assert_eq!(navigate_menus, 1, "{}", migration.kdl);
    assert!(!migration.kdl.contains("\"w\" {"), "{}", migration.kdl);
    assert!(
        migration.kdl.contains("\"w\" pane.close"),
        "{}",
        migration.kdl
    );
    assert!(
        migration.kdl.contains("id=navigate"),
        "the copy keeps its id: {}",
        migration.kdl
    );

    let space = ch(' ');
    let (stack, _) = press(&keymap, &[ctrl('b'), space.clone()]);
    assert_eq!(titles(&keymap, stack), ["prefix", "navigate"]);
    let (stack, ran) = press(&keymap, &[ctrl('b'), space.clone(), ctrl('h')]);
    assert_eq!(ran, ["pane.focus.left"]);
    assert_eq!(titles(&keymap, stack), ["prefix", "navigate"]);
    // Its new key closes it, as `w` did.
    let (stack, ran) = press(&keymap, &[ctrl('b'), space.clone(), space.clone()]);
    assert_eq!(stack, None);
    assert!(ran.is_empty(), "{ran:?}");
    // `w` now falls through to the user's prefix binding.
    let (stack, ran) = press(&keymap, &[ctrl('b'), space, ch('w')]);
    assert_eq!(ran, ["pane.close"]);
    assert_eq!(stack, None);
    // The menu can be opened by id too.
    assert!(keymap
        .menus
        .iter()
        .any(|menu| menu.title == "navigate" && menu.fallthrough));
}

#[test]
fn navigate_mode_falls_back_to_changed_prefix_keys() {
    let (migration, keymap) = assert_matches_legacy(
        "navigate fallthrough",
        r#"
new_tab = "prefix+t"
close_tab = "prefix+x"
close_pane = "prefix+shift+x"
[[command]]
key = "prefix+g"
type = "popup"
command = "lazygit"
"#,
    );
    assert!(
        !migration.kdl.contains("\"w\" {"),
        "prefix changes are not copied into navigate: {}",
        migration.kdl
    );
    for (key, runs) in [
        (ch('t'), "tab.new"),
        (ch('x'), "tab.close"),
        (shift('x'), "pane.close"),
        (ch('g'), "popup lazygit"),
    ] {
        let (stack, ran) = press(&keymap, &[ctrl('b'), ch('w'), key]);
        assert_eq!(ran, [runs]);
        assert_eq!(stack, None, "{runs} closes navigate");
    }
    // A key the migration unbound does nothing in navigate mode.
    let (stack, ran) = press(&keymap, &[ctrl('b'), ch('w'), ch('c')]);
    assert_eq!(ran, ["ignore"]);
    assert_eq!(titles(&keymap, stack), ["prefix", "navigate"]);
}

#[test]
fn a_prefix_without_modifiers_migrates_to_a_working_keymap() {
    let (migration, keymap) = assert_matches_legacy("backtick", "prefix = \"`\"");
    assert_eq!(
        migration.kdl,
        format!("{HEADER}base classic prefix=\"`\"\n")
    );
    assert_eq!(press(&keymap, &[ch('`'), ch('c')]).1, ["tab.new"]);
    assert_eq!(press(&keymap, &[ch('`'), ch('`')]).1, ["literal"]);
    let (stack, _) = press(&keymap, &[ch('`'), ch('w'), ch('`')]);
    assert_eq!(stack, None, "the prefix closes navigate");

    let (_, keymap) = assert_matches_legacy("minus", r#"prefix = "-""#);
    assert_eq!(press(&keymap, &[ch('-'), ch('-')]).1, ["literal"]);
    assert_eq!(press(&keymap, &[ch('-'), ch('v')]).1, ["pane.split.right"]);

    // Moving the default that the prefix displaced must not unbind the
    // prefix inside its own menu.
    let (migration, keymap) = assert_matches_legacy(
        "minus with split moved",
        r#"
prefix = "-"
split_horizontal = "prefix+_"
"#,
    );
    assert!(!migration.kdl.contains("\"-\" none"), "{}", migration.kdl);
    assert_eq!(press(&keymap, &[ch('-'), ch('-')]).1, ["literal"]);
    assert_eq!(press(&keymap, &[ch('-'), ch('_')]).1, ["pane.split.down"]);

    let esc = key((KeyCode::Esc, KeyModifiers::empty()));
    let (_, keymap) = assert_matches_legacy("esc", r#"prefix = "esc""#);
    assert_eq!(press(&keymap, &[esc.clone(), ch('c')]).1, ["tab.new"]);
    assert_eq!(press(&keymap, &[esc.clone(), esc.clone()]).1, ["literal"]);
    let (stack, _) = press(&keymap, &[esc.clone(), ch('w'), esc]);
    assert_eq!(stack, None, "esc closes navigate");

    // Navigate moved under a modifier-free prefix: the copy drops its own
    // key for the prefix, as the built-in menu does.
    migrated(
        r#"
prefix = "esc"
workspace_picker = "prefix+space"
"#,
    );
}

#[test]
fn direct_navigate_keys_are_noted() {
    let (migration, keymap) = assert_matches_legacy(
        "direct navigate",
        r#"
workspace_picker = "alt+w"
navigate_workspace_down = "j"
"#,
    );
    assert!(
        migration
            .notes
            .iter()
            .any(|note| note.contains("alt+w does not fall back to prefix keys")),
        "{:?}",
        migration.notes
    );
    let navigate_menus = keymap
        .menus
        .iter()
        .filter(|menu| menu.view == Some(ViewKind::WorkspaceList))
        .count();
    assert_eq!(navigate_menus, 1, "{}", migration.kdl);
    let (stack, ran) = press(&keymap, &[alt('w'), ch('j')]);
    assert_eq!(ran, ["workspace.list.down"]);
    assert_eq!(titles(&keymap, stack), ["navigate"]);
}

#[test]
fn navigate_overrides_without_a_navigate_key_are_noted() {
    let (migration, _) = migrated(
        r#"
workspace_picker = []
navigate_pane_left = "a"
"#,
    );
    assert!(!migration.kdl.contains("\"a\""), "{}", migration.kdl);
    assert!(
        migration
            .notes
            .iter()
            .any(|note| note.contains("navigate mode has no key")),
        "{:?}",
        migration.notes
    );
}

#[test]
fn the_classic_navigate_menu_can_be_copied() {
    let copy = navigate_copy(
        DEFAULT_PREFIX,
        Scope::Prefix,
        (KeyCode::Char(' '), KeyModifiers::empty()),
        &ScopeDiff::default(),
    )
    .expect("classic navigate menu");
    assert!(copy.starts_with("\"space\" navigate "), "{copy}");
    assert!(copy.contains("id=navigate"), "{copy}");
    assert!(copy.contains("fallthrough"), "{copy}");
    assert!(copy.contains("\"space\" menu.cancel hidden"), "{copy}");
    assert!(!copy.contains("\"w\""), "{copy}");
}

#[test]
fn quoted_strings_read_back_exactly() {
    let mut samples: Vec<String> = [
        "plain",
        "",
        "say \"hi\" \\ bye",
        "a\nb\r\nc\td",
        "emoji 🦀 and ünïcode",
        "#{ } /* not a comment */ // nor this",
        "\u{1b}[31mred\u{1b}[0m",
        "rtl \u{202E}mark\u{2066}\u{2069}\u{FEFF}",
        "line\u{2028}sep\u{2029}\u{85}\u{0B}\u{0C}\u{08}",
    ]
    .iter()
    .map(|text| (*text).to_owned())
    .collect();
    samples.push((0u32..=0xa0).filter_map(char::from_u32).collect());
    for text in samples {
        let document = format!("node {}", quoted(&text));
        let parsed = document
            .parse::<kdl::KdlDocument>()
            .unwrap_or_else(|error| panic!("{text:?} as {document}: {error:?}"));
        let value = parsed.nodes()[0].entries()[0].value().as_string();
        assert_eq!(value, Some(text.as_str()), "{document}");
    }
}

#[test]
fn chords_are_written_so_they_read_back() {
    for text in [
        "a",
        "N",
        "ctrl+a",
        "alt+shift+l",
        "shift+tab",
        "ctrl+shift+tab",
        "-",
        "_",
        "plus",
        "ctrl+plus",
        "alt+plus",
        "space",
        "\"",
        "\\",
        "#",
        ";",
        "=",
        "[",
        "{",
        "`",
        "f5",
        "ctrl+f12",
        "home",
        "pageup",
        "delete",
        "1..9",
        "alt+1..9",
        "ctrl+shift+1..9",
        "?",
    ] {
        let chord = Chord::parse(text).expect(text);
        let written = chord_text(chord);
        assert_eq!(
            Chord::parse(&written),
            Ok(chord),
            "{text} written as {written}"
        );
        let keymap = build(&format!(
            "base none\nctrl+g {{\n    {} tab.new\n}}\n",
            quoted(&written)
        ));
        assert!(
            keymap.diagnostics.is_empty() || matches!(chord, Chord::Digits(_)),
            "{text}: {:?}",
            keymap.diagnostics
        );
    }
}

#[test]
fn commands_with_control_characters_survive_migration() {
    let command = "printf 'a\\nb' \"$HOME\"\n\u{1b}[0m\ttab \\ end";
    let description = "two\nlines \"quoted\"";
    let mut keys = toml::Table::new();
    let mut entry = toml::Table::new();
    entry.insert("key".to_owned(), toml::Value::String("prefix+y".to_owned()));
    entry.insert(
        "command".to_owned(),
        toml::Value::String(command.to_owned()),
    );
    entry.insert(
        "description".to_owned(),
        toml::Value::String(description.to_owned()),
    );
    keys.insert(
        "command".to_owned(),
        toml::Value::Array(vec![toml::Value::Table(entry)]),
    );
    let migration = convert_legacy_keys(&keys);
    let keymap = build(&migration.kdl);
    assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
    assert_eq!(keymap.commands.len(), 1, "{}", migration.kdl);
    assert_eq!(keymap.commands[0].spec.command, command);
    assert_eq!(keymap.commands[0].hint.as_deref(), Some(description));
}

#[test]
fn command_options_are_converted_or_reported() {
    let (migration, keymap) = migrated(
        r#"
[[command]]
key = "prefix+y"
type = "shell"
command = "sized"
width = 20

[[command]]
key = "prefix+u"
type = "popup"
command = "big"
width = "wide"
height = 200

[[command]]
key = "prefix+i"
type = "plugin_action"
command = "demo.run"

[[command]]
command = "keyless"

[[command]]
key = "prefix+o"
command = "   "
"#,
    );
    assert!(
        migration.kdl.contains("\"y\" shell \"sized\"\n"),
        "{}",
        migration.kdl
    );
    assert!(
        migration.kdl.contains("\"u\" popup \"big\" height=200\n"),
        "{}",
        migration.kdl
    );
    assert!(
        migration.kdl.contains("\"i\" plugin \"demo.run\"\n"),
        "{}",
        migration.kdl
    );
    for expected in [
        "keys.command[0]: width only applies to popup commands",
        "keys.command[1]: width = \"wide\" is not a cell count",
        "keys.command[3] (\"keyless\") has no key",
        "keys.command[4] has no command",
    ] {
        assert!(
            migration.notes.iter().any(|note| note.contains(expected)),
            "missing {expected:?} in {:?}",
            migration.notes
        );
    }
    assert_eq!(keymap.commands.len(), 3);
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "herdr-keymap-migrate-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn a_keymap_that_would_not_load_is_not_written() {
    let dir = scratch_dir("invalid");
    let config_path = dir.join("config.toml");
    let keymap_path = dir.join("keymap.kdl");
    let content = "[keys]\nnew_tab = \"prefix+t\"\n";
    std::fs::write(&config_path, content).expect("write config");
    let migration = LegacyMigration {
        kdl: "prefix {\n    \"t\" shell \"unterminated\n}\n".to_owned(),
        notes: Vec::new(),
        image_paste_key: Some("ctrl+v".to_owned()),
    };
    assert!(check_converted(&migration.kdl).is_err());
    let code =
        write_migration(&migration, &config_path, content, &keymap_path).expect("migration runs");
    assert_eq!(code, 1);
    assert!(!keymap_path.exists());
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("read config"),
        content
    );
    let entries = std::fs::read_dir(&dir).expect("read dir").count();
    assert_eq!(entries, 1, "no backup is made");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_converted_keymap_is_written_and_keys_are_removed() {
    let dir = scratch_dir("valid");
    let config_path = dir.join("config.toml");
    let keymap_path = dir.join("nested").join("keymap.kdl");
    let content = "[ui]\naccent = \"red\"\n\n[keys]\nnew_tab = \"prefix+t\"\nremote_image_paste = \"ctrl+shift+v\"\n";
    std::fs::write(&config_path, content).expect("write config");
    let migration = convert_legacy_keys(&legacy(
        "new_tab = \"prefix+t\"\nremote_image_paste = \"ctrl+shift+v\"",
    ));
    let code =
        write_migration(&migration, &config_path, content, &keymap_path).expect("migration runs");
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read_to_string(&keymap_path).expect("read keymap"),
        migration.kdl
    );
    let updated = std::fs::read_to_string(&config_path).expect("read config");
    let table = updated.parse::<toml::Table>().expect("valid toml");
    assert!(table.get("keys").is_none(), "{updated}");
    assert_eq!(
        table["remote"]["image_paste_key"].as_str(),
        Some("ctrl+shift+v")
    );
    assert_eq!(table["ui"]["accent"].as_str(), Some("red"));
    let _ = std::fs::remove_dir_all(&dir);
}
