use super::*;

#[path = "../shell/overlays.rs"]
mod overlays;
#[path = "../shell/sidebar.rs"]
pub(in crate::client::shell) mod sidebar;
#[path = "../shell/tabs.rs"]
mod tabs;

pub(super) use super::agent_sidebar::{ordered_agent_pane_ids, render_agent_panel};
pub(super) use super::aggregate_navigation::navigator_rows as client_navigator_rows;
pub(super) use overlays::{
    help_scroll_to_menu, render_client_overlay, render_context_menu, render_global_menu,
};
pub(super) use sidebar::{render_collapsed_sidebar, render_sidebar, workspace_entries};
pub(super) use tabs::{render_tab_bar, tab_bar_status_width};

pub(in crate::client::shell) fn render_sidebar_background(
    buffer: &mut Buffer,
    area: Rect,
    palette: &Palette,
) {
    buffer.set_style(area, Style::default().bg(palette.sidebar_bg));
    let separator_x = area.right().saturating_sub(1);
    for y in area.y..area.bottom() {
        if let Some(cell) = buffer.cell_mut((separator_x, y)) {
            cell.set_symbol("│");
            cell.set_style(Style::default().fg(palette.surface_dim));
        }
    }
}

/// What the bottom bar shows.
pub(super) struct ModeBar<'a> {
    /// The menu that receives keys, if any.
    pub(super) menu: Option<&'a crate::input::keymap::CompiledMenu>,
    pub(super) copy_mode: Option<&'a ClientCopyModeState>,
    pub(super) endpoint_error: Option<&'a str>,
    /// Show the update badge (the workspace list is open and an update waits).
    pub(super) update_available: bool,
    /// Presentation for menus that do not set `bar=`.
    pub(super) default_bar: crate::input::keymap::BarVisibility,
}

pub(super) fn render_mode_bar(
    buffer: &mut Buffer,
    pane_area: Rect,
    bar_state: ModeBar<'_>,
    palette: &Palette,
) -> Option<Rect> {
    use crate::input::keymap::{BarVisibility, SegmentKind, ViewKind};

    let visibility = bar_state
        .menu
        .map(|menu| menu.bar.unwrap_or(bar_state.default_bar));
    // The copy search prompt is a text field, so it shows even when the bar is hidden.
    let search_prompt = bar_state
        .menu
        .filter(|menu| menu.view == Some(ViewKind::Copy))
        .and(bar_state.copy_mode)
        .and_then(|copy_mode| copy_mode.search_prompt.as_ref());
    let menu = bar_state.menu.filter(|_| {
        bar_state.endpoint_error.is_some()
            || search_prompt.is_some()
            || visibility != Some(BarVisibility::Hidden)
    });
    if (menu.is_none() && bar_state.endpoint_error.is_none()) || pane_area.is_empty() {
        return None;
    }

    let bar = Rect::new(
        pane_area.x,
        pane_area.y + pane_area.height.saturating_sub(1),
        pane_area.width,
        1,
    );
    let base = Style::default().fg(palette.overlay0).bg(palette.panel_bg);
    for x in bar.x..bar.x + bar.width {
        buffer[(x, bar.y)].set_symbol(" ").set_style(base);
    }

    let key = Style::default()
        .fg(palette.accent)
        .bg(palette.panel_bg)
        .add_modifier(Modifier::BOLD);
    let sticky_key = Style::default()
        .fg(palette.mauve)
        .bg(palette.panel_bg)
        .add_modifier(Modifier::BOLD);
    let badge_style = |sticky: bool| {
        Style::default()
            .fg(match palette.panel_bg {
                ratatui::style::Color::Reset => palette.surface_dim,
                color => color,
            })
            .bg(if sticky {
                palette.mauve
            } else {
                palette.accent
            })
            .add_modifier(Modifier::BOLD)
    };

    let mut segments = Vec::<(String, Style)>::new();
    if let Some(error) = bar_state.endpoint_error {
        segments.extend([
            (" ERROR ".to_owned(), badge_style(false)),
            (format!(" {error}"), base),
        ]);
        write_bar_segments(buffer, bar, &segments);
        return Some(bar);
    }
    let menu = menu?;
    let copy_view = menu.view == Some(ViewKind::Copy);
    let mode_style = badge_style(menu.sticky && menu.view.is_none());
    if let Some(prompt) = search_prompt {
        render_copy_search_prompt(buffer, bar, menu, prompt, mode_style, key, base, palette);
        return Some(bar);
    }
    let badge = format!(" {} ", menu.badge);
    if visibility == Some(BarVisibility::Badge) {
        write_bar_segments(buffer, bar, &[(badge, mode_style)]);
        return Some(bar);
    }

    let copy_state = bar_state.copy_mode.filter(|_| copy_view);
    let hints = menu
        .bar_plan
        .segments
        .iter()
        .map(|segment| {
            let mut label = segment.label.clone();
            if let Some(copy_mode) = copy_state {
                match segment.action_id {
                    Some("copy.select") if copy_mode.selection.is_some() => {
                        label = "selecting".to_owned();
                    }
                    Some("copy.search.next") => {
                        let status = copy_mode
                            .search_current_global
                            .map(|current| format!(" {}/{}", current + 1, copy_mode.search_total))
                            .or_else(|| {
                                (!copy_mode.search_query.is_empty()).then(|| " 0/0".to_owned())
                            })
                            .unwrap_or_default();
                        label.push_str(&status);
                    }
                    Some("copy.escape")
                        if copy_mode.search_query.is_empty() && copy_mode.selection.is_none() =>
                    {
                        label = "exit".to_owned();
                    }
                    _ => {}
                }
            }
            let key_style = if segment.kind == SegmentKind::Sticky {
                sticky_key
            } else {
                key
            };
            (segment.keys.clone(), label, key_style, segment.kind)
        })
        .collect::<Vec<_>>();

    let trailing = if bar_state.update_available { 13 } else { 0 };
    let available = usize::from(bar.width).saturating_sub(trailing);
    let hint_width = |hint: &(String, String, Style, SegmentKind)| {
        2 + UnicodeWidthStr::width(hint.0.as_str()) + 1 + UnicodeWidthStr::width(hint.1.as_str())
    };
    let badge_width = UnicodeWidthStr::width(badge.as_str());
    let mut kept = hints.iter().collect::<Vec<_>>();
    let total = |kept: &[&(String, String, Style, SegmentKind)]| {
        badge_width + kept.iter().map(|hint| hint_width(hint)).sum::<usize>()
    };
    let mut truncated = false;
    while total(&kept) + if truncated { 2 } else { 0 } > available {
        let Some(position) = kept
            .iter()
            .rposition(|hint| !matches!(hint.3, SegmentKind::Exit | SegmentKind::Help))
        else {
            break;
        };
        kept.remove(position);
        truncated = true;
    }

    segments.push((badge, mode_style));
    for (index, (keys, label, key_style, _)) in kept.iter().enumerate() {
        segments.push((if index == 0 { " " } else { "  " }.to_owned(), base));
        segments.push((keys.clone(), *key_style));
        segments.push((format!(" {label}"), base));
    }
    if truncated {
        segments.push((" …".to_owned(), base));
    }
    write_bar_segments(buffer, bar, &segments);

    if bar_state.update_available {
        let width = 13.min(bar.width);
        let area = Rect::new(bar.right().saturating_sub(width), bar.y, width, 1);
        buffer.set_style(area, Style::default().bg(palette.panel_bg));
        put_right_text(
            buffer,
            area,
            area.y,
            " update ready",
            Style::default()
                .fg(palette.accent)
                .bg(palette.panel_bg)
                .add_modifier(Modifier::BOLD),
        );
    }
    Some(bar)
}

