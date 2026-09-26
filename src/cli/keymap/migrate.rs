//! `herdr keymap migrate`: convert `[keys]` in config.toml into a keymap.kdl
//! overlay on the classic base.
//!
//! Herdr used to resolve `[keys]` by letting chords be claimed in a fixed
//! order: the fields the user set, then `[keys.indexed]`, then
//! `[[keys.command]]`, then the defaults of the fields left unset. A later
//! claim on a chord that was already taken was disabled. The conversion
//! replays those claims and writes only what differs from the classic tree,
//! which holds the old defaults.

use std::fmt::Write as _;
use std::path::Path;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::config::KeyCombo;
use crate::input::keymap::{
    lookup_action, parse_document, CatalogAction, Chord, CompiledKeymap, KeymapText,
    CLASSIC_KEYMAP, DEFAULT_PREFIX,
};

#[cfg(test)]
mod tests;

pub(super) fn migrate(args: &[String]) -> std::io::Result<i32> {
    let dry_run = match args {
        [] => false,
        [flag] if flag == "--dry-run" => true,
        _ => return super::usage("herdr keymap migrate [--dry-run]"),
    };
    let config_path = crate::config::config_path();
    let content = match std::fs::read_to_string(&config_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "No config file at {}; nothing to migrate.",
                config_path.display()
            );
            return Ok(0);
        }
        Err(err) => return Err(err),
    };
    let table = match content.parse::<toml::Table>() {
        Ok(table) => table,
        Err(err) => {
            eprintln!(
                "config file at {} is invalid TOML: {err}; fix it before migrating",
                config_path.display()
            );
            return Ok(1);
        }
    };
    let Some(keys) = table.get("keys").and_then(toml::Value::as_table) else {
        println!(
            "No [keys] in {}; nothing to migrate.",
            config_path.display()
        );
        return Ok(0);
    };
    let migration = convert_legacy_keys(keys);
    if dry_run {
        print!("{}", migration.kdl);
        let loads = report_check(&migration);
        for note in &migration.notes {
            eprintln!("note: {note}");
        }
        return Ok(if loads { 0 } else { 1 });
    }

    let config = crate::config::Config::load().config;
    let keymap_path = crate::config::keymap_path(&config);
    if keymap_path.exists() {
        println!(
            "{} already exists; add these lines to it by hand, then remove [keys] from {}:\n",
            keymap_path.display(),
            config_path.display()
        );
        print!("{}", migration.kdl);
        report_check(&migration);
        for note in &migration.notes {
            eprintln!("note: {note}");
        }
        return Ok(1);
    }
    write_migration(&migration, &config_path, &content, &keymap_path)
}

/// Load the converted text the way Herdr would. `Err` holds the problems
/// when Herdr would ignore the whole file; `Ok` holds lines it would drop.
fn check_converted(kdl: &str) -> Result<Vec<String>, Vec<String>> {
    let mut problems = Vec::new();
    if parse_document(kdl, "keymap.kdl", &mut problems).is_none() {
        return Err(problems);
    }
    let keymap = CompiledKeymap::build(
        Some(&KeymapText {
            source: "keymap.kdl".to_owned(),
            text: kdl.to_owned(),
        }),
        &[],
    );
    if keymap
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("ignoring this file"))
    {
        return Err(keymap.diagnostics);
    }
    Ok(keymap.diagnostics)
}

/// Print what `check_converted` found. Returns whether the text would load.
fn report_check(migration: &LegacyMigration) -> bool {
    match check_converted(&migration.kdl) {
        Ok(warnings) => {
            for warning in &warnings {
                eprintln!("warning: {warning}");
            }
            true
        }
        Err(problems) => {
            eprintln!("error: Herdr would ignore this keymap:");
            for problem in &problems {
                eprintln!("  {problem}");
            }
            false
        }
    }
}

/// Write keymap.kdl and strip `[keys]` from config.toml, unless the converted
/// keymap would not load; then neither file changes.
fn write_migration(
    migration: &LegacyMigration,
    config_path: &Path,
    content: &str,
    keymap_path: &Path,
) -> std::io::Result<i32> {
    let warnings = match check_converted(&migration.kdl) {
        Ok(warnings) => warnings,
        Err(problems) => {
            eprintln!(
                "error: Herdr would ignore the converted keymap, so {} was not written and {} is unchanged:",
                keymap_path.display(),
                config_path.display()
            );
            for problem in &problems {
                eprintln!("  {problem}");
            }
            for note in &migration.notes {
                eprintln!("note: {note}");
            }
            eprintln!("Run `herdr keymap migrate --dry-run` to see the converted text.");
            return Ok(1);
        }
    };
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    if let Some(parent) = keymap_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(keymap_path, &migration.kdl)?;
    println!("Wrote {}.", keymap_path.display());

    let (mut updated, removed) = crate::config::remove_keybinding_config_sections(content);
    if let Some(key) = &migration.image_paste_key {
        updated = crate::config::upsert_section_value(
            &updated,
            "remote",
            "image_paste_key",
            &toml::Value::String(key.clone()).to_string(),
        );
    }
    if removed && updated.parse::<toml::Value>().is_ok() {
        let backup = super::backup_path(config_path, "keys");
        std::fs::copy(config_path, &backup)?;
        std::fs::write(config_path, updated)?;
        println!(
            "Removed [keys] from {} (backup: {}).",
            config_path.display(),
            backup.display()
        );
    } else {
        println!(
            "Could not remove [keys] from {} automatically; delete it by hand.",
            config_path.display()
        );
    }
    for note in &migration.notes {
        println!("note: {note}");
    }
    println!("If a Herdr server is running, run `herdr server reload-config` to apply it now.");
    Ok(0)
}

