use std::fs;
use std::io::{self, Write};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::layout::Direction;

use super::{App, Mode};

static NEXT_COMMAND_NAMESPACE: AtomicU64 = AtomicU64::new(1);

pub(super) fn new_command_namespace() -> String {
    let counter = NEXT_COMMAND_NAMESPACE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}-{counter}", std::process::id())
}

#[derive(Debug)]
pub(super) struct EndpointCommandRegistry {
    entries: Vec<EndpointCommand>,
}

#[derive(Debug)]
struct EndpointCommand {
    id: String,
    command: crate::input::keymap::CompiledCommand,
    action: crate::protocol::ClientShellCommandAction,
}

impl EndpointCommandRegistry {
    /// Mint opaque ids for every command leaf in the server's keymap. Clients
    /// find a command's id through the keymap projection's fingerprints.
    pub(super) fn new(commands: &[crate::input::keymap::CompiledCommand]) -> Self {
        let namespace = new_command_namespace();
        let entries = commands
            .iter()
            .enumerate()
            .map(|(index, command)| EndpointCommand {
                action: command.spec.kind.into(),
                id: format!("cmd_{namespace}_{index}"),
                command: command.clone(),
            })
            .collect();
        Self { entries }
    }

    /// Command ids with the chord path and fingerprint of each command, for
    /// the keymap projection.
    pub(super) fn projection(
        &self,
        salt: &str,
    ) -> Vec<crate::protocol::endpoint::EndpointKeymapCommand> {
        self.entries
            .iter()
            .filter_map(|entry| {
                Some(crate::protocol::endpoint::EndpointKeymapCommand {
                    command_id: entry.id.clone(),
                    path: entry.command.path_label.clone(),
                    identity: entry.command.identity(salt)?,
                })
            })
            .collect()
    }
}

/// The label Herdr clients before the keymap tree understand: `prefix+x`
/// for one key after the prefix, the chord itself at the top level, and
/// nothing for deeper keys.
fn legacy_binding_labels(path_label: &str, prefix: crate::config::KeyCombo) -> Vec<String> {
    let chords = path_label.split(' ').collect::<Vec<_>>();
    if chords.iter().any(|chord| chord.contains("..")) {
        return Vec::new();
    }
    match chords.as_slice() {
        [chord] => vec![(*chord).to_owned()],
        [first, chord] if *first == crate::config::format_key_combo(prefix) => {
            vec![format!("prefix+{chord}")]
        }
        _ => Vec::new(),
    }
}

impl App {
    pub(crate) fn client_shell_command_manifest(&self) -> Vec<crate::protocol::ClientShellCommand> {
        self.endpoint_commands
            .entries
            .iter()
            .map(|entry| crate::protocol::ClientShellCommand {
                command_id: entry.id.clone(),
                binding_label: entry.command.path_label.clone(),
                binding_labels: legacy_binding_labels(
                    &entry.command.path_label,
                    self.keymap.prefix,
                ),
                action: entry.action,
                description: Some(entry.command.hint.clone()),
            })
            .collect()
    }

