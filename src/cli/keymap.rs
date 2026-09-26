//! `herdr keymap`: print, check, migrate, and reset the keymap tree.

use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::schema::KeymapInfo;
use crate::input::keymap::{CompiledKeymap, KeymapText, CLASSIC_KEYMAP, DEFAULT_KEYMAP};

mod migrate;

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
        "migrate" => migrate::migrate(rest),
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
    eprintln!("  herdr keymap print [--json] [--node PATH]");
    eprintln!("                                       show the effective keymap tree, or one menu");
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

const PRINT_USAGE: &str = "herdr keymap print [--json] [--node PATH]";

fn print(args: &[String]) -> std::io::Result<i32> {
    let mut json = false;
    let mut node: Option<String> = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--node" => match rest.next() {
                Some(path) if node.is_none() => node = Some(path.trim().to_owned()),
                _ => return usage(PRINT_USAGE),
            },
            _ => return usage(PRINT_USAGE),
        }
    }
    let (mut keymap, source) = effective_keymap();
    let root = node.as_deref().unwrap_or("");
    let Some(root_menu) = keymap.menus.iter().find(|menu| menu.path == root) else {
        eprintln!(
            "keymap: no menu at {root:?}; `herdr keymap print` shows every menu with its keys"
        );
        return Ok(1);
    };
    if json {
        if !root.is_empty() {
            let under = format!("{root} ");
            keymap
                .menus
                .retain(|menu| menu.path == root || menu.path.starts_with(&under));
        }
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
    let depth = if root.is_empty() {
        0
    } else {
        let _ = writeln!(output, "{root}  +{}", root_menu.title);
        1
    };
    write_menu(&keymap, root, depth, &mut output);
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