fn write_bar_segments(buffer: &mut Buffer, bar: Rect, segments: &[(String, Style)]) {
    let mut x = bar.x;
    let end = bar.x + bar.width;
    for (text, style) in segments {
        if x >= end {
            break;
        }
        let remaining = end - x;
        buffer.set_stringn(x, bar.y, text, usize::from(remaining), *style);
        x = x.saturating_add(
            u16::try_from(UnicodeWidthStr::width(text.as_str()))
                .unwrap_or(u16::MAX)
                .min(remaining),
        );
    }
}

#[allow(clippy::too_many_arguments)] // Bar geometry plus the styles shared with the main bar.
fn render_copy_search_prompt(
    buffer: &mut Buffer,
    bar: Rect,
    menu: &crate::input::keymap::CompiledMenu,
    prompt: &ClientCopySearchPrompt,
    mode_style: Style,
    key: Style,
    base: Style,
    palette: &Palette,
) {
    let marker = match prompt.direction {
        crate::api::schema::PaneCopySearchDirection::Forward => "/",
        crate::api::schema::PaneCopySearchDirection::Backward => "?",
    };
    let badge = format!(" {} ", menu.badge);
    let badge_width = u16::try_from(UnicodeWidthStr::width(badge.as_str())).unwrap_or(u16::MAX);
    buffer.set_stringn(bar.x, bar.y, &badge, usize::from(bar.width), mode_style);
    let prefix = badge_width.saturating_add(2).min(bar.width);
    if bar.width >= prefix && prefix >= 1 {
        buffer.set_string(bar.x + prefix - 1, bar.y, marker, key);
    }
    let footer = "  enter search  esc cancel";
    let footer_width = if bar.width >= 50 {
        footer.len() as u16
    } else {
        0
    };
    let field = Rect::new(
        bar.x + prefix,
        bar.y,
        bar.width.saturating_sub(prefix + footer_width),
        1,
    );
    if let Some(cursor) = text_editor::render(
        buffer,
        field,
        &prompt.query,
        Style::default().fg(palette.text).bg(palette.panel_bg),
    ) {
        buffer[(cursor.x, cursor.y)]
            .set_style(Style::default().fg(palette.panel_bg).bg(palette.text));
    }
    if footer_width > 0 {
        buffer.set_string(bar.right() - footer_width, bar.y, footer, base);
    }
}