pub(super) struct LegacyMigration {
    pub(super) kdl: String,
    pub(super) notes: Vec<String>,
    pub(super) image_paste_key: Option<String>,
}

/// Old `[keys]` fields in the order Herdr registered them, what each runs in
/// the classic keymap, and its classic default bindings. Targets starting
/// with `@` open a classic menu by id. `fullscreen` is an old spelling of
/// `zoom`.
const LEGACY_FIELDS: &[(&str, &str, &[&str])] = &[
    ("help", "app.help", &["prefix+?"]),
    ("settings", "app.settings", &["prefix+s"]),
    ("new_workspace", "workspace.new", &["prefix+shift+n"]),
    ("new_worktree", "worktree.new", &["prefix+shift+g"]),
    ("open_worktree", "worktree.open", &[]),
    ("remove_worktree", "worktree.remove", &[]),
    ("rename_workspace", "workspace.rename", &["prefix+shift+w"]),
    ("close_workspace", "workspace.close", &["prefix+shift+d"]),
    ("workspace_picker", "@navigate", &["prefix+w"]),
    ("goto", "app.navigator", &["prefix+g"]),
    ("detach", "app.detach", &["prefix+q"]),
    ("reload_config", "app.reload", &["prefix+shift+r"]),
    (
        "open_notification_target",
        "agent.notification",
        &["prefix+o"],
    ),
    ("previous_workspace", "workspace.previous", &[]),
    ("next_workspace", "workspace.next", &[]),
    ("previous_agent", "agent.previous", &[]),
    ("next_agent", "agent.next", &[]),
    ("focus_agent", "agent.focus", &[]),
    ("new_tab", "tab.new", &["prefix+c"]),
    ("rename_tab", "tab.rename", &["prefix+shift+t"]),
    ("previous_tab", "tab.previous", &["prefix+p"]),
    ("next_tab", "tab.next", &["prefix+n"]),
    ("move_tab_previous", "tab.move.left", &[]),
    ("move_tab_next", "tab.move.right", &[]),
    ("switch_tab", "tab.switch", &["prefix+1..9"]),
    ("switch_workspace", "workspace.switch", &[]),
    ("close_tab", "tab.close", &["prefix+shift+x"]),
    ("rename_pane", "pane.rename", &["prefix+shift+p"]),
    ("edit_scrollback", "pane.scrollback", &["prefix+e"]),
    ("clear_pane", "pane.clear", &[]),
    ("copy_mode", "@copy", &["prefix+["]),
    ("focus_pane_left", "pane.focus.left", &["prefix+h"]),
    ("focus_pane_down", "pane.focus.down", &["prefix+j"]),
    ("focus_pane_up", "pane.focus.up", &["prefix+k"]),
    ("focus_pane_right", "pane.focus.right", &["prefix+l"]),
    ("swap_pane_left", "pane.swap.left", &["prefix+shift+h"]),
    ("swap_pane_down", "pane.swap.down", &["prefix+shift+j"]),
    ("swap_pane_up", "pane.swap.up", &["prefix+shift+k"]),
    ("swap_pane_right", "pane.swap.right", &["prefix+shift+l"]),
    ("last_pane", "pane.last", &[]),
    ("cycle_pane_next", "pane.cycle.next", &["prefix+tab"]),
    (
        "cycle_pane_previous",
        "pane.cycle.previous",
        &["prefix+shift+tab"],
    ),
    ("split_vertical", "pane.split.right", &["prefix+v"]),
    ("split_horizontal", "pane.split.down", &["prefix+minus"]),
    ("close_pane", "pane.close", &["prefix+x"]),
    ("zoom", "pane.zoom", &["prefix+z"]),
    ("resize_mode", "@resize", &["prefix+r"]),
    ("resize_pane_left", "pane.resize.left", &[]),
    ("resize_pane_down", "pane.resize.down", &[]),
    ("resize_pane_up", "pane.resize.up", &[]),
    ("resize_pane_right", "pane.resize.right", &[]),
    ("toggle_sidebar", "app.sidebar", &["prefix+b"]),
];

