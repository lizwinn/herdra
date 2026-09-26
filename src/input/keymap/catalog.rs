//! Every action a keymap leaf can name, with the labels the hint bar and the
//! help overlay show for it.

use crate::input::keybindings::{CopyCommand, KeybindAction, WorkspaceListCommand};

/// How a catalog entry turns into a runnable action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CatalogAction {
    Fixed(KeybindAction),
    Indexed(IndexedAction),
}

/// Actions that take the digit pressed on a `1..9` chord as their target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IndexedAction {
    SwitchWorkspace,
    SwitchTab,
    FocusAgent,
}

impl IndexedAction {
    pub(crate) fn with_index(self, index: usize) -> KeybindAction {
        match self {
            Self::SwitchWorkspace => KeybindAction::SwitchWorkspace(index),
            Self::SwitchTab => KeybindAction::SwitchTab(index),
            Self::FocusAgent => KeybindAction::FocusAgent(index),
        }
    }
}

/// Client views that a menu can attach. Actions that operate on a view need
/// a menu with that view somewhere in the active stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ViewKind {
    WorkspaceList,
    Copy,
}

impl ViewKind {
    pub(crate) const ALL: [Self; 2] = [Self::WorkspaceList, Self::Copy];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::WorkspaceList => "workspace-list",
            Self::Copy => "copy",
        }
    }

    pub(crate) fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.id() == id)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CatalogEntry {
    pub(crate) id: &'static str,
    pub(crate) action: CatalogAction,
    /// Short label for the hint bar.
    pub(crate) hint: &'static str,
    /// Longer label for the help overlay.
    pub(crate) description: &'static str,
    /// The view this action operates on, if any.
    pub(crate) view: Option<ViewKind>,
    /// The action closes its view itself when it succeeds, so the resolver
    /// never closes the menu around it.
    pub(crate) exits_itself: bool,
}

const fn fixed(
    id: &'static str,
    action: KeybindAction,
    hint: &'static str,
    description: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        id,
        action: CatalogAction::Fixed(action),
        hint,
        description,
        view: None,
        exits_itself: false,
    }
}

const fn indexed(
    id: &'static str,
    action: IndexedAction,
    hint: &'static str,
    description: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        id,
        action: CatalogAction::Indexed(action),
        hint,
        description,
        view: None,
        exits_itself: false,
    }
}

const fn list(
    id: &'static str,
    command: WorkspaceListCommand,
    hint: &'static str,
    description: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        id,
        action: CatalogAction::Fixed(KeybindAction::WorkspaceList(command)),
        hint,
        description,
        view: Some(ViewKind::WorkspaceList),
        exits_itself: false,
    }
}

const fn copy(
    id: &'static str,
    command: CopyCommand,
    hint: &'static str,
    description: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        id,
        action: CatalogAction::Fixed(KeybindAction::Copy(command)),
        hint,
        description,
        view: Some(ViewKind::Copy),
        exits_itself: false,
    }
}

const fn exits_itself(mut entry: CatalogEntry) -> CatalogEntry {
    entry.exits_itself = true;
    entry
}

use CopyCommand as C;
use KeybindAction as A;
use WorkspaceListCommand as L;

