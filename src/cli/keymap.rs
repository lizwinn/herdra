//! `herdr keymap`: print, check, migrate, and reset the keymap tree.

use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::schema::KeymapInfo;
use crate::input::keymap::{CompiledKeymap, KeymapText, CLASSIC_KEYMAP, DEFAULT_KEYMAP};

pub(super) fn run_keymap_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(String::as_str) else {
        print_help();
        return Ok(2);
    };
    let rest = &args[1..];
    match subcommand {
        "print" => print(rest),
        "default" => default(rest),
        "check" => check(rest),
        "path" => path(rest),
        "migrate" => migrate(rest),
        "reset" => reset(rest),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(0)
        }
        _ => {
            print_help();
            Ok(2)
        }
    }
}

pub(super) fn print_help() {
    eprintln!("herdr keymap commands:");
    eprintln!("  herdr keymap print [--json]          show the effective keymap tree");
    eprintln!("  herdr keymap default [herdra|classic] print a built-in keymap");
    eprintln!("  herdr keymap check [FILE]            validate keymap.kdl and print diagnostics");
    eprintln!("  herdr keymap path                    print where keymap.kdl is read from");
    eprintln!("  herdr keymap migrate [--dry-run]     convert [keys] in config.toml to keymap.kdl");
    eprintln!("  herdr keymap reset                   back up and remove keymap.kdl");
}

fn usage(line: &str) -> std::io::Result<i32> {
    eprintln!("usage: {line}");
    Ok(2)
}

/// The effective keymap: from the running server when there is one, which
/// includes plugin menus, else built from the local files.
fn effective_keymap() -> (KeymapInfo, &'static str) {
    let request = crate::api::schema::Request {
        id: "cli:keymap:get".into(),
        method: crate::api::schema::Method::KeymapGet(crate::api::schema::EmptyParams::default()),
    };
    if let Ok(response) = super::send_request(&request) {
        if let Some(keymap) = response
            .get("result")
            .and_then(|result| result.get("keymap"))
            .cloned()
            .and_then(|keymap| serde_json::from_value::<KeymapInfo>(keymap).ok())
        {
            return (keymap, "the running server");
        }
    }
    let config = crate::config::Config::load().config;
    (
        crate::input::keymap::describe(&config.keymap()),
        "local files (no server running)",
    )
}

fn print(args: &[String]) -> std::io::Result<i32> {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => return usage("herdr keymap print [--json]"),
    };
    let (keymap, source) = effective_keymap();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&keymap).unwrap_or_else(|_| "{}".to_owned())
        );
        return Ok(0);
    }
    println!(
        "keymap: base {}, prefix {}, from {source}",
        keymap.base, keymap.prefix
    );
    let mut output = String::new();
    write_menu(&keymap, "", 0, &mut output);
    print!("{output}");
    for diagnostic in keymap.diagnostics.iter().chain(&keymap.conflicts) {
        eprintln!("{diagnostic}");
    }
    Ok(0)
}

fn write_menu(keymap: &KeymapInfo, path: &str, depth: usize, output: &mut String) {
    let Some(menu) = keymap.menus.iter().find(|menu| menu.path == path) else {
        return;
    };
    let indent = "  ".repeat(depth);
    for binding in &menu.bindings {
        let chord = &binding.chord;
        if binding.kind == "menu" && !binding.target.starts_with("menu.") {
            let Some(child) = keymap.menus.iter().find(|menu| menu.path == binding.target) else {
                continue;
            };
            let mut flags = Vec::new();
            if child.sticky {
                flags.push("sticky".to_owned());
            }
            if child.mode {
                flags.push("mode".to_owned());
            }
            if let Some(view) = &child.view {
                flags.push(view.clone());
            }
            let flags = if flags.is_empty() {
                String::new()
            } else {
                format!(" ({})", flags.join(", "))
            };
            if binding.target == binding.keys {
                let _ = writeln!(output, "{indent}{chord:<12} +{}{flags}", child.title);
                write_menu(keymap, &child.path, depth + 1, output);
            } else {
                let _ = writeln!(
                    output,
                    "{indent}{chord:<12} +{}{flags} (opens {})",
                    child.title, child.path
                );
            }
            continue;
        }
        let mut notes = Vec::new();
        if binding.hidden {
            notes.push("hidden".to_owned());
        }
        if let Some(exit) = &binding.exit {
            notes.push(exit.clone());
        }
        if binding.owner != "builtin" {
            notes.push(binding.owner.clone());
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", notes.join(", "))
        };
        let _ = writeln!(
            output,
            "{indent}{chord:<12} {:<22} {}{notes}",
            binding.hint, binding.target
        );
    }
}