/// Navigate-mode fields, what they run, their default key, and the words the
/// classic navigate menu writes after the action.
const LEGACY_NAVIGATE_FIELDS: &[(&str, &str, &str, &str)] = &[
    (
        "navigate_workspace_up",
        "workspace.list.up",
        "up",
        "workspace stay",
    ),
    (
        "navigate_workspace_down",
        "workspace.list.down",
        "down",
        "workspace stay",
    ),
    ("navigate_pane_left", "pane.focus.left", "h", "hidden stay"),
    ("navigate_pane_down", "pane.focus.down", "j", "hidden stay"),
    ("navigate_pane_up", "pane.focus.up", "k", "hidden stay"),
    (
        "navigate_pane_right",
        "pane.focus.right",
        "l",
        "hidden stay",
    ),
];

/// `[keys.indexed]` entries, the field whose default each one displaced,
/// and the action it runs.
const LEGACY_INDEXED: &[(&str, &str, &str)] = &[
    ("tabs", "switch_tab", "tab.switch"),
    ("workspaces", "switch_workspace", "workspace.switch"),
    ("agents", "focus_agent", "agent.focus"),
];

/// Where navigate lives in the classic keymap: `prefix w`.
const CLASSIC_NAVIGATE_KEY: KeyCombo = (KeyCode::Char('w'), KeyModifiers::empty());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    /// Chords pressed while typing in a pane.
    Top,
    /// Chords pressed after the prefix.
    Prefix,
    /// Keys pressed inside navigate mode.
    Navigate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    /// A catalog action and the words the leaf writes after it.
    Action {
        id: &'static str,
        words: &'static str,
    },
    /// A classic menu, by id.
    Menu(&'static str),
    /// A command leaf, as written after the chord.
    Command(String),
}

impl Target {
    fn field(target: &'static str) -> Self {
        match target.strip_prefix('@') {
            Some(menu) => Self::Menu(menu),
            None => Self::Action {
                id: target,
                words: "",
            },
        }
    }

    fn leaf(&self) -> String {
        match self {
            Self::Action { id, words: "" } => (*id).to_owned(),
            Self::Action { id, words } => format!("{id} {words}"),
            Self::Menu(id) => format!("menu.open {id}"),
            Self::Command(words) => words.clone(),
        }
    }

    fn is_indexed(&self) -> bool {
        match self {
            Self::Action { id, .. } => lookup_action(id)
                .is_some_and(|entry| matches!(entry.action, CatalogAction::Indexed(_))),
            _ => false,
        }
    }
}

/// A chord taken by a field, `[keys.indexed]`, or a command.
struct Claim {
    scope: Scope,
    combo: KeyCombo,
    target: Target,
    source: String,
}

struct Claims {
    prefix: KeyCombo,
    list: Vec<Claim>,
    notes: Vec<String>,
}

impl Claims {
    fn holder(&self, scope: Scope, combo: KeyCombo) -> Option<&Claim> {
        self.list
            .iter()
            .find(|claim| claim.scope == scope && claim.combo == combo)
    }

