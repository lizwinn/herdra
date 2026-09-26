//! Reads `keymap.kdl` text into one layer of raw tree nodes.
//!
//! Grammar, one node per binding:
//!
//! ```kdl
//! <chord> <target> [hint] [exit|stay|hidden] [hint=… priority=…]
//! <chord> [title] [sticky|mode|replace|hidden] [view=… unmatched=… bar=…] { children }
//! ```
//!
//! A node with a children block is a menu; its first argument is its title.
//! Anything else is a leaf; its first argument is the target it runs.

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

use super::catalog::{self, CatalogAction, CatalogEntry, ViewKind};
use super::chord::Chord;
use crate::config::KeyCombo;
use crate::popup_size::PopupSize;

/// The prefix chord when a keymap does not set one.
pub(crate) const DEFAULT_PREFIX: KeyCombo = (
    crossterm::event::KeyCode::Char('b'),
    crossterm::event::KeyModifiers::CONTROL,
);

/// Menus can nest this many levels below the top level.
pub(crate) const MAX_MENU_DEPTH: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Base {
    Herdra,
    Classic,
    Empty,
}

impl Base {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Herdra => "herdra",
            Self::Classic => "classic",
            Self::Empty => "none",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LayerOwner {
    Builtin,
    Plugin(String),
    User,
}

impl LayerOwner {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Builtin => "builtin".to_owned(),
            Self::Plugin(id) => format!("plugin:{id}"),
            Self::User => "user".to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct KeymapLayer {
    /// Name shown in diagnostics, usually a file name.
    pub(crate) source: String,
    pub(crate) owner: LayerOwner,
    pub(crate) nodes: Vec<RawNode>,
}

/// Settings from a user keymap's `base` node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LayerSettings {
    pub(crate) base: Option<Base>,
    pub(crate) prefix: Option<KeyCombo>,
}

#[derive(Clone, Debug)]
pub(crate) struct RawNode {
    pub(crate) chord: Chord,
    pub(crate) line: usize,
    pub(crate) owner: LayerOwner,
    pub(crate) body: RawBody,
}

#[derive(Clone, Debug)]
pub(crate) enum RawBody {
    Leaf(RawLeaf),
    Menu(RawMenu),
    Unbind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExitPolicy {
    Exit,
    Stay,
}

#[derive(Clone, Debug)]
pub(crate) struct RawLeaf {
    pub(crate) target: LeafTarget,
    pub(crate) hint: Option<String>,
    pub(crate) exit: Option<ExitPolicy>,
    pub(crate) hidden: bool,
    pub(crate) priority: i32,
}

#[derive(Clone, Debug)]
pub(crate) enum LeafTarget {
    Action(&'static CatalogEntry),
    Command(CommandSpec),
    /// Open the menu declared with this `id`.
    Open(String),
    Back,
    Cancel,
    Literal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandKind {
    Shell,
    Pane,
    Popup,
    Plugin,
}

impl CommandKind {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "shell" => Some(Self::Shell),
            "pane" => Some(Self::Pane),
            "popup" => Some(Self::Popup),
            "plugin" => Some(Self::Plugin),
            _ => None,
        }
    }

    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Pane => "pane",
            Self::Popup => "popup",
            Self::Plugin => "plugin",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    pub(crate) kind: CommandKind,
    pub(crate) command: String,
    pub(crate) width: Option<PopupSize>,
    pub(crate) height: Option<PopupSize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unmatched {
    Forward,
    Ignore,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarVisibility {
    Full,
    Badge,
    Hidden,
}

impl From<crate::config::ModeHintBarConfig> for BarVisibility {
    fn from(config: crate::config::ModeHintBarConfig) -> Self {
        match config {
            crate::config::ModeHintBarConfig::Full => Self::Full,
            crate::config::ModeHintBarConfig::Badge => Self::Badge,
            crate::config::ModeHintBarConfig::Hidden => Self::Hidden,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RawMenu {
    pub(crate) id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) hint: Option<String>,
    pub(crate) sticky: Option<bool>,
    pub(crate) anchor: Option<bool>,
    pub(crate) view: Option<ViewKind>,
    pub(crate) unmatched: Option<Unmatched>,
    pub(crate) bar: Option<BarVisibility>,
    pub(crate) hidden: Option<bool>,
    pub(crate) priority: Option<i32>,
    pub(crate) replace: bool,
    pub(crate) children: Vec<RawNode>,
}

struct Context<'a> {
    text: &'a str,
    source: &'a str,
    owner: &'a LayerOwner,
    prefix: KeyCombo,
    diagnostics: &'a mut Vec<String>,
    /// A line whose unquoted chord was cut short by a KDL terminator; the
    /// fragments that follow on that line are not reported again.
    truncated_line: Option<usize>,
}

impl Context<'_> {
    fn line(&self, offset: usize) -> usize {
        let end = offset.min(self.text.len());
        let prefix = self.text.get(..end).unwrap_or(self.text);
        prefix.bytes().filter(|byte| *byte == b'\n').count() + 1
    }

    fn report(&mut self, line: usize, message: impl AsRef<str>) {
        let diagnostic = format!("keymap {}:{line}: {}", self.source, message.as_ref());
        tracing::warn!(message = %diagnostic, "keymap diagnostic");
        self.diagnostics.push(diagnostic);
    }
}

/// Parse KDL text, reporting syntax errors. A file that is not valid KDL
/// contributes no bindings.
pub(crate) fn parse_document(
    text: &str,
    source: &str,
    diagnostics: &mut Vec<String>,
) -> Option<KdlDocument> {
    match text.parse::<KdlDocument>() {
        Ok(document) => Some(document),
        Err(error) => {
            let owner = LayerOwner::User;
            let mut context = Context {
                text,
                source,
                owner: &owner,
                prefix: DEFAULT_PREFIX,
                diagnostics,
                truncated_line: None,
            };
            if error.diagnostics.is_empty() {
                context.report(1, "not valid KDL; ignoring this file");
            }
            for diagnostic in &error.diagnostics {
                let line = context.line(diagnostic.span.offset());
                let message = diagnostic
                    .message
                    .clone()
                    .unwrap_or_else(|| "not valid KDL".to_owned());
                let help = diagnostic
                    .help
                    .as_ref()
                    .map(|help| format!(" ({help})"))
                    .unwrap_or_default();
                context.report(line, format!("{message}{help}; ignoring this file"));
            }
            None
        }
    }
}

/// Read the `base` node: which tree to start from and the prefix chord.
pub(crate) fn document_settings(
    document: &KdlDocument,
    text: &str,
    source: &str,
    diagnostics: &mut Vec<String>,
) -> LayerSettings {
    let owner = LayerOwner::User;
    let mut context = Context {
        text,
        source,
        owner: &owner,
        prefix: DEFAULT_PREFIX,
        diagnostics,
        truncated_line: None,
    };
    let mut settings = LayerSettings::default();
    let mut seen = false;
    for node in document.nodes() {
        if node.name().value() != "base" {
            continue;
        }
        let line = context.line(node.span().offset());
        if seen {
            context.report(line, "base is set more than once; keeping the first");
            continue;
        }
        seen = true;
        for entry in positional(node) {
            settings.base = match entry.value().as_string() {
                Some("herdra") => Some(Base::Herdra),
                Some("classic") => Some(Base::Classic),
                Some("none") => Some(Base::Empty),
                other => {
                    context.report(
                        line,
                        format!(
                            "unknown base {:?}; use herdra, classic, or none",
                            other.unwrap_or_default()
                        ),
                    );
                    None
                }
            };
        }
        for (name, value) in properties(node) {
            match name {
                "prefix" => match value.as_string().map(Chord::parse) {
                    Some(Ok(Chord::Key(combo))) => settings.prefix = Some(combo),
                    Some(Ok(Chord::Digits(_))) => {
                        context.report(line, "prefix must be a single chord")
                    }
                    Some(Err(error)) => context.report(line, format!("prefix: {error}")),
                    None => context.report(line, "prefix must be a chord like ctrl+a"),
                },
                other => context.report(line, format!("unknown property {other:?}")),
            }
        }
    }
    settings
}

/// Convert a parsed document into a layer of nodes. `prefix` is the chord
/// that the `prefix` placeholder stands for.
pub(crate) fn layer_from_document(
    document: &KdlDocument,
    text: &str,
    source: &str,
    owner: LayerOwner,
    prefix: KeyCombo,
    diagnostics: &mut Vec<String>,
) -> KeymapLayer {
    let mut context = Context {
        text,
        source,
        owner: &owner,
        prefix,
        diagnostics,
        truncated_line: None,
    };
    let mut layer = KeymapLayer {
        source: source.to_owned(),
        owner: owner.clone(),
        nodes: Vec::new(),
    };
    for node in document.nodes() {
        if node.name().value() == "base" {
            if owner != LayerOwner::User {
                let line = context.line(node.span().offset());
                context.report(line, "base is only allowed in your keymap.kdl");
            }
            continue;
        }
        if let Some(raw) = parse_node(node, 1, &mut context) {
            push_unique(&mut layer.nodes, raw, &mut context);
        }
    }
    layer
}

/// Parse a whole keymap file into a layer.
pub(crate) fn parse_layer(
    text: &str,
    source: &str,
    owner: LayerOwner,
    prefix: KeyCombo,
    diagnostics: &mut Vec<String>,
) -> KeymapLayer {
    match parse_document(text, source, diagnostics) {
        Some(document) => layer_from_document(&document, text, source, owner, prefix, diagnostics),
        None => KeymapLayer {
            source: source.to_owned(),
            owner,
            nodes: Vec::new(),
        },
    }
}

fn push_unique(nodes: &mut Vec<RawNode>, node: RawNode, context: &mut Context<'_>) {
    if let Some(existing) = nodes
        .iter()
        .find(|existing| existing.chord.overlaps(node.chord))
    {
        let first_line = existing.line;
        let label = node.chord.label();
        context.report(
            node.line,
            format!("{label} is already bound on line {first_line}; keeping the first binding"),
        );
        return;
    }
    nodes.push(node);
}

fn parse_node(node: &KdlNode, depth: usize, context: &mut Context<'_>) -> Option<RawNode> {
    let line = context.line(node.span().offset());
    let name = node.name().value();
    let parsed = if name == "prefix" {
        Ok(Chord::Key(context.prefix))
    } else {
        Chord::parse(name)
    };
    let chord = match parsed {
        Ok(chord) => chord,
        Err(error) => {
            if context.truncated_line == Some(line) {
                return None;
            }
            if name.ends_with('+') {
                context.truncated_line = Some(line);
            }
            context.report(line, error);
            return None;
        }
    };
    if depth == 1 && chord.intercepts_typing() {
        context.report(
            line,
            format!(
                "{} at the top level would intercept typing in panes; add a modifier or bind it inside a menu",
                chord.label()
            ),
        );
        return None;
    }
    let body = if let Some(children) = node.children() {
        if matches!(chord, Chord::Digits(_)) {
            context.report(line, "1..9 cannot open a menu");
            return None;
        }
        if depth > MAX_MENU_DEPTH {
            context.report(
                line,
                format!("menus nest at most {MAX_MENU_DEPTH} levels deep"),
            );
            return None;
        }
        RawBody::Menu(parse_menu(node, children, depth, line, context)?)
    } else {
        parse_leaf(node, chord, line, context)?
    };
    Some(RawNode {
        chord,
        line,
        owner: context.owner.clone(),
        body,
    })
}

fn positional(node: &KdlNode) -> impl Iterator<Item = &KdlEntry> {
    node.entries().iter().filter(|entry| entry.name().is_none())
}

fn properties(node: &KdlNode) -> impl Iterator<Item = (&str, &KdlValue)> {
    node.entries()
        .iter()
        .filter_map(|entry| entry.name().map(|name| (name.value(), entry.value())))
}

fn bool_value(value: &KdlValue) -> Option<bool> {
    value.as_bool()
}

fn priority_value(value: &KdlValue) -> Option<i32> {
    value
        .as_integer()
        .and_then(|value| i32::try_from(value).ok())
}

fn popup_size(value: &KdlValue) -> Result<PopupSize, String> {
    if let Some(cells) = value.as_integer() {
        return u16::try_from(cells)
            .map(PopupSize::Cells)
            .map_err(|_| "popup size is out of range".to_owned());
    }
    if let Some(text) = value.as_string() {
        return PopupSize::parse_cli(text);
    }
    Err("popup size must be a number of cells or a percentage like \"80%\"".to_owned())
}

fn parse_leaf(
    node: &KdlNode,
    chord: Chord,
    line: usize,
    context: &mut Context<'_>,
) -> Option<RawBody> {
    let mut args = positional(node);
    let Some(first) = args.next() else {
        context.report(
            line,
            format!("{} needs a target, for example tab.new", chord.label()),
        );
        return None;
    };
    let Some(word) = first.value().as_string() else {
        context.report(line, "the target must be a word like tab.new");
        return None;
    };
    let target = match word {
        "" | "none" => return Some(RawBody::Unbind),
        "menu.back" => LeafTarget::Back,
        "menu.cancel" => LeafTarget::Cancel,
        "menu.literal" => LeafTarget::Literal,
        "menu.open" => {
            let Some(id) = args.next().and_then(|entry| entry.value().as_string()) else {
                context.report(
                    line,
                    "menu.open needs the id of a menu, for example menu.open resize",
                );
                return None;
            };
            LeafTarget::Open(id.to_owned())
        }
        word => {
            if let Some(kind) = CommandKind::parse(word) {
                let Some(command) = args.next().and_then(|entry| entry.value().as_string()) else {
                    context.report(
                        line,
                        format!("{word} needs a command in quotes, for example {word} \"lazygit\""),
                    );
                    return None;
                };
                if command.trim().is_empty() {
                    context.report(line, format!("{word} command is empty"));
                    return None;
                }
                let command = match (kind, context.owner) {
                    (CommandKind::Plugin, LayerOwner::Plugin(plugin_id))
                        if !command.contains('.') =>
                    {
                        format!("{plugin_id}.{command}")
                    }
                    _ => command.to_owned(),
                };
                LeafTarget::Command(CommandSpec {
                    kind,
                    command,
                    width: None,
                    height: None,
                })
            } else if let Some(entry) = catalog::lookup(word) {
                LeafTarget::Action(entry)
            } else {
                let hint = catalog::suggestion(word)
                    .map(|suggestion| format!("; did you mean {suggestion}?"))
                    .unwrap_or_default();
                context.report(line, format!("unknown action {word:?}{hint}"));
                return None;
            }
        }
    };

    let indexed = matches!(
        target,
        LeafTarget::Action(CatalogEntry {
            action: CatalogAction::Indexed(_),
            ..
        })
    );
    match (chord, indexed) {
        (Chord::Digits(_), false) => {
            context.report(
                line,
                "1..9 can only run workspace.switch, tab.switch, or agent.focus",
            );
            return None;
        }
        (Chord::Key((code, _)), true)
            if !matches!(code, crossterm::event::KeyCode::Char('1'..='9')) =>
        {
            context.report(line, format!("{word} needs a 1..9 chord or a single digit"));
            return None;
        }
        _ => {}
    }

    let mut leaf = RawLeaf {
        target,
        hint: None,
        exit: None,
        hidden: false,
        priority: 0,
    };
    for entry in args {
        match entry.value().as_string() {
            Some("exit") => leaf.exit = Some(ExitPolicy::Exit),
            Some("stay") => leaf.exit = Some(ExitPolicy::Stay),
            Some("hidden") => leaf.hidden = true,
            Some(hint) if leaf.hint.is_none() => leaf.hint = Some(hint.to_owned()),
            Some(extra) => {
                context.report(line, format!("unexpected argument {extra:?}"));
            }
            None => context.report(line, "arguments after the target must be words or strings"),
        }
    }
    for (name, value) in properties(node) {
        match name {
            "hint" => match value.as_string() {
                Some(hint) => leaf.hint = Some(hint.to_owned()),
                None => context.report(line, "hint must be a string"),
            },
            "priority" => match priority_value(value) {
                Some(priority) => leaf.priority = priority,
                None => context.report(line, "priority must be a whole number"),
            },
            "exit" => match bool_value(value) {
                Some(true) => leaf.exit = Some(ExitPolicy::Exit),
                Some(false) => leaf.exit = Some(ExitPolicy::Stay),
                None => context.report(line, "exit must be #true or #false"),
            },
            "hidden" => match bool_value(value) {
                Some(hidden) => leaf.hidden = hidden,
                None => context.report(line, "hidden must be #true or #false"),
            },
            "width" | "height" => {
                let LeafTarget::Command(spec) = &mut leaf.target else {
                    context.report(line, format!("{name} only applies to popup commands"));
                    continue;
                };
                if spec.kind != CommandKind::Popup {
                    context.report(line, format!("{name} only applies to popup commands"));
                    continue;
                }
                match popup_size(value) {
                    Ok(size) if name == "width" => spec.width = Some(size),
                    Ok(size) => spec.height = Some(size),
                    Err(error) => context.report(line, format!("{name}: {error}")),
                }
            }
            other => context.report(line, format!("unknown property {other:?}")),
        }
    }
    Some(RawBody::Leaf(leaf))
}

fn parse_menu(
    node: &KdlNode,
    children: &KdlDocument,
    depth: usize,
    line: usize,
    context: &mut Context<'_>,
) -> Option<RawMenu> {
    let mut menu = RawMenu::default();
    for entry in positional(node) {
        match entry.value().as_string() {
            Some("sticky") => menu.sticky = Some(true),
            Some("mode") => menu.anchor = Some(true),
            Some("replace") => menu.replace = true,
            Some("hidden") => menu.hidden = Some(true),
            Some(title) if menu.title.is_none() => menu.title = Some(title.to_owned()),
            Some(extra) => context.report(line, format!("unexpected argument {extra:?}")),
            None => context.report(line, "menu arguments must be words or strings"),
        }
    }
    for (name, value) in properties(node) {
        match name {
            "title" => match value.as_string() {
                Some(title) => menu.title = Some(title.to_owned()),
                None => context.report(line, "title must be a string"),
            },
            "id" => match value.as_string() {
                Some(id) if !id.is_empty() => menu.id = Some(id.to_owned()),
                _ => context.report(line, "id must be a non-empty word"),
            },
            "hint" => match value.as_string() {
                Some(hint) => menu.hint = Some(hint.to_owned()),
                None => context.report(line, "hint must be a string"),
            },
            "sticky" => match bool_value(value) {
                Some(sticky) => menu.sticky = Some(sticky),
                None => context.report(line, "sticky must be #true or #false"),
            },
            "mode" => match bool_value(value) {
                Some(anchor) => menu.anchor = Some(anchor),
                None => context.report(line, "mode must be #true or #false"),
            },
            "replace" => match bool_value(value) {
                Some(replace) => menu.replace = replace,
                None => context.report(line, "replace must be #true or #false"),
            },
            "hidden" => match bool_value(value) {
                Some(hidden) => menu.hidden = Some(hidden),
                None => context.report(line, "hidden must be #true or #false"),
            },
            "priority" => match priority_value(value) {
                Some(priority) => menu.priority = Some(priority),
                None => context.report(line, "priority must be a whole number"),
            },
            "view" => match value.as_string().and_then(ViewKind::parse) {
                Some(view) => menu.view = Some(view),
                None => {
                    let known = ViewKind::ALL
                        .iter()
                        .map(|view| view.id())
                        .collect::<Vec<_>>()
                        .join(", ");
                    context.report(line, format!("unknown view; known views: {known}"));
                }
            },
            "unmatched" => match value.as_string() {
                Some("forward") => menu.unmatched = Some(Unmatched::Forward),
                Some("ignore") => menu.unmatched = Some(Unmatched::Ignore),
                Some("cancel") => menu.unmatched = Some(Unmatched::Cancel),
                _ => context.report(line, "unmatched must be forward, ignore, or cancel"),
            },
            "bar" => match value.as_string() {
                Some("full") => menu.bar = Some(BarVisibility::Full),
                Some("badge") => menu.bar = Some(BarVisibility::Badge),
                Some("hidden") => menu.bar = Some(BarVisibility::Hidden),
                _ => context.report(line, "bar must be full, badge, or hidden"),
            },
            other => context.report(line, format!("unknown property {other:?}")),
        }
    }
    for child in children.nodes() {
        if child.name().value() == "base" {
            let child_line = context.line(child.span().offset());
            context.report(child_line, "base is only allowed at the top level");
            continue;
        }
        if let Some(raw) = parse_node(child, depth + 1, context) {
            push_unique(&mut menu.children, raw, context);
        }
    }
    Some(menu)
}

/// Replace the command text of `shell`, `pane`, and `popup` leaves with a
/// placeholder, so a server can share its keymap with clients without sharing
/// the commands it runs. Clients invoke commands by chord path.
pub(crate) fn redact_commands(text: &str) -> Option<String> {
    let mut document = text.parse::<KdlDocument>().ok()?;
    fn walk(document: &mut KdlDocument) {
        for node in document.nodes_mut() {
            if let Some(children) = node.children_mut().as_mut() {
                walk(children);
                continue;
            }
            let positional = node
                .entries()
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.name().is_none())
                .map(|(index, _)| index)
                .take(2)
                .collect::<Vec<_>>();
            let is_command = positional
                .first()
                .and_then(|index| node.entries()[*index].value().as_string())
                .is_some_and(|word| matches!(word, "shell" | "pane" | "popup"));
            if let (true, Some(index)) = (is_command, positional.get(1)) {
                node.entries_mut()[*index] = KdlEntry::new("…");
            }
        }
    }
    walk(&mut document);
    Some(document.to_string())
}