fn default(args: &[String]) -> std::io::Result<i32> {
    match args {
        [] => print!("{DEFAULT_KEYMAP}"),
        [name] if name == "herdra" => print!("{DEFAULT_KEYMAP}"),
        [name] if name == "classic" => print!("{CLASSIC_KEYMAP}"),
        _ => return usage("herdr keymap default [herdra|classic]"),
    }
    Ok(0)
}

fn check(args: &[String]) -> std::io::Result<i32> {
    let (path, text) = match args {
        [] => {
            let config = crate::config::Config::load().config;
            let path = crate::config::keymap_path(&config);
            match std::fs::read_to_string(&path) {
                Ok(text) => (path, text),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    println!(
                        "keymap: ok (no file at {}; using the default keymap)",
                        path.display()
                    );
                    return Ok(0);
                }
                Err(err) => {
                    eprintln!("keymap: cannot read {}: {err}", path.display());
                    return Ok(1);
                }
            }
        }
        [file] if !file.starts_with('-') => {
            let path = std::path::PathBuf::from(file);
            match std::fs::read_to_string(&path) {
                Ok(text) => (path, text),
                Err(err) => {
                    eprintln!("keymap: cannot read {}: {err}", path.display());
                    return Ok(1);
                }
            }
        }
        _ => return usage("herdr keymap check [FILE]"),
    };
    let source = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("keymap.kdl")
        .to_owned();
    let keymap = CompiledKeymap::build(Some(&KeymapText { source, text }), &[]);
    if keymap.diagnostics.is_empty() {
        println!("keymap: ok");
        Ok(0)
    } else {
        println!("keymap: issues found");
        for diagnostic in &keymap.diagnostics {
            println!("{diagnostic}");
        }
        Ok(1)
    }
}

fn path(args: &[String]) -> std::io::Result<i32> {
    if !args.is_empty() {
        return usage("herdr keymap path");
    }
    let config = crate::config::Config::load().config;
    println!("{}", crate::config::keymap_path(&config).display());
    Ok(0)
}

fn backup_path(path: &std::path::Path, tag: &str) -> std::path::PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("keymap.kdl");
    path.with_file_name(format!("{file_name}.bak-{tag}-{timestamp}"))
}

fn reset(args: &[String]) -> std::io::Result<i32> {
    if !args.is_empty() {
        return usage("herdr keymap reset");
    }
    let config = crate::config::Config::load().config;
    let path = crate::config::keymap_path(&config);
    if !path.exists() {
        println!(
            "No keymap file at {}. The default keymap already applies.",
            path.display()
        );
        return Ok(0);
    }
    let backup = backup_path(&path, "keymap");
    std::fs::rename(&path, &backup)?;
    println!("Moved {} to {}.", path.display(), backup.display());
    println!("The default keymap applies after Herdr restarts or reloads config.");
    println!("If a Herdr server is running, run `herdr server reload-config` to apply it now.");
    Ok(0)
}

