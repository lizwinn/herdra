//! The keymap engine: a tree of menus defined in `keymap.kdl`.
//!
//! Built-in defaults, plugins, and users define keys with the same grammar,
//! and one resolver routes every key press. The shipped trees live in
//! `default.kdl` and `classic.kdl`.

mod catalog;
mod chord;
mod compile;
mod describe;
mod parse;
mod resolve;
#[cfg(test)]
mod tests;

#[allow(unused_imports)] // Re-exported for the client shell and API layers.
pub(crate) use catalog::{
    lookup as lookup_action, CatalogAction, CatalogEntry, IndexedAction, ViewKind, CATALOG,
};
#[allow(unused_imports)] // Re-exported for the client shell and API layers.
pub(crate) use chord::{display_label, Chord};
#[allow(unused_imports)] // Re-exported for the client shell and API layers.
pub(crate) use compile::{
    binding_action_id, BarPlan, BarSegment, CompiledBinding, CompiledCommand, CompiledKeymap,
    CompiledMenu, CompiledTarget, HelpGroup, KeymapText, MenuId, SegmentKind, CLASSIC_KEYMAP,
    DEFAULT_KEYMAP,
};
pub(crate) use describe::describe;
#[allow(unused_imports)] // Re-exported for the client shell and API layers.
pub(crate) use parse::{
    parse_document, redact_commands, BarVisibility, Base, CommandKind, CommandSpec, ExitPolicy,
    LayerOwner, Unmatched, DEFAULT_PREFIX,
};
#[allow(unused_imports)] // Re-exported for the client shell and API layers.
pub(crate) use resolve::{resolve, Effect, MenuStack, Step, MAX_STACK};
