//! The effective keymap as API data, for `keymap.get` and `herdr keymap print`.

use super::compile::{binding_action_id, CompiledKeymap, CompiledTarget};
use super::parse::ExitPolicy;
use crate::api::schema::{KeymapBindingInfo, KeymapInfo, KeymapMenuInfo};

pub(crate) fn describe(keymap: &CompiledKeymap) -> KeymapInfo {
    let menus = keymap
        .menus
        .iter()
        .enumerate()
        .map(|(index, menu)| KeymapMenuInfo {
            path: menu.path_label.clone(),
            title: menu.title.clone(),
            sticky: menu.sticky,
            mode: menu.anchor,
            view: menu.view.map(|view| view.id().to_owned()),
            bindings: menu
                .bindings
                .iter()
                .map(|binding| {
                    let (kind, target) = match &binding.target {
                        CompiledTarget::Enter(child) => {
                            ("menu", keymap.menu(*child).path_label.clone())
                        }
                        CompiledTarget::Command(command) => (
                            "command",
                            keymap
                                .commands
                                .get(*command)
                                .map(|command| {
                                    format!("{} {}", command.spec.kind.word(), command.spec.command)
                                })
                                .unwrap_or_default(),
                        ),
                        CompiledTarget::Back => ("menu", "menu.back".to_owned()),
                        CompiledTarget::Cancel => ("menu", "menu.cancel".to_owned()),
                        CompiledTarget::Literal => ("menu", "menu.literal".to_owned()),
                        CompiledTarget::Action(_) => (
                            "action",
                            binding_action_id(binding).unwrap_or_default().to_owned(),
                        ),
                    };
                    KeymapBindingInfo {
                        chord: binding.chord.label(),
                        keys: if index == 0 {
                            binding.chord.label()
                        } else {
                            format!("{} {}", menu.path_label, binding.chord.label())
                        },
                        kind: kind.to_owned(),
                        target,
                        hint: binding.hint.clone(),
                        description: binding.description.clone(),
                        hidden: binding.hidden,
                        exit: binding.exit.map(|exit| {
                            match exit {
                                ExitPolicy::Exit => "exit",
                                ExitPolicy::Stay => "stay",
                            }
                            .to_owned()
                        }),
                        owner: binding.owner.label(),
                    }
                })
                .collect(),
        })
        .collect();
    KeymapInfo {
        base: keymap.base.id().to_owned(),
        prefix: crate::config::format_key_combo(keymap.prefix),
        menus,
        diagnostics: keymap.diagnostics.clone(),
        conflicts: keymap.conflicts.clone(),
    }
}
