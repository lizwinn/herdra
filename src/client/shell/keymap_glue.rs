//! Connects the keymap engine to the client shell: routes key presses,
//! applies menu stack changes, and opens and closes the views menus attach.

use std::sync::Arc;

use super::*;
use crate::input::keymap::{
    self, CatalogAction, Chord, CompiledBinding, CompiledKeymap, CompiledTarget, Effect, MenuId,
    MenuStack, Step, ViewKind,
};
use crate::input::{KeybindAction, KeybindMatch};

impl ClientShellMode {
    pub(super) fn stack(self) -> Option<MenuStack> {
        match self {
            Self::Terminal => None,
            Self::Menu(stack) => Some(stack),
        }
    }

    pub(super) fn from_stack(stack: Option<MenuStack>) -> Self {
        stack.map_or(Self::Terminal, Self::Menu)
    }
}

impl ClientShellState {
    /// Whether any open menu attaches `view`.
    pub(super) fn view_active(&self, view: ViewKind) -> bool {
        self.mode.stack().is_some_and(|stack| {
            stack
                .frames()
                .iter()
                .any(|id| self.config.keymap.menu(*id).view == Some(view))
        })
    }

    /// The menu that receives keys.
    pub(super) fn top_menu(&self) -> Option<&crate::input::keymap::CompiledMenu> {
        self.mode
            .stack()
            .map(|stack| self.config.keymap.menu(stack.top()))
    }

    /// The view of the menu that receives keys.
    pub(super) fn top_view(&self) -> Option<ViewKind> {
        self.mode
            .stack()
            .and_then(|stack| self.config.keymap.menu(stack.top()).view)
    }

    /// The workspace list is showing (it was navigate mode).
    pub(super) fn workspace_list_active(&self) -> bool {
        self.view_active(ViewKind::WorkspaceList)
    }

    /// Copy mode receives keys (not covered by a menu opened from it).
    pub(super) fn copy_mode_focused(&self) -> bool {
        self.top_view() == Some(ViewKind::Copy)
    }

    /// The menu that shows copy mode for the current copy session.
    pub(super) fn copy_menu(&self) -> Option<MenuId> {
        let keymap = &self.config.keymap;
        self.copy_mode
            .as_ref()
            .and_then(|copy_mode| copy_mode.menu_path.as_deref())
            .and_then(|path| keymap.menu_by_path(path))
            .filter(|id| keymap.menu(*id).view == Some(ViewKind::Copy))
            .or_else(|| keymap.menu_with_view(ViewKind::Copy))
    }

    /// Where keys go when menus close: copy mode when the focused pane has a
    /// copy session, otherwise the terminal.
    pub(super) fn copy_or_terminal_mode(&self) -> ClientShellMode {
        let focused = self.focused_pane_id();
        let copy_on_focused = self
            .copy_mode
            .as_ref()
            .is_some_and(|copy_mode| focused.as_deref() == Some(copy_mode.pane_id.as_str()));
        match (copy_on_focused, self.copy_menu()) {
            (true, Some(menu)) => ClientShellMode::Menu(MenuStack::single(menu)),
            _ => ClientShellMode::Terminal,
        }
    }

    /// Replace the open menus and open or close the views that change.
    #[cfg(test)]
    pub(super) fn set_menus(&mut self, next: Option<MenuStack>, outcome: &mut ClientShellInput) {
        let before = self.open_menus(next, outcome);
        self.close_views(before, outcome);
    }

    /// Replace the open menus and open views that appear. Views that close
    /// stay intact until `close_views`, so an action leaving the workspace
    /// list still acts on its selection.
    fn open_menus(&mut self, next: Option<MenuStack>, outcome: &mut ClientShellInput) -> OpenViews {
        let before = OpenViews {
            mode: self.mode,
            list: self.workspace_list_active(),
            copy: self.view_active(ViewKind::Copy),
        };
        // Closing every menu returns to a copy session on the focused pane
        // that no open menu showed, the way leaving a mode used to.
        let next = match next {
            None if !before.copy && self.copy_session_on_focused_pane() => {
                self.copy_menu().map(MenuStack::single)
            }
            next => next,
        };
        self.mode = ClientShellMode::from_stack(next);
        if self.mode != before.mode {
            outcome.repaint = true;
        }
        let has_list = self.workspace_list_active();
        let has_copy = self.view_active(ViewKind::Copy);
        if has_list && !before.list {
            self.open_workspace_list();
        }
        if has_copy && !before.copy {
            let menu = next.and_then(|stack| {
                stack
                    .frames()
                    .iter()
                    .copied()
                    .find(|id| self.config.keymap.menu(*id).view == Some(ViewKind::Copy))
            });
            if !self.start_copy_session(menu, outcome) {
                self.drop_view_menus(ViewKind::Copy);
            }
        }
        before
    }

    fn close_views(&mut self, before: OpenViews, outcome: &mut ClientShellInput) {
        if before.list && !self.workspace_list_active() {
            self.navigate_workspace_id = None;
        }
        if before.copy && !self.view_active(ViewKind::Copy) {
            self.end_copy_session(false, outcome);
        }
    }

