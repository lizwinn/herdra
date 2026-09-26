use crossterm::event::{KeyCode, KeyModifiers};

mod io;
mod keybinds;
mod model;
mod sidebar;
mod sound;
mod tab_bar;
mod theme;
mod window_title;
mod write;

pub use self::{
    io::{
        config_diagnostic_summary, config_dir, config_path, keymap_path, load_live_config,
        remove_keybinding_config_sections, remove_section_key, state_dir, upsert_section_bool,
        upsert_section_value,
    },
    keybinds::{format_key_combo, normalize_key_combo, terminal_key_matches_combo},
    model::{
        validated_sidebar_bounds, AgentPanelSortConfig, Config, ConfigReloadReport,
        ConfigReloadStatus, HostCursorModeConfig, ModeHintBarConfig, NewTerminalCwdConfig,
        PaneBordersConfig, ShellModeConfig, SidebarCollapsedModeConfig, StatusIndicatorStyle,
        TabBarPositionConfig, ToastClipboardPosition, ToastConfig, ToastDelivery,
        ToastHerdrPosition, UpdateChannelConfig, MAX_TOAST_DELAY_SECONDS,
    },
    sidebar::{
        AgentSidebarToken, AgentsSidebarConfig, SidebarConfig, SidebarTokenStyle,
        SpaceSidebarToken, SpacesSidebarConfig,
    },
    sound::SoundConfig,
    tab_bar::TabBarRightEntryConfig,
    theme::{parse_color, CustomThemeColors, ModeThemeColors, ThemeConfig, THEME_NAMES},
    window_title::{WindowTitlePart, WindowTitleTemplate, WindowTitleToken},
};

pub(crate) use self::keybinds::{
    indexed_key_index, parse_key_combo, parse_range_modifiers, KeyCombo,
};
pub(crate) use self::write::{update_file_at, write_edit, ConfigEdit};
pub(crate) use self::{
    io::upsert_top_level_bool,
    tab_bar::{
        parse_tab_bar_datetime_format, tab_bar_right_diagnostics,
        MAX_TAB_BAR_COMMAND_INTERVAL_SECONDS, MAX_TAB_BAR_COMMAND_TIMEOUT_SECONDS,
        MAX_TAB_BAR_RIGHT_ENTRIES,
    },
    theme::canonical_theme_name,
    window_title::{sanitize_window_title_text, window_title_diagnostics},
};

pub const CONFIG_PATH_ENV_VAR: &str = "HERDR_CONFIG_PATH";

pub(crate) fn is_keybinding_config_diagnostic(diagnostic: &str) -> bool {
    if diagnostic.starts_with("config parse error:") || diagnostic.starts_with("config read error:")
    {
        return false;
    }
    diagnostic.contains("keybinding")
        || diagnostic.contains("keys.")
        || diagnostic.starts_with("keymap")
}

pub(crate) fn config_diagnostic_summary_without_keybindings(
    diagnostics: &[String],
) -> Option<String> {
    let diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| !is_keybinding_config_diagnostic(diagnostic))
        .cloned()
        .collect::<Vec<_>>();
    config_diagnostic_summary(&diagnostics)
}
pub const DEFAULT_SCROLLBACK_LIMIT_BYTES: usize = 10_000_000;
pub const DEFAULT_MOUSE_SCROLL_LINES: usize = 3;
pub const DEFAULT_MOBILE_WIDTH_THRESHOLD: u16 = 64;
pub const DEFAULT_HEADLESS_COLS: u16 = 120;
pub const DEFAULT_HEADLESS_ROWS: u16 = 40;

#[cfg(test)]
pub(crate) fn app_dir_name() -> &'static str {
    io::app_dir_name()
}

/// Serializes tests that point config env vars at temporary files.
///
/// `HERDR_KEYMAP_PATH` from the environment running the tests would redirect
/// every keymap read, so it is cleared once, before the first test takes this
/// lock. Tests that set it restore it before releasing the lock.
#[cfg(test)]
pub(crate) fn test_config_env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| {
        std::env::remove_var(io::KEYMAP_PATH_ENV_VAR);
        std::sync::Mutex::new(())
    })
}

impl Config {
    pub fn should_show_onboarding(&self) -> bool {
        self.onboarding.unwrap_or(true)
    }

    pub fn kitty_graphics_enabled(&self) -> bool {
        self.terminal
            .kitty_graphics
            .or(self.experimental.kitty_graphics)
            .unwrap_or(true)
    }

    /// The effective keymap from the base tree and the user's keymap file.
    /// The server adds plugin trees on top of this.
    pub(crate) fn keymap(&self) -> crate::input::keymap::CompiledKeymap {
        crate::input::keymap::CompiledKeymap::build(self.keymap_file.as_ref(), &[])
    }

