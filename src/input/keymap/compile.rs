//! Merges keymap layers and compiles the merged tree into the menus that the
//! resolver, the hint bar, and the help overlay read.

use super::catalog::{self, CatalogAction, ViewKind};
use super::chord::{display_label, Chord};
use super::parse::{
    document_settings, layer_from_document, parse_document, parse_layer, BarVisibility, Base,
    CommandSpec, ExitPolicy, KeymapLayer, LayerOwner, LeafTarget, RawBody, RawMenu, RawNode,
    Unmatched, DEFAULT_PREFIX,
};
use crossterm::event::{KeyCode, KeyModifiers};

pub(crate) const DEFAULT_KEYMAP: &str = include_str!("default.kdl");
pub(crate) const CLASSIC_KEYMAP: &str = include_str!("classic.kdl");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MenuId(pub(crate) u16);

impl MenuId {
    /// The top level: keys pressed while typing in a pane.
    pub(crate) const TOP: Self = Self(0);

    pub(crate) fn index(self) -> usize {
        usize::from(self.0)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum CompiledTarget {
    Action(CatalogAction),
    Command(usize),
    Enter(MenuId),
    Back,
    Cancel,
    Literal,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledBinding {
    pub(crate) chord: Chord,
    pub(crate) target: CompiledTarget,
    pub(crate) exit: Option<ExitPolicy>,
    /// The action closes its own view, so the resolver keeps the menu open.
    pub(crate) exits_itself: bool,
    pub(crate) hint: String,
    pub(crate) description: String,
    pub(crate) hidden: bool,
    pub(crate) priority: i32,
    pub(crate) owner: LayerOwner,
}

impl CompiledBinding {
    /// The zero-based target of an indexed action: the digit matched on a
    /// `1..9` chord, or the digit of a single-digit chord.
    pub(crate) fn digit_index(&self, matched: Option<usize>) -> Option<usize> {
        matched.or(match self.chord {
            Chord::Key((KeyCode::Char(digit @ '1'..='9'), _)) => {
                Some(usize::from(digit as u8 - b'1'))
            }
            _ => None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SegmentKind {
    Exit,
    Action,
    Sticky,
    Submenu,
    Help,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BarSegment {
    pub(crate) keys: String,
    pub(crate) label: String,
    pub(crate) kind: SegmentKind,
    /// The catalog action this hint runs, so views can show live state
    /// (for example copy mode's selection and match count).
    pub(crate) action_id: Option<&'static str>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BarPlan {
    pub(crate) segments: Vec<BarSegment>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledMenu {
    pub(crate) parent: Option<MenuId>,
    pub(crate) entry_chord: Option<Chord>,
    /// Chords from the top level to this menu, space separated.
    pub(crate) path_label: String,
    pub(crate) title: String,
    pub(crate) badge: String,
    pub(crate) sticky: bool,
    pub(crate) anchor: bool,
    pub(crate) view: Option<ViewKind>,
    pub(crate) unmatched: Unmatched,
    /// `None` follows the `[ui] mode_hint_bar` setting.
    pub(crate) bar: Option<BarVisibility>,
    pub(crate) bindings: Vec<CompiledBinding>,
    pub(crate) bar_plan: BarPlan,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledCommand {
    /// Chords that reach this command, space separated. Client and server
    /// agree on command identity through this label.
    pub(crate) path_label: String,
    pub(crate) spec: CommandSpec,
    pub(crate) hint: String,
    /// Which layer defined the command; plugin commands run on the server
    /// that owns the plugin.
    pub(crate) owner: LayerOwner,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HelpGroup {
    pub(crate) title: String,
    pub(crate) entries: Vec<(String, String)>,
}

/// A user or plugin keymap file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeymapText {
    pub(crate) source: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledKeymap {
    pub(crate) menus: Vec<CompiledMenu>,
    pub(crate) common: Vec<CompiledBinding>,
    pub(crate) commands: Vec<CompiledCommand>,
    pub(crate) help: Vec<HelpGroup>,
    /// Problems in the user's keymap file.
    pub(crate) diagnostics: Vec<String>,
    /// Bindings that plugins could not claim, and plugin file problems.
    pub(crate) conflicts: Vec<String>,
    pub(crate) base: Base,
    /// The chord the `prefix` placeholder stands for.
    pub(crate) prefix: crate::config::KeyCombo,
}

impl Default for CompiledKeymap {
    fn default() -> Self {
        Self::build(None, &[])
    }
}

impl CompiledKeymap {
    /// Build the effective keymap: the base tree, then plugin trees in plugin
    /// id order, then the user's tree.
    pub(crate) fn build(user: Option<&KeymapText>, plugins: &[(String, KeymapText)]) -> Self {
        let mut diagnostics = Vec::new();
        let mut conflicts = Vec::new();
        let user_document = user.and_then(|user| {
            parse_document(&user.text, &user.source, &mut diagnostics)
                .map(|document| (user, document))
        });
        let settings = user_document
            .as_ref()
            .map(|(user, document)| {
                document_settings(document, &user.text, &user.source, &mut diagnostics)
            })
            .unwrap_or_default();
        let base = settings.base.unwrap_or(Base::Herdra);
        let prefix = settings.prefix.unwrap_or(DEFAULT_PREFIX);
        let mut layers = Vec::new();
        let base_text = match base {
            Base::Herdra => Some(DEFAULT_KEYMAP),
            Base::Classic => Some(CLASSIC_KEYMAP),
            Base::Empty => None,
        };
        match base_text {
            Some(text) => {
                let mut builtin_diagnostics = Vec::new();
                layers.push(parse_layer(
                    text,
                    base.id(),
                    LayerOwner::Builtin,
                    prefix,
                    &mut builtin_diagnostics,
                ));
                conflicts.extend(builtin_diagnostics);
            }
            None => layers.push(KeymapLayer {
                source: base.id().to_owned(),
                owner: LayerOwner::Builtin,
                nodes: Vec::new(),
            }),
        }
        let mut sorted_plugins = plugins.iter().collect::<Vec<_>>();
        sorted_plugins.sort_by(|left, right| left.0.cmp(&right.0));
        for (plugin_id, text) in sorted_plugins {
            layers.push(parse_layer(
                &text.text,
                &text.source,
                LayerOwner::Plugin(plugin_id.clone()),
                prefix,
                &mut conflicts,
            ));
        }
        if let Some((user, document)) = &user_document {
            layers.push(layer_from_document(
                document,
                &user.text,
                &user.source,
                LayerOwner::User,
                prefix,
                &mut diagnostics,
            ));
        }
        let merged = merge_layers(layers, &mut conflicts);
        let mut compiler = Compiler {
            menus: Vec::new(),
            commands: Vec::new(),
            menu_ids: Vec::new(),
            pending_opens: Vec::new(),
            diagnostics: &mut diagnostics,
        };
        compiler.add_menu(None, None, TopLevel::menu(), &merged, &[]);
        compiler.resolve_opens();
        // Views can also open without a key (the mobile switcher, resuming
        // copy mode on refocus). Give views that no menu attaches an empty,
        // unreachable menu so those paths still work.
        for view in ViewKind::ALL {
            if !compiler.menus.iter().any(|menu| menu.view == Some(view)) {
                compiler.add_menu(
                    None,
                    None,
                    RawMenu {
                        title: Some(view.id().to_owned()),
                        view: Some(view),
                        sticky: Some(true),
                        anchor: Some(view == ViewKind::Copy),
                        ..RawMenu::default()
                    },
                    &[],
                    &[],
                );
            }
        }
        let menus = compiler.menus;
        let commands = compiler.commands;
        let common = vec![
            common_binding(
                (KeyCode::Esc, KeyModifiers::empty()),
                CompiledTarget::Cancel,
                "cancel",
            ),
            common_binding(
                (KeyCode::Backspace, KeyModifiers::empty()),
                CompiledTarget::Back,
                "back",
            ),
        ];
        let mut keymap = Self {
            menus,
            common,
            commands,
            help: Vec::new(),
            diagnostics,
            conflicts,
            base,
            prefix,
        };
        keymap.finish();
        keymap
    }

    pub(crate) fn menu(&self, id: MenuId) -> &CompiledMenu {
        &self.menus[id.index()]
    }

    pub(crate) fn top(&self) -> &CompiledMenu {
        self.menu(MenuId::TOP)
    }

    /// The first menu, in tree order, that attaches a view.
    pub(crate) fn menu_with_view(&self, view: ViewKind) -> Option<MenuId> {
        self.menus
            .iter()
            .position(|menu| menu.view == Some(view))
            .and_then(|index| u16::try_from(index).ok())
            .map(MenuId)
    }

    pub(crate) fn menu_by_path(&self, path_label: &str) -> Option<MenuId> {
        self.menus
            .iter()
            .skip(1)
            .position(|menu| menu.path_label == path_label)
            .and_then(|index| u16::try_from(index + 1).ok())
            .map(MenuId)
    }

    fn finish(&mut self) {
        for index in 1..self.menus.len() {
            let badge = self.badge_for(MenuId(index as u16));
            let plan = self.bar_plan_for(index);
            let menu = &mut self.menus[index];
            menu.badge = badge;
            menu.bar_plan = plan;
        }
        self.help = self.help_groups();
    }

    fn badge_for(&self, id: MenuId) -> String {
        let mut titles = Vec::new();
        let mut current = Some(id);
        while let Some(menu_id) = current {
            if menu_id == MenuId::TOP {
                break;
            }
            let menu = self.menu(menu_id);
            titles.push(menu.title.to_uppercase());
            if menu.anchor {
                break;
            }
            current = menu.parent;
        }
        titles.reverse();
        titles.join(" › ")
    }

    fn bar_plan_for(&self, index: usize) -> BarPlan {
        let menu = &self.menus[index];
        let esc = (KeyCode::Esc, KeyModifiers::empty());
        let mut segments = Vec::new();
        let own_esc = menu
            .bindings
            .iter()
            .find(|binding| binding.chord == Chord::Key(esc));
        match own_esc {
            Some(binding) if binding.hidden => {}
            Some(binding) => segments.push(BarSegment {
                keys: "esc".to_owned(),
                label: binding.hint.clone(),
                kind: SegmentKind::Exit,
                action_id: binding_action_id(binding),
            }),
            None => segments.push(BarSegment {
                keys: "esc".to_owned(),
                label: "cancel".to_owned(),
                kind: SegmentKind::Exit,
                action_id: None,
            }),
        }
        let mut body = Vec::new();
        let mut help = Vec::new();
        for binding in &menu.bindings {
            if binding.hidden || binding.chord == Chord::Key(esc) {
                continue;
            }
            let (label, kind) = match &binding.target {
                CompiledTarget::Enter(_) => (format!("+{}", binding.hint), SegmentKind::Submenu),
                CompiledTarget::Action(CatalogAction::Fixed(crate::input::KeybindAction::Help)) => {
                    help.push(BarSegment {
                        keys: display_label(binding.chord),
                        label: binding.hint.clone(),
                        kind: SegmentKind::Help,
                        action_id: binding_action_id(binding),
                    });
                    continue;
                }
                CompiledTarget::Action(_) | CompiledTarget::Command(_) => {
                    let stays = !binding.exits_itself
                        && match binding.exit {
                            Some(ExitPolicy::Stay) => true,
                            Some(ExitPolicy::Exit) => false,
                            None => menu.sticky,
                        };
                    (
                        binding.hint.clone(),
                        if stays {
                            SegmentKind::Sticky
                        } else {
                            SegmentKind::Action
                        },
                    )
                }
                CompiledTarget::Back | CompiledTarget::Cancel | CompiledTarget::Literal => {
                    (binding.hint.clone(), SegmentKind::Action)
                }
            };
            body.push((
                binding.priority,
                BarSegment {
                    keys: display_label(binding.chord),
                    label,
                    kind,
                    action_id: binding_action_id(binding),
                },
            ));
        }
        // Higher priority first; among equals, submenus before leaves, as
        // which-key lists groups first.
        body.sort_by_key(|(priority, segment)| {
            (
                std::cmp::Reverse(*priority),
                segment.kind != SegmentKind::Submenu,
            )
        });
        segments.extend(body.into_iter().map(|(_, segment)| segment));
        segments.extend(help);
        let mut merged: Vec<BarSegment> = Vec::new();
        for segment in segments {
            if let Some(last) = merged.last_mut() {
                if last.label == segment.label
                    && last.kind == segment.kind
                    && segment.kind != SegmentKind::Exit
                {
                    last.keys = format!("{}/{}", last.keys, segment.keys);
                    continue;
                }
            }
            merged.push(segment);
        }
        BarPlan { segments: merged }
    }

    fn help_groups(&self) -> Vec<HelpGroup> {
        let mut groups = Vec::new();
        for (index, menu) in self.menus.iter().enumerate() {
            let entries = menu
                .bindings
                .iter()
                .map(|binding| {
                    let key = if index == 0 {
                        display_label(binding.chord)
                    } else {
                        format!("{} {}", menu.path_label, display_label(binding.chord))
                    };
                    (key, binding.description.clone())
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                continue;
            }
            let title = if index == 0 {
                "top level".to_owned()
            } else {
                self.badge_path(MenuId(index as u16))
            };
            groups.push(HelpGroup { title, entries });
        }
        groups
    }

    fn badge_path(&self, id: MenuId) -> String {
        let mut titles = Vec::new();
        let mut current = Some(id);
        while let Some(menu_id) = current {
            if menu_id == MenuId::TOP {
                break;
            }
            let menu = self.menu(menu_id);
            titles.push(menu.title.clone());
            current = menu.parent;
        }
        titles.reverse();
        titles.join(" › ")
    }
}

fn common_binding(
    combo: crate::config::KeyCombo,
    target: CompiledTarget,
    hint: &str,
) -> CompiledBinding {
    CompiledBinding {
        chord: Chord::Key(combo),
        target,
        exit: None,
        exits_itself: false,
        hint: hint.to_owned(),
        description: hint.to_owned(),
        hidden: false,
        priority: 0,
        owner: LayerOwner::Builtin,
    }
}

struct TopLevel;

impl TopLevel {
    fn menu() -> RawMenu {
        RawMenu {
            title: Some(String::new()),
            unmatched: Some(Unmatched::Forward),
            bar: Some(BarVisibility::Hidden),
            ..RawMenu::default()
        }
    }
}

struct Compiler<'a> {
    menus: Vec<CompiledMenu>,
    commands: Vec<CompiledCommand>,
    menu_ids: Vec<(String, MenuId)>,
    /// `menu.open` bindings waiting for every menu id to be known:
    /// (menu, binding index, target id, path label).
    pending_opens: Vec<(MenuId, usize, String, String)>,
    diagnostics: &'a mut Vec<String>,
}

impl Compiler<'_> {
    fn add_menu(
        &mut self,
        parent: Option<MenuId>,
        entry_chord: Option<Chord>,
        meta: RawMenu,
        children: &[RawNode],
        views_in_scope: &[ViewKind],
    ) -> MenuId {
        let id = MenuId(self.menus.len() as u16);
        let path_label = match (parent, entry_chord) {
            (Some(parent), Some(chord)) if parent != MenuId::TOP => format!(
                "{} {}",
                self.menus[parent.index()].path_label,
                chord.label()
            ),
            (_, Some(chord)) => chord.label(),
            _ => String::new(),
        };
        let view = meta.view;
        let sticky = meta.sticky.unwrap_or(view == Some(ViewKind::Copy));
        let anchor = meta.anchor.unwrap_or(view == Some(ViewKind::Copy));
        let title = meta
            .title
            .clone()
            .unwrap_or_else(|| entry_chord.map(Chord::label).unwrap_or_default());
        self.menus.push(CompiledMenu {
            parent,
            entry_chord,
            path_label,
            title,
            badge: String::new(),
            sticky,
            anchor,
            view,
            unmatched: meta.unmatched.unwrap_or(if sticky {
                Unmatched::Ignore
            } else {
                Unmatched::Cancel
            }),
            bar: meta.bar,
            bindings: Vec::new(),
            bar_plan: BarPlan::default(),
        });
        if let Some(menu_id) = &meta.id {
            if self
                .menu_ids
                .iter()
                .any(|(existing, _)| existing == menu_id)
            {
                self.diagnostics.push(format!(
                    "keymap: menu id {menu_id:?} is used more than once; keeping the first"
                ));
            } else {
                self.menu_ids.push((menu_id.clone(), id));
            }
        }
        let mut scope = views_in_scope.to_vec();
        if let Some(view) = view {
            scope.push(view);
        }
        let top_level = id == MenuId::TOP;
        let mut bindings = Vec::new();
        for child in children {
            if let Some((binding, open)) = self.compile_child(id, child, &scope, top_level) {
                if let Some((target, path_label)) = open {
                    self.pending_opens
                        .push((id, bindings.len(), target, path_label));
                }
                bindings.push(binding);
            }
        }
        self.menus[id.index()].bindings = bindings;
        id
    }

    fn resolve_opens(&mut self) {
        let mut dropped = Vec::new();
        for (menu, index, target, path_label) in std::mem::take(&mut self.pending_opens) {
            match self.menu_ids.iter().find(|(id, _)| *id == target) {
                Some((_, target_menu)) => {
                    let title = self.menus[target_menu.index()].title.clone();
                    let binding = &mut self.menus[menu.index()].bindings[index];
                    binding.target = CompiledTarget::Enter(*target_menu);
                    if binding.hint.is_empty() {
                        binding.hint = title.clone();
                    }
                    binding.description = format!("+{title}");
                }
                None => {
                    self.diagnostics.push(format!(
                        "keymap: {path_label} opens unknown menu id {target:?}"
                    ));
                    dropped.push((menu, index));
                }
            }
        }
        dropped.sort_by_key(|(_, index)| std::cmp::Reverse(*index));
        for (menu, index) in dropped {
            self.menus[menu.index()].bindings.remove(index);
        }
    }

    fn compile_child(
        &mut self,
        menu_id: MenuId,
        node: &RawNode,
        scope: &[ViewKind],
        top_level: bool,
    ) -> Option<(CompiledBinding, Option<(String, String)>)> {
        match &node.body {
            RawBody::Unbind => None,
            RawBody::Menu(menu) => {
                let child = self.add_menu(
                    Some(menu_id),
                    Some(node.chord),
                    menu.clone(),
                    &menu.children,
                    scope,
                );
                let title = self.menus[child.index()].title.clone();
                Some((
                    CompiledBinding {
                        chord: node.chord,
                        target: CompiledTarget::Enter(child),
                        exit: None,
                        exits_itself: false,
                        hint: menu.hint.clone().unwrap_or_else(|| title.clone()),
                        description: format!("+{title}"),
                        hidden: menu.hidden.unwrap_or(false),
                        priority: menu.priority.unwrap_or(0),
                        owner: node.owner.clone(),
                    },
                    None,
                ))
            }
            RawBody::Leaf(leaf) => {
                let menu = &self.menus[menu_id.index()];
                let path_label = if menu_id == MenuId::TOP {
                    node.chord.label()
                } else {
                    format!("{} {}", menu.path_label, node.chord.label())
                };
                let mut open = None;
                let mut exits_itself = false;
                let (target, default_hint, description) = match &leaf.target {
                    LeafTarget::Open(target) => {
                        open = Some((target.clone(), path_label.clone()));
                        (CompiledTarget::Back, String::new(), String::new())
                    }
                    LeafTarget::Action(entry) => {
                        exits_itself = entry.exits_itself;
                        if let Some(view) = entry.view {
                            if !top_level && !scope.contains(&view) {
                                self.diagnostics.push(format!(
                                    "keymap: {path_label} runs {}, which only works inside a menu with view={}",
                                    entry.id,
                                    view.id()
                                ));
                            }
                        }
                        (
                            CompiledTarget::Action(entry.action),
                            entry.hint.to_owned(),
                            entry.description.to_owned(),
                        )
                    }
                    LeafTarget::Command(spec) => {
                        let index = self.commands.len();
                        let default_hint = spec.command.clone();
                        self.commands.push(CompiledCommand {
                            path_label: path_label.clone(),
                            spec: spec.clone(),
                            hint: leaf.hint.clone().unwrap_or_else(|| default_hint.clone()),
                            owner: node.owner.clone(),
                        });
                        (
                            CompiledTarget::Command(index),
                            default_hint,
                            format!("{} {}", spec.kind.word(), spec.command),
                        )
                    }
                    LeafTarget::Back => {
                        (CompiledTarget::Back, "back".to_owned(), "back".to_owned())
                    }
                    LeafTarget::Cancel => (
                        CompiledTarget::Cancel,
                        "cancel".to_owned(),
                        "cancel".to_owned(),
                    ),
                    LeafTarget::Literal => {
                        let entry = menu.entry_chord.map(Chord::label).unwrap_or_default();
                        let hint = if menu.parent == Some(MenuId::TOP) {
                            "send prefix".to_owned()
                        } else {
                            format!("send {entry}")
                        };
                        (
                            CompiledTarget::Literal,
                            hint,
                            format!("send {entry} to the pane"),
                        )
                    }
                };
                let hint = leaf.hint.clone().unwrap_or(default_hint);
                let description = match &leaf.hint {
                    Some(hint) if *hint != description => format!("{hint} ({description})"),
                    _ => description,
                };
                Some((
                    CompiledBinding {
                        chord: node.chord,
                        target,
                        exit: leaf.exit,
                        exits_itself,
                        hint,
                        description,
                        hidden: leaf.hidden,
                        priority: leaf.priority,
                        owner: node.owner.clone(),
                    },
                    open,
                ))
            }
        }
    }
}

/// Merge later layers over earlier ones, matching nodes by chord path.
fn merge_layers(layers: Vec<KeymapLayer>, conflicts: &mut Vec<String>) -> Vec<RawNode> {
    let mut layers = layers.into_iter();
    let Some(first) = layers.next() else {
        return Vec::new();
    };
    let mut merged = first.nodes;
    for layer in layers {
        merge_level(
            &mut merged,
            layer.nodes,
            &layer.owner,
            &layer.source,
            conflicts,
        );
    }
    merged
}

fn merge_level(
    merged: &mut Vec<RawNode>,
    overlay: Vec<RawNode>,
    owner: &LayerOwner,
    source: &str,
    conflicts: &mut Vec<String>,
) {
    for node in overlay {
        let overlapping = merged
            .iter()
            .enumerate()
            .filter(|(_, existing)| existing.chord.overlaps(node.chord))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let menu_merge_index = match &node.body {
            RawBody::Menu(menu) if !menu.replace => match overlapping.as_slice() {
                [index]
                    if merged[*index].chord == node.chord
                        && matches!(merged[*index].body, RawBody::Menu(_)) =>
                {
                    Some(*index)
                }
                _ => None,
            },
            _ => None,
        };
        if let LayerOwner::Plugin(_) = owner {
            let foreign = overlapping
                .iter()
                .find(|index| merged[**index].owner != *owner)
                .copied();
            if let (Some(foreign), None) = (foreign, menu_merge_index) {
                conflicts.push(format!(
                    "keymap {source}:{}: {} is already bound by {}; keeping that binding",
                    node.line,
                    node.chord.label(),
                    merged[foreign].owner.label()
                ));
                continue;
            }
        }
        if let Some(index) = menu_merge_index {
            let existing_owner = merged[index].owner.clone();
            let RawBody::Menu(incoming) = node.body else {
                continue;
            };
            if let RawBody::Menu(existing) = &mut merged[index].body {
                let may_edit = !matches!(owner, LayerOwner::Plugin(_)) || existing_owner == *owner;
                if may_edit {
                    merge_menu_metadata(existing, &incoming);
                }
                merge_level(
                    &mut existing.children,
                    incoming.children,
                    owner,
                    source,
                    conflicts,
                );
            }
            continue;
        }
        let insert_at = overlapping.first().copied();
        for index in overlapping.iter().rev() {
            merged.remove(*index);
        }
        if matches!(node.body, RawBody::Unbind) {
            continue;
        }
        match insert_at {
            Some(index) => merged.insert(index.min(merged.len()), node),
            None => merged.push(node),
        }
    }
}

fn merge_menu_metadata(existing: &mut RawMenu, incoming: &RawMenu) {
    if incoming.title.is_some() {
        existing.title.clone_from(&incoming.title);
    }
    if incoming.hint.is_some() {
        existing.hint.clone_from(&incoming.hint);
    }
    if incoming.sticky.is_some() {
        existing.sticky = incoming.sticky;
    }
    if incoming.anchor.is_some() {
        existing.anchor = incoming.anchor;
    }
    if incoming.view.is_some() {
        existing.view = incoming.view;
    }
    if incoming.unmatched.is_some() {
        existing.unmatched = incoming.unmatched;
    }
    if incoming.bar.is_some() {
        existing.bar = incoming.bar;
    }
    if incoming.hidden.is_some() {
        existing.hidden = incoming.hidden;
    }
    if incoming.priority.is_some() {
        existing.priority = incoming.priority;
    }
}

/// The catalog entry that labels a binding, for help and API output.
pub(crate) fn binding_action_id(binding: &CompiledBinding) -> Option<&'static str> {
    match binding.target {
        CompiledTarget::Action(action) => catalog::CATALOG
            .iter()
            .find(|entry| entry.action == action)
            .map(|entry| entry.id),
        _ => None,
    }
}