pub(crate) const CATALOG: &[CatalogEntry] = &[
    // workspaces
    fixed("workspace.new", A::NewWorkspace, "new", "new workspace"),
    fixed(
        "workspace.rename",
        A::RenameWorkspace,
        "rename",
        "rename workspace",
    ),
    fixed(
        "workspace.close",
        A::CloseWorkspace,
        "close",
        "close workspace",
    ),
    fixed("workspace.next", A::NextWorkspace, "next", "next workspace"),
    fixed(
        "workspace.previous",
        A::PreviousWorkspace,
        "previous",
        "previous workspace",
    ),
    indexed(
        "workspace.switch",
        IndexedAction::SwitchWorkspace,
        "switch",
        "switch workspace",
    ),
    list(
        "workspace.list.up",
        L::Up,
        "up",
        "move up the workspace list",
    ),
    list(
        "workspace.list.down",
        L::Down,
        "down",
        "move down the workspace list",
    ),
    exits_itself(list(
        "workspace.list.open",
        L::Open,
        "open",
        "open the selected workspace",
    )),
    // worktrees
    fixed("worktree.new", A::NewWorktree, "new", "new worktree"),
    fixed("worktree.open", A::OpenWorktree, "open", "open worktree"),
    fixed(
        "worktree.remove",
        A::RemoveWorktree,
        "remove",
        "delete worktree checkout",
    ),
    // tabs
    fixed("tab.new", A::NewTab, "new", "new tab"),
    fixed("tab.rename", A::RenameTab, "rename", "rename tab"),
    fixed("tab.close", A::CloseTab, "close", "close tab"),
    fixed("tab.next", A::NextTab, "next", "next tab"),
    fixed("tab.previous", A::PreviousTab, "previous", "previous tab"),
    fixed(
        "tab.move.left",
        A::MoveTabPrevious,
        "move left",
        "move tab left",
    ),
    fixed(
        "tab.move.right",
        A::MoveTabNext,
        "move right",
        "move tab right",
    ),
    indexed(
        "tab.switch",
        IndexedAction::SwitchTab,
        "switch",
        "switch tab",
    ),
    // panes
    fixed(
        "pane.split.right",
        A::SplitVertical,
        "split right",
        "split pane right",
    ),
    fixed(
        "pane.split.down",
        A::SplitHorizontal,
        "split down",
        "split pane down",
    ),
    fixed("pane.close", A::ClosePane, "close", "close pane"),
    fixed("pane.rename", A::RenamePane, "rename", "rename pane"),
    fixed("pane.zoom", A::Zoom, "zoom", "zoom pane"),
    fixed(
        "pane.scrollback",
        A::EditScrollback,
        "scrollback",
        "edit scrollback",
    ),
    fixed("pane.clear", A::ClearPane, "clear", "clear pane"),
    fixed(
        "pane.focus.left",
        A::FocusPaneLeft,
        "focus left",
        "focus pane left",
    ),
    fixed(
        "pane.focus.down",
        A::FocusPaneDown,
        "focus down",
        "focus pane down",
    ),
    fixed("pane.focus.up", A::FocusPaneUp, "focus up", "focus pane up"),
    fixed(
        "pane.focus.right",
        A::FocusPaneRight,
        "focus right",
        "focus pane right",
    ),
    fixed(
        "pane.swap.left",
        A::SwapPaneLeft,
        "swap left",
        "swap pane left",
    ),
    fixed(
        "pane.swap.down",
        A::SwapPaneDown,
        "swap down",
        "swap pane down",
    ),
    fixed("pane.swap.up", A::SwapPaneUp, "swap up", "swap pane up"),
    fixed(
        "pane.swap.right",
        A::SwapPaneRight,
        "swap right",
        "swap pane right",
    ),
    fixed(
        "pane.cycle.next",
        A::CyclePaneNext,
        "next",
        "cycle pane next",
    ),
    fixed(
        "pane.cycle.previous",
        A::CyclePanePrevious,
        "previous",
        "cycle pane previous",
    ),
    fixed("pane.last", A::LastPane, "last", "last pane"),
    fixed(
        "pane.resize.left",
        A::ResizePaneLeft,
        "left",
        "resize pane left",
    ),
    fixed(
        "pane.resize.down",
        A::ResizePaneDown,
        "down",
        "resize pane down",
    ),
    fixed("pane.resize.up", A::ResizePaneUp, "up", "resize pane up"),
    fixed(
        "pane.resize.right",
        A::ResizePaneRight,
        "right",
        "resize pane right",
    ),
    // agents
    fixed("agent.next", A::NextAgent, "next", "next agent"),
    fixed(
        "agent.previous",
        A::PreviousAgent,
        "previous",
        "previous agent",
    ),
    indexed(
        "agent.focus",
        IndexedAction::FocusAgent,
        "focus",
        "focus agent",
    ),
    fixed(
        "agent.notification",
        A::OpenNotificationTarget,
        "notification",
        "open notification target",
    ),
    // app
    fixed("app.help", A::Help, "keybinds", "keybinds"),
    fixed("app.settings", A::Settings, "settings", "settings"),
    fixed("app.reload", A::ReloadConfig, "reload", "reload config"),
    fixed("app.detach", A::Detach, "detach", "detach"),
    fixed(
        "app.navigator",
        A::OpenNavigator,
        "navigator",
        "session navigator",
    ),
    fixed("app.sidebar", A::ToggleSidebar, "sidebar", "toggle sidebar"),
    fixed("app.whats-new", A::WhatsNew, "what's new", "what's new"),
    // copy view
    copy("copy.move.left", C::MoveLeft, "left", "move left"),
    copy("copy.move.down", C::MoveDown, "down", "move down"),
    copy("copy.move.up", C::MoveUp, "up", "move up"),
    copy("copy.move.right", C::MoveRight, "right", "move right"),
    copy("copy.word.next", C::WordNext, "word", "next word start"),
    copy(
        "copy.word.previous",
        C::WordPrevious,
        "word back",
        "previous word start",
    ),
    copy("copy.word.end", C::WordEnd, "word end", "next word end"),
    copy(
        "copy.bigword.next",
        C::BigWordNext,
        "WORD",
        "next WORD start",
    ),
    copy(
        "copy.bigword.previous",
        C::BigWordPrevious,
        "WORD back",
        "previous WORD start",
    ),
    copy(
        "copy.bigword.end",
        C::BigWordEnd,
        "WORD end",
        "next WORD end",
    ),
    copy(
        "copy.paragraph.previous",
        C::ParagraphPrevious,
        "paragraph up",
        "previous paragraph",
    ),
    copy(
        "copy.paragraph.next",
        C::ParagraphNext,
        "paragraph down",
        "next paragraph",
    ),
    copy(
        "copy.line.start",
        C::LineStart,
        "line start",
        "start of line",
    ),
    copy("copy.line.end", C::LineEnd, "line end", "end of line"),
    copy(
        "copy.line.first-nonblank",
        C::LineFirstNonBlank,
        "first char",
        "first non-blank character",
    ),
    copy(
        "copy.history.start",
        C::HistoryStart,
        "top",
        "start of history",
    ),
    copy(
        "copy.history.end",
        C::HistoryEnd,
        "bottom",
        "end of history",
    ),
    copy("copy.page.up", C::PageUp, "page up", "page up"),
    copy("copy.page.down", C::PageDown, "page down", "page down"),
    copy(
        "copy.halfpage.up",
        C::HalfPageUp,
        "half page up",
        "half page up",
    ),
    copy(
        "copy.halfpage.down",
        C::HalfPageDown,
        "half page down",
        "half page down",
    ),
    copy("copy.select", C::Select, "select", "start selection"),
    copy(
        "copy.select.line",
        C::SelectLine,
        "select line",
        "start line selection",
    ),
    copy(
        "copy.search.forward",
        C::SearchForward,
        "search",
        "search forward",
    ),
    copy(
        "copy.search.backward",
        C::SearchBackward,
        "search back",
        "search backward",
    ),
    copy("copy.search.next", C::SearchNext, "repeat", "repeat search"),
    copy(
        "copy.search.reverse",
        C::SearchReverse,
        "reverse",
        "repeat search in reverse",
    ),
    exits_itself(copy(
        "copy.yank",
        C::Yank,
        "copy",
        "copy selection and exit",
    )),
    exits_itself(copy("copy.exit", C::Exit, "exit", "exit copy mode")),
    exits_itself(copy(
        "copy.escape",
        C::Escape,
        "clear",
        "clear selection or search, else exit",
    )),
];

pub(crate) fn lookup(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|entry| entry.id == id)
}

/// Closest catalog id for a misspelled one, for diagnostics.
pub(crate) fn suggestion(id: &str) -> Option<&'static str> {
    CATALOG
        .iter()
        .map(|entry| (edit_distance(id, entry.id), entry.id))
        .filter(|(distance, _)| *distance <= 3)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, id)| id)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.chars().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(row + 1);
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            let insertion = current[column] + 1;
            let deletion = previous[column + 1] + 1;
            current.push(substitution.min(insertion).min(deletion));
        }
        previous = current;
    }
    previous[right.len()]
}