    /// Why old Herdr refused a key in this scope, if it did.
    fn reserved(&self, scope: Scope, combo: KeyCombo) -> Option<&'static str> {
        if combo == self.prefix {
            return Some(match scope {
                Scope::Top => "is the prefix key",
                Scope::Prefix => "after the prefix sends the prefix key to the pane",
                Scope::Navigate => "is the prefix key, which closes navigate mode",
            });
        }
        let (code, modifiers) = combo;
        match scope {
            Scope::Top => None,
            Scope::Prefix => (code == KeyCode::Esc && modifiers.is_empty())
                .then_some("after the prefix cancels the prefix"),
            Scope::Navigate => {
                let runtime = modifiers.is_empty()
                    && matches!(
                        code,
                        KeyCode::Enter
                            | KeyCode::Tab
                            | KeyCode::BackTab
                            | KeyCode::Left
                            | KeyCode::Right
                            | KeyCode::Char('1'..='9')
                    );
                (code == KeyCode::Esc || runtime).then_some("is reserved in navigate mode")
            }
        }
    }

    /// Claim every key of `chord`. User claims that lose to a reserved key
    /// or an earlier claim are reported; defaults give way silently.
    fn add(&mut self, scope: Scope, chord: Chord, target: &Target, source: &str, user: bool) {
        let mut refused: Vec<(String, Vec<String>)> = Vec::new();
        for combo in combos(chord) {
            let reason = match self.reserved(scope, combo) {
                Some(reason) => Some(reason.to_owned()),
                None => self
                    .holder(scope, combo)
                    .map(|holder| format!("is already taken by {}", holder.source)),
            };
            match reason {
                Some(reason) => match refused.iter_mut().find(|(known, _)| *known == reason) {
                    Some((_, keys)) => keys.push(key_name(scope, Chord::Key(combo))),
                    None => refused.push((reason, vec![key_name(scope, Chord::Key(combo))])),
                },
                None => self.list.push(Claim {
                    scope,
                    combo,
                    target: target.clone(),
                    source: source.to_owned(),
                }),
            }
        }
        if user {
            for (reason, keys) in refused {
                self.notes.push(format!(
                    "{source}: {} {reason}; not migrated",
                    keys.join(", ")
                ));
            }
        }
    }

    /// Claim the bindings of an action field (`prefix+x` or a direct chord).
    fn add_field(&mut self, source: &str, target: &Target, values: &[String], user: bool) {
        for raw in values {
            let (scope, text) = match raw.strip_prefix("prefix+") {
                Some(rest) => (Scope::Prefix, rest),
                None => (Scope::Top, raw.as_str()),
            };
            let problem = match Chord::parse(text) {
                Err(error) => Some(error),
                Ok(Chord::Digits(_)) if !target.is_indexed() => {
                    Some("only switch_tab, switch_workspace, and focus_agent take 1..9".to_owned())
                }
                Ok(Chord::Key((code, _)))
                    if target.is_indexed() && !matches!(code, KeyCode::Char('1'..='9')) =>
                {
                    Some("needs a 1..9 range or a single digit".to_owned())
                }
                Ok(chord) if scope == Scope::Top && chord.intercepts_typing() => {
                    Some("would intercept typing in panes without the prefix".to_owned())
                }
                Ok(chord) => {
                    self.add(scope, chord, target, source, user);
                    None
                }
            };
            if let (Some(problem), true) = (problem, user) {
                self.notes
                    .push(format!("{source} = {raw:?}: {problem}; not migrated"));
            }
        }
    }

    /// Claim navigate-mode keys, which old Herdr matched without the prefix.
    fn add_navigate(&mut self, source: &str, target: &Target, values: &[String], user: bool) {
        for raw in values {
            let problem = if raw.starts_with("prefix+") {
                Some("navigate keys are pressed without the prefix".to_owned())
            } else {
                match Chord::parse(raw) {
                    Err(error) => Some(error),
                    Ok(Chord::Digits(_)) => Some("navigate keys cannot use 1..9".to_owned()),
                    Ok(chord) => {
                        self.add(Scope::Navigate, chord, target, source, user);
                        None
                    }
                }
            };
            if let (Some(problem), true) = (problem, user) {
                self.notes
                    .push(format!("{source} = {raw:?}: {problem}; not migrated"));
            }
        }
    }

    /// Claimed keys in one scope, in claim order.
    fn in_scope(&self, scope: Scope) -> Vec<(KeyCombo, Target)> {
        self.list
            .iter()
            .filter(|claim| claim.scope == scope)
            .map(|claim| (claim.combo, claim.target.clone()))
            .collect()
    }
}

fn combos(chord: Chord) -> Vec<KeyCombo> {
    match chord {
        Chord::Key(combo) => vec![combo],
        Chord::Digits(modifiers) => ('1'..='9')
            .map(|digit| (KeyCode::Char(digit), modifiers))
            .collect(),
    }
}

fn key_name(scope: Scope, chord: Chord) -> String {
    match scope {
        Scope::Prefix => format!("prefix+{}", chord.label()),
        Scope::Top | Scope::Navigate => chord.label(),
    }
}

/// A chord as keymap.kdl text that parses back to the same chord.
fn chord_text(chord: Chord) -> String {
    match chord {
        // `+` separates modifiers, so the key itself is spelled `plus`.
        Chord::Key((KeyCode::Char('+'), modifiers)) => {
            let label = Chord::Key((KeyCode::Char('+'), modifiers)).label();
            format!("{}plus", label.strip_suffix('+').unwrap_or_default())
        }
        Chord::Key((KeyCode::Char(ch), KeyModifiers::SHIFT)) if ch.is_ascii_lowercase() => {
            ch.to_ascii_uppercase().to_string()
        }
        chord => chord.label(),
    }
}

/// A KDL string literal for `text`. Characters KDL does not allow in a
/// string, or reads as a line break, are escaped.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            ch if ch.is_control()
                || matches!(
                    ch,
                    '\u{200E}'..='\u{200F}'
                        | '\u{2028}'..='\u{2029}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{FEFF}'
                ) =>
            {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(ch));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn line(chord: Chord, target: Option<&Target>) -> String {
    let leaf = target.map_or_else(|| "none".to_owned(), Target::leaf);
    format!("{} {leaf}", quoted(&chord_text(chord)))
}

