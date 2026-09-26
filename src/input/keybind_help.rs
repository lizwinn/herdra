use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::input::{keymap::CompiledKeymap, TerminalKey};

pub(crate) type KeybindHelpEntry = (String, Cow<'static, str>);
pub(crate) type KeybindHelpGroup = (Cow<'static, str>, Vec<KeybindHelpEntry>);

pub(crate) fn keybind_help_text_char(key: &TerminalKey) -> Option<char> {
    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
        return None;
    }
    if let Some(character) = key.shifted_codepoint.and_then(char::from_u32) {
        return Some(character);
    }
    let KeyCode::Char(character) = key.code else {
        return None;
    };
    Some(character)
}

/// Help overlay groups for a keymap: one group per menu, in tree order.
pub(crate) fn keybind_help_groups(keymap: &CompiledKeymap) -> Vec<KeybindHelpGroup> {
    keymap
        .help
        .iter()
        .map(|group| {
            (
                Cow::Owned(group.title.clone()),
                group
                    .entries
                    .iter()
                    .map(|(key, label)| (key.clone(), Cow::Owned(label.clone())))
                    .collect(),
            )
        })
        .collect()
}

pub(crate) fn filter_keybind_help_groups(
    groups: Vec<KeybindHelpGroup>,
    query: &str,
) -> Vec<KeybindHelpGroup> {
    if query.is_empty() {
        return groups;
    }
    let query = query.to_lowercase();
    groups
        .into_iter()
        .filter_map(|(group, entries)| {
            let entries = entries
                .into_iter()
                .filter(|(key, label)| {
                    key.to_lowercase().contains(&query) || label.to_lowercase().contains(&query)
                })
                .collect::<Vec<_>>();
            (!entries.is_empty()).then_some((group, entries))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, label: &'static str) -> KeybindHelpEntry {
        (key.to_owned(), Cow::Borrowed(label))
    }

    fn groups() -> Vec<KeybindHelpGroup> {
        vec![
            (
                Cow::Borrowed("workspaces / tabs"),
                vec![entry("w", "workspace navigation"), entry("c", "new tab")],
            ),
            (
                Cow::Borrowed("panes"),
                vec![entry("v", "split vertical"), entry("x", "close pane")],
            ),
        ]
    }

    #[test]
    fn default_keymap_help_lists_every_menu_by_path() {
        let groups = keybind_help_groups(&CompiledKeymap::default());
        let titles = groups
            .iter()
            .map(|(title, _)| title.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(titles[0], "top level");
        assert!(titles.contains(&"herdra › pane › resize"), "{titles:?}");
        let pane = groups
            .iter()
            .find(|(title, _)| title == "herdra › pane")
            .expect("pane group");
        assert!(pane
            .1
            .iter()
            .any(|(key, label)| key == "ctrl+b p v" && label == "split pane right"));
    }

    #[test]
    fn filter_matches_labels_and_shortcuts_case_insensitively() {
        let filtered = filter_keybind_help_groups(groups(), "WoRk");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1[0].1, "workspace navigation");

        let filtered = filter_keybind_help_groups(groups(), "x");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1[0].1, "close pane");
        assert!(filter_keybind_help_groups(groups(), "panes").is_empty());
    }
}
