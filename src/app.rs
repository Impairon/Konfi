use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime};
use anyhow::Context;

use crate::config::*;
use crate::ops;

#[derive(Clone)]
pub struct Node { pub path: PathBuf, pub depth: usize, pub is_dir: bool, pub expanded: bool, pub size: Option<u64>, pub broken_symlink: bool, pub has_broken_descendant: bool, pub is_symlink: bool, pub is_executable: bool }

#[derive(PartialEq, Clone, Copy)]
pub enum InputMode {
    Normal, AddingPath, AddingAlias, AddingDir, AddingNote, Renaming, Search, VersionSelect,
    SudoPrompt, Help, Shell, VisualSelect, DeployConfirm, SelectEditor, AddingTag,
    Settings, SettingsInput, HooksMenu, HookPathInput, RootMenu, AddingRoot,
    AddingThemePath, AddingThemeName, Info, TrashManagement,
    ChmodPrompt, ServicesMenu, QuickMove,
    KeybindMenu, KeybindCapture, AddCustomBind, KeyPicker,
}

pub enum UndoAction { Delete { original: PathBuf, trash: PathBuf }, Rename { from: PathBuf, to: PathBuf }, Move { from: PathBuf, to: PathBuf } }

#[derive(Clone, Copy)]
pub enum SettingAction {
    CycleTheme, AddCustomTheme, CycleColorMode, ToggleFolders, ToggleSizes,
    ToggleHidden, ToggleSymlinks, InputListWidth, SelectEditor, ToggleVersioning,
    InputVersionLimit, CycleDeployMode, CycleBackupBehavior, CycleOverwrite,
    ToggleConfirmDel, ToggleConfirmMove, ToggleMouse, CycleSearchMode, ToggleHooks,
    OpenTrash, CycleTrashRetention, EditGlobalHooks, EditKeybinds,
    ToggleTerminalWindow,
}

impl SettingAction {
    pub fn category(self) -> &'static str {
        match self {
            SettingAction::CycleTheme | SettingAction::AddCustomTheme | SettingAction::CycleColorMode
            | SettingAction::ToggleFolders | SettingAction::ToggleSizes | SettingAction::ToggleHidden
            | SettingAction::ToggleSymlinks | SettingAction::InputListWidth => "Appearance",
            SettingAction::SelectEditor => "Editor",
            SettingAction::ToggleVersioning | SettingAction::InputVersionLimit => "Versioning",
            SettingAction::CycleDeployMode | SettingAction::CycleBackupBehavior | SettingAction::CycleOverwrite => "Deployment",
            SettingAction::ToggleConfirmDel | SettingAction::ToggleConfirmMove => "Safety",
            SettingAction::ToggleMouse | SettingAction::CycleSearchMode | SettingAction::ToggleHooks | SettingAction::EditGlobalHooks | SettingAction::EditKeybinds => "Input & Hooks",
            SettingAction::ToggleTerminalWindow => "Terminal",
            SettingAction::OpenTrash | SettingAction::CycleTrashRetention => "Maintenance",
        }
    }
}

pub const SETTINGS_MENU: &[SettingAction] = &[
    SettingAction::CycleTheme, SettingAction::AddCustomTheme, SettingAction::CycleColorMode,
    SettingAction::ToggleFolders, SettingAction::ToggleSizes, SettingAction::ToggleHidden,
    SettingAction::ToggleSymlinks, SettingAction::InputListWidth, SettingAction::SelectEditor,
    SettingAction::ToggleVersioning, SettingAction::InputVersionLimit,
    SettingAction::CycleDeployMode, SettingAction::CycleBackupBehavior, SettingAction::CycleOverwrite,
    SettingAction::ToggleConfirmDel, SettingAction::ToggleConfirmMove,
    SettingAction::ToggleMouse, SettingAction::CycleSearchMode, SettingAction::ToggleHooks, SettingAction::EditGlobalHooks, SettingAction::EditKeybinds,
    SettingAction::ToggleTerminalWindow, SettingAction::OpenTrash, SettingAction::CycleTrashRetention,
];

pub const HOOK_LIST: &[&str] = &[
    "pre_edit", "post_edit", "pre_deploy", "post_deploy", "pre_archive", "post_archive",
    "pre_move", "post_move", "pre_delete", "post_delete", "pre_save_version", "post_save_version",
    "pre_discover", "post_discover",
];

pub const OBJECT_HOOK_LIST: &[&str] = &[
    "pre_edit", "post_edit", "pre_deploy", "post_deploy", "pre_archive", "post_archive",
    "pre_move", "post_move", "pre_delete", "post_delete", "pre_save_version", "post_save_version",
];

fn is_internal_path_for(assets: &Path, p: &Path) -> bool {
    p.starts_with(assets) || p.file_name().map(|n| n == ".assets").unwrap_or(false)
}

const MAX_TREE_DEPTH: usize = 16; // symlink-cycle guard

pub struct App {
    // tree + ui state
    pub nodes: Vec<Node>, pub filtered_nodes: Vec<Node>, pub state: ratatui::widgets::ListState,
    pub preview_text: Vec<u8>, pub input_mode: InputMode, pub input: String, pub add_source_path: String,
    pub status: String, pub last_status_time: Option<SystemTime>, pub status_sticky: bool,
    pub pending_g: bool, pub needs_clear: bool, pub hide_password: bool,
    pub preview_pinned: bool, pub file_list_area: ratatui::layout::Rect,
    pub help_scroll: usize, pub help_page_len: usize, pub help_total: usize, pub help_page: usize,
    pub key_picker_group: bool, pub key_picker_state: ratatui::widgets::ListState,
    // dirs
    pub confy_dir: PathBuf, pub assets_dir: PathBuf, pub history_dir: PathBuf, pub trash_dir: PathBuf,
    // fs watching
    pub last_dir_mtime: Option<SystemTime>, pub last_file_mtime: Option<SystemTime>,
    pub file_baselines: HashMap<PathBuf, (u64, Option<SystemTime>)>,
    // versions
    pub version_state: ratatui::widgets::ListState, pub available_versions: Vec<(String, PathBuf)>,
    pub current_rel_path: Option<PathBuf>,
    // clipboard-ish
    pub cut_source: Option<PathBuf>, pub yank_source: Option<PathBuf>,
    pub selected_nodes: HashSet<PathBuf>, pub pending_delete: Option<PathBuf>,
    // sudo
    pub sudo_target_file: Option<PathBuf>, pub sudo_target_node: Option<PathBuf>,
    // editors
    pub selected_editor: Option<String>, pub available_editors: Vec<String>,
    pub editor_state: ratatui::widgets::ListState, pub tui_edit_target: Option<PathBuf>,
    // data
    pub state_data: ConfyState, pub hooks: ConfyHooks,
    pub roots: Vec<PathBuf>, pub root_state: ratatui::widgets::ListState,
    pub keymap: KeyMap, pub shell_aliases: AliasMap,
    pub keybind_state: ratatui::widgets::ListState,
    pub pending_keybind: Option<KeyBind>, pub pending_reset: Option<u8>,
    pub deploy_index: HashMap<String, ops::DeployIndex>, pub modified_cache: HashSet<PathBuf>,
    // menus
    pub settings_state: ratatui::widgets::ListState, pub hooks_state: ratatui::widgets::ListState,
    pub hooks_target_path: Option<String>, pub hook_edit_type: String,
    pub settings_input_target: String, pub return_mode: InputMode,
    pub trash_items: Vec<PathBuf>, pub trash_state: ratatui::widgets::ListState,
    pub available_services: Vec<(String, String, String)>, pub service_state: ratatui::widgets::ListState,
    pub service_user_scope: bool,
    pub service_filter: String, pub service_name_input: String,
    // filters
    pub show_hidden: bool, pub bookmarks_only: bool, pub host_filter: bool, pub jump_list: bool,
    // misc flows
    pub tag_type: String, pub undo_stack: Vec<UndoAction>,
    pub deploy_target: Option<PathBuf>,
    pub pending_theme_colors: Option<ThemeColors>, pub pending_theme_path: String,
    pub is_dragging: bool, pub theme_dirty: bool,
    pub last_mouse_click: Option<(Instant, PathBuf)>,
}

impl App {
    pub fn new(confy_dir: PathBuf) -> anyhow::Result<Self> {
        let assets_dir = confy_dir.join(".assets");
        let history_dir = assets_dir.join(".history");
        let trash_dir = assets_dir.join(".trash");
        std::fs::create_dir_all(&confy_dir).context("confy dir")?;
        std::fs::create_dir_all(&assets_dir).context("assets dir")?;
        std::fs::create_dir_all(&history_dir).context("history dir")?;
        std::fs::create_dir_all(&trash_dir).context("trash dir")?;
        let _ = crate::secrets::ensure_root_layout(&confy_dir);

        let state_data = ConfyState::load(&assets_dir.join(".state.json"));
        if let Err(e) = state_data.settings.validate() { tracing::warn!(?e, "settings validation"); }
        let hooks = crate::config::load_hooks(&assets_dir.join(".hooks.json"));
        let km = crate::config::load_keymap(&assets_dir.join(".keybinds.json"));
        let deploy_index = ops::load_deploy_index(&assets_dir.join(".deploy_index.json"));
        let show_hidden = state_data.settings.show_hidden_by_default;
        let selected_editor = state_data.settings.default_editor.clone();
        let mut roots = load_roots(&assets_dir.join(".roots.json"));
        if !roots.contains(&confy_dir) { roots.push(confy_dir.clone()); }

        let mut state = ratatui::widgets::ListState::default(); state.select(Some(0));
        let mut app = App {
            nodes: Vec::new(), filtered_nodes: Vec::new(), state,
            preview_text: Vec::new(), input_mode: InputMode::Normal, input: String::new(),
            add_source_path: String::new(), status: String::new(), last_status_time: None,
            status_sticky: false, pending_g: false, needs_clear: false, hide_password: true,
            preview_pinned: false, file_list_area: ratatui::layout::Rect::default(),
            help_scroll: 0, help_page_len: 20, help_total: 0, help_page: 0,
            key_picker_group: false, key_picker_state: ratatui::widgets::ListState::default(),
            confy_dir: confy_dir.clone(), assets_dir: assets_dir.clone(), history_dir, trash_dir,
            last_dir_mtime: None, last_file_mtime: None, file_baselines: HashMap::new(),
            version_state: ratatui::widgets::ListState::default(), available_versions: Vec::new(),
            current_rel_path: None, cut_source: None, yank_source: None,
            selected_nodes: HashSet::new(), pending_delete: None,
            sudo_target_file: None, sudo_target_node: None,
            selected_editor, available_editors: Vec::new(),
            editor_state: ratatui::widgets::ListState::default(), tui_edit_target: None,
            state_data, hooks, roots, root_state: ratatui::widgets::ListState::default(),
            keymap: km.map, shell_aliases: km.aliases,
            keybind_state: ratatui::widgets::ListState::default(),
            pending_keybind: None, pending_reset: None,
            deploy_index, modified_cache: HashSet::new(),
            settings_state: ratatui::widgets::ListState::default(), hooks_state: ratatui::widgets::ListState::default(),
            hooks_target_path: None, hook_edit_type: String::new(), settings_input_target: String::new(),
            return_mode: InputMode::Normal,
            trash_items: Vec::new(), trash_state: ratatui::widgets::ListState::default(),
            available_services: Vec::new(), service_state: ratatui::widgets::ListState::default(),
            service_user_scope: true,
            service_filter: String::new(), service_name_input: String::new(),
            show_hidden, bookmarks_only: false, host_filter: false, jump_list: false,
            tag_type: String::new(), undo_stack: Vec::new(),
            deploy_target: None,
            pending_theme_colors: None, pending_theme_path: String::new(),
            is_dragging: false, theme_dirty: false,
            last_mouse_click: None,
        };
        app.clean_old_trash();
        app.refresh_items();
        app.capture_baselines();
        Ok(app)
    }