    pub fn collect_diagnostics(&self) -> Vec<String> {
        self.legacy_keys_diagnostic()
            .into_iter()
            .chain(self.keymap().diagnostics)
            .chain(self.remote_image_paste_key().err())
            .chain(self.theme.diagnostics())
            .chain(self.ui.sound.diagnostics())
            .chain(tab_bar_right_diagnostics(&self.ui.tab_bar_right))
            .chain(window_title_diagnostics(&self.ui.window_title))
            .chain(self.invalid_sidebar_bounds_diagnostic())
            .chain(self.invalid_headless_size_diagnostic())
            .collect()
    }

    pub(crate) fn legacy_keys_diagnostic(&self) -> Option<String> {
        self.keys
            .as_ref()
            .is_some_and(|keys| !keys.is_empty())
            .then(|| {
                "keys.* keybindings are no longer read; keys live in keymap.kdl now (herdr keymap migrate converts them)"
                    .to_owned()
            })
    }

    pub(crate) fn headless_size(&self) -> (u16, u16) {
        if self.invalid_headless_size_diagnostic().is_some() {
            (DEFAULT_HEADLESS_COLS, DEFAULT_HEADLESS_ROWS)
        } else {
            (self.server.headless_cols, self.server.headless_rows)
        }
    }

    pub(crate) fn invalid_headless_size_diagnostic(&self) -> Option<String> {
        (self.server.headless_cols == 0 || self.server.headless_rows == 0).then(|| {
            format!(
                "server.headless_cols and server.headless_rows must be greater than zero (got {}x{})",
                self.server.headless_cols, self.server.headless_rows
            )
        })
    }

    pub(crate) fn invalid_sidebar_bounds_diagnostic(&self) -> Option<String> {
        validated_sidebar_bounds(self.ui.sidebar_min_width, self.ui.sidebar_max_width)
            .is_none()
            .then(|| {
                format!(
                    "ui.sidebar_min_width ({}) is greater than sidebar_max_width ({})",
                    self.ui.sidebar_min_width, self.ui.sidebar_max_width
                )
            })
    }

    pub(crate) fn remote_image_paste_key(&self) -> Result<Option<(KeyCode, KeyModifiers)>, String> {
        let raw = self.remote.image_paste_key.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        parse_key_combo(raw).map(Some).ok_or_else(|| {
            format!("invalid key: remote.image_paste_key = {raw:?}; disabling image paste")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_keys_section_points_at_the_keymap() {
        let config: Config = toml::from_str("[keys]\nprefix = 'ctrl+a'\n").unwrap();
        let diagnostics = config.collect_diagnostics();
        assert_eq!(
            diagnostics,
            ["keys.* keybindings are no longer read; keys live in keymap.kdl now (herdr keymap migrate converts them)"]
        );
        assert!(is_keybinding_config_diagnostic(&diagnostics[0]));
        assert!(Config::default().collect_diagnostics().is_empty());
    }

    #[test]
    fn keymap_diagnostics_are_config_diagnostics() {
        let config = Config {
            keymap_file: Some(crate::input::keymap::KeymapText {
                source: "keymap.kdl".to_owned(),
                text: "prefix { t { n tab.nwe } }".to_owned(),
            }),
            ..Config::default()
        };
        let diagnostics = config.collect_diagnostics();
        assert_eq!(
            diagnostics,
            ["keymap keymap.kdl:1: unknown action \"tab.nwe\"; did you mean tab.new?"]
        );
        assert!(is_keybinding_config_diagnostic(&diagnostics[0]));
    }

    #[test]
    fn remote_image_paste_key_defaults_to_ctrl_v() {
        let config = Config::default();
        assert_eq!(
            config.remote_image_paste_key().unwrap(),
            Some((KeyCode::Char('v'), KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn remote_image_paste_key_can_be_disabled() {
        let config: Config = toml::from_str("[remote]\nimage_paste_key = ''\n").unwrap();
        assert_eq!(config.remote_image_paste_key().unwrap(), None);
    }

    #[test]
    fn ui_host_cursor_defaults_to_auto_and_parses_overrides() {
        let default_config = Config::default();
        assert_eq!(default_config.ui.host_cursor, HostCursorModeConfig::Auto);

        let native: Config = toml::from_str("[ui]\nhost_cursor = 'native'\n").unwrap();
        assert_eq!(native.ui.host_cursor, HostCursorModeConfig::Native);

        let drawn: Config = toml::from_str("[ui]\nhost_cursor = 'drawn'\n").unwrap();
        assert_eq!(drawn.ui.host_cursor, HostCursorModeConfig::Drawn);
    }
}