    pub(crate) fn resolve_client_shell_command(
        &self,
        id: &str,
    ) -> Option<crate::input::keymap::CompiledCommand> {
        self.endpoint_commands
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.command.clone())
    }

    pub(crate) fn handle_command_invoke(
        &mut self,
        id: String,
        params: crate::api::schema::CommandInvokeParams,
    ) -> String {
        let Some(binding) = self.resolve_client_shell_command(&params.command_id) else {
            return crate::app::api::responses::encode_error(
                id,
                "command_not_found",
                "custom command manifest is stale; reload configuration",
            );
        };
        if let Err((code, message)) = self.focus_client_shell_command_target(&params) {
            return crate::app::api::responses::encode_error(id, code, message);
        }
        let selected_text = if binding.spec.kind == crate::input::keymap::CommandKind::Plugin {
            let Some(selection) = params.selection.as_ref() else {
                return self.execute_custom_command_response(id, &binding, None);
            };
            if params.pane_id.as_deref() != Some(selection.pane_id.as_str()) {
                return crate::app::api::responses::encode_error(
                    id,
                    "command_target_mismatch",
                    "command selection does not belong to the requested pane",
                );
            }
            match self.pane_selection_text(selection) {
                Ok(text) => Some(text),
                Err((code, message)) => {
                    return crate::app::api::responses::encode_error(id, code, message);
                }
            }
        } else {
            None
        };
        self.execute_custom_command_response(id, &binding, selected_text)
    }

    fn execute_custom_command_response(
        &mut self,
        id: String,
        binding: &crate::input::keymap::CompiledCommand,
        selected_text: Option<String>,
    ) -> String {
        match self.execute_custom_command_binding(binding, selected_text) {
            Ok(()) => crate::app::api::responses::encode_success(
                id,
                crate::api::schema::ResponseResult::Ok {},
            ),
            Err(error) => {
                crate::app::api::responses::encode_error(id, "command_failed", error.to_string())
            }
        }
    }

    fn focus_client_shell_command_target(
        &mut self,
        params: &crate::api::schema::CommandInvokeParams,
    ) -> Result<(), (&'static str, String)> {
        if let Some(pane_id) = params.pane_id.as_deref() {
            let Some((workspace_index, pane)) = self.parse_pane_id(pane_id) else {
                return Err(("pane_not_found", format!("pane not found: {pane_id}")));
            };
            let Some(tab_index) =
                self.state.workspaces[workspace_index].find_tab_index_for_pane(pane)
            else {
                return Err(("pane_not_found", format!("pane not found: {pane_id}")));
            };
            self.validate_client_shell_command_parent_ids(params, workspace_index, tab_index)?;
            self.state.focus_pane_in_workspace(workspace_index, pane);
            return Ok(());
        }

        if let Some(tab_id) = params.tab_id.as_deref() {
            let Some((workspace_index, tab_index)) = self.parse_tab_id(tab_id) else {
                return Err(("tab_not_found", format!("tab not found: {tab_id}")));
            };
            self.validate_client_shell_command_workspace_id(params, workspace_index)?;
            self.state.switch_workspace_tab(workspace_index, tab_index);
            return Ok(());
        }

        if let Some(workspace_id) = params.workspace_id.as_deref() {
            let Some(workspace_index) = self.parse_workspace_id(workspace_id) else {
                return Err((
                    "workspace_not_found",
                    format!("workspace not found: {workspace_id}"),
                ));
            };
            self.state.switch_workspace(workspace_index);
        }
        Ok(())
    }

    fn validate_client_shell_command_parent_ids(
        &self,
        params: &crate::api::schema::CommandInvokeParams,
        workspace_index: usize,
        tab_index: usize,
    ) -> Result<(), (&'static str, String)> {
        self.validate_client_shell_command_workspace_id(params, workspace_index)?;
        if let Some(tab_id) = params.tab_id.as_deref() {
            let actual = self
                .public_tab_id(workspace_index, tab_index)
                .unwrap_or_default();
            if tab_id != actual {
                return Err((
                    "command_target_mismatch",
                    "command pane does not belong to the requested tab".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn validate_client_shell_command_workspace_id(
        &self,
        params: &crate::api::schema::CommandInvokeParams,
        workspace_index: usize,
    ) -> Result<(), (&'static str, String)> {
        if params
            .workspace_id
            .as_deref()
            .is_some_and(|workspace_id| workspace_id != self.public_workspace_id(workspace_index))
        {
            return Err((
                "command_target_mismatch",
                "command target does not belong to the requested workspace".to_owned(),
            ));
        }
        Ok(())
    }
    pub(crate) fn execute_custom_command_binding(
        &mut self,
        binding: &crate::input::keymap::CompiledCommand,
        selected_text: Option<String>,
    ) -> io::Result<()> {
        let spec = &binding.spec;
        match spec.kind {
            crate::input::keymap::CommandKind::Shell => self.spawn_custom_command(&spec.command),
            crate::input::keymap::CommandKind::Pane => {
                self.spawn_pane_command(&spec.command, Vec::new())
            }
            crate::input::keymap::CommandKind::Popup => self.spawn_custom_popup_command(spec),
            crate::input::keymap::CommandKind::Plugin => self
                .invoke_plugin_action_from_keybind(spec.command.clone(), selected_text)
                .map_err(io::Error::other),
        }
    }

    fn spawn_custom_popup_command(
        &mut self,
        spec: &crate::input::keymap::CommandSpec,
    ) -> io::Result<()> {
        self.spawn_popup_shell_command(
            &spec.command,
            None,
            self.custom_command_env().0,
            crate::app::popup::PopupGeometry {
                width: spec.width,
                height: spec.height,
            },
        )
    }

    pub(crate) fn custom_command_env(&self) -> (Vec<(String, String)>, Option<std::path::PathBuf>) {
        let mut env = vec![(
            crate::api::SOCKET_PATH_ENV_VAR.to_string(),
            crate::api::socket_path().display().to_string(),
        )];
        if let Ok(current_exe) = std::env::current_exe() {
            env.push((
                "HERDR_BIN_PATH".to_string(),
                current_exe.display().to_string(),
            ));
        }

        let mut cwd = None;
        if let Some(ws_idx) = self.state.active {
            env.push((
                "HERDR_ACTIVE_WORKSPACE_ID".to_string(),
                self.public_workspace_id(ws_idx),
            ));
            if let Some(workspace) = self.state.workspaces.get(ws_idx) {
                let tab_idx = workspace.active_tab_index();
                if let Some(tab_id) = self.public_tab_id(ws_idx, tab_idx) {
                    env.push(("HERDR_ACTIVE_TAB_ID".to_string(), tab_id));
                }
                if let Some(pane_id) = workspace.focused_pane_id() {
                    if let Some(public_pane_id) = self.public_pane_id(ws_idx, pane_id) {
                        env.push(("HERDR_ACTIVE_PANE_ID".to_string(), public_pane_id));
                    }
                    if let Some(pane_cwd) = workspace.active_tab().and_then(|tab| {
                        tab.cwd_for_pane(pane_id, &self.state.terminals, &self.terminal_runtimes)
                    }) {
                        env.push((
                            "HERDR_ACTIVE_PANE_CWD".to_string(),
                            pane_cwd.display().to_string(),
                        ));
                        if pane_cwd.is_dir() {
                            cwd = Some(pane_cwd);
                        }
                    }
                }
            }
        }
        (env, cwd)
    }

    fn spawn_custom_command(&mut self, command_text: &str) -> std::io::Result<()> {
        let mut command = crate::platform::detached_custom_command_process(command_text);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let (env, cwd) = self.custom_command_env();
        command.envs(env);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let child = command.spawn()?;
        self.detached_process_children.push(child);
        Ok(())
    }

    pub(crate) fn open_focused_scrollback_in_editor(&mut self) -> std::io::Result<()> {
        let ws_idx = self
            .state
            .active
            .ok_or_else(|| std::io::Error::other("no active workspace"))?;
        let ws = self
            .state
            .workspaces
            .get(ws_idx)
            .ok_or_else(|| std::io::Error::other("active workspace disappeared"))?;
        let pane_id = ws
            .focused_pane_id()
            .ok_or_else(|| std::io::Error::other("no focused pane"))?;
        let scrollback = self
            .state
            .runtime_for_pane_in_workspace(&self.terminal_runtimes, ws_idx, pane_id)
            .ok_or_else(|| std::io::Error::other("focused pane has no scrollback runtime"))?
            .recent_unwrapped_text_snapshot(usize::MAX)
            .text;

        let path = write_scrollback_temp_file(&scrollback)?;

        let argv = match crate::platform::scrollback_editor_argv(&path) {
            Ok(argv) => argv,
            Err(err) => {
                let _ = fs::remove_file(&path);
                return Err(err);
            }
        };
        let (env, _) = self.custom_command_env();
        let new_pane = match self.spawn_overlay_argv_command(&argv, None, env, vec![path.clone()]) {
            Ok((_, new_pane)) => new_pane,
            Err(err) => {
                let _ = fs::remove_file(&path);
                return Err(err);
            }
        };
        let terminal_id = new_pane.terminal.id.clone();
        self.terminal_runtimes
            .insert(terminal_id.clone(), new_pane.runtime);
        self.state
            .remove_alias_shadowed_by_new_pane(new_pane.pane_id);
        self.state.terminals.insert(terminal_id, new_pane.terminal);

        if let Some(public_pane_id) = self.public_pane_id(ws_idx, pane_id) {
            self.state.toast = Some(crate::app::state::ToastNotification {
                kind: crate::app::state::ToastKind::Finished,
                title: "opened scrollback".to_string(),
                context: format!("focused pane {public_pane_id}"),
                position: None,
                target: None,
            });
        }
        Ok(())
    }

    fn spawn_pane_command(
        &mut self,
        command: &str,
        temp_files: Vec<std::path::PathBuf>,
    ) -> std::io::Result<()> {
        let Some(ws_idx) = self.state.active else {
            return Err(std::io::Error::other("no active workspace"));
        };
        let previous_focus_target = self.state.current_pane_focus_target();
        let (rows, cols) = self.state.estimate_pane_size();
        let new_rows = rows.max(4);
        let new_cols = cols.max(10);
        let (env, _) = self.custom_command_env();

        let ws = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .ok_or_else(|| std::io::Error::other("active workspace disappeared"))?;
        let tab_idx = ws.active_tab_index();
        let previous_focus = ws
            .focused_pane_id()
            .ok_or_else(|| std::io::Error::other("no focused pane"))?;
        let previous_zoomed = ws.active_tab().map(|tab| tab.zoomed).unwrap_or(false);
        let cwd = ws.active_tab().and_then(|tab| {
            tab.cwd_for_pane(
                previous_focus,
                &self.state.terminals,
                &self.terminal_runtimes,
            )
        });
        let new_pane = ws.split_focused_command(
            Direction::Horizontal,
            new_rows,
            new_cols,
            cwd,
            command,
            env,
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.state.host_terminal_appearance,
        )?;
        let new_pane_id = new_pane.pane_id;
        self.terminal_runtimes
            .insert(new_pane.terminal.id.clone(), new_pane.runtime);
        self.state
            .terminals
            .insert(new_pane.terminal.id.clone(), new_pane.terminal);
        let new_focus_target = crate::app::state::PaneFocusTarget {
            workspace_id: ws.id.clone(),
            pane_id: new_pane_id,
        };
        if previous_focus_target.as_ref() != Some(&new_focus_target) {
            self.state.previous_pane_focus = previous_focus_target;
        }
        ws.active_tab_mut()
            .expect("workspace must have an active tab")
            .layout
            .focus_pane(new_pane_id);
        ws.active_tab_mut()
            .expect("workspace must have an active tab")
            .zoomed = true;
        self.overlay_panes.insert(
            new_pane_id,
            super::OverlayPaneState {
                ws_idx,
                tab_idx,
                previous_focus,
                previous_zoomed,
                temp_files,
            },
        );
        self.state.remove_alias_shadowed_by_new_pane(new_pane_id);
        self.state.mode = Mode::Terminal;
        Ok(())
    }

    pub(crate) fn spawn_overlay_argv_command(
        &mut self,
        argv: &[String],
        cwd: Option<std::path::PathBuf>,
        extra_env: Vec<(String, String)>,
        temp_files: Vec<std::path::PathBuf>,
    ) -> std::io::Result<(usize, crate::workspace::NewPane)> {
        let Some(ws_idx) = self.state.active else {
            return Err(std::io::Error::other("no active workspace"));
        };
        let previous_focus_target = self.state.current_pane_focus_target();
        let (rows, cols) = self.state.estimate_pane_size();
        let new_rows = rows.max(4);
        let new_cols = cols.max(10);

        let ws = self
            .state
            .workspaces
            .get(ws_idx)
            .ok_or_else(|| std::io::Error::other("active workspace disappeared"))?;
        let previous_focus = ws
            .focused_pane_id()
            .ok_or_else(|| std::io::Error::other("no focused pane"))?;
        let cwd = cwd.or_else(|| {
            ws.active_tab().and_then(|tab| {
                tab.cwd_for_pane(
                    previous_focus,
                    &self.state.terminals,
                    &self.terminal_runtimes,
                )
            })
        });

        let (tab_idx, new_pane, workspace_id) = {
            let ws = self
                .state
                .workspaces
                .get_mut(ws_idx)
                .ok_or_else(|| std::io::Error::other("active workspace disappeared"))?;
            let previous_zoomed = ws.active_tab().map(|tab| tab.zoomed).unwrap_or(false);
            let result = ws.split_pane_argv_command(
                previous_focus,
                Direction::Horizontal,
                new_rows,
                new_cols,
                cwd,
                argv,
                extra_env,
                self.state.pane_scrollback_limit_bytes,
                self.state.host_terminal_theme,
                self.state.host_terminal_appearance,
                true,
            );
            let (tab_idx, new_pane) = match result {
                Some(Ok(result)) => result,
                Some(Err(err)) => return Err(err),
                None => return Err(std::io::Error::other("focused pane disappeared")),
            };
            ws.tabs
                .get_mut(tab_idx)
                .ok_or_else(|| std::io::Error::other("plugin overlay tab disappeared"))?
                .zoomed = true;
            self.overlay_panes.insert(
                new_pane.pane_id,
                super::OverlayPaneState {
                    ws_idx,
                    tab_idx,
                    previous_focus,
                    previous_zoomed,
                    temp_files,
                },
            );
            (tab_idx, new_pane, ws.id.clone())
        };

        let new_focus_target = crate::app::state::PaneFocusTarget {
            workspace_id,
            pane_id: new_pane.pane_id,
        };
        if previous_focus_target.as_ref() != Some(&new_focus_target) {
            self.state.previous_pane_focus = previous_focus_target;
        }
        self.state.switch_workspace_tab(ws_idx, tab_idx);
        self.state.mode = Mode::Terminal;
        Ok((ws_idx, new_pane))
    }
}

fn write_scrollback_temp_file(content: &str) -> io::Result<std::path::PathBuf> {
    let mut last_collision = None;
    for attempt in 0..16 {
        let path = unique_scrollback_path(attempt);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(content.as_bytes())?;
                return Ok(path);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some(err);
            }
            Err(err) => return Err(err),
        }
    }

    Err(last_collision.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "failed to create unique scrollback temp file",
        )
    }))
}