fn migrate(args: &[String]) -> std::io::Result<i32> {
    let dry_run = match args {
        [] => false,
        [flag] if flag == "--dry-run" => true,
        _ => return usage("herdr keymap migrate [--dry-run]"),
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
        for note in &migration.notes {
            eprintln!("note: {note}");
        }
        return Ok(0);
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
        for note in &migration.notes {
            eprintln!("note: {note}");
        }
        return Ok(1);
    }
    let check = CompiledKeymap::build(
        Some(&KeymapText {
            source: "keymap.kdl".to_owned(),
            text: migration.kdl.clone(),
        }),
        &[],
    );
    for diagnostic in &check.diagnostics {
        eprintln!("warning: {diagnostic}");
    }
    if let Some(parent) = keymap_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&keymap_path, &migration.kdl)?;
    println!("Wrote {}.", keymap_path.display());

    let (mut updated, removed) = crate::config::remove_keybinding_config_sections(&content);
    if let Some(key) = &migration.image_paste_key {
        updated = crate::config::upsert_section_value(
            &updated,
            "remote",
            "image_paste_key",
            &toml::Value::String(key.clone()).to_string(),
        );
    }
    if removed && updated.parse::<toml::Value>().is_ok() {
        let backup = backup_path(&config_path, "keys");
        std::fs::copy(&config_path, &backup)?;
        std::fs::write(&config_path, updated)?;
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

/// Old `[keys]` field, what it runs in the classic keymap, and the classic
/// default bindings it replaces. Targets starting with `@` open a classic menu.
const LEGACY_FIELDS: &[(&str, &str, &[&str])] = &[
    ("help", "app.help", &["prefix+?"]),
    ("settings", "app.settings", &["prefix+s"]),
    ("detach", "app.detach", &["prefix+q"]),
    ("reload_config", "app.reload", &["prefix+shift+r"]),
    (
        "open_notification_target",
        "agent.notification",
        &["prefix+o"],
    ),
    ("workspace_picker", "@navigate", &["prefix+w"]),
    ("goto", "app.navigator", &["prefix+g"]),
    ("new_workspace", "workspace.new", &["prefix+shift+n"]),
    ("new_worktree", "worktree.new", &["prefix+shift+g"]),
    ("open_worktree", "worktree.open", &[]),
    ("remove_worktree", "worktree.remove", &[]),
    ("rename_workspace", "workspace.rename", &["prefix+shift+w"]),
    ("close_workspace", "workspace.close", &["prefix+shift+d"]),
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
    ("cycle_pane_next", "pane.cycle.next", &["prefix+tab"]),
    (
        "cycle_pane_previous",
        "pane.cycle.previous",
        &["prefix+shift+tab"],
    ),
    ("last_pane", "pane.last", &[]),
    ("split_vertical", "pane.split.right", &["prefix+v"]),
    ("split_horizontal", "pane.split.down", &["prefix+minus"]),
    ("close_pane", "pane.close", &["prefix+x"]),
    ("zoom", "pane.zoom", &["prefix+z"]),
    ("fullscreen", "pane.zoom", &["prefix+z"]),
    ("resize_mode", "@resize", &["prefix+r"]),
    ("resize_pane_left", "pane.resize.left", &[]),
    ("resize_pane_down", "pane.resize.down", &[]),
    ("resize_pane_up", "pane.resize.up", &[]),
    ("resize_pane_right", "pane.resize.right", &[]),
    ("toggle_sidebar", "app.sidebar", &["prefix+b"]),
];

/// Navigate-mode fields and the classic navigate menu bindings they replace.
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

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn binding_values(value: &toml::Value) -> Vec<String> {
    match value {
        toml::Value::String(value) => vec![value.trim().to_owned()],
        toml::Value::Array(values) => values
            .iter()
            .filter_map(toml::Value::as_str)
            .map(|value| value.trim().to_owned())
            .collect(),
        _ => Vec::new(),
    }
    .into_iter()
    .filter(|value| !value.is_empty())
    .collect()
}

fn target_words(target: &str) -> String {
    match target.strip_prefix('@') {
        Some(menu) => format!("menu.open {menu}"),
        None => target.to_owned(),
    }
}

/// Convert an old `[keys]` table into a keymap overlay on the classic base.
pub(super) fn convert_legacy_keys(keys: &toml::Table) -> LegacyMigration {
    let mut notes = Vec::new();
    let mut prefix_lines: Vec<String> = Vec::new();
    let mut navigate_lines: Vec<String> = Vec::new();
    let mut top_lines: Vec<String> = Vec::new();
    let mut header = String::from("base classic");
    if let Some(prefix) = keys.get("prefix").and_then(toml::Value::as_str) {
        let prefix = prefix.trim();
        if !prefix.is_empty() && prefix != "ctrl+b" {
            header.push_str(&format!(" prefix={}", quoted(prefix)));
        }
    }

    let mut prefix_bound = std::collections::HashSet::new();
    let mut bindings: Vec<(String, String, bool)> = Vec::new(); // (chord, target words, in prefix)
    let mut unbinds: Vec<String> = Vec::new();
    for (field, target, defaults) in LEGACY_FIELDS {
        let Some(value) = keys.get(*field) else {
            continue;
        };
        let values = binding_values(value);
        let is_menu = target.starts_with('@');
        for default in *defaults {
            if values.iter().any(|value| value == default) {
                continue;
            }
            if let Some(chord) = default.strip_prefix("prefix+") {
                if is_menu {
                    notes.push(format!(
                        "keys.{field}: the classic {target} menu stays on prefix {chord}; remove it there by hand if you do not want it"
                    ));
                } else {
                    unbinds.push(chord.to_owned());
                }
            }
        }
        for value in values {
            if defaults.contains(&value.as_str()) {
                continue;
            }
            match value.strip_prefix("prefix+") {
                Some(chord) => {
                    prefix_bound.insert(chord.to_owned());
                    bindings.push((chord.to_owned(), target_words(target), true));
                }
                None => bindings.push((value, target_words(target), false)),
            }
        }
    }
    for chord in unbinds {
        if !prefix_bound.contains(&chord) {
            prefix_lines.push(format!("{} none", quoted(&chord)));
        }
    }
    for (chord, target, in_prefix) in bindings {
        let line = format!("{} {target}", quoted(&chord));
        if in_prefix {
            prefix_lines.push(line);
        } else {
            top_lines.push(line);
        }
    }

    for (field, target, default, flags) in LEGACY_NAVIGATE_FIELDS {
        let Some(value) = keys.get(*field) else {
            continue;
        };
        let values = binding_values(value);
        if !values.iter().any(|value| value == default) {
            navigate_lines.push(format!("{} none", quoted(default)));
        }
        for value in values {
            if value == *default {
                continue;
            }
            navigate_lines.push(format!("{} {target} {flags}", quoted(&value)));
        }
    }

    if let Some(indexed) = keys.get("indexed").and_then(toml::Value::as_table) {
        for (field, target) in [
            ("tabs", "tab.switch"),
            ("workspaces", "workspace.switch"),
            ("agents", "agent.focus"),
        ] {
            if let Some(modifiers) = indexed.get(field).and_then(toml::Value::as_str) {
                let modifiers = modifiers.trim();
                if !modifiers.is_empty() {
                    top_lines.push(format!("{} {target}", quoted(&format!("{modifiers}+1..9"))));
                }
            }
        }
    }

    if let Some(commands) = keys.get("command").and_then(toml::Value::as_array) {
        for command in commands.iter().filter_map(toml::Value::as_table) {
            let Some(text) = command.get("command").and_then(toml::Value::as_str) else {
                continue;
            };
            let kind = match command.get("type").and_then(toml::Value::as_str) {
                None | Some("shell") => "shell",
                Some("pane") => "pane",
                Some("popup") => "popup",
                Some("plugin_action") => "plugin",
                Some(other) => {
                    notes.push(format!(
                        "keys.command type {other:?} is not supported; skipped"
                    ));
                    continue;
                }
            };
            let mut leaf = format!("{kind} {}", quoted(text));
            if let Some(description) = command.get("description").and_then(toml::Value::as_str) {
                leaf.push(' ');
                leaf.push_str(&quoted(description));
            }
            for size in ["width", "height"] {
                match command.get(size) {
                    Some(toml::Value::Integer(cells)) => leaf.push_str(&format!(" {size}={cells}")),
                    Some(toml::Value::String(percent)) => {
                        leaf.push_str(&format!(" {size}={}", quoted(percent)));
                    }
                    _ => {}
                }
            }
            let keys = command.get("key").map(binding_values).unwrap_or_default();
            if keys.is_empty() {
                notes.push(format!("keys.command {text:?} has no key; skipped"));
            }
            for key in keys {
                match key.strip_prefix("prefix+") {
                    Some(chord) => prefix_lines.push(format!("{} {leaf}", quoted(chord))),
                    None => top_lines.push(format!("{} {leaf}", quoted(&key))),
                }
            }
        }
    }

    let image_paste_key = keys
        .get("remote_image_paste")
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    if image_paste_key.is_some() {
        notes.push("keys.remote_image_paste moved to [remote] image_paste_key".to_owned());
    }

    let mut kdl = String::from(
        "// Converted from [keys] in config.toml by `herdr keymap migrate`.\n// It starts from Herdr's classic layout and applies your changes.\n",
    );
    kdl.push_str(&header);
    kdl.push('\n');
    if !prefix_lines.is_empty() || !navigate_lines.is_empty() {
        kdl.push_str("\nprefix {\n");
        for line in &prefix_lines {
            let _ = writeln!(kdl, "    {line}");
        }
        if !navigate_lines.is_empty() {
            kdl.push_str("    w {\n");
            for line in &navigate_lines {
                let _ = writeln!(kdl, "        {line}");
            }
            kdl.push_str("    }\n");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_rebuilds_old_keys_on_the_classic_base() {
        let keys: toml::Table = toml::from_str(
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
        )
        .expect("legacy keys");
        let migration = convert_legacy_keys(&keys);
        assert_eq!(
            migration.kdl,
            r#"// Converted from [keys] in config.toml by `herdr keymap migrate`.
// It starts from Herdr's classic layout and applies your changes.
base classic prefix="ctrl+a"

prefix {
    "c" none
    "t" tab.new
    "_" pane.split.down
    "g" popup "lazygit" "lazygit" width="80%"
    w {
        "h" none
        "ctrl+h" pane.focus.left hidden stay
    }
}

"alt+shift+l" tab.next
"ctrl+1..9" tab.switch
"#
        );
        assert_eq!(migration.image_paste_key.as_deref(), Some(""));

        let keymap = CompiledKeymap::build(
            Some(&KeymapText {
                source: "keymap.kdl".to_owned(),
                text: migration.kdl,
            }),
            &[],
        );
        assert!(keymap.diagnostics.is_empty(), "{:?}", keymap.diagnostics);
        assert_eq!(
            keymap.prefix,
            (
                crossterm::event::KeyCode::Char('a'),
                crossterm::event::KeyModifiers::CONTROL
            )
        );
        assert_eq!(keymap.commands[0].path_label, "ctrl+a g");
    }
}