    // ---------- selection & nodes ----------

    pub fn selected_node(&self) -> Option<Node> { let i = self.state.selected()?; self.current_nodes().get(i).cloned() }
    pub fn selected_node_ref(&self) -> Option<&Node> { let i = self.state.selected()?; self.current_nodes().get(i) }
    pub fn current_nodes(&self) -> &Vec<Node> {
        if self.input_mode == InputMode::Search || self.bookmarks_only || self.host_filter || self.jump_list { &self.filtered_nodes } else { &self.nodes }
    }
    pub fn current_nodes_mut(&mut self) -> &mut Vec<Node> {
        if self.input_mode == InputMode::Search || self.bookmarks_only || self.host_filter || self.jump_list { &mut self.filtered_nodes } else { &mut self.nodes }
    }
    pub fn is_internal_path(&self, path: &Path) -> bool { is_internal_path_for(&self.assets_dir, path) }

    // ---------- status ----------

    pub fn set_status(&mut self, msg: impl Into<String>) { self.status = msg.into(); self.last_status_time = Some(SystemTime::now()); self.status_sticky = false; }
    pub fn set_sticky_status(&mut self, msg: impl Into<String>) { self.status = msg.into(); self.last_status_time = Some(SystemTime::now()); self.status_sticky = true; }

    // ---------- persistence ----------

    pub fn save_state(&self) { if let Err(e) = self.state_data.save(&self.assets_dir.join(".state.json")) { tracing::error!(?e, "save state"); } }
    pub fn save_hooks(&self) { if let Err(e) = crate::config::save_hooks(&self.hooks, &self.assets_dir.join(".hooks.json")) { tracing::error!(?e, "save hooks"); } self.save_state(); }
    pub fn save_roots(&self) { if let Err(e) = save_roots(&self.roots, &self.assets_dir.join(".roots.json")) { tracing::error!(?e, "save roots"); } }
    pub fn save_keymap(&self) {
        if let Err(e) = crate::config::save_keymap(&self.keymap, &self.shell_aliases, &self.assets_dir.join(".keybinds.json")) {
            tracing::error!(?e, "save keymap");
        }
    }
    pub fn theme_names(&self) -> Vec<String> {
        let mut n: Vec<String> = ["auto", "latte", "nord", "dracula", "gruvbox", "tokyonight", "mocha"].iter().map(|s| s.to_string()).collect();
        let mut c: Vec<String> = self.state_data.settings.custom_themes.keys().cloned().collect();
        c.sort(); n.extend(c); n
    }

    // ---------- hooks ----------

    pub fn hook_values(&self, name: &str, file: &Path) -> Vec<String> {
        if !self.state_data.settings.enable_hooks { return Vec::new(); }
        let mut out = Vec::new();
        if let Some(v) = self.hooks.get(name) { if !v.trim().is_empty() { out.push(v.clone()); } }
        if let Ok(rel) = file.strip_prefix(&self.confy_dir) {
            let rel_str = ops::path_to_string(&rel);
            if let Some(obj) = self.state_data.object_hooks.get(&rel_str) {
                if let Some(v) = obj.get(name) { if !v.trim().is_empty() { out.push(v.clone()); } }
            }
        }
        out
    }
    pub fn hook_context(&self, file: &Path, op: &str, success: bool) -> ops::HookContext {
        ops::HookContext::new(&self.confy_dir, file, op, success, &self.state_data.tags)
    }
    pub fn run_hook_reported(&self, name: &str, file: &Path, op: &str, success: bool) -> Option<String> {
        let values = self.hook_values(name, file);
        if values.is_empty() { return None; }
        let ctx = self.hook_context(file, op, success);
        let mut failures = Vec::new();
        let mut ran = 0;
        for v in &values { let r = ops::run_hook_value(v, &ctx); ran += 1; if !r.ok { failures.push(format!("{}: {}", name, r.detail)); } }
        if failures.is_empty() { Some(format!("{} hook ok ({} run)", name, ran)) } else { Some(format!("✗ {}", failures.join(" | "))) }
    }
    pub fn run_hook(&self, name: &str, file: &Path, op: &str, success: bool) { let _ = self.run_hook_reported(name, file, op, success); }
    pub fn test_hook(&mut self, value: &str) {
        let file = self.selected_node().map(|n| n.path).unwrap_or_else(|| self.confy_dir.clone());
        match ops::validate_hook(value) {
            Err(e) => self.set_sticky_status(format!("✗ {}", e)),
            Ok(desc) => {
                let ctx = self.hook_context(&file, "test", true);
                let r = ops::run_hook_value(value, &ctx);
                if r.ok { self.set_sticky_status(format!("✓ {} → {}", desc, r.detail)); }
                else { self.set_sticky_status(format!("✗ {} → {}", desc, r.detail)); }
            }
        }
    }

    // ---------- trash ----------

