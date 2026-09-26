use super::keymap::CompiledCommand;

/// What a key press runs: a builtin action, or a command defined in the
/// keymap that the server executes.
#[derive(Debug, Clone)]
pub(crate) enum KeybindMatch {
    Action(KeybindAction),
    Command(CompiledCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeybindAction {
    NewWorkspace,
    NewWorktree,
    OpenWorktree,
    RemoveWorktree,
    RenameWorkspace,
    CloseWorkspace,
    SwitchWorkspace(usize),
    SwitchTab(usize),
    FocusAgent(usize),
    PreviousWorkspace,
    NextWorkspace,
    PreviousAgent,
    NextAgent,
    NewTab,
    RenameTab,
    PreviousTab,
    NextTab,
    MoveTabPrevious,
    MoveTabNext,
    CloseTab,
    RenamePane,
    FocusPaneLeft,
    FocusPaneDown,
    FocusPaneUp,
    FocusPaneRight,
    SwapPaneLeft,
    SwapPaneDown,
    SwapPaneUp,
    SwapPaneRight,
    SplitVertical,
    SplitHorizontal,
    ClosePane,
    EditScrollback,
    ClearPane,
    Zoom,
    ResizePaneLeft,
    ResizePaneDown,
    ResizePaneUp,
    ResizePaneRight,
    ToggleSidebar,
    CyclePaneNext,
    CyclePanePrevious,
    LastPane,
    Help,
    Settings,
    ReloadConfig,
    OpenNotificationTarget,
    Detach,
    OpenNavigator,
    WhatsNew,
    WorkspaceList(WorkspaceListCommand),
    Copy(CopyCommand),
    NavigatorView(NavigatorCommand),
    HelpView(HelpCommand),
    SettingsView(SettingsCommand),
}

/// Commands for the workspace list view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceListCommand {
    Up,
    Down,
    Open,
}

/// Commands for the navigator view (the goto picker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigatorCommand {
    Search,
    Up,
    Down,
    PageUp,
    PageDown,
    SectionPrevious,
    SectionNext,
    Top,
    Bottom,
    FilterBlocked,
    FilterWorking,
    FilterIdle,
    FilterDone,
    FilterAll,
    FilterClear,
    Open,
    Close,
}

/// Commands for the keybind help view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelpCommand {
    Filter,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Close,
}

/// Commands for the settings view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsCommand {
    SectionNext,
    SectionPrevious,
    ChoiceUp,
    ChoiceDown,
    Apply,
    Close,
}

/// Commands for the copy view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyCommand {
    MoveLeft,
    MoveDown,
    MoveUp,
    MoveRight,
    WordNext,
    WordPrevious,
    WordEnd,
    BigWordNext,
    BigWordPrevious,
    BigWordEnd,
    ParagraphPrevious,
    ParagraphNext,
    LineStart,
    LineEnd,
    LineFirstNonBlank,
    HistoryStart,
    HistoryEnd,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    Select,
    SelectLine,
    SearchForward,
    SearchBackward,
    SearchNext,
    SearchReverse,
    Yank,
    Exit,
    Escape,
}
