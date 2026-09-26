use serde::{Deserialize, Serialize};

/// The effective keymap tree: every menu and the keys in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct KeymapInfo {
    /// Base tree: `herdra`, `classic`, or `none`.
    pub base: String,
    /// The prefix chord, for example `ctrl+b`.
    pub prefix: String,
    /// Menus in tree order. The first entry, with an empty path, is the top
    /// level where keys otherwise go to the pane.
    pub menus: Vec<KeymapMenuInfo>,
    /// Problems in the user's keymap file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    /// Plugin keymap problems and keys plugins could not claim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct KeymapMenuInfo {
    /// Chords from the top level to this menu, space separated.
    pub path: String,
    pub title: String,
    pub sticky: bool,
    /// Whether menu cancels return here, like copy mode.
    pub mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    pub bindings: Vec<KeymapBindingInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct KeymapBindingInfo {
    /// The chord inside its menu, for example `v`.
    pub chord: String,
    /// The full key sequence from the top level, for example `ctrl+b p v`.
    pub keys: String,
    /// `action`, `command`, or `menu`.
    pub kind: String,
    /// Action id, command, menu path, or menu operation such as `menu.back`.
    pub target: String,
    /// Label shown in the menu bar.
    pub hint: String,
    /// Label shown in the keybind help.
    pub description: String,
    pub hidden: bool,
    /// `exit` or `stay` when the key overrides its menu's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<String>,
    /// `builtin`, `user`, or `plugin:<id>`.
    pub owner: String,
}