    pub fn clean_old_trash(&self) {
        let days = self.state_data.settings.trash_retention_days;
        if days == 0 { return; }
        if let Ok(entries) = std::fs::read_dir(&self.trash_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.to_string_lossy().ends_with(".json") { continue; }
                if let Some(left) = ops::trash_time_left(&p, days) {
                    if left <= 0 {
                        let _ = ops::remove_entry(&p);
                        ops::remove_trash_metadata(&p);
                        tracing::info!(file = ?p, retention_days = days, "trash auto-cleaned");
                    }
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir(&self.trash_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let s = ops::path_to_string(&p);
                if let Some(base) = s.strip_suffix(".json") {
                    if !Path::new(base).exists() { let _ = std::fs::remove_file(&p); }
                }
            }
        }
    }

    pub fn reload_trash_items(&mut self) {
        self.clean_old_trash();
        self.trash_items = std::fs::read_dir(&self.trash_dir).ok()
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path())
                .filter(|p| !p.to_string_lossy().ends_with(".json")).collect())
            .unwrap_or_default();
        self.trash_items.sort();
        if !self.trash_items.is_empty() {
            let i = self.trash_state.selected().unwrap_or(0).min(self.trash_items.len() - 1);
            self.trash_state.select(Some(i));
        } else { self.trash_state.select(None); }
    }

    pub fn restore_trash_item(&mut self) {
        let Some(i) = self.trash_state.selected() else { return };
        let Some(p) = self.trash_items.get(i).cloned() else { return };
        let orig = ops::load_trash_metadata(&p).map(|m| m.original_path)
            .unwrap_or_else(|| self.confy_dir.join(p.file_name().unwrap_or_default()));
        if let Some(par) = orig.parent() { let _ = std::fs::create_dir_all(par); }
        if ops::path_exists(&orig) { self.set_status("Target exists — not overwriting"); return; }
        match std::fs::rename(&p, &orig) {
            Ok(()) => { ops::remove_trash_metadata(&p); self.set_status(format!("Restored → {}", orig.display())); tracing::info!(?orig, "trash restored"); }
            Err(e) => self.set_status(format!("Restore failed: {}", e)),
        }
        self.reload_trash_items();
        self.refresh_items();
        self.capture_baselines();
    }

    pub fn purge_trash_item(&mut self) {
        let Some(i) = self.trash_state.selected() else { return };
        let Some(p) = self.trash_items.get(i).cloned() else { return };
        match ops::remove_entry(&p) {
            Ok(()) => { ops::remove_trash_metadata(&p); self.set_status("Purged."); }
            Err(e) => self.set_status(format!("Purge failed: {}", e)),
        }
        self.reload_trash_items();
    }

    // ---------- recent / versioning ----------

    pub fn add_to_recent(&mut self, path: &Path) {
        if let Ok(rel) = path.strip_prefix(&self.confy_dir) {
            let rs = ops::path_to_string(&rel);
            self.state_data.recent.retain(|x| x != &rs);
            self.state_data.recent.push_front(rs);
            self.state_data.recent.truncate(5);
            self.save_state();
        }
    }

    pub fn take_snapshot_and_cleanup(&mut self) {
        if let Err(e) = ops::take_snapshot(&self.confy_dir, &self.history_dir, &self.hooks, &self.state_data.settings, &self.state_data.tags) {
            tracing::warn!(?e, "snapshot failed");
        }
        self.capture_baselines();
    }

    // ---------- input lifecycle ----------

    /// Leave a menu: go back to Settings if we came from there, else Normal. Always clears return_mode.
    pub fn return_from_menu(&mut self) {
        self.input_mode = if self.return_mode == InputMode::Settings { InputMode::Settings } else { InputMode::Normal };
        self.return_mode = InputMode::Normal;
    }

    pub fn cancel_input(&mut self) {
        self.input_mode = if self.return_mode == InputMode::Settings { InputMode::Settings } else { InputMode::Normal };
        self.return_mode = InputMode::Normal;
        self.input.clear(); self.add_source_path.clear();
        self.pending_delete = None; self.cut_source = None; self.yank_source = None;
        self.sudo_target_file = None; self.sudo_target_node = None; self.status_sticky = false;
        self.deploy_target = None; self.tui_edit_target = None;
        self.hide_password = true; self.pending_g = false; self.pending_theme_colors = None;
        self.pending_theme_path.clear(); self.hooks_target_path = None; self.pending_keybind = None;
    }

    pub fn start_search(&mut self) {
        self.input_mode = InputMode::Search; self.input.clear();
        self.apply_filter(); self.state.select(Some(0)); self.update_preview();
    }

    // ---------- undo ----------

    fn push_undo(&mut self, a: UndoAction) {
        self.undo_stack.push(a);
        if self.undo_stack.len() > 50 { self.undo_stack.remove(0); }
    }

    pub fn undo(&mut self) {
        let Some(action) = self.undo_stack.pop() else { self.set_status("Nothing to undo."); return };
        match action {
            UndoAction::Delete { original, trash } => {
                self.run_hook("pre_delete", &original, "delete", false);
                if std::fs::rename(&trash, &original).is_ok() {
                    ops::remove_trash_metadata(&trash);
                    self.set_status("Undo: restored.");
                    self.refresh_items(); self.capture_baselines();
                    self.run_hook("post_delete", &original, "delete", true);
                } else {
                    self.set_status("Undo failed (target exists?).");
                    self.push_undo(UndoAction::Delete { original, trash });
                }
            }
            UndoAction::Rename { from, to } | UndoAction::Move { from, to } => {
                self.run_hook("pre_move", &to, "move", false);
                if std::fs::rename(&to, &from).is_ok() {
                    self.set_status("Undo: moved back.");
                    self.refresh_items(); self.capture_baselines();
                    self.run_hook("post_move", &from, "move", true);
                } else {
                    self.set_status("Undo failed (target exists?).");
                    self.push_undo(UndoAction::Move { from, to });
                }
            }
        }
    }

    // ---------- editors ----------

    pub fn enter_editor_select(&mut self, target: Option<PathBuf>) {
        let known = ["nvim", "vim", "vi", "nano", "code", "codium", "zed", "emacs", "micro", "kak", "hx"];
        let mut avail = Vec::new();
        if let Ok(ed) = std::env::var("EDITOR") { if !ed.trim().is_empty() { avail.push(ed.trim().to_string()); } }
        if let Ok(ed) = std::env::var("VISUAL") { if !ed.trim().is_empty() && !avail.iter().any(|a| a == ed.trim()) { avail.push(ed.trim().to_string()); } }
        for e in &known {
            if avail.iter().any(|a| a == e) { continue; }
            if ops::command_exists(e) { avail.push((*e).to_string()); }
        }
        if avail.is_empty() { self.set_status("No editors found."); return; }
        self.available_editors = avail; self.editor_state.select(Some(0));
        self.tui_edit_target = target; self.input_mode = InputMode::SelectEditor;
    }

    pub fn submit_editor_select(&mut self, set_default: bool) {
        if let Some(i) = self.editor_state.selected() {
            if let Some(ed) = self.available_editors.get(i).cloned() {
                let target = self.tui_edit_target.take();
                self.selected_editor = Some(ed.clone());
                if set_default || target.is_none() {
                    self.state_data.settings.default_editor = Some(ed.clone());
                    self.save_state();
                    self.set_status(format!("Saved '{}' as default.", ed));
                } else { self.set_status(format!("Using '{}'.", ed)); }
                match target {
                    Some(p) => { self.return_mode = InputMode::Normal; self.execute_edit(&p, &ed); }
                    None => self.return_from_menu(),
                }
                return;
            }
        }
        self.input_mode = InputMode::Normal;
    }

    fn edit_with_live_hooks(&mut self, tp: &Path, ed: &str, sudo_pass: Option<&str>) -> (bool, usize) {
        let post = self.hook_values("post_edit", tp);
        let ctx = self.hook_context(tp, "edit", true);
        let fired = std::cell::Cell::new(0usize);
        let interval = self.state_data.settings.post_edit_poll_interval_ms;
        let on_save = || {
            for v in &post { let r = ops::run_hook_value(v, &ctx); tracing::info!(file = ?ctx.file, ok = r.ok, "post_edit (live save)"); }
            if !post.is_empty() { fired.set(fired.get() + 1); }
        };
        let ok = if self.state_data.settings.post_edit_live {
            match sudo_pass {
                Some(pass) => ops::open_editor_sudo_watched(tp, ed, pass, interval, on_save),
                None => ops::open_editor_watched(tp, ed, interval, on_save),
            }
        } else {
            match sudo_pass {
                Some(pass) => { let _ = ops::open_editor_sudo_watched(tp, ed, pass, interval, on_save); true }
                None => ops::open_editor(tp, ed),
            }
        };
        (ok, fired.get())
    }

    pub fn execute_edit(&mut self, tp: &Path, ed: &str) {
        let np = self.selected_node().map(|n| n.path).unwrap_or_else(|| tp.to_path_buf());
        if let Some(report) = self.run_hook_reported("pre_edit", tp, "edit", false) {
            if report.starts_with('✗') { self.set_sticky_status(report); }
        }
        if !ops::command_exists(ed) {
            self.set_status("Editor not found — pick another.");
            self.enter_editor_select(Some(tp.to_path_buf()));
            return;
        }
        // sudo detection only makes sense for files (dirs fail write-open with IsADirectory)
        let needs_sudo = !tp.is_dir() && match std::fs::OpenOptions::new().write(true).open(tp) {
            Ok(_) => false, Err(e) => e.kind() == std::io::ErrorKind::PermissionDenied,
        };
        if needs_sudo {
            self.sudo_target_file = Some(tp.to_path_buf()); self.sudo_target_node = Some(np.clone());
            self.input_mode = InputMode::SudoPrompt; self.input.clear(); self.hide_password = true;
            self.set_status("Enter Sudo Password (Empty=View, Ctrl+H=Hide/Show, Esc=cancel)");
        } else {
            self.input_mode = InputMode::Normal; self.add_to_recent(&np);
            let (ok, fired) = self.edit_with_live_hooks(tp, ed, None);
            if !ok && fired == 0 { self.set_status("Editor exited without saving."); }
            self.needs_clear = true;
            tracing::info!(file = ?tp, editor = ed, saves = fired, "edit complete");
            self.take_snapshot_and_cleanup();
            if fired > 0 { self.set_status(format!("Saved {}× — post_edit hooks ran.", fired)); }
            else { self.run_hook("post_edit", tp, "edit", true); }
        }
    }

    pub fn submit_sudo_password(&mut self) {
        if let Some(file) = self.sudo_target_file.clone() {
            let node = self.sudo_target_node.clone().unwrap_or_else(|| file.clone());
            let pass = self.input.clone();
            if pass.is_empty() {
                if let Some(ed) = self.selected_editor.clone() { let _ = ops::open_editor(&file, &ed); }
                self.set_status("View-only."); self.needs_clear = true;
                self.cancel_input(); self.take_snapshot_and_cleanup();
                self.run_hook("post_edit", &file, "edit", true);
                return;
            }
            if ops::validate_sudo(&pass) {
                let mut fired = 0usize;
                if let Some(ed) = self.selected_editor.clone() {
                    let (ok, f) = self.edit_with_live_hooks(&file, &ed, Some(&pass));
                    fired = f;
                    if ok || f > 0 {
                        self.input_mode = InputMode::Normal; self.add_to_recent(&node);
                        self.set_status("Edited."); tracing::info!(file = ?file, saves = f, "sudo edit complete");
                    } else { self.set_status("Editor failed."); }
                }
                self.needs_clear = true; self.cancel_input(); self.take_snapshot_and_cleanup();
                if fired == 0 { self.run_hook("post_edit", &file, "edit", true); }
            } else { self.set_status("Wrong password. (Esc=cancel)"); self.input.clear(); }
        }
    }

    // ---------- tags / bookmarks / notes ----------

    pub fn begin_add_tag(&mut self, tt: &str) {
        if let Some(node) = self.selected_node() {
            if let Ok(r) = node.path.strip_prefix(&self.confy_dir) {
                self.add_source_path = ops::path_to_string(&r);
                self.tag_type = tt.to_string(); self.input_mode = InputMode::AddingTag;
                self.input.clear(); self.status.clear();
            }
        }
    }

    pub fn toggle_bookmark(&mut self) {
        if let Some(node) = self.selected_node() {
            if let Ok(r) = node.path.strip_prefix(&self.confy_dir) {
                let rs = ops::path_to_string(&r);
                if self.state_data.bookmarks.contains(&rs) { 
                    self.state_data.bookmarks.remove(&rs); 
                    self.set_status("Removed."); 
                }
                else { 
                    // Add bookmark and propagate to all parent directories
                    ops::propagate_bookmarks_to_parents(&rs, &mut self.state_data.bookmarks);
                    self.set_status("Added (parents marked)."); 
                }
                self.save_state(); self.refresh_items();
            }
        }
    }

    pub fn begin_add_note(&mut self) {
        if let Some(node) = self.selected_node() {
            if let Ok(r) = node.path.strip_prefix(&self.confy_dir) {
                let rs = ops::path_to_string(&r);
                self.add_source_path = rs.clone();
                self.input = self.state_data.notes.get(&rs).cloned().unwrap_or_default();
                self.input_mode = InputMode::AddingNote; self.status.clear();
            }
        }
    }

    // ---------- versions ----------

    pub fn enter_version_mode(&mut self) {
        if let Some(node) = self.selected_node() {
            if node.is_dir { self.set_status("Versions work on files."); return; }
            if let Ok(rel) = node.path.strip_prefix(&self.confy_dir) {
                self.current_rel_path = Some(rel.to_path_buf()); self.available_versions.clear();
                if let Ok(entries) = std::fs::read_dir(&self.history_dir) {
                    let mut snaps: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
                    snaps.sort();
                    for snap in snaps.iter().rev() {
                        let hf = snap.join(rel);
                        if hf.exists() {
                            let ds = ops::path_name(&snap);
                            let fmt = if ds.len() >= 13 { format!("{}-{}-{} {}:{}", &ds[0..4], &ds[4..6], &ds[6..8], &ds[9..11], &ds[11..13]) } else { ds.clone() };
                            self.available_versions.push((fmt, hf));
                        }
                    }
                }
                if !self.available_versions.is_empty() { self.input_mode = InputMode::VersionSelect; self.version_state.select(Some(0)); }
                else { self.set_status("No history."); }
            }
        }
    }

    pub fn restore_version(&mut self) {
        if let Some(ri) = self.version_state.selected() {
            let vi = ri / 2;
            if let Some((_, hp)) = self.available_versions.get(vi).cloned() {
                if let Some(rp) = &self.current_rel_path {
                    let dest = self.confy_dir.join(rp);
                    let is_sym = dest.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
                    let needs_sudo = match std::fs::OpenOptions::new().write(true).open(&dest) {
                        Ok(_) => false, Err(e) => e.kind() == std::io::ErrorKind::PermissionDenied,
                    };
                    if needs_sudo { self.set_status("Permission denied."); return; }
                    if !is_sym { let _ = ops::remove_entry(&dest); }
                    match std::fs::copy(&hp, &dest) {
                        Ok(_) => {
                            tracing::info!(file = ?dest, version = ?hp, "restored version");
                            self.set_status("Restored."); self.exit_version_mode();
                            self.refresh_items(); self.capture_baselines();
                        }
                        Err(e) => self.set_status(format!("Restore failed: {}", e)),
                    }
                }
            }
        }
    }

    pub fn show_diff(&mut self) {
        if let Some(ri) = self.version_state.selected() {
            let vi = ri / 2;
            if let Some((_, hp)) = self.available_versions.get(vi).cloned() {
                if let Some(rp) = &self.current_rel_path {
                    let cp = self.confy_dir.join(rp);
                    self.preview_pinned = true; self.preview_text.clear();
                    match ops::generate_diff(&hp, &cp) {
                        Ok(d) => {
                            self.preview_text = format!("\x1b[1;36m\u{f44e}  Diff (Esc=return)\x1b[0m\n\x1b[90m────────────────\x1b[0m\n{}", d).into_bytes();
                            self.set_sticky_status("Diff. (Esc=return)");
                        }
                        Err(e) => { self.preview_text = format!("Diff failed: {}", e).into_bytes(); self.set_status("Diff failed."); }
                    }
                }
            }
        }
    }

    pub fn exit_version_mode(&mut self) { self.input_mode = InputMode::Normal; self.available_versions.clear(); self.current_rel_path = None; self.version_state.select(None); self.update_preview(); }
    pub fn next_version(&mut self) { let n = self.available_versions.len(); if n == 0 { return; } let cur = self.version_state.selected().unwrap_or(0); let next = if cur + 2 >= 2 * n - 1 { 0 } else { cur + 2 }; self.version_state.select(Some(next)); }
    pub fn previous_version(&mut self) { let n = self.available_versions.len(); if n == 0 { return; } let cur = self.version_state.selected().unwrap_or(0); let prev = if cur < 2 { 2 * (n - 1) } else { cur - 2 }; self.version_state.select(Some(prev)); }

    // ---------- tree ----------

    pub fn sort_nodes(ns: &mut [Node], cd: &Path, bk: &HashSet<String>, sel: &HashSet<PathBuf>, ff: bool) {
        ns.sort_by(|a, b| {
            let ar = a.path.strip_prefix(cd).map(|p| ops::path_to_string(&p)).unwrap_or_default();
            let br = b.path.strip_prefix(cd).map(|p| ops::path_to_string(&p)).unwrap_or_default();
            bk.contains(&br).cmp(&bk.contains(&ar))
                .then_with(|| sel.contains(&b.path).cmp(&sel.contains(&a.path)))
                .then_with(|| if ff { b.is_dir.cmp(&a.is_dir) } else { std::cmp::Ordering::Equal })
                .then_with(|| a.path.file_name().unwrap_or_default().cmp(&b.path.file_name().unwrap_or_default()))
        });
    }

    fn scan_children(&self, dir: &Path, depth: usize) -> Vec<Node> {
        let sh = self.show_hidden;
        let assets = self.assets_dir.clone();
        let mut out: Vec<Node> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                let link_meta = std::fs::symlink_metadata(&path).ok();
                let is_symlink = link_meta.as_ref().is_some_and(|m| m.file_type().is_symlink());
                let broken_symlink = is_symlink && std::fs::metadata(&path).is_err();
                let meta = std::fs::metadata(&path).ok();
                let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = meta.as_ref().map(|m| m.len());
                let is_executable = meta.as_ref().map(|m| {
                    use std::os::unix::fs::PermissionsExt;
                    m.permissions().mode() & 0o111 != 0
                }).unwrap_or(false);
                let nm = ops::path_name(&path);
                if is_internal_path_for(&assets, &path) || nm == ".assets" || (!sh && nm.starts_with('.')) {
                    continue;
                }
                let has_broken_descendant = is_dir && Self::directory_has_broken_symlink(&path);
                out.push(Node { path, depth, is_dir, expanded: false, size, broken_symlink, has_broken_descendant, is_symlink, is_executable });
            }
        }
        Self::sort_nodes(&mut out, &self.confy_dir, &self.state_data.bookmarks, &self.selected_nodes, self.state_data.settings.folders_first);
        out
    }

    fn directory_has_broken_symlink(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else { return false; };
        entries.filter_map(|entry| entry.ok()).any(|entry| {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else { return false; };
            if meta.file_type().is_symlink() { return std::fs::metadata(&path).is_err(); }
            meta.is_dir() && Self::directory_has_broken_symlink(&path)
        })

    }

    pub fn refresh_items(&mut self) {
        let mut expanded: HashSet<PathBuf> = self.nodes.iter().filter(|n| n.is_dir && n.expanded).map(|n| n.path.clone()).collect();
        self.nodes = self.scan_children(&self.confy_dir.clone(), 0);
        let mut i = 0;
        while i < self.nodes.len() {
            if self.nodes[i].is_dir && self.nodes[i].depth < MAX_TREE_DEPTH && expanded.contains(&self.nodes[i].path) {
                let path = self.nodes[i].path.clone();
                let depth = self.nodes[i].depth + 1;
                self.nodes[i].expanded = true;
                expanded.remove(&path); // each dir expanded at most once per refresh (cycle guard)
                let children = self.scan_children(&path, depth);
                for (offset, child) in children.into_iter().enumerate() { self.nodes.insert(i + 1 + offset, child); }
            }
            i += 1;
        }
        self.apply_filter();
        let cur = self.state.selected().unwrap_or(0);
        if !self.current_nodes().is_empty() {
            self.state.select(Some(cur.min(self.current_nodes().len() - 1)));
        } else { self.state.select(None); }
        let mut max_mtime: Option<SystemTime> = std::fs::metadata(&self.confy_dir).ok().and_then(|m| m.modified().ok());
        for node in &self.nodes {
            if node.is_dir && node.expanded {
                if let Ok(meta) = std::fs::metadata(&node.path) {
                    if let Ok(mtime) = meta.modified() { if Some(mtime) > max_mtime { max_mtime = Some(mtime); } }
                }
            }
        }
        self.last_dir_mtime = max_mtime;

        // recompute "modified since deploy" markers here (NOT per frame — big files would freeze the UI)
        self.modified_cache.clear();
        if !self.deploy_index.is_empty() {
            for node in &self.nodes {
                if node.is_dir { continue; }
                if let Ok(rel) = node.path.strip_prefix(&self.confy_dir) {
                    if let Some(d) = self.deploy_index.get(&ops::path_to_string(&rel)) {
                        if ops::hash_file(&node.path).unwrap_or(u64::MAX) != d.last_deployed_hash {
                            self.modified_cache.insert(node.path.clone());
                        }
                    }
                }
            }
        }
        self.update_preview();
    }

    pub fn apply_filter(&mut self) {
        self.pending_delete = None;
        let mut base_nodes = self.nodes.clone();
        if self.bookmarks_only {
            base_nodes.retain(|n| match n.path.strip_prefix(&self.confy_dir) {
                Ok(r) => self.state_data.bookmarks.contains(&ops::path_to_string(&r)), Err(_) => false,
            });
        }
        if self.host_filter {
            let tag = format!("host:{}", ops::hostname());
            base_nodes.retain(|n| match n.path.strip_prefix(&self.confy_dir) {
                Ok(r) => self.state_data.tags.get(&ops::path_to_string(&r)).map(|t| t.contains(&tag)).unwrap_or(false),
                Err(_) => false,
            });
        }
        if self.jump_list {
            let rec: HashSet<String> = self.state_data.recent.iter().cloned().collect();
            base_nodes.retain(|n| match n.path.strip_prefix(&self.confy_dir) {
                Ok(r) => { let rs = ops::path_to_string(&r); self.state_data.bookmarks.contains(&rs) || rec.contains(&rs) }
                Err(_) => false,
            });
        }
        if self.input.is_empty() {
            self.filtered_nodes = base_nodes;
        } else {
            let q = self.input.to_lowercase();
            let fz = self.state_data.settings.search_mode.is_fuzzy();
            self.filtered_nodes = base_nodes.into_iter().filter(|n| {
                let name = n.path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
                ops::name_matches(&name, &q, fz)
            }).collect();
        }
        if !self.filtered_nodes.is_empty() {
            let cur = self.state.selected().unwrap_or(0);
            self.state.select(Some(cur.min(self.filtered_nodes.len() - 1)));
        } else if self.input_mode == InputMode::Search || self.bookmarks_only || self.host_filter || self.jump_list {
            self.state.select(None);
        }
    }

    pub fn expand_node(&mut self, idx: usize) {
        let (path, depth) = {
            let ns = self.current_nodes_mut();
            match ns.get_mut(idx) {
                Some(n) if n.is_dir => { n.expanded = true; (n.path.clone(), n.depth + 1) }
                _ => { self.update_preview(); return; }
            }
        };
        let children = self.scan_children(&path, depth);
        let ns = self.current_nodes_mut();
        for (offset, child) in children.into_iter().enumerate() { ns.insert(idx + 1 + offset, child); }
        self.update_preview();
    }

    pub fn collapse_node(&mut self, idx: usize) {
        let ns = self.current_nodes_mut();
        if let Some(node) = ns.get_mut(idx) {
            node.expanded = false;
            let pd = node.depth;
            let mut rc = 0;
            for i in (idx + 1)..ns.len() { if ns[i].depth > pd { rc += 1; } else { break; } }
            ns.drain(idx + 1..idx + 1 + rc);
        }
        self.update_preview();
    }

    pub fn expand_selected(&mut self) {
        let i = self.state.selected().unwrap_or(0);
        let is_dir = self.current_nodes().get(i).map(|n| n.is_dir).unwrap_or(false);
        if is_dir {
            if let Some(node) = self.current_nodes().get(i) {
                if node.expanded { return; }
            }
            self.expand_node(i);
        }
    }

    pub fn collapse_selected(&mut self) {
        let i = self.state.selected().unwrap_or(0);
        let Some(n) = self.current_nodes().get(i).cloned() else { return };
        if n.is_dir { self.collapse_node(i); return; }
        if n.depth == 0 { return; }
        // Left on a file → collapse its parent and land on it
        let ns = self.current_nodes();
        let mut target = None;
        for (idx, m) in ns.iter().enumerate().take(i) {
            if m.depth == n.depth - 1 && n.path.starts_with(&m.path) { target = Some(idx); }
        }
        if let Some(t) = target { self.collapse_node(t); self.state.select(Some(t)); self.update_preview(); }
    }

    // ---------- navigation ----------

    pub fn next(&mut self) { let n = self.current_nodes().len(); if n == 0 { return; } let i = self.state.selected().map_or(0, |i| (i + 1).min(n - 1)); self.state.select(Some(i)); self.update_preview(); }
    pub fn previous(&mut self) { let n = self.current_nodes().len(); if n == 0 { return; } let i = self.state.selected().map_or(0, |i| i.saturating_sub(1)); self.state.select(Some(i)); self.update_preview(); }
    pub fn jump_top(&mut self) { if !self.current_nodes().is_empty() { self.state.select(Some(0)); self.update_preview(); } }
    pub fn jump_bot(&mut self) { let n = self.current_nodes().len(); if n > 0 { self.state.select(Some(n - 1)); self.update_preview(); } }
    pub fn jump_mid(&mut self) { let n = self.current_nodes().len(); if n > 0 { self.state.select(Some(n / 2)); self.update_preview(); } }
    pub fn half_down(&mut self) { let n = self.current_nodes().len(); if n == 0 { return; } let i = self.state.selected().unwrap_or(0).saturating_add(10).min(n - 1); self.state.select(Some(i)); self.update_preview(); }
    pub fn half_up(&mut self) { let n = self.current_nodes().len(); if n == 0 { return; } let i = self.state.selected().unwrap_or(0).saturating_sub(10); self.state.select(Some(i)); self.update_preview(); }

    // ---------- preview ----------

    pub fn update_preview(&mut self) {
        if self.preview_pinned { return; }
        self.preview_text.clear();
        let Some(node) = self.selected_node() else { return };
        let target = std::fs::canonicalize(&node.path).unwrap_or_else(|_| node.path.clone());
        self.last_file_mtime = std::fs::metadata(&target).ok().and_then(|m| m.modified().ok());
        self.preview_symlink_info(&node); self.preview_notes_and_tags(&node);
        if target.is_dir() { self.preview_directory(&target); return; }
        self.preview_file(&target);
    }

    fn preview_symlink_info(&mut self, node: &Node) {
        if !self.state_data.settings.show_symlinks_in_list { return; }
        if node.is_symlink {
            if let Ok(target) = std::fs::read_link(&node.path) {
                self.preview_text.extend(format!("\x1b[1;35m\u{f481}  Symlink -> {}\x1b[0m\n\x1b[90m────────────────\x1b[0m\n", target.display()).as_bytes());
            }
        }
    }

    fn preview_notes_and_tags(&mut self, node: &Node) {
        let Ok(rel) = node.path.strip_prefix(&self.confy_dir) else { return };
        let rs = ops::path_to_string(&rel);
        if let Some(note) = self.state_data.notes.get(&rs) {
            self.preview_text.extend(format!("\x1b[1;33m📝  Note: {}\x1b[0m\n\x1b[90m────────────────\x1b[0m\n", note).as_bytes());
        }
        if let Some(tags) = self.state_data.tags.get(&rs) {
            if !tags.is_empty() {
                let ts = tags.iter().cloned().collect::<Vec<_>>().join(", ");
                self.preview_text.extend(format!("\x1b[1;34m🏷️  Tags: {}\x1b[0m\n\x1b[90m────────────────\x1b[0m\n", ts).as_bytes());
            }
        }
    }

    fn preview_directory(&mut self, target: &Path) {
        let mut output = "\x1b[1;36m\u{f115}  Directory Contents\x1b[0m\n\x1b[90m────────────────\x1b[0m\n".to_string();
        if let Ok(entries) = std::fs::read_dir(target) {
            let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            paths.sort_by_key(|p| (!p.is_dir(), p.file_name().unwrap_or_default().to_os_string()));
            for p in paths {
                let name = ops::path_name(&p);
                let icon = if p.is_dir() { "\u{f115}" } else { ops::get_icon(&p) };
                output.push_str(&format!("{} {}\n", icon, name));
            }
        }
        self.preview_text.extend(output.as_bytes());
    }

    fn preview_file(&mut self, target: &Path) {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let p_w = ((cols as f32) * 0.6).max(10.0) as u16;
        let p_h = ((rows as f32) * 0.7).max(4.0) as u16;
        let size_arg = format!("{}x{}", p_w, p_h);
        let content = match ops::detect_kind(target) {
            ops::PreviewKind::Image => self.preview_image(target, &size_arg),
            ops::PreviewKind::Video => self.preview_video(target, &size_arg),
            ops::PreviewKind::Text => self.preview_text_file(target),
        };
        self.preview_text.extend(content);
    }

    fn preview_image(&self, target: &Path, size_arg: &str) -> Vec<u8> {
        let out = Command::new("chafa").arg("--format=symbols").arg("--symbols=solid").arg("--view-size").arg(size_arg)
            .arg(target).stdout(Stdio::piped()).stderr(Stdio::null()).output();
        let mut output = format!("\x1b[1;36m\u{f1c5}  Image\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\x1b[33m{}\x1b[0m\n", ops::get_file_info(target));
        if let Ok(out) = out {
            if out.status.success() && !out.stdout.is_empty() {
                output.push_str("\n\x1b[90m─── Preview ───\x1b[0m\n");
                output.push_str(&String::from_utf8_lossy(&out.stdout));
            }
        }
        output.into_bytes()
    }

    fn preview_video(&self, target: &Path, size_arg: &str) -> Vec<u8> {
        let info = ops::get_video_meta(target);
        let mut output = format!("\x1b[1;36m\u{f03d}  Video\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\x1b[33m{}\x1b[0m\n", info);
        if let Some(thumb) = ops::extract_video_thumbnail_cached(target) {
            let out = Command::new("chafa").arg("--format=symbols").arg("--symbols=solid").arg("--view-size").arg(size_arg)
                .arg(&thumb).stdout(Stdio::piped()).stderr(Stdio::null()).output();
            if let Ok(out) = out {
                if out.status.success() && !out.stdout.is_empty() {
                    output.push_str("\n\x1b[90m─── Thumbnail ───\x1b[0m\n");
                    output.push_str(&String::from_utf8_lossy(&out.stdout));
                }
            }
        }
        output.into_bytes()
    }

    fn preview_text_file(&self, target: &Path) -> Vec<u8> {
        let out = Command::new("bat").arg("--color=always").arg("--style=plain").arg("--paging=never").arg("--theme=ansi")
            .arg(target).stdout(Stdio::piped()).stderr(Stdio::piped()).output();
        if let Ok(out) = out {
            let so = String::from_utf8_lossy(&out.stdout);
            let se = String::from_utf8_lossy(&out.stderr);
            if so.contains("[bat warning]") || se.contains("[bat warning]") {
                return "\x1b[1;33m\u{f071}  Binary File\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\n\x1b[90mPreview not available for binary files.\x1b[0m".to_string().into_bytes();
            }
            if out.status.success() { return out.stdout; }
        }
        ops::read_small_fallback(target)
    }

    // ---------- external change detection ----------

    pub fn capture_baselines(&mut self) {
        self.file_baselines.clear();
        for node in &self.nodes {
            if node.is_dir { continue; }
            let sig = std::fs::metadata(&node.path).ok().map(|m| (m.len(), m.modified().ok()));
            self.file_baselines.insert(node.path.clone(), sig.unwrap_or((0, None)));
        }
    }

    /// Detects files changed/deleted outside confy (user edited the real dotfile directly),
    /// auto-saves a version, refreshes, and reports. metadata() follows symlinks, so edits to
    /// the *targets* of our symlinks are caught too.
    pub fn detect_external_changes(&mut self) {
        if self.file_baselines.is_empty() { return; }
        let now = SystemTime::now();
        let mut changed: Vec<String> = Vec::new();
        let keys: Vec<PathBuf> = self.file_baselines.keys().cloned().collect();
        for k in keys {
            let prev = self.file_baselines.get(&k).cloned().unwrap_or((0, None));
            let cur = std::fs::metadata(&k).ok().map(|m| (m.len(), m.modified().ok()));
            match cur {
                None => {
                    if k.symlink_metadata().is_err() && (prev.0 != 0 || prev.1.is_some()) {
                        changed.push(format!("{} (deleted)", ops::path_name(&k)));
                        self.file_baselines.insert(k, (0, None));
                    }
                }
                Some((len, mtime)) => {
                    if len != prev.0 || mtime != prev.1 {
                        // let in-flight writes settle before snapshotting
                        if let Some(t) = mtime {
                            if now.duration_since(t).map(|d| d.as_millis() < 250).unwrap_or(false) { continue; }
                        }
                        changed.push(ops::path_name(&k));
                        self.file_baselines.insert(k, (len, mtime));
                    }
                }
            }
        }
        if changed.is_empty() { return; }
        tracing::info!(?changed, "external changes detected");
        self.take_snapshot_and_cleanup();
        self.refresh_items();
        self.capture_baselines();
        let mut list = changed.join(", ");
        if list.chars().count() > 60 {
            list = changed.iter().take(3).cloned().collect::<Vec<_>>().join(", ") + "…";
        }
        self.set_status(format!("⟳ Changed outside confy: {} — version saved", list));
    }

    pub fn check_fs_changes(&mut self) {
        if let Some(time) = self.last_status_time {
            if !self.status_sticky && SystemTime::now().duration_since(time).unwrap_or_default().as_secs() >= 1 {
                self.status.clear(); self.last_status_time = None;
            }
        }
        let mut need_refresh = false;
        if let Ok(meta) = std::fs::metadata(&self.confy_dir) {
            if let Ok(mtime) = meta.modified() { if Some(mtime) > self.last_dir_mtime { need_refresh = true; } }
        }
        if need_refresh { self.refresh_items(); return; }
        if !self.preview_pinned {
            if let Some(node) = self.selected_node() {
                let target = std::fs::canonicalize(&node.path).unwrap_or_else(|_| node.path.clone());
                if let Ok(meta) = std::fs::metadata(&target) {
                    if let Ok(mtime) = meta.modified() { if Some(mtime) != self.last_file_mtime { self.update_preview(); } }
                }
            }
        }
    }

    // ---------- add symlink / folder ----------

    pub fn begin_add(&mut self) { self.input_mode = InputMode::AddingPath; self.input.clear(); self.add_source_path.clear(); self.status.clear(); }

    pub fn submit_add_step(&mut self) {
        match self.input_mode {
            InputMode::AddingPath => {
                let p = self.input.trim().to_string();
                if p.is_empty() { self.set_status("Empty"); self.input.clear(); return; }
                let source = ops::expand_tilde(&p);
                if !source.exists() { self.set_status("Not found"); self.input.clear(); return; }
                self.add_source_path = ops::path_to_string(&source);
                self.input = ops::path_name(&source); // prefill alias
                self.input_mode = InputMode::AddingAlias;
            }
            InputMode::AddingAlias => {
                let alias = self.input.trim().to_string();
                if alias.is_empty() || alias.starts_with('.') || alias.contains('/') {
                    self.set_status("Invalid alias"); self.input.clear(); return;
                }
                let source = PathBuf::from(&self.add_source_path);
                let dest_dir = match self.selected_node() {
                    Some(node) if node.is_dir => node.path.clone(),
                    Some(node) => node.path.parent().unwrap_or(&self.confy_dir).to_path_buf(),
                    None => self.confy_dir.clone(),
                };
                let dest = dest_dir.join(&alias);
                if !ops::is_path_safe(&self.confy_dir, &dest) { self.set_status("Cannot escape root"); self.input.clear(); return; }
                if ops::path_exists(&dest) { self.set_status("Already exists"); self.input.clear(); return; }
                if let Some(parent) = dest.parent() { if let Err(e) = std::fs::create_dir_all(parent) { self.set_status(format!("{}", e)); return; } }
                match std::os::unix::fs::symlink(&source, &dest) {
                    Ok(_) => {
                        self.set_status(format!("Added: {}", alias));
                        tracing::info!(alias = ?alias, target = ?source, "symlink added");
                        self.input.clear(); self.add_source_path.clear();
                        self.input_mode = InputMode::Normal;
                        self.refresh_items(); self.take_snapshot_and_cleanup();
                    }
                    Err(e) => self.set_status(format!("{}", e)),
                }
            }
            InputMode::AddingNote => {
                let note = self.input.trim().to_string();
                if !self.add_source_path.is_empty() {
                    if note.is_empty() { self.state_data.notes.remove(&self.add_source_path); }
                    else { self.state_data.notes.insert(self.add_source_path.clone(), note); }
                    self.set_status("Note saved.");
                    self.save_state(); self.refresh_items();
                }
                self.input.clear(); self.add_source_path.clear();
                self.input_mode = InputMode::Normal;
            }
            InputMode::AddingTag => {
                let tag = self.input.trim().to_string();
                if !self.add_source_path.is_empty() && !tag.is_empty() {
                    // Add tag and propagate to all parent directories
                    ops::propagate_bookmarks_to_parents(&self.add_source_path, &mut self.state_data.bookmarks);
                    self.state_data.tags.entry(self.add_source_path.clone()).or_insert_with(std::collections::HashSet::new).insert(tag);
                    self.set_status("Tag added (parents marked).");
                    self.save_state(); self.refresh_items();
                }
                self.input.clear(); self.add_source_path.clear();
                self.input_mode = InputMode::Normal;
            }
            _ => {}
        }
    }

    pub fn begin_add_folder(&mut self) { self.input_mode = InputMode::AddingDir; self.input.clear(); self.status.clear(); }

    // ---------- rename ----------

    pub fn begin_rename(&mut self) {
        if let Some(node) = self.selected_node() {
            if self.is_internal_path(&node.path) { self.set_status("Protected: confy internal object."); return; }
            self.add_source_path = node.path.display().to_string();
            self.input = ops::path_name(&node.path);
            self.input_mode = InputMode::Renaming; self.status.clear();
        }
    }

    pub fn submit_rename(&mut self) {
        let new_name = self.input.trim();
        if new_name.is_empty() || new_name == "." || new_name == ".." || new_name.contains('/') || new_name.contains("..") {
            self.set_status("Invalid"); self.input.clear(); return;
        }
        let src = PathBuf::from(&self.add_source_path);
        if !ops::path_exists(&src) { self.set_status("Source missing"); self.cancel_input(); return; }
        let dest = src.parent().unwrap_or(&self.confy_dir).join(new_name);
        if !ops::is_path_safe(&self.confy_dir, &dest) { self.set_status("Must stay inside root"); self.input.clear(); return; }
        if self.is_internal_path(&dest) { self.set_status("Protected target"); self.input.clear(); return; }
        let dest_norm = std::fs::canonicalize(dest.parent().unwrap_or(Path::new("/")))
            .map(|p| p.join(dest.file_name().unwrap_or_default())).unwrap_or(dest);
        if dest_norm == src { self.cancel_input(); return; }
        if ops::path_exists(&dest_norm) { self.set_status("Target exists"); self.input.clear(); return; }
        self.run_hook("pre_move", &src, "move", false);
        match std::fs::rename(&src, &dest_norm) {
            Ok(()) => {
                self.push_undo(UndoAction::Rename { from: src.clone(), to: dest_norm.clone() });
                self.set_status("Renamed."); tracing::info!(from = ?src, to = ?dest_norm, "renamed");
                self.run_hook("post_move", &dest_norm, "move", true);
                self.refresh_items(); self.cancel_input();
            }
            Err(e) => { self.set_status(format!("Rename failed: {}", e)); self.run_hook("post_move", &src, "move", false); self.input.clear(); }
        }
    }

    // ---------- delete ----------

    pub fn request_delete(&mut self) {
        if let Some(node) = self.selected_node() {
            if self.is_internal_path(&node.path) { self.set_status("Protected: confy internal object."); return; }
            if !self.state_data.settings.confirm_delete { self.pending_delete = Some(node.path.clone()); self.execute_delete(); return; }
            if let Some(p) = &self.pending_delete {
                if *p == node.path { self.execute_delete(); return; }
            }
            self.pending_delete = Some(node.path.clone());
            self.set_sticky_status(format!("Delete {}? Press d again to confirm",
                node.path.file_name().unwrap_or_default().to_string_lossy()));
        }
    }

    pub fn execute_delete(&mut self) {
        let Some(path) = self.pending_delete.take() else { return };
        if !ops::path_exists(&path) { self.set_status("Already gone"); return; }
        self.take_snapshot_and_cleanup(); // safety-net version before destructive op
        self.run_hook("pre_delete", &path, "delete", false);
        let mut name = format!("{}_{}", ops::timestamp_now(), path.file_name().unwrap_or_default().to_string_lossy());
        if name.len() > 200 { name.truncate(200); }
        let trash_path = self.trash_dir.join(&name);
        match std::fs::rename(&path, &trash_path) {
            Ok(()) => {
                let _ = ops::save_trash_metadata(&trash_path, &path);
                self.push_undo(UndoAction::Delete { original: path.clone(), trash: trash_path });
                self.set_status("Deleted (u=undo).");
                tracing::info!(?path, "deleted to trash");
                self.refresh_items(); self.capture_baselines();
                self.run_hook("post_delete", &path, "delete", true);
            }
            Err(e) => { self.set_status(format!("Delete failed: {}", e)); self.run_hook("post_delete", &path, "delete", false); }
        }
    }

    // ---------- cut / copy / paste ----------

    fn resolve_paste_dest(&self, src: &Path) -> PathBuf {
        let dest_dir = match self.selected_node() {
            Some(node) if node.is_dir => node.path.clone(),
            Some(node) => node.path.parent().unwrap_or(&self.confy_dir).to_path_buf(),
            None => self.confy_dir.clone(),
        };
        dest_dir.join(src.file_name().unwrap_or_default())
    }

    pub fn cut_item(&mut self) {
        if let Some(node) = self.selected_node() {
            if self.is_internal_path(&node.path) { self.set_status("Protected: confy internal object."); return; }
            self.cut_source = Some(node.path.clone()); self.yank_source = None;
            self.set_sticky_status(format!("Cut: {}. (p=paste, Esc=cancel)", node.path.file_name().unwrap_or_default().to_string_lossy()));
        }
    }

    pub fn yank_item(&mut self) {
        if let Some(node) = self.selected_node() {
            ops::copy_to_clipboard(&node.path.display().to_string());
            self.yank_source = Some(node.path.clone()); self.cut_source = None;
            self.set_sticky_status(format!("Yanked: {} (p=paste, Esc=cancel)", node.path.file_name().unwrap_or_default().to_string_lossy()));
        }
    }

    pub fn paste_item(&mut self) {
        let Some(src) = self.cut_source.clone().or_else(|| self.yank_source.clone()) else { return };
        let is_cut = self.cut_source.is_some();
        
        // Get cached symlink info from nodes if available, otherwise check filesystem
        let is_sym = self.nodes.iter()
            .find(|n| n.path == src)
            .map(|n| n.is_symlink)
            .unwrap_or_else(|| src.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false));
        
        let src_abs = std::fs::canonicalize(&src).unwrap_or_else(|_| src.clone());
        let dest_path = self.resolve_paste_dest(&src);
        let dest_dir = dest_path.parent().unwrap_or(&self.confy_dir).to_path_buf();
        let dest_dir_abs = std::fs::canonicalize(&dest_dir).unwrap_or_else(|_| dest_dir.clone());
        if dest_dir_abs.starts_with(&src_abs) {
            self.set_status("Cannot paste inside itself"); self.cut_source = None; self.yank_source = None; return;
        }
        let dest_canon = std::fs::canonicalize(&dest_path).unwrap_or_else(|_| dest_path.clone());
        if dest_canon == src_abs {
            self.set_status("Source and destination are the same"); self.cut_source = None; self.yank_source = None; return;
        }
        if ops::path_exists(&dest_path) {
            if self.state_data.settings.confirm_moves { self.set_status("Paste failed: target exists (Confirm Moves is ON)"); return; }
            let _ = ops::remove_entry(&dest_path);
        }
        self.run_hook("pre_move", &src, "move", false);
        let result = if is_cut { std::fs::rename(&src, &dest_path) }
            else if is_sym { std::fs::read_link(&src).and_then(|t| std::os::unix::fs::symlink(&t, &dest_path)) }
            else if src.is_dir() { ops::copy_dir_recursive(&src, &dest_path) }
            else { std::fs::copy(&src, &dest_path).map(|_| ()) };
        match result {
            Ok(()) => {
                self.set_status(format!("Pasted to {}", dest_dir.display()));
                tracing::info!(from = ?src, to = ?dest_path, "pasted");
                if is_cut { self.push_undo(UndoAction::Move { from: src, to: dest_path.clone() }); }
                self.cut_source = None; self.yank_source = None;
                self.refresh_items(); self.capture_baselines();
                self.run_hook("post_move", &dest_path, "move", true);
            }
            Err(e) => { self.set_status(format!("Paste failed: {}", e)); self.run_hook("post_move", &src, "move", false); }
        }
    }

    // ---------- chmod / quickmove ----------

    pub fn begin_chmod(&mut self) {
        if let Some(node) = self.selected_node() {
            if self.is_internal_path(&node.path) { self.set_status("Protected: confy internal object."); return; }
            self.add_source_path = node.path.display().to_string();
            self.input.clear(); self.input_mode = InputMode::ChmodPrompt;
        }
    }

    pub fn submit_chmod(&mut self) {
        let mode = self.input.trim();
        if !mode.is_empty() {
            // reject anything but sane chmod tokens
            let ok_chars = mode.chars().all(|c| c.is_ascii_digit() || "+-=rwxugoasts,".contains(c));
            if !ok_chars || mode.len() > 8 { self.set_status("Invalid mode"); self.cancel_input(); return; }
            match Command::new("chmod").arg(mode).arg(&self.add_source_path).status() {
                Ok(s) if s.success() => self.set_status(format!("Permissions changed to {}", mode)),
                _ => self.set_status("Failed to change permissions"),
            }
        }
        self.cancel_input(); self.refresh_items();
    }

    pub fn begin_quick_move(&mut self) {
        if let Some(node) = self.selected_node() {
            if self.is_internal_path(&node.path) { self.set_status("Protected: confy internal object."); return; }
            self.add_source_path = node.path.display().to_string();
            self.input.clear(); self.input_mode = InputMode::QuickMove;
        }
    }

    pub fn submit_quick_move(&mut self) {
        let dest = self.input.trim().to_string();
        if !dest.is_empty() {
            let dest_path = ops::expand_tilde(&dest);
            if dest_path.starts_with(self.confy_dir.join(".assets")) { self.set_status("Cannot move into .assets"); self.cancel_input(); return; }
            // fs::rename silently overwrites on unix — refuse existing targets
            if ops::path_exists(&dest_path) { self.set_status("Target exists — refusing to overwrite"); self.cancel_input(); return; }
            let src = PathBuf::from(&self.add_source_path);
            self.run_hook("pre_move", &src, "move", false);
            match std::fs::rename(&src, &dest_path) {
                Ok(()) => {
                    self.push_undo(UndoAction::Move { from: src.clone(), to: dest_path.clone() });
                    self.set_status(format!("Moved to {}", dest_path.display()));
                    tracing::info!(from = ?src, to = ?dest_path, "quick move");
                    self.refresh_items(); self.run_hook("post_move", &dest_path, "move", true);
                }
                Err(e) => { self.set_status(format!("Move failed: {}", e)); self.run_hook("post_move", &src, "move", false); }
            }
        }
        self.cancel_input(); self.capture_baselines();
    }

    // ---------- filters ----------

    pub fn toggle_host_filter(&mut self) {
        self.host_filter = !self.host_filter;
        if self.host_filter { self.jump_list = false; }
        self.refresh_items();
        self.set_status(if self.host_filter { "Host filter enabled." } else { "Host filter disabled." });
    }

    pub fn toggle_jump_list(&mut self) {
        self.jump_list = !self.jump_list;
        if self.jump_list { self.bookmarks_only = false; }
        self.refresh_items();
        self.set_status(if self.jump_list { "Jump list (recent + bookmarks). Esc to exit." } else { "Jump list off." });
    }

    pub fn toggle_selection(&mut self) {
        if let Some(n) = self.selected_node() {
            let p = n.path.clone();
            if self.selected_nodes.contains(&p) { self.selected_nodes.remove(&p); } else { self.selected_nodes.insert(p); }
            self.refresh_items();
        }
    }

    // ---------- services ----------

    pub fn enter_services_menu(&mut self) {
        self.available_services = ops::list_services(self.service_user_scope);
        if self.available_services.is_empty() { self.set_status("No services found or systemctl unavailable."); return; }
        self.service_state.select(Some(0));
        self.input_mode = InputMode::ServicesMenu;
    }

    pub fn switch_service_scope(&mut self, user: bool) {
        self.service_user_scope = user;
        self.available_services = ops::list_services(user);
        if self.available_services.is_empty() { self.set_status("No services found."); self.input_mode = InputMode::Normal; return; }
        self.service_state.select(Some(0));
    }

    pub fn filtered_services(&self) -> Vec<(String, String, String)> {
        if self.service_filter.is_empty() { return self.available_services.clone(); }
        let query = self.service_filter.to_lowercase();
        self.available_services.iter().filter(|(name, _, desc)| name.to_lowercase().contains(&query) || desc.to_lowercase().contains(&query)).cloned().collect()
    }

    pub fn begin_add_service(&mut self) {
        self.service_name_input.clear();
        self.input.clear();
        self.input_mode = InputMode::SettingsInput;
        self.return_mode = InputMode::ServicesMenu;
        self.settings_input_target = "service_name".into();
        self.set_sticky_status("Enter unit name, e.g. example.service or backup.timer");
    }

    pub fn add_service_job(&mut self) {
        let name = self.input.trim().to_string();
        if name.is_empty() || name.contains('/') || !(name.ends_with(".service") || name.ends_with(".timer")) {
            self.set_status("Name must end in .service or .timer and contain no '/'");
            return;
        }
        let editor = self.selected_editor.clone().unwrap_or_else(|| "vi".into());
        let path = ops::service_unit_path(&name, self.service_user_scope);
        if let Some(parent) = path.parent() { if let Err(e) = std::fs::create_dir_all(parent) { self.set_status(format!("Cannot create unit directory: {}", e)); return; } }
        if path.exists() { self.set_status("That unit already exists"); return; }
        if !ops::open_editor(&path, &editor) { self.set_status("Editor failed; unit was not reloaded"); return; }
        let ok = ops::daemon_reload(self.service_user_scope);
        self.available_services = ops::list_services(self.service_user_scope);
        self.input.clear();
        self.input_mode = InputMode::ServicesMenu;
        self.set_status(if ok { format!("Added and reloaded {}", name) } else { format!("Added {}; daemon reload failed", name) });
    }

    pub fn restart_selected_service(&mut self) {
        if let Some(i) = self.service_state.selected() {
            if let Some((name, _, _)) = self.available_services.get(i) {
                let ok = ops::restart_service(name, self.service_user_scope);
                self.set_status(if ok { format!("Restarted {}", name) } else { format!("Failed to restart {}", name) });
                self.available_services = ops::list_services(self.service_user_scope);
            }
        }
    }

    pub fn stop_selected_service(&mut self) {
        if let Some(i) = self.service_state.selected() {
            if let Some((name, _, _)) = self.available_services.get(i) {
                let ok = ops::stop_service(name, self.service_user_scope);
                self.set_status(if ok { format!("Stopped {}", name) } else { format!("Failed to stop {}", name) });
                self.available_services = ops::list_services(self.service_user_scope);
            }
        }
    }

    pub fn delete_selected_service(&mut self) {
        if let Some(i) = self.service_state.selected() {
            if let Some((name, _, _)) = self.available_services.get(i).cloned() {
                let path = ops::service_unit_path(&name, self.service_user_scope);
                match std::fs::remove_file(&path) {
                    Ok(_) => { let _ = ops::daemon_reload(self.service_user_scope); self.available_services = ops::list_services(self.service_user_scope); self.set_status(format!("Deleted {}", name)); }
                    Err(e) => self.set_status(format!("Delete failed: {}", e)),
                }
            }
        }
    }

    // ---------- archive & deploy ----------

    pub fn archive_selected(&mut self) {
        if self.selected_nodes.is_empty() { self.set_status("Nothing selected (Space to select)"); return; }
        let files: Vec<(String, PathBuf)> = self.selected_nodes.iter().cloned()
            .filter_map(|p| {
                let alias = p.strip_prefix(&self.confy_dir).ok().map(|x| ops::path_to_string(&x)).unwrap_or_default();
                Some((alias, p))
            })
            .filter(|(_, p)| !self.is_internal_path(p))
            .collect();
        if files.is_empty() { self.set_status("Nothing to archive"); return; }
        let mut hd: HooksData = HashMap::new();
        for (alias, _) in &files {
            if let Some(m) = self.state_data.object_hooks.get(alias) {
                if !m.is_empty() { hd.insert(alias.clone(), m.clone()); }
            }
        }
        self.run_hook("pre_archive", &self.confy_dir, "archive", false);
        match ops::create_archive(&self.confy_dir, &files, None, &hd) {
            Ok(ap) => { self.set_status(format!("Archived → {}", ap.display())); self.run_hook("post_archive", &ap, "archive", true); }
            Err(e) => { self.set_status(format!("Archive failed: {}", e)); self.run_hook("post_archive", &self.confy_dir, "archive", false); }
        }
        self.refresh_items();
    }

    pub fn start_deploy(&mut self) {
        let Some(node) = self.selected_node() else { return };
        if node.is_dir { self.set_status("Deploy works on .zip files"); return; }
        let name = ops::path_name(&node.path);
        if !name.ends_with(".zip") { self.set_status("Not a .zip archive"); return; }
        self.deploy_target = Some(node.path.clone());
        match ops::deploy_archive(&self.confy_dir, &node.path, false, None, None, false, &self.state_data.settings) {
            Ok(sum) => {
                self.preview_pinned = true;
                self.preview_text = sum.output.clone().into_bytes();
                // auto-apply only when apply-mode is on AND nothing would be overwritten
                if self.state_data.settings.default_deploy_mode.is_apply() && sum.updated == 0 && sum.conflicts == 0 {
                    self.set_status("No overwrites in plan — auto-applying (Default Deploy = apply)");
                    self.apply_deploy();
                } else {
                    self.input_mode = InputMode::DeployConfirm;
                    self.set_sticky_status("Deploy: e=browse, y=apply, n=cancel");
                }
            }
            Err(e) => { self.set_status(format!("Deploy failed: {}", e)); self.deploy_target = None; }
        }
    }

    pub fn apply_deploy(&mut self) {
        let Some(t) = self.deploy_target.clone() else { return };
        self.run_hook("pre_deploy", &t, "deploy", false);
        match ops::deploy_archive(&self.confy_dir, &t, true, None, None, false, &self.state_data.settings) {
            Ok(sum) => {
                self.preview_pinned = true;
                self.preview_text = sum.output.clone().into_bytes();
                self.set_status(if sum.conflicts > 0 {
                    format!("Deployed with {} conflict(s) — markers + .ours/.theirs saved", sum.conflicts)
                } else { "Deployed.".into() });
                self.run_hook("post_deploy", &t, "deploy", true);
            }
            Err(e) => { self.set_status(format!("Deploy failed: {}", e)); self.run_hook("post_deploy", &t, "deploy", false); }
        }
        self.input_mode = InputMode::Normal;
        self.deploy_target = None;
        self.deploy_index = ops::load_deploy_index(&self.assets_dir.join(".deploy_index.json"));
        self.refresh_items(); self.capture_baselines();
    }

    pub fn browse_archive(&mut self) {
        if let Some(t) = self.deploy_target.clone() {
            self.preview_pinned = true;
            self.preview_text = ops::list_archive_contents(&t).into_bytes();
        }
    }

    // ---------- edit entry point ----------

    pub fn edit_selected(&mut self) {
        let Some(node) = self.selected_node() else { return };
        let path = node.path.clone();
        let Some(ed) = self.selected_editor.clone() else { self.enter_editor_select(Some(path)); return; };
        self.execute_edit(&path, &ed);
    }

    // ---------- settings ----------

    pub fn execute_setting(&mut self, action: SettingAction) {
        match action {
            SettingAction::CycleTheme => {
                let names = self.theme_names();
                let current_name = self.state_data.settings.theme_name.clone();
                let new_name = match names.iter().position(|n| n == &current_name) {
                    Some(i) => names[(i + 1) % names.len()].clone(),
                    None => names[0].clone(),
                };
                self.state_data.settings.theme_name = new_name;
                self.theme_dirty = true;
            }
            SettingAction::AddCustomTheme => { self.return_mode = InputMode::Settings; self.input_mode = InputMode::AddingThemePath; self.input.clear(); }
            SettingAction::CycleColorMode => { self.state_data.settings.color_mode = self.state_data.settings.color_mode.next(); self.theme_dirty = true; }
            SettingAction::ToggleFolders => self.state_data.settings.folders_first = !self.state_data.settings.folders_first,
            SettingAction::ToggleSizes => self.state_data.settings.show_sizes = !self.state_data.settings.show_sizes,
            SettingAction::ToggleHidden => { self.state_data.settings.show_hidden_by_default = !self.state_data.settings.show_hidden_by_default; self.show_hidden = self.state_data.settings.show_hidden_by_default; }
            SettingAction::ToggleSymlinks => self.state_data.settings.show_symlinks_in_list = !self.state_data.settings.show_symlinks_in_list,
            SettingAction::InputListWidth => { self.settings_input_target = "list_width".into(); self.input = self.state_data.settings.list_width.to_string(); self.return_mode = InputMode::Settings; self.input_mode = InputMode::SettingsInput; }
            SettingAction::SelectEditor => { self.return_mode = InputMode::Settings; self.enter_editor_select(None); }
            SettingAction::ToggleVersioning => self.state_data.settings.enable_versioning = !self.state_data.settings.enable_versioning,
            SettingAction::InputVersionLimit => { self.settings_input_target = "version_limit".into(); self.input = self.state_data.settings.version_limit.to_string(); self.return_mode = InputMode::Settings; self.input_mode = InputMode::SettingsInput; }
            SettingAction::CycleDeployMode => self.state_data.settings.default_deploy_mode = self.state_data.settings.default_deploy_mode.next(),
            SettingAction::CycleBackupBehavior => self.state_data.settings.backup_behavior = self.state_data.settings.backup_behavior.next(),
            SettingAction::CycleOverwrite => self.state_data.settings.confirm_overwrite_deploy = self.state_data.settings.confirm_overwrite_deploy.next(),
            SettingAction::ToggleConfirmDel => self.state_data.settings.confirm_delete = !self.state_data.settings.confirm_delete,
            SettingAction::ToggleConfirmMove => self.state_data.settings.confirm_moves = !self.state_data.settings.confirm_moves,
            SettingAction::ToggleMouse => self.state_data.settings.enable_mouse = !self.state_data.settings.enable_mouse,
            SettingAction::CycleSearchMode => self.state_data.settings.search_mode = self.state_data.settings.search_mode.next(),
            SettingAction::ToggleHooks => self.state_data.settings.enable_hooks = !self.state_data.settings.enable_hooks,
            SettingAction::ToggleTerminalWindow => self.state_data.settings.open_terminal_in_new_window = !self.state_data.settings.open_terminal_in_new_window,
            SettingAction::EditGlobalHooks => { self.return_mode = InputMode::Settings; self.enter_global_hooks(); }
            SettingAction::EditKeybinds => { self.return_mode = InputMode::Settings; self.keybind_state.select(Some(0)); self.input_mode = InputMode::KeybindMenu; }
            SettingAction::OpenTrash => { self.return_mode = InputMode::Settings; self.reload_trash_items(); self.input_mode = InputMode::TrashManagement; }
            SettingAction::CycleTrashRetention => {
                let opts = [0u32, 7, 30, 90, 365];
                let cur = opts.iter().position(|&d| d == self.state_data.settings.trash_retention_days).unwrap_or(2);
                self.state_data.settings.trash_retention_days = opts[(cur + 1) % opts.len()];
            }
        }
        self.save_state();
        if matches!(action, SettingAction::ToggleFolders | SettingAction::ToggleHidden) { self.refresh_items(); }
    }

    pub fn submit_settings_input(&mut self) {
        let v = self.input.trim().to_string();
        match self.settings_input_target.as_str() {
            "list_width" => match v.parse::<u16>() {
                Ok(w) if w >= 10 => { self.state_data.settings.list_width = w; self.save_state(); self.set_status("Saved."); }
                _ => self.set_status("Invalid width (number >= 10)"),
            },
            "version_limit" => match v.parse::<usize>() {
                Ok(n) if n > 0 && n <= 100 => { self.state_data.settings.version_limit = n; self.save_state(); self.set_status("Saved."); }
                _ => self.set_status("Invalid limit (1-100)"),
            },
            _ => {}
        }
        self.input.clear(); self.input_mode = InputMode::Settings;
    }

    // ---------- hooks menu ----------

    pub fn enter_global_hooks(&mut self) { self.hooks_target_path = None; self.hooks_state.select(Some(0)); self.input_mode = InputMode::HooksMenu; }

    pub fn enter_object_hooks(&mut self) {
        let Some(node) = self.selected_node() else { return };
        if self.is_internal_path(&node.path) { self.set_status("Protected."); return; }
        if let Ok(rel) = node.path.strip_prefix(&self.confy_dir) {
            self.hooks_target_path = Some(ops::path_to_string(&rel));
            self.hooks_state.select(Some(0));
            self.input_mode = InputMode::HooksMenu;
        }
    }

    pub fn current_hook_value(&self, name: &str) -> String {
        if let Some(rel) = &self.hooks_target_path {
            self.state_data.object_hooks.get(rel).and_then(|m| m.get(name)).cloned().unwrap_or_default()
        } else { self.hooks.get(name).cloned().unwrap_or_default() }
    }

    pub fn set_hook_value(&mut self, value: String) {
        let name = self.hook_edit_type.clone();
        if let Some(rel) = self.hooks_target_path.clone() {
            let m = self.state_data.object_hooks.entry(rel).or_default();
            if value.trim().is_empty() { m.remove(&name); } else { m.insert(name, value); }
            self.save_state();
        } else {
            if value.trim().is_empty() { self.hooks.remove(&name); } else { self.hooks.insert(name, value); }
            self.save_hooks();
        }
        self.input.clear();
        self.input_mode = InputMode::HooksMenu;
    }

    pub fn test_hook_value(&mut self, name: &str) {
        let v = self.current_hook_value(name);
        if v.trim().is_empty() { self.set_status("No hook set."); return; }
        self.test_hook(&v);
    }

    pub fn clear_hook_value(&mut self, name: &str) {
        if let Some(rel) = self.hooks_target_path.clone() {
            if let Some(m) = self.state_data.object_hooks.get_mut(&rel) { m.remove(name); }
            self.save_state();
        } else { self.hooks.remove(name); self.save_hooks(); }
        self.set_status(format!("Cleared {}", name));
    }

    // ---------- roots ----------

    pub fn add_root(&mut self) {
        let p = ops::expand_tilde(self.input.trim());
        if !p.is_dir() { self.set_status("Not a directory."); self.input.clear(); return; }
        if !self.roots.contains(&p) { self.roots.push(p); self.save_roots(); }
        self.input.clear(); self.input_mode = InputMode::RootMenu;
        let n = self.roots.len();
        self.root_state.select(Some(n.saturating_sub(1)));
        self.set_status("Root added.");
    }

    pub fn remove_root(&mut self) {
        let i = self.root_state.selected().unwrap_or(0);
        if let Some(r) = self.roots.get(i).cloned() {
            if r == self.confy_dir { self.set_status("Cannot remove current root."); return; }
            self.roots.remove(i);
            self.save_roots();
            let n = self.roots.len();
            if n > 0 { self.root_state.select(Some(i.min(n - 1))); }
            self.set_status("Root removed.");
        }
    }

    pub fn switch_root(&mut self, new_dir: PathBuf) -> bool {
        if !new_dir.is_dir() { self.set_status("Not a directory."); return false; }
        self.confy_dir = new_dir.clone();
        self.assets_dir = new_dir.join(".assets");
        self.history_dir = self.assets_dir.join(".history");
        self.trash_dir = self.assets_dir.join(".trash");
        let _ = std::fs::create_dir_all(&self.assets_dir);
        let _ = std::fs::create_dir_all(&self.history_dir);
        let _ = std::fs::create_dir_all(&self.trash_dir);
        self.state_data = ConfyState::load(&self.assets_dir.join(".state.json"));
        self.hooks = crate::config::load_hooks(&self.assets_dir.join(".hooks.json"));
        let km = crate::config::load_keymap(&self.assets_dir.join(".keybinds.json"));
        self.keymap = km.map; self.shell_aliases = km.aliases;
        self.deploy_index = ops::load_deploy_index(&self.assets_dir.join(".deploy_index.json"));
        self.selected_editor = self.state_data.settings.default_editor.clone();
        let mut roots = load_roots(&self.assets_dir.join(".roots.json"));
        if !roots.contains(&new_dir) { roots.push(new_dir.clone()); save_roots(&roots, &self.assets_dir.join(".roots.json")).ok(); }
        self.roots = roots;
        self.show_hidden = self.state_data.settings.show_hidden_by_default;
        self.undo_stack.clear(); self.selected_nodes.clear(); self.modified_cache.clear();
        self.cut_source = None; self.yank_source = None; self.pending_delete = None; self.pending_reset = None;
        self.bookmarks_only = false; self.host_filter = false; self.jump_list = false;
        self.input_mode = InputMode::Normal; self.return_mode = InputMode::Normal;
        self.preview_pinned = false; self.theme_dirty = true;
        self.refresh_items(); self.capture_baselines();
        tracing::info!(root = ?new_dir, "switched root");
        true
    }

    // ---------- themes ----------

    pub fn submit_theme_path(&mut self) {
        let p = ops::expand_tilde(self.input.trim());
        match crate::config::ThemeColors::from_file(&p) {
            Some(tc) => { self.pending_theme_colors = Some(tc); self.pending_theme_path = self.input.clone(); self.input.clear(); self.input_mode = InputMode::AddingThemeName; }
            None => { self.set_status("Couldn't parse colors from that file"); self.input.clear(); }
        }
    }

    pub fn submit_theme_name(&mut self) {
        let name = self.input.trim().to_string();
        if name.is_empty() { self.set_status("Empty name"); self.input.clear(); return; }
        if let Some(tc) = self.pending_theme_colors.take() {
            self.state_data.settings.custom_themes.insert(name.clone(), tc);
            self.state_data.settings.theme_name = name.clone();
            self.save_state(); self.theme_dirty = true;
            self.set_status(format!("Theme '{}' saved & applied", name));
        }
        self.input.clear();
        self.return_from_menu();
    }

    // ---------- shell, aliases, custom binds ----------

    pub fn expand_shell_input(&self, cmd: &str, file: Option<&Path>) -> String {
        let file_str = file.map(|f| f.display().to_string()).unwrap_or_default();
        let mut out: Vec<String> = Vec::new();
        for (i, tok) in cmd.split_whitespace().enumerate() {
            let mut t = tok;
            if let Some(name) = tok.strip_prefix('!') {
                if !name.is_empty() {
                    if let Some(real) = self.shell_aliases.get(&format!("!{}", name)) {
                        out.push(if file_str.is_empty() { real.clone() } else { real.replace("{f}", &file_str) });
                        continue;
                    }
                    if i == 0 { t = name; } // leading "!raw-cmd" behaves like hook syntax
                }
            }
            out.push(if file_str.is_empty() { t.to_string() } else { t.replace("{f}", &file_str) });
        }
        out.join(" ")
    }

    pub fn run_shell_input(&mut self, timeout: std::time::Duration) {
        let _ = timeout;
        let raw = self.input.clone();
        if raw.trim().is_empty() { self.input_mode = InputMode::Normal; return; }
        let file = self.selected_node().map(|n| n.path);
        let cmd = self.expand_shell_input(&raw, file.as_deref());
        if cmd.trim().is_empty() { self.input.clear(); return; }
        if ops::is_command_dangerous(&cmd) {
            self.set_sticky_status("Blocked: command looks destructive."); self.input.clear(); return;
        }
        let command_result = std::process::Command::new("sh").arg("-lc").arg(&cmd).output();
        self.needs_clear = true;
        match command_result {
            Ok(out) => {
                let mut rendered = String::new();
                if !out.stdout.is_empty() { rendered.push_str(&String::from_utf8_lossy(&out.stdout)); }
                if !out.stderr.is_empty() { rendered.push_str(&String::from_utf8_lossy(&out.stderr)); }
                if rendered.trim().is_empty() && out.status.success() { rendered = "Command exited successfully.".to_string(); }
                self.preview_pinned = true;
                self.preview_text = rendered.into_bytes();
                self.input.clear();
                self.input_mode = InputMode::Shell;
                self.set_status(if out.status.success() { "Command OK".into() } else { format!("Exit {}", out.status.code().unwrap_or(-1)) });
            }
            Err(err) => {
                self.preview_pinned = true;
                self.preview_text = format!("Command failed: {}", err).into_bytes();
                self.input.clear();
                self.input_mode = InputMode::Shell;
                self.set_status("Command failed.");
            }
        }
        self.refresh_items(); self.capture_baselines();
    }

    pub fn toggle_preview_terminal(&mut self) {
        if self.state_data.settings.open_terminal_in_new_window {
            let mut launched = false;
            for term in ["alacritty", "kitty", "gnome-terminal", "konsole", "xterm", "wezterm", "foot"] {
                if !ops::command_exists(term) { continue; }
                let mut cmd = std::process::Command::new(term);
                match term {
                    "alacritty" => { cmd.arg("-e").arg("bash").arg("-lc"); }
                    "kitty" => { cmd.arg("bash").arg("-lc"); }
                    "gnome-terminal" => { cmd.arg("--").arg("bash").arg("-lc"); }
                    "konsole" => { cmd.arg("-e").arg("bash").arg("-lc"); }
                    "xterm" => { cmd.arg("-e").arg("bash").arg("-lc"); }
                    "wezterm" => { cmd.arg("start").arg("--").arg("bash").arg("-lc"); }
                    "foot" => { cmd.arg("bash").arg("-lc"); }
                    _ => {}
                }
                let _ = cmd.spawn();
                launched = true;
                break;
            }
            if launched { self.set_status("Opened terminal window."); } else { self.set_status("No terminal emulator found."); }
            return;
        }
        if self.input_mode == InputMode::Shell {
            self.input_mode = InputMode::Normal;
            self.preview_pinned = false;
            self.input.clear();
            self.update_preview();
            self.set_status("Preview restored.");
            return;
        }
        self.input_mode = InputMode::Shell;
        self.preview_pinned = true;
        self.input.clear();
        self.preview_text = b"Konfi preview shell\nType a command and press Enter.\nType exit to return to the normal preview.\n".to_vec();
        self.set_status("Terminal preview active.");
    }

    pub fn open_key_picker(&mut self) {
        let store = crate::secrets::load_key_store(&self.confy_dir);
        self.key_picker_group = false;
        self.key_picker_state.select(Some(0));
        self.input_mode = InputMode::KeyPicker;
        self.set_status(if store.shared.is_empty() && store.generated.is_empty() { "No saved keys." } else { "Use s/g to switch list." });
    }

    pub fn run_custom_bind(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() { return; }
        if cmd == "keys" { self.open_key_picker(); return; }
        if let Some(path) = cmd.strip_prefix("edit:") {
            let p = ops::expand_tilde(path);
            if p.exists() {
                let ed = self.selected_editor.clone().unwrap_or_else(|| "vi".into());
                let np = self.selected_node().map(|n| n.path).unwrap_or_else(|| self.confy_dir.clone());
                self.add_to_recent(&np);
                let (ok, _) = self.edit_with_live_hooks(&p, &ed, None);
                self.needs_clear = true;
                self.take_snapshot_and_cleanup();
                self.set_status(if ok { "Opened." } else { "Editor failed." });
                return;
            }
            self.set_status(format!("Path not found: {}", path));
            return;
        }
        let file = self.selected_node().map(|n| n.path);
        let full = self.expand_shell_input(cmd, file.as_deref());
        if ops::is_command_dangerous(&full) { self.set_sticky_status("Blocked: command looks destructive."); return; }
        let (ok, code) = ops::run_shell_command(&full, std::time::Duration::from_secs(60));
        self.needs_clear = true;
        self.set_status(if ok { "✓ bind ok".into() } else { format!("✗ bind exited {}", code.map(|c| c.to_string()).unwrap_or("?".into())) });
        self.refresh_items(); self.capture_baselines();
    }

    pub fn submit_custom_bind(&mut self) {
        let raw = self.input.trim().to_string();
        if raw.is_empty() { self.cancel_input(); return; }
        if let Some(cmd) = raw.strip_prefix('!') {
            let cmd = cmd.trim();
            if cmd.is_empty() { self.set_status("Empty command"); self.input.clear(); return; }
            let mut i = 1;
            let name = loop {
                let n = format!("!c{}", i);
                if !self.shell_aliases.contains_key(&n) { break n; }
                i += 1;
            };
            self.shell_aliases.insert(name.clone(), cmd.to_string());
            self.pending_keybind = Some(KeyBind::Custom(cmd.to_string()));
            self.input.clear(); self.input_mode = InputMode::KeybindCapture;
            self.set_sticky_status(format!("Alias {} registered — press a key to bind (Esc=cancel)", name));
        } else if raw.starts_with('/') || raw.starts_with("~/") {
            let p = ops::expand_tilde(&raw);
            if !p.exists() { self.set_status("Path not found"); self.input.clear(); return; }
            self.pending_keybind = Some(KeyBind::Custom(format!("edit:{}", p.display())));
            self.input.clear(); self.input_mode = InputMode::KeybindCapture;
            self.set_sticky_status("Press a key to bind (opens in editor)");
        } else { self.set_status("Must start with ! or /"); self.input.clear(); }
    }

    // ---------- keybind reset (with backup + undo) ----------

    pub fn reset_keymap(&mut self, include_aliases: bool) {
        let kb_path = self.assets_dir.join(".keybinds.json");
        if let Ok(contents) = std::fs::read(&kb_path) {
            let _ = std::fs::write(self.assets_dir.join(".keybinds.json.pre-reset.bak"), contents);
        }
        self.keymap = crate::config::default_keymap();
        if include_aliases { self.shell_aliases.clear(); }
        self.save_keymap(); // aliases preserved unless factory reset
        self.keybind_state.select(Some(0));
        self.set_status(if include_aliases {
            "Factory reset done — backup at .keybinds.json.pre-reset.bak (u=undo)"
        } else {
            "Keybinds reset, aliases kept — backup saved (u=undo)"
        });
        tracing::info!(include_aliases, "keybinds reset");
    }

    pub fn restore_keymap_backup(&mut self) -> bool {
        let kb_path = self.assets_dir.join(".keybinds.json");
        let bak = self.assets_dir.join(".keybinds.json.pre-reset.bak");
        let Ok(saved) = std::fs::read(&bak) else { return false };
        // Swap: current → backup, backup → live. Pressing u again swaps back.
        if let Ok(cur) = std::fs::read(&kb_path) { let _ = std::fs::write(&bak, cur); }
        if std::fs::write(&kb_path, saved).is_err() { return false; }
        let km = crate::config::load_keymap(&kb_path);
        self.keymap = km.map;
        self.shell_aliases = km.aliases;
        self.keybind_state.select(Some(0));
        self.set_status("Keybinds restored from pre-reset backup.");
        true
    }
}
