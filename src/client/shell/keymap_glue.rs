//! Connects the keymap engine to the client shell: routes key presses,
//! applies menu stack changes, and opens and closes the views menus attach.

use std::sync::Arc;

use super::*;
use crate::input::keymap::{
    self, CatalogAction, Chord, CompiledBinding, CompiledKeymap, CompiledMenu, CompiledTarget,
    Effect, MenuId, MenuStack, Step, ViewKind,
};
use crate::input::{KeybindAction, KeybindMatch};

/// Views drawn as popup overlays. Each has a menu on the stack while its
/// popup is open, and the menu's keys drive the popup.
const POPUP_VIEWS: [ViewKind; 3] = [ViewKind::Navigator, ViewKind::Help, ViewKind::Settings];

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

    /// The view of the popup overlay that is open, if it is one.
    pub(super) fn overlay_view(&self) -> Option<ViewKind> {
        match self.overlay.as_ref()? {
            ClientShellOverlay::Navigator(_) => Some(ViewKind::Navigator),
            ClientShellOverlay::Help(_) => Some(ViewKind::Help),
            ClientShellOverlay::Settings(_) => Some(ViewKind::Settings),
            _ => None,
        }
    }

    /// The menu whose keys drive the open popup: the highest open menu with
    /// its view, else the first menu in the keymap that has it.
    pub(super) fn overlay_view_menu(&self) -> Option<&CompiledMenu> {
        let view = self.overlay_view()?;
        let keymap = &self.config.keymap;
        self.mode
            .stack()
            .and_then(|stack| {
                stack
                    .frames()
                    .iter()
                    .rev()
                    .copied()
                    .find(|id| keymap.menu(*id).view == Some(view))
            })
            .or_else(|| keymap.menu_with_view(view))
            .map(|id| keymap.menu(id))
    }

    /// Open the first menu with `view` over the open menus, unless an open
    /// menu already has it. For popups opened without their key: a mouse
    /// click, the global menu, or a leaf such as `app.help`.
    pub(super) fn open_view_menu(&mut self, view: ViewKind) {
        if self.view_active(view) {
            return;
        }
        let Some(menu) = self.config.keymap.menu_with_view(view) else {
            return;
        };
        self.mode = ClientShellMode::Menu(match self.mode.stack() {
            Some(stack) => stack.pushed(menu),
            None => MenuStack::single(menu),
        });
    }

    /// Keep popups and their menus in step: an open popup has its menu on
    /// the stack, and a popup's menu closes when the popup closes. Popups
    /// open and close outside the key path too (mouse clicks, endpoint
    /// results, keymap reloads), so this runs whenever input settles.
    pub(super) fn reconcile_overlay_views(&mut self) {
        let open = self.overlay_view();
        for view in POPUP_VIEWS {
            if Some(view) != open && self.view_active(view) {
                self.drop_view_menus(view);
                if self.mode == ClientShellMode::Terminal {
                    self.mode = self.copy_or_terminal_mode();
                }
            }
        }
        if let Some(view) = open {
            self.open_view_menu(view);
        }
    }

    fn popup_views_active(&self) -> [bool; POPUP_VIEWS.len()] {
        POPUP_VIEWS.map(|view| self.view_active(view))
    }

    /// Open the popup for a view whose menu just opened. Another popup in
    /// place of settings closes settings first, undoing its theme preview.
    fn open_popup(&mut self, view: ViewKind) {
        if view != ViewKind::Settings
            && self.overlay_view() == Some(ViewKind::Settings)
            && !self.close_settings_overlay()
        {
            return;
        }
        match view {
            ViewKind::Navigator => self.open_navigator_overlay(),
            ViewKind::Help => self.open_help_overlay(),
            ViewKind::Settings => self.open_settings_overlay(),
            ViewKind::WorkspaceList | ViewKind::Copy => {}
        }
    }

    /// Close the popup for a view whose menu just closed. Settings stays
    /// open while it installs integrations, and its menu comes back.
    fn close_popup(&mut self, view: ViewKind, outcome: &mut ClientShellInput) {
        if self.overlay_view() != Some(view) {
            return;
        }
        if view == ViewKind::Settings {
            if self.close_settings_overlay() {
                outcome.repaint = true;
            }
            return;
        }
        self.overlay = None;
        outcome.repaint = true;
    }

    /// Scroll a keybind list that just opened to the keys of `menu`, the
    /// menu `?` was pressed in. The main menus under the top level keep
    /// the list at the top, where the prefix-free chords are.
    fn focus_help_on(&mut self, menu: MenuId) {
        let keymap = &self.config.keymap;
        if keymap.menu(menu).parent == Some(MenuId::TOP) {
            return;
        }
        let scroll = render::help_scroll_to_menu(keymap, menu, self.last_composed_size);
        if let Some(ClientShellOverlay::Help(help)) = self.overlay.as_mut() {
            help.scroll = scroll;
        }
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
        let popups_before = self.popup_views_active();
        let mut before = OpenViews {
            mode: self.mode,
            list: self.workspace_list_active(),
            copy: self.view_active(ViewKind::Copy),
            closed_popups: [false; POPUP_VIEWS.len()],
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
        let popups = self.popup_views_active();
        for (index, view) in POPUP_VIEWS.into_iter().enumerate() {
            if popups[index] && !popups_before[index] && self.overlay_view() != Some(view) {
                self.open_popup(view);
                outcome.repaint = true;
            }
            // Only the key's own menu change closes a popup; an action that
            // rebuilds the keymap resets the menus but keeps its popup.
            before.closed_popups[index] = popups_before[index] && !popups[index];
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
        for (index, view) in POPUP_VIEWS.into_iter().enumerate() {
            if before.closed_popups[index] && !self.view_active(view) {
                self.close_popup(view, outcome);
            }
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
        let popup_focused = self.top_view().is_some_and(ViewKind::is_overlay);
        if self.workspace_list_active() && !popup_focused {
            self.pending_workspace_highlight = None;
        }
        let keymap = Arc::clone(&self.config.keymap);
        // The menu `?` is pressed in, for a keybind list opened by this key.
        let source = self.mode.stack().map(|stack| stack.top());
        let help_was_open = self.overlay_view() == Some(ViewKind::Help);
        let target = self.resolve_keymap_key(&keymap, key, outcome);
        if !help_was_open && self.overlay_view() == Some(ViewKind::Help) {
            if let Some(menu) = source {
                self.focus_help_on(menu);
            }
        }
        self.reconcile_overlay_views();
        target
    }

    fn resolve_keymap_key(
        &mut self,
        keymap: &CompiledKeymap,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> Option<ClientInputTarget> {
        match keymap::resolve(keymap, self.mode.stack(), key) {
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
                    // View commands act on their view, not on the workspace
                    // the list previews.
                    let view_command = matches!(
                        action,
                        Some(
                            KeybindAction::WorkspaceList(_)
                                | KeybindAction::NavigatorView(_)
                                | KeybindAction::HelpView(_)
                                | KeybindAction::SettingsView(_)
                        )
                    );
                    if matches!(effect, Effect::Run { .. })
                        && !view_command
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
                    Effect::Literal { menu } => self.send_literal(keymap, menu, key, outcome),
                    Effect::Run { binding, index } => {
                        self.run_keymap_binding(keymap, binding, index, outcome);
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
    /// Popups whose menus the key closed.
    closed_popups: [bool; POPUP_VIEWS.len()],
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