pub(super) struct ShellRenderState<'a> {
    pub(super) machine_diagnostics: &'a super::machine_diagnostics::MachineDiagnostics,
    pub(super) endpoints: &'a [ClientShellEndpoint],
    pub(super) active_endpoint_id: &'a ClientEndpointId,
    pub(super) collapsed_endpoints: &'a HashSet<ClientEndpointId>,
    pub(super) collapsed_groups: &'a HashSet<String>,
    pub(super) remote_collapsed_groups: &'a HashMap<ClientEndpointId, HashSet<String>>,
    pub(super) workspace_scroll: &'a mut usize,
    pub(super) agent_scroll: &'a mut usize,
    pub(super) tab_scroll: &'a mut usize,
    pub(super) reveal_focused_workspace: &'a mut bool,
    pub(super) reveal_focused_tab: &'a mut bool,
    pub(super) sidebar_collapsed: bool,
    pub(super) sidebar_section_split: f32,
    pub(super) tab_drag_insert_index: Option<usize>,
    pub(super) selected_workspace_id: Option<&'a WorkspaceNavigationTarget>,
    pub(super) reveal_navigation_workspace: &'a mut bool,
    pub(super) dragged_workspace_id: Option<&'a str>,
    pub(super) workspace_drop_indicator_row: Option<u16>,
}

pub(super) fn render_shell(
    buffer: &mut Buffer,
    layout: ClientShellLayout,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    mut state: ShellRenderState<'_>,
) -> ShellHitMap {
    let mut hits = ShellHitMap::default();
    if layout.mobile_header.height > 0 {
        super::mobile::render_mobile_header(
            buffer,
            layout.mobile_header,
            snapshot,
            config,
            &mut hits,
        );
    }
    if layout.sidebar.width > 0 {
        if state.endpoints.len() > 1 {
            if state.sidebar_collapsed {
                super::endpoint_sidebar::render_collapsed(
                    buffer,
                    layout.sidebar,
                    config,
                    &mut state,
                    &mut hits,
                );
            } else {
                super::endpoint_sidebar::render_expanded(
                    buffer,
                    layout.sidebar,
                    Some(snapshot),
                    config,
                    &mut state,
                    &mut hits,
                );
            }
        } else if state.sidebar_collapsed {
            render_collapsed_sidebar(
                buffer,
                layout.sidebar,
                snapshot,
                config,
                state
                    .selected_workspace_id
                    .map(|target| target.workspace_id.as_str()),
                &mut hits,
            );
        } else {
            render_sidebar(
                buffer,
                layout.sidebar,
                snapshot,
                config,
                &mut state,
                &mut hits,
            );
        }
    }
    if layout.tab_bar.height > 0 {
        render_tab_bar(
            buffer,
            layout.tab_bar,
            snapshot,
            config,
            state.tab_scroll,
            state.reveal_focused_tab,
            state.tab_drag_insert_index,
            &mut hits,
        );
    }
    if !config.mouse_capture {
        hits.sidebar_divider = Rect::default();
        hits.sidebar_section_divider = Rect::default();
        hits.workspace_scrollbar = Rect::default();
        hits.agent_scrollbar = Rect::default();
        hits.agent_sort_toggle = Rect::default();
        hits.new_workspace = Rect::default();
        hits.machines.clear();
        hits.workspaces.clear();
        hits.agents.clear();
        hits.endpoint_agents.clear();
        hits.tab_scroll_left = Rect::default();
        hits.tab_scroll_right = Rect::default();
        hits.new_tab = Rect::default();
        hits.pane_splits.clear();
    }
    hits
}

pub(super) fn put_right_text(buffer: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    let width = display_width(text).min(area.width);
    put_text(
        buffer,
        area.right().saturating_sub(width),
        y,
        width,
        text,
        style,
    );
}

pub(super) fn put_segment(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    right: u16,
    text: &str,
    style: Style,
) -> u16 {
    let width = display_width(text).min(right.saturating_sub(x));
    put_text(buffer, x, y, width, text, style);
    x.saturating_add(width)
}

pub(super) fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    if width == 0 || y >= buffer.area.bottom() || x >= buffer.area.right() {
        return;
    }
    buffer.set_stringn(x, y, text, width as usize, style);
}

pub(super) fn display_width(text: &str) -> u16 {
    UnicodeWidthStr::width(text).min(u16::MAX as usize) as u16
}