fn unique_scrollback_path(attempt: u32) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "herdr-scrollback-{}-{nanos}-{attempt}.txt",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    fn test_app() -> crate::app::App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        crate::app::App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    use crate::input::keymap::{CommandKind, CommandSpec, CompiledCommand};

    fn binding(kind: CommandKind) -> CompiledCommand {
        CompiledCommand {
            path_label: "ctrl+b z".into(),
            spec: CommandSpec {
                kind,
                command: "secret-command --token hidden".into(),
                width: None,
                height: None,
            },
            hint: "safe description".into(),
            owner: crate::input::keymap::LayerOwner::User,
        }
    }

    fn install(app: &mut crate::app::App, binding: CompiledCommand) {
        app.endpoint_commands = super::EndpointCommandRegistry::new(&[binding]);
    }

    #[test]
    fn manifest_exposes_opaque_ids_without_command_text() {
        let mut app = test_app();
        install(&mut app, binding(CommandKind::Shell));
        let manifest = app.client_shell_command_manifest();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].binding_label, "ctrl+b z");
        assert_eq!(manifest[0].binding_labels, ["prefix+z"]);
        let projection = app.keymap_projection("boot");
        assert_eq!(projection.commands.len(), 1);
        assert_eq!(projection.commands[0].command_id, manifest[0].command_id);
        assert!(!format!("{projection:?}").contains("secret-command"));
        assert_eq!(manifest[0].description.as_deref(), Some("safe description"));
        assert!(!format!("{:?}", manifest).contains("secret-command"));
        assert_eq!(
            app.resolve_client_shell_command(&manifest[0].command_id)
                .map(|binding| binding.spec.command),
            Some("secret-command --token hidden".into())
        );
    }

    #[test]
    fn stale_command_id_is_rejected_after_definition_changes() {
        let mut app = test_app();
        install(&mut app, binding(CommandKind::Shell));
        let old_id = app.client_shell_command_manifest()[0].command_id.clone();
        let mut replacement = binding(CommandKind::Shell);
        replacement.spec.command = "replacement-command".into();
        install(&mut app, replacement);

        let response = app.handle_command_invoke(
            "request-1".into(),
            crate::api::schema::CommandInvokeParams {
                command_id: old_id,
                workspace_id: None,
                tab_id: None,
                pane_id: None,
                selection: None,
            },
        );
        let error: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "command_not_found");
    }

    #[test]
    fn command_ids_do_not_alias_across_endpoint_restarts() {
        let mut old_app = test_app();
        install(&mut old_app, binding(CommandKind::Shell));
        let old_id = old_app.client_shell_command_manifest()[0]
            .command_id
            .clone();

        let mut replacement_app = test_app();
        let mut replacement = binding(CommandKind::Shell);
        replacement.spec.command = "replacement-command".into();
        install(&mut replacement_app, replacement);
        assert_ne!(
            replacement_app.client_shell_command_manifest()[0].command_id,
            old_id
        );
        let response = replacement_app.handle_command_invoke(
            "request-1".into(),
            crate::api::schema::CommandInvokeParams {
                command_id: old_id,
                workspace_id: None,
                tab_id: None,
                pane_id: None,
                selection: None,
            },
        );
        let error: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "command_not_found");
    }

    #[tokio::test]
    async fn plugin_command_rejects_stale_client_selection_before_invocation() {
        let mut app = test_app();
        let workspace = crate::workspace::Workspace::test_new("plugin-selection");
        let pane_id = workspace.tabs[0].root_pane;
        let terminal_id = workspace.terminal_id(pane_id).cloned().unwrap();
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.terminal_runtimes.insert(
            terminal_id,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 24, b"selected text\n"),
        );
        let mut plugin = binding(CommandKind::Plugin);
        plugin.spec.command = "missing.plugin-action".into();
        install(&mut app, plugin);
        let command_id = app.client_shell_command_manifest()[0].command_id.clone();
        let workspace_id = app.public_workspace_id(0);
        let tab_id = app.public_tab_id(0, 0).unwrap();
        let pane_id = app.public_pane_id(0, pane_id).unwrap();

        let response = app.handle_command_invoke(
            "request-selection".into(),
            crate::api::schema::CommandInvokeParams {
                command_id,
                workspace_id: Some(workspace_id),
                tab_id: Some(tab_id),
                pane_id: Some(pane_id.clone()),
                selection: Some(crate::api::schema::PaneSelectionReadParams {
                    pane_id,
                    anchor: crate::api::schema::PaneTextPoint { row: 0, col: 0 },
                    cursor: crate::api::schema::PaneTextPoint { row: 0, col: 7 },
                    content_revision: Some(u64::MAX),
                }),
            },
        );

        let error: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "stale_content");
    }

    #[cfg(unix)]
    #[test]
    fn shell_command_invocation_executes_endpoint_owned_definition() {
        let mut app = test_app();
        let path = std::path::PathBuf::from(format!(
            "/var/tmp/herdr-command-invoke-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);
        let mut command = binding(CommandKind::Shell);
        command.spec.command = format!("printf invoked > {}", path.display());
        install(&mut app, command);
        let command_id = app.client_shell_command_manifest()[0].command_id.clone();

        let response = app.handle_command_invoke(
            "request-1".into(),
            crate::api::schema::CommandInvokeParams {
                command_id,
                workspace_id: None,
                tab_id: None,
                pane_id: None,
                selection: None,
            },
        );
        let success: crate::api::schema::SuccessResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(success.result, crate::api::schema::ResponseResult::Ok {});
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "invoked");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn popup_commands_are_advertised_without_exposing_the_command_text() {
        let mut app = test_app();
        install(&mut app, binding(CommandKind::Popup));

        let manifest = app.client_shell_command_manifest();
        assert_eq!(manifest.len(), 1);
        assert_eq!(
            manifest[0].action,
            crate::protocol::ClientShellCommandAction::Popup
        );
        assert!(!format!("{manifest:?}").contains("secret-command"));
    }
}