/// A field's configured bindings: one string or a list of strings.
fn binding_values(value: &toml::Value) -> Option<Vec<String>> {
    let values = match value {
        toml::Value::String(value) => vec![value.as_str()],
        toml::Value::Array(values) => values.iter().filter_map(toml::Value::as_str).collect(),
        _ => return None,
    };
    Some(
        values
            .into_iter()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

/// The configured bindings of `field`, with the name it was set under.
/// `None` when the field is not set (or not usable), so its defaults apply.
fn field_values(
    keys: &toml::Table,
    field: &str,
    notes: &mut Vec<String>,
) -> Option<(String, Vec<String>)> {
    let (name, value) = keys.get_key_value(field).or_else(|| {
        (field == "zoom")
            .then(|| keys.get_key_value("fullscreen"))
            .flatten()
    })?;
    match binding_values(value) {
        Some(values) => Some((format!("keys.{name}"), values)),
        None => {
            notes.push(format!(
                "keys.{name} is not a key or a list of keys; not migrated"
            ));
            None
        }
    }
}

fn legacy_prefix(keys: &toml::Table, notes: &mut Vec<String>) -> KeyCombo {
    let Some(value) = keys.get("prefix") else {
        return DEFAULT_PREFIX;
    };
    let text = value.as_str().map(str::trim).unwrap_or_default();
    match Chord::parse(text) {
        Ok(Chord::Key(combo)) => combo,
        _ => {
            notes.push(format!(
                "keys.prefix = {value} is not a single key; keeping ctrl+b"
            ));
            DEFAULT_PREFIX
        }
    }
}

/// The leaf a `[[keys.command]]` entry runs, or why it cannot be converted.
fn command_leaf(command: &toml::Table, text: &str) -> Result<(String, Vec<String>), String> {
    let kind = match command.get("type").and_then(toml::Value::as_str) {
        None | Some("shell") => "shell",
        Some("pane") => "pane",
        Some("popup") => "popup",
        Some("plugin_action") => "plugin",
        Some(other) => return Err(format!("type {other:?} is not supported")),
    };
    let mut notes = Vec::new();
    let mut leaf = format!("{kind} {}", quoted(text));
    if let Some(description) = command
        .get("description")
        .and_then(toml::Value::as_str)
        .filter(|description| !description.trim().is_empty())
    {
        let _ = write!(leaf, " hint={}", quoted(description));
    }
    for size in ["width", "height"] {
        let Some(value) = command.get(size) else {
            continue;
        };
        if kind != "popup" {
            notes.push(format!("{size} only applies to popup commands; dropped"));
            continue;
        }
        let parsed = match value {
            toml::Value::Integer(cells) => {
                u16::try_from(*cells).ok().map(|cells| cells.to_string())
            }
            toml::Value::String(text) if text.trim().ends_with('%') => {
                crate::popup_size::PopupSize::parse_cli(text.trim())
                    .ok()
                    .map(|_| quoted(text.trim()))
            }
            _ => None,
        };
        match parsed {
            Some(size_text) => {
                let _ = write!(leaf, " {size}={size_text}");
            }
            None => notes.push(format!(
                "{size} = {value} is not a cell count or a percentage like \"80%\"; dropped"
            )),
        }
    }
    Ok((leaf, notes))
}

/// The classic defaults for one scope, minus keys that equal the prefix:
/// Herdr drops those built-in keys so the prefix keeps working.
fn classic_defaults(scope: Scope, prefix: KeyCombo) -> Vec<(KeyCombo, Target)> {
    let mut defaults = Vec::new();
    match scope {
        Scope::Top => {}
        Scope::Prefix => {
            for (_, target, bindings) in LEGACY_FIELDS {
                for binding in *bindings {
                    let Some(Ok(chord)) = binding.strip_prefix("prefix+").map(Chord::parse) else {
                        continue;
                    };
                    for combo in combos(chord) {
                        defaults.push((combo, Target::field(target)));
                    }
                }
            }
        }
        Scope::Navigate => {
            for (_, id, key, words) in LEGACY_NAVIGATE_FIELDS {
                if let Ok(Chord::Key(combo)) = Chord::parse(key) {
                    defaults.push((combo, Target::Action { id, words }));
                }
            }
        }
    }
    defaults.retain(|(combo, _)| *combo != prefix);
    defaults
}

/// Lines that turn one classic scope into the claimed one.
#[derive(Default)]
struct ScopeDiff {
    /// `1..9` lines. They come first in a block, so a single digit after
    /// them changes only its own key.
    ranges: Vec<String>,
    unbinds: Vec<String>,
    binds: Vec<String>,
    /// Keys the diff unbinds or rebinds.
    touched: Vec<Chord>,
    /// Keys the diff binds.
    bound: Vec<Chord>,
}

impl ScopeDiff {
    fn is_empty(&self) -> bool {
        self.ranges.is_empty() && self.unbinds.is_empty() && self.binds.is_empty()
    }

    fn lines(&self) -> Vec<String> {
        self.ranges
            .iter()
            .chain(&self.unbinds)
            .chain(&self.binds)
            .cloned()
            .collect()
    }

    /// Diff `claimed` against `defaults`. The key in `elsewhere` is written
    /// by the caller, so it is neither bound nor unbound here.
    fn new(
        defaults: &[(KeyCombo, Target)],
        claimed: &[(KeyCombo, Target)],
        elsewhere: Option<KeyCombo>,
    ) -> Self {
        let mut diff = Self::default();
        let digit_modifiers = |combo: &KeyCombo| match combo {
            (KeyCode::Char('1'..='9'), modifiers) => Some(*modifiers),
            _ => None,
        };
        let mut groups: Vec<KeyModifiers> = Vec::new();
        for (combo, _) in defaults.iter().chain(claimed) {
            if let Some(modifiers) = digit_modifiers(combo) {
                if !groups.contains(&modifiers) {
                    groups.push(modifiers);
                }
            }
        }
        for modifiers in groups {
            diff.digits(modifiers, defaults, claimed, elsewhere);
        }
        for (combo, _) in defaults {
            if digit_modifiers(combo).is_none() && !claimed.iter().any(|(key, _)| key == combo) {
                diff.touched.push(Chord::Key(*combo));
                diff.unbinds.push(line(Chord::Key(*combo), None));
            }
        }
        for (combo, target) in claimed {
            if digit_modifiers(combo).is_some() || Some(*combo) == elsewhere {
                continue;
            }
            if !defaults.contains(&(*combo, target.clone())) {
                diff.touched.push(Chord::Key(*combo));
                diff.bound.push(Chord::Key(*combo));
                diff.binds.push(line(Chord::Key(*combo), Some(target)));
            }
        }
        diff
    }

    /// Digits with one set of modifiers: a whole range where possible, else
    /// single digits over the classic range.
    fn digits(
        &mut self,
        modifiers: KeyModifiers,
        defaults: &[(KeyCombo, Target)],
        claimed: &[(KeyCombo, Target)],
        elsewhere: Option<KeyCombo>,
    ) {
        let at = |list: &[(KeyCombo, Target)], digit: char| {
            list.iter()
                .find(|(combo, _)| *combo == (KeyCode::Char(digit), modifiers))
                .map(|(_, target)| target.clone())
        };
        let default: Vec<Option<Target>> = ('1'..='9').map(|digit| at(defaults, digit)).collect();
        let wanted: Vec<Option<Target>> = ('1'..='9').map(|digit| at(claimed, digit)).collect();
        if default == wanted {
            return;
        }
        let range = Chord::Digits(modifiers);
        let whole = |list: &[Option<Target>]| {
            list.first()
                .cloned()
                .flatten()
                .filter(|first| list.iter().all(|target| target.as_ref() == Some(first)))
        };
        if let Some(target) = whole(&wanted).filter(Target::is_indexed) {
            self.touched.push(range);
            self.bound.push(range);
            self.ranges.push(line(range, Some(&target)));
            return;
        }
        let keeps = default
            .iter()
            .zip(&wanted)
            .any(|(default, wanted)| default.is_some() && default == wanted);
        let cleared = whole(&default).is_some() && !keeps;
        if cleared {
            self.touched.push(range);
            self.ranges.push(line(range, None));
        }
        for (index, digit) in ('1'..='9').enumerate() {
            let combo = (KeyCode::Char(digit), modifiers);
            let chord = Chord::Key(combo);
            match (&default[index], &wanted[index]) {
                (_, Some(_)) if Some(combo) == elsewhere => {}
                (default, Some(target)) if cleared || default.as_ref() != Some(target) => {
                    self.touched.push(chord);
                    self.bound.push(chord);
                    self.binds.push(line(chord, Some(target)));
                }
                (Some(_), None) if !cleared => {
                    self.touched.push(chord);
                    self.unbinds.push(line(chord, None));
                }
                _ => {}
            }
        }
    }
}

/// The classic navigate menu rebuilt at `home`, with `changes` applied.
/// Keys it does not bind still fall through to the prefix menu.
fn navigate_copy(
    prefix: KeyCombo,
    home_scope: Scope,
    home: KeyCombo,
    changes: &ScopeDiff,
) -> Option<String> {
    let document = CLASSIC_KEYMAP.parse::<kdl::KdlDocument>().ok()?;
    let navigate = document
        .nodes()
        .iter()
        .find(|node| node.name().value() == "prefix")?
        .children()?
        .nodes()
        .iter()
        .find(|node| {
            Chord::parse(node.name().value()).ok() == Some(Chord::Key(CLASSIC_NAVIGATE_KEY))
        })?;
    let words = |node: &kdl::KdlNode| {
        node.entries()
            .iter()
            .map(|entry| entry.to_string().trim().to_owned())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut children = Vec::new();
    let mut bound = Vec::new();
    for child in navigate.children()?.nodes() {
        let name = child.name().value();
        if name == "prefix" {
            children.push(format!("prefix {}", words(child)));
            continue;
        }
        let chord = Chord::parse(name).ok()?;
        // The classic `w` closes navigate because `w` opened it. At its new
        // key, `w` falls through to the prefix menu instead.
        let toggle = chord == Chord::Key(CLASSIC_NAVIGATE_KEY);
        let replaced = changes
            .touched
            .iter()
            .any(|touched| touched.overlaps(chord));
        if toggle || replaced || chord.overlaps(Chord::Key(prefix)) {
            continue;
        }
        bound.push(chord);
        children.push(format!("{} {}", quoted(&chord_text(chord)), words(child)));
    }
    bound.extend(changes.bound.iter().copied());
    let home_chord = Chord::Key(home);
    if home_scope == Scope::Prefix && !bound.iter().any(|chord| chord.overlaps(home_chord)) {
        children.push(format!(
            "{} menu.cancel hidden",
            quoted(&chord_text(home_chord))
        ));
    }
    children.extend(changes.binds.iter().cloned());
    let mut block = format!(
        "{} {} {{\n",
        quoted(&chord_text(home_chord)),
        words(navigate)
    );
    for child in children {
        let _ = writeln!(block, "    {child}");
    }
    block.push('}');
    Some(block)
}

/// Convert an old `[keys]` table into a keymap overlay on the classic base.
pub(super) fn convert_legacy_keys(keys: &toml::Table) -> LegacyMigration {
    let mut notes = Vec::new();
    let prefix = legacy_prefix(keys, &mut notes);
    let mut claims = Claims {
        prefix,
        list: Vec::new(),
        notes: Vec::new(),
    };

    // Fields the user set claim their keys first, in Herdr's old order.
    let configured: Vec<Option<(String, Vec<String>)>> = LEGACY_FIELDS
        .iter()
        .map(|(field, _, _)| field_values(keys, field, &mut notes))
        .collect();
    for ((_, target, _), values) in LEGACY_FIELDS.iter().zip(&configured) {
        if let Some((source, values)) = values {
            claims.add_field(source, &Target::field(target), values, true);
        }
    }

    // `[keys.indexed]` replaced the default of the field it stands for.
    let mut displaced = Vec::new();
    if let Some(indexed) = keys.get("indexed").and_then(toml::Value::as_table) {
        for (name, field, action) in LEGACY_INDEXED {
            let Some(value) = indexed.get(*name) else {
                continue;
            };
            let modifiers = value.as_str().map(str::trim).unwrap_or_default();
            if modifiers.is_empty() {
                continue;
            }
            displaced.push(*field);
            let source = format!("keys.indexed.{name}");
            match Chord::parse(&format!("{modifiers}+1..9")) {
                Ok(chord @ Chord::Digits(held)) if !held.is_empty() => {
                    let values = [chord.label()];
                    claims.add_field(&source, &Target::field(action), &values, true);
                }
                _ => claims.notes.push(format!(
                    "{source} = {value} is not a set of modifiers like \"ctrl\"; not migrated"
                )),
            }
        }
    }

    if let Some(commands) = keys.get("command").and_then(toml::Value::as_array) {
        for (index, command) in commands.iter().enumerate() {
            let source = format!("keys.command[{index}]");
            let Some(command) = command.as_table() else {
                claims
                    .notes
                    .push(format!("{source} is not a table; skipped"));
                continue;
            };
            let Some(text) = command
                .get("command")
                .and_then(toml::Value::as_str)
                .filter(|text| !text.trim().is_empty())
            else {
                claims
                    .notes
                    .push(format!("{source} has no command; skipped"));
                continue;
            };
            let (leaf, leaf_notes) = match command_leaf(command, text) {
                Ok(converted) => converted,
                Err(problem) => {
                    claims.notes.push(format!("{source}: {problem}; skipped"));
                    continue;
                }
            };
            for note in leaf_notes {
                claims.notes.push(format!("{source}: {note}"));
            }
            let bindings = command
                .get("key")
                .and_then(binding_values)
                .unwrap_or_default();
            if bindings.is_empty() {
                claims
                    .notes
                    .push(format!("{source} ({text:?}) has no key; skipped"));
            }
            claims.add_field(&source, &Target::Command(leaf), &bindings, true);
        }
    }

    // The defaults of unset fields take whatever keys are left.
    for ((field, target, defaults), values) in LEGACY_FIELDS.iter().zip(&configured) {
        if values.is_some() || displaced.contains(field) {
            continue;
        }
        let defaults = defaults
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        claims.add_field(
            &format!("keys.{field}"),
            &Target::field(target),
            &defaults,
            false,
        );
    }

    let navigate_configured: Vec<Option<(String, Vec<String>)>> = LEGACY_NAVIGATE_FIELDS
        .iter()
        .map(|(field, _, _, _)| field_values(keys, field, &mut notes))
        .collect();
    for ((_, id, _, words), values) in LEGACY_NAVIGATE_FIELDS.iter().zip(&navigate_configured) {
        if let Some((source, values)) = values {
            claims.add_navigate(source, &Target::Action { id, words }, values, true);
        }
    }
    for ((field, id, default, words), values) in
        LEGACY_NAVIGATE_FIELDS.iter().zip(&navigate_configured)
    {
        if values.is_none() {
            let defaults = [(*default).to_owned()];
            let target = Target::Action { id, words };
            claims.add_navigate(&format!("keys.{field}"), &target, &defaults, false);
        }
    }
    notes.append(&mut claims.notes);

    // Navigate lives at its classic key while that key still opens it;
    // otherwise at its first new key after the prefix (or its first key),
    // as a copy of the classic menu. After the prefix, keys navigate does
    // not bind fall through to the prefix menu, as they did before.
    let navigate_diff = ScopeDiff::new(
        &classic_defaults(Scope::Navigate, prefix),
        &claims.in_scope(Scope::Navigate),
        None,
    );
    let locations: Vec<(Scope, KeyCombo)> = claims
        .list
        .iter()
        .filter(|claim| claim.target == Target::Menu("navigate"))
        .map(|claim| (claim.scope, claim.combo))
        .collect();
    let prefix_defaults = classic_defaults(Scope::Prefix, prefix);
    let mut navigate_block: Option<(Scope, KeyCombo, String)> = None;
    let mut navigate_overrides = Vec::new();
    if locations.contains(&(Scope::Prefix, CLASSIC_NAVIGATE_KEY)) {
        navigate_overrides = navigate_diff.lines();
    } else if let Some(&(scope, home)) = locations
        .iter()
        .find(|(scope, _)| *scope == Scope::Prefix)
        .or(locations.first())
    {
        // A classic menu already sits at `home` in the prefix menu; a menu
        // written there would merge with it.
        let taken = scope == Scope::Prefix
            && prefix_defaults
                .iter()
                .any(|(combo, target)| *combo == home && matches!(target, Target::Menu(_)));
        match (taken, navigate_copy(prefix, scope, home, &navigate_diff)) {
            (false, Some(block)) => navigate_block = Some((scope, home, block)),
            _ if !navigate_diff.is_empty() => notes.push(format!(
                "navigate mode moved to {}, where its keys.navigate_* changes cannot follow it; add them by hand",
                key_name(scope, Chord::Key(home))
            )),
            _ => {}
        }
    } else if !navigate_diff.is_empty() {
        notes.push(
            "navigate mode has no key, so keys.navigate_* changes were not migrated".to_owned(),
        );
    }
    for (scope, combo) in &locations {
        if *scope == Scope::Top {
            notes.push(format!(
                "navigate mode opened with {} does not fall back to prefix keys; open it after the prefix for that",
                chord_text(Chord::Key(*combo))
            ));
        }
    }
    let home_in = |scope: Scope| {
        navigate_block
            .as_ref()
            .filter(|(home_scope, _, _)| *home_scope == scope)
            .map(|(_, home, block)| (*home, block.as_str()))
    };

    let prefix_diff = ScopeDiff::new(
        &prefix_defaults,
        &claims.in_scope(Scope::Prefix),
        home_in(Scope::Prefix).map(|(home, _)| home),
    );
    let mut prefix_lines = prefix_diff.lines();
    if !navigate_overrides.is_empty() {
        let mut block = format!(
            "{} {{\n",
            quoted(&chord_text(Chord::Key(CLASSIC_NAVIGATE_KEY)))
        );
        for line in &navigate_overrides {
            let _ = writeln!(block, "    {line}");
        }
        block.push('}');
        prefix_lines.push(block);
    }
    if let Some((_, block)) = home_in(Scope::Prefix) {
        prefix_lines.push(block.to_owned());
    }
    let top_diff = ScopeDiff::new(
        &classic_defaults(Scope::Top, prefix),
        &claims.in_scope(Scope::Top),
        home_in(Scope::Top).map(|(home, _)| home),
    );
    let mut top_lines = top_diff.lines();
    if let Some((_, block)) = home_in(Scope::Top) {
        top_lines.push(block.to_owned());
    }

    let image_paste_key = keys
        .get("remote_image_paste")
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    if image_paste_key.is_some() {
        notes.push("keys.remote_image_paste moved to [remote] image_paste_key".to_owned());
    }

    let mut kdl = String::from(
        "// Converted from [keys] in config.toml by `herdr keymap migrate`.\n// It starts from Herdr's classic layout and applies your changes.\nbase classic",
    );
    if prefix != DEFAULT_PREFIX {
        let _ = write!(kdl, " prefix={}", quoted(&chord_text(Chord::Key(prefix))));
    }
    kdl.push('\n');
    if !prefix_lines.is_empty() {
        kdl.push_str("\nprefix {\n");
        for line in &prefix_lines {
            for row in line.lines() {
                let _ = writeln!(kdl, "    {row}");
            }
        }
        kdl.push_str("}\n");
    }
    if !top_lines.is_empty() {
        kdl.push('\n');
        for line in &top_lines {
            let _ = writeln!(kdl, "{line}");
        }
    }
    LegacyMigration {
        kdl,
        notes,
        image_paste_key,
    }
}