    /// Close the menus that attach `view`, every menu opened above them, and
    /// the one-shot menus passed through to reach them, without running view
    /// hooks. A workspace list closed this way forgets its selection.
    pub(super) fn drop_view_menus(&mut self, view: ViewKind) {
        let Some(stack) = self.mode.stack() else {
            return;
        };
        let keymap = &self.config.keymap;
        if let Some(position) = stack
            .frames()
            .iter()
            .position(|id| keymap.menu(*id).view == Some(view))
        {
            let next = stack
                .truncated(position)
                .and_then(|stack| stack.without_passed_through(keymap));
            self.mode = ClientShellMode::from_stack(next);
        }
        if !self.workspace_list_active() {
            self.navigate_workspace_id = None;
        }
    }

    /// Close the workspace list menus, returning to copy mode or the
    /// terminal when nothing else stays open.
    pub(super) fn close_workspace_list(&mut self) {
        self.drop_view_menus(ViewKind::WorkspaceList);
        if self.mode == ClientShellMode::Terminal {
            self.mode = self.copy_or_terminal_mode();
        }
        self.navigate_workspace_id = None;
    }

    fn open_workspace_list(&mut self) {
        self.pending_workspace_highlight = None;
        self.mobile_switcher_scroll = 0;
        self.reveal_mobile_workspace = false;
        self.navigate_workspace_id = self.focused_navigation_target();
        self.reveal_navigation_workspace = true;
    }

    /// Route a key press through the keymap. Returns the target when the key
    /// belongs to a pane.
    pub(super) fn route_keymap_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> Option<ClientInputTarget> {
        if self.workspace_list_active() {
            self.pending_workspace_highlight = None;
        }
        let keymap = Arc::clone(&self.config.keymap);
        match keymap::resolve(&keymap, self.mode.stack(), key) {
            Step::Forward => self.focused_pane_id().map(ClientInputTarget::Pane),
            Step::Ignore => None,
            Step::Apply { next, mut effect } => {
                if let Effect::Run { binding, index } = &effect {
                    let action = binding_action(binding, *index);
                    if let Some(action) = action {
                        if !self.indexed_navigation_target_exists(&KeybindMatch::Action(action)) {
                            // The workspace list waits for a real choice;
                            // other menus close as if the key ran.
                            if self.top_view() == Some(ViewKind::WorkspaceList) {
                                return None;
                            }
                            effect = Effect::None;
                        }
                    }
                    let list_command = matches!(action, Some(KeybindAction::WorkspaceList(_)));
                    if matches!(effect, Effect::Run { .. })
                        && !list_command
                        && self.workspace_list_active()
                        && self.workspace_preview_action_blocked()
                    {
                        self.push_endpoint_notice(
                            ClientEndpointNoticeKind::Rejected,
                            "navigate_endpoint_inactive",
                            "Confirm workspace first",
                            "Select an available workspace and press Enter before using workspace or pane actions",
                        );
                        outcome.repaint = true;
                        return None;
                    }
                }
                let before = self.open_menus(next, outcome);
                let target = match effect {
                    Effect::None => None,
                    Effect::ForwardKey => self.focused_pane_id().map(ClientInputTarget::Pane),
                    Effect::Literal { menu } => self.send_literal(&keymap, menu, key, outcome),
                    Effect::Run { binding, index } => {
                        self.run_keymap_binding(&keymap, binding, index, outcome);
                        None
                    }
                };
                self.close_views(before, outcome);
                target
            }
        }
    }

    fn send_literal(
        &mut self,
        keymap: &CompiledKeymap,
        menu: MenuId,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> Option<ClientInputTarget> {
        let Some(Chord::Key(combo)) = keymap.menu(menu).entry_chord else {
            return None;
        };
        let target = self.focused_pane_id().map(ClientInputTarget::Pane)?;
        if crate::config::terminal_key_matches_combo(key, combo) {
            return Some(target);
        }
        self.push_pane_key(
            target,
            crate::input::TerminalKey::new(combo.0, combo.1),
            outcome,
        );
        None
    }

    fn run_keymap_binding(
        &mut self,
        keymap: &CompiledKeymap,
        binding: &CompiledBinding,
        index: Option<usize>,
        outcome: &mut ClientShellInput,
    ) {
        if let Some(action) = binding_action(binding, index) {
            self.record_binding(KeybindMatch::Action(action), outcome);
            return;
        }
        if let CompiledTarget::Command(command) = binding.target {
            if let Some(command) = keymap.commands.get(command) {
                self.record_binding(KeybindMatch::Command(command.clone()), outcome);
            }
        }
    }
}

/// Menu state before a key press, to open and close views as it changes.
#[derive(Clone, Copy)]
struct OpenViews {
    mode: ClientShellMode,
    list: bool,
    copy: bool,
}

/// The builtin action a binding runs, with its digit applied.
fn binding_action(binding: &CompiledBinding, index: Option<usize>) -> Option<KeybindAction> {
    match binding.target {
        CompiledTarget::Action(CatalogAction::Fixed(action)) => Some(action),
        CompiledTarget::Action(CatalogAction::Indexed(indexed)) => binding
            .digit_index(index)
            .map(|index| indexed.with_index(index)),
        _ => None,
    }
}
