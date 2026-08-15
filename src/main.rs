use ansi_to_tui::IntoText;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::symlink;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;
use std::collections::hash_map::DefaultHasher;

// --- THEME ---
struct Theme { border: Color, text: Color, muted: Color, accent: Color, highlight: Style }
impl Theme {
    fn new() -> Self {
        let term = env::var("TERM").unwrap_or_default().to_lowercase();
        let colorterm = env::var("COLORTERM").unwrap_or_default().to_lowercase();
        let truecolor = colorterm.contains("truecolor") || colorterm.contains("24bit");
        if truecolor {
            Self {
                border: Color::Rgb(69, 71, 90), text: Color::Rgb(205, 214, 244),
                muted: Color::Rgb(108, 112, 134), accent: Color::Rgb(137, 180, 250),
                highlight: Style::default().fg(Color::Rgb(205, 214, 244)).bg(Color::Rgb(49, 50, 68)).add_modifier(Modifier::BOLD),
            }
        } else if term.contains("256color") {
            Self {
                border: Color::DarkGray, text: Color::White, muted: Color::Gray, accent: Color::Cyan,
                highlight: Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD),
            }
        } else {
            Self {
                border: Color::Gray, text: Color::White, muted: Color::DarkGray, accent: Color::Blue,
                highlight: Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            }
        }
    }
}

// --- DATA STRUCTURES ---
#[derive(Clone)]
struct Node { path: PathBuf, depth: usize, is_dir: bool, expanded: bool }

pub enum PreviewKind { Text, Image, Video }
pub fn detect_kind(path: &Path) -> PreviewKind {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return match ext.to_lowercase().as_str() {
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => PreviewKind::Image,
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "flv" => PreviewKind::Video,
            _ => PreviewKind::Text,
        };
    }
    PreviewKind::Text
}

#[derive(PartialEq)]
enum InputMode { Normal, AddingPath, AddingAlias, AddingDir, AddingNote, Renaming, Search, VersionSelect, SudoPrompt, Help, Shell, VisualSelect, ArchivePrompt, DeployConfirm }

#[derive(Serialize, Deserialize, Default, Clone)]
struct ConfyState {
    #[serde(default)] pub bookmarks: HashSet<String>,
    #[serde(default)] pub notes: HashMap<String, String>,
    #[serde(default)] pub recent: VecDeque<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ManifestEntry { alias: String, target: String }
#[derive(Serialize, Deserialize, Debug)]
struct Manifest { files: Vec<ManifestEntry> }

struct App {
    nodes: Vec<Node>, filtered_nodes: Vec<Node>, state: ListState, preview_text: Vec<u8>,
    input_mode: InputMode, input: String, add_source_path: String, status: String,
    confy_dir: PathBuf, history_dir: PathBuf, version_limit: usize,
    last_dir_mtime: Option<SystemTime>, last_file_mtime: Option<SystemTime>,
    pending_delete: Option<PathBuf>, last_status_time: Option<SystemTime>, status_sticky: bool, preview_pinned: bool,
    version_state: ListState, available_versions: Vec<(String, PathBuf)>, current_rel_path: Option<PathBuf>,
    cut_source: Option<PathBuf>, yank_source: Option<PathBuf>, show_hidden: bool,
    saved_sudo_pass: Option<String>, sudo_target_file: Option<PathBuf>,
    selected_nodes: HashSet<PathBuf>, selected_editor: Option<String>,
    state_data: ConfyState, deploy_target: Option<PathBuf>, deploy_pass: Option<String>,
}

impl App {
    fn new(version_limit: usize) -> Self {
        let home = dirs::home_dir().expect("Could not find home directory");
        let confy_dir = home.join(".confy");
        let history_dir = home.join(".confy_history");
        fs::create_dir_all(&confy_dir).expect("Failed to create ~/.confy");
        fs::create_dir_all(&history_dir).expect("Failed to create ~/.confy_history");

        let sudo_pass_path = home.join(".confy_sudo");
        let saved_sudo_pass = fs::read_to_string(&sudo_pass_path).ok().filter(|s| !s.is_empty());
        let state_path = home.join(".confy_state.json");
        let state_data = fs::read_to_string(&state_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();

        let mut state = ListState::default();
        state.select(Some(0));

        let mut app = App {
            nodes: Vec::new(), filtered_nodes: Vec::new(), state, preview_text: Vec::new(),
            input_mode: InputMode::Normal, input: String::new(), add_source_path: String::new(), status: String::new(),
            confy_dir, history_dir, version_limit,
            last_dir_mtime: None, last_file_mtime: None, pending_delete: None, last_status_time: None,
            status_sticky: false, preview_pinned: false,
            version_state: ListState::default(), available_versions: Vec::new(), current_rel_path: None,
            cut_source: None, yank_source: None, show_hidden: false, saved_sudo_pass, sudo_target_file: None,
            selected_nodes: HashSet::new(), selected_editor: None, state_data, deploy_target: None, deploy_pass: None,
        };
        app.refresh_items();
        app
    }

    fn set_status(&mut self, msg: impl Into<String>) { self.status = msg.into(); self.last_status_time = Some(SystemTime::now()); self.status_sticky = false; }
    fn set_sticky_status(&mut self, msg: impl Into<String>) { self.status = msg.into(); self.last_status_time = Some(SystemTime::now()); self.status_sticky = true; }
    fn save_state(&self) { if let Ok(s) = serde_json::to_string_pretty(&self.state_data) { let _ = fs::write(dirs::home_dir().unwrap().join(".confy_state.json"), s); } }
    fn current_nodes(&self) -> &Vec<Node> { if self.input_mode == InputMode::Search { &self.filtered_nodes } else { &self.nodes } }
    fn current_nodes_mut(&mut self) -> &mut Vec<Node> { if self.input_mode == InputMode::Search { &mut self.filtered_nodes } else { &mut self.nodes } }

    // --- STATE & METADATA ---
    fn toggle_bookmark(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            if let Ok(rel) = node.path.strip_prefix(&self.confy_dir) {
                let rel_str = rel.to_string_lossy().to_string();
                if self.state_data.bookmarks.contains(&rel_str) { self.state_data.bookmarks.remove(&rel_str); self.set_status("Bookmark removed."); }
                else { self.state_data.bookmarks.insert(rel_str); self.set_status("Bookmark added."); }
                self.save_state();
            }}
        }
    }

    fn begin_add_note(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            if let Ok(rel) = node.path.strip_prefix(&self.confy_dir) {
                let rel_str = rel.to_string_lossy().to_string();
                self.add_source_path = rel_str.clone();
                self.input = self.state_data.notes.get(&rel_str).cloned().unwrap_or_default();
                self.input_mode = InputMode::AddingNote; self.status.clear();
            }}
        }
    }

    fn submit_note(&mut self) {
        let note = self.input.trim().to_string();
        if note.is_empty() { self.state_data.notes.remove(&self.add_source_path); }
        else { self.state_data.notes.insert(self.add_source_path.clone(), note); }
        self.save_state(); self.set_status("Note saved."); self.cancel_input();
    }

    fn add_to_recent(&mut self, path: &Path) {
        if let Ok(rel) = path.strip_prefix(&self.confy_dir) {
            let rel_str = rel.to_string_lossy().to_string();
            self.state_data.recent.retain(|p| p != &rel_str);
            self.state_data.recent.push_front(rel_str);
            self.state_data.recent.truncate(5);
            self.save_state();
        }
    }

    fn jump_to_recent(&mut self) {
        if self.state_data.recent.is_empty() { self.set_status("No recent files."); return; }
        disable_raw_mode().ok(); execute!(std::io::stdout(), LeaveAlternateScreen).ok();
        let list = self.state_data.recent.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
        let out = Command::new("sh").arg("-c").arg(format!("echo \"{}\" | fzf --prompt='Jump to Recent> '", list))
            .stdin(Stdio::inherit()).stdout(Stdio::piped()).stderr(Stdio::null()).output();
        execute!(std::io::stdout(), EnterAlternateScreen).ok(); enable_raw_mode().ok();
        if let Ok(out) = out { if out.status.success() {
            let selected = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !selected.is_empty() {
                let target_path = self.confy_dir.join(&selected);
                if let Some(idx) = self.current_nodes().iter().position(|n| n.path == target_path) { self.state.select(Some(idx)); self.update_preview(); }
            }
        }}
    }

    // --- SMART SNAPSHOT LOGIC ---
    fn take_snapshot_and_cleanup(&self) {
        let out = Command::new("date").arg("+%Y%m%d_%H%M%S").stdout(Stdio::piped()).stderr(Stdio::null()).output();
        if let Ok(out) = out {
            let timestamp = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let snapshot_dir = self.history_dir.join(&timestamp);
            let mut changed = false;
            if let Ok(entries) = fs::read_dir(&self.confy_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() { continue; }
                    if let Ok(rel) = p.strip_prefix(&self.confy_dir) {
                        if self.has_file_changed(&p, rel) {
                            if !changed { let _ = fs::create_dir_all(&snapshot_dir); changed = true; }
                            let dest = snapshot_dir.join(rel);
                            if let Some(parent) = dest.parent() { let _ = fs::create_dir_all(parent); }
                            let _ = Command::new("cp").arg("-rL").arg(&p).arg(&dest).stdout(Stdio::null()).stderr(Stdio::null()).status();
                        }
                    }
                }
            }
            if let Ok(entries) = fs::read_dir(&self.history_dir) {
                let mut snaps: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
                snaps.sort();
                if snaps.len() > self.version_limit {
                    for old_snap in snaps.iter().take(snaps.len() - self.version_limit) {
                        let _ = fs::remove_dir_all(old_snap);
                    }
                }
            }
        }
    }

    fn has_file_changed(&self, current_path: &Path, rel_path: &Path) -> bool {
        let mut latest_snap: Option<PathBuf> = None;
        if let Ok(entries) = fs::read_dir(&self.history_dir) {
            let mut snaps: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            snaps.sort();
            for snap in snaps.iter().rev() {
                let hist_file = snap.join(rel_path);
                if hist_file.exists() { latest_snap = Some(hist_file); break; }
            }
        }
        match latest_snap {
            None => true,
            Some(hist) => {
                let cur_hash = hash_file(current_path).unwrap_or(0);
                let hist_hash = hash_file(&hist).unwrap_or(1);
                cur_hash != hist_hash
            }
        }
    }

    // --- ARCHIVE & DEPLOY LOGIC ---
    fn toggle_selection(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            if self.selected_nodes.contains(&node.path) { self.selected_nodes.remove(&node.path); self.set_status("Deselected."); }
            else { self.selected_nodes.insert(node.path.clone()); self.set_status("Selected. Press 'z' to archive."); }
        }}
    }

    fn archive_selected(&mut self, password: &str) {
        if self.selected_nodes.is_empty() { self.set_status("No files selected."); return; }
        let mut files_to_archive = Vec::new();
        for path in &self.selected_nodes {
            if let Ok(real_path) = fs::canonicalize(path) {
                if real_path.is_file() {
                    let alias = path.file_name().unwrap().to_string_lossy().to_string();
                    files_to_archive.push((alias, real_path));
                }
            }
        }
        match create_archive(&files_to_archive, password) {
            Ok(archive_path) => {
                self.set_status(format!("Archive created: {}", archive_path.display()));
                self.selected_nodes.clear(); self.input.clear(); self.input_mode = InputMode::Normal;
                self.refresh_items();
            }
            Err(e) => self.set_status(format!("Archive failed: {}", e)),
        }
    }

    fn try_deploy(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            let name = node.path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if name.ends_with(".zip") {
                self.preview_pinned = true; self.preview_text.clear();
                match deploy_archive(&node.path, false, None) {
                    Ok(output) => {
                        self.preview_text = output.into_bytes();
                        self.deploy_target = Some(node.path.clone());
                        self.deploy_pass = None;
                        self.input_mode = InputMode::DeployConfirm;
                        self.set_sticky_status("Dry run complete. Press 'y' to apply, 'n' to cancel.");
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str == "PASSWORD_REQUIRED" {
                            self.deploy_target = Some(node.path.clone());
                            self.input_mode = InputMode::ArchivePrompt;
                            self.input.clear();
                            self.set_status("🔒 enter passwd (esc=cancel)");
                        } else {
                            self.preview_text = format!("Deploy failed: {}", err_str).into_bytes();
                            self.set_status("Deploy failed.");
                        }
                    }
                }
            } else { self.set_status("Deploy only works on .zip archives. (Press 'D')"); }
        }}
    }

    // --- VERSION VIEW LOGIC ---
    fn enter_version_mode(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            if let Ok(rel_path) = node.path.strip_prefix(&self.confy_dir) {
                self.current_rel_path = Some(rel_path.to_path_buf());
                self.available_versions.clear();
                if let Ok(entries) = fs::read_dir(&self.history_dir) {
                    let mut snaps: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
                    snaps.sort();
                    for snap in snaps.iter().rev() {
                        let hist_file = snap.join(rel_path);
                        if hist_file.exists() {
                            let date_str = snap.file_name().unwrap().to_string_lossy().to_string();
                            let formatted = if date_str.len() >= 13 {
                                format!("{}-{}-{} {}:{}", &date_str[0..4], &date_str[4..6], &date_str[6..8], &date_str[9..11], &date_str[11..13])
                            } else { date_str.clone() };
                            self.available_versions.push((formatted, hist_file));
                        }
                    }
                }
                if !self.available_versions.is_empty() { self.input_mode = InputMode::VersionSelect; self.version_state.select(Some(0)); }
                else { self.set_status("No history for this item"); }
            }}
        }
    }

    fn restore_version(&mut self) {
        if let Some(i) = self.version_state.selected() { if let Some((_, hist_path)) = self.available_versions.get(i).cloned() {
            if let Some(rel_path) = &self.current_rel_path {
                let dest = self.confy_dir.join(rel_path);
                let is_symlink = dest.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
                if !is_symlink {
                    if dest.is_dir() { let _ = fs::remove_dir_all(&dest); }
                    else { let _ = fs::remove_file(&dest); }
                }
                let _ = Command::new("cp").arg("-rL").arg(&hist_path).arg(&dest).stdout(Stdio::null()).stderr(Stdio::null()).status();
                self.set_status("Restored from history"); self.exit_version_mode(); self.refresh_items();
            }}
        }
    }

    fn show_diff(&mut self) {
        if let Some(i) = self.version_state.selected() { if let Some((_, hist_path)) = self.available_versions.get(i).cloned() {
            if let Some(rel_path) = &self.current_rel_path {
                let current_path = self.confy_dir.join(rel_path);
                self.preview_pinned = true; self.preview_text.clear();
                match generate_diff(&hist_path, &current_path) {
                    Ok(diff_output) => {
                        let header = format!("\x1b[1;36m\u{f44e}  Diff View (Esc to return)\x1b[0m\n\x1b[90m────────────────\x1b[0m\n");
                        self.preview_text = format!("{}{}", header, diff_output).into_bytes();
                        self.set_sticky_status("Diff generated. (Esc to return)");
                    }
                    Err(e) => {
                        self.preview_text = format!("Diff failed: {}", e).into_bytes();
                        self.set_status("Diff failed.");
                    }
                }
            }}
        }
    }

    fn exit_version_mode(&mut self) {
        self.input_mode = InputMode::Normal; self.available_versions.clear();
        self.current_rel_path = None; self.version_state.select(None); self.update_preview();
    }

    fn next_version(&mut self) {
        let len = self.available_versions.len(); if len == 0 { return; }
        let i = match self.version_state.selected() { Some(i) => if i >= len - 1 { 0 } else { i + 1 }, None => 0 };
        self.version_state.select(Some(i));
    }

    fn previous_version(&mut self) {
        let len = self.available_versions.len(); if len == 0 { return; }
        let i = match self.version_state.selected() { Some(i) => if i == 0 { len - 1 } else { i - 1 }, None => 0 };
        self.version_state.select(Some(i));
    }

    // --- FILE TREE LOGIC ---
    fn refresh_items(&mut self) {
        let mut expanded_paths = std::collections::HashSet::new();
        for node in &self.nodes { if node.is_dir && node.expanded { expanded_paths.insert(node.path.clone()); } }
        let show_hidden = self.show_hidden;
        self.nodes = match fs::read_dir(&self.confy_dir) {
            Ok(dir) => {
                let mut roots: Vec<Node> = dir.filter_map(|e| e.ok())
                    .map(|e| {
                        let path = e.path();
                        let is_dir = fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
                        Node { path, depth: 0, is_dir, expanded: false }
                    })
                    .filter(|n| show_hidden || !n.path.file_name().unwrap_or_default().to_string_lossy().starts_with('.'))
                    .collect();
                sort_nodes(&mut roots); roots
            }
            Err(_) => Vec::new(),
        };
        let mut i = 0;
        while i < self.nodes.len() {
            if self.nodes[i].is_dir && expanded_paths.contains(&self.nodes[i].path) {
                let path = self.nodes[i].path.clone();
                let depth = self.nodes[i].depth + 1;
                self.nodes[i].expanded = true;
                if let Ok(entries) = fs::read_dir(&path) {
                    let mut children: Vec<Node> = entries.flatten().map(|e| {
                        let p = e.path(); let is_dir = fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
                        Node { path: p, depth, is_dir, expanded: false }
                    })
                    .filter(|n| show_hidden || !n.path.file_name().unwrap_or_default().to_string_lossy().starts_with('.'))
                    .collect();
                    sort_nodes(&mut children);
                    for (offset, child) in children.into_iter().enumerate() { self.nodes.insert(i + 1 + offset, child); }
                }
            }
            i += 1;
        }
        self.apply_filter();
        let current = self.state.selected().unwrap_or(0);
        if !self.filtered_nodes.is_empty() { self.state.select(Some(current.min(self.filtered_nodes.len() - 1))); }
        else { self.state.select(None); }
        let mut max_dir_mtime = None;
        if let Ok(meta) = fs::metadata(&self.confy_dir) { max_dir_mtime = meta.modified().ok(); }
        for node in &self.nodes {
            if node.is_dir && node.expanded {
                if let Ok(meta) = fs::metadata(&node.path) {
                    if let Ok(mtime) = meta.modified() {
                        if Some(mtime) > max_dir_mtime { max_dir_mtime = Some(mtime); }
                    }
                }
            }
        }
        self.last_dir_mtime = max_dir_mtime;
        self.update_preview();
    }

    fn apply_filter(&mut self) {
        self.pending_delete = None;
        if self.input.is_empty() { self.filtered_nodes = self.nodes.clone(); }
        else {
            let query = self.input.to_lowercase(); let mut filtered = Vec::new();
            for node in &self.nodes { search_recursive(node, &query, &mut filtered); }
            self.filtered_nodes = filtered;
        }
    }

    fn expand_node(&mut self, index: usize) {
        let show_hidden = self.show_hidden; let nodes = self.current_nodes_mut();
        if let Some(node) = nodes.get_mut(index) {
            node.expanded = true; let path = node.path.clone(); let depth = node.depth + 1;
            if let Ok(entries) = fs::read_dir(&path) {
                let mut children: Vec<Node> = entries.flatten().map(|e| {
                    let p = e.path(); let is_dir = fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
                    Node { path: p, depth, is_dir, expanded: false }
                })
                .filter(|n| show_hidden || !n.path.file_name().unwrap_or_default().to_string_lossy().starts_with('.'))
                .collect();
                sort_nodes(&mut children);
                for (offset, child) in children.into_iter().enumerate() { nodes.insert(index + 1 + offset, child); }
            }
        }
        self.update_preview();
    }

    fn collapse_node(&mut self, index: usize) {
        let nodes = self.current_nodes_mut();
        if let Some(node) = nodes.get_mut(index) {
            node.expanded = false; let parent_depth = node.depth;
            let mut remove_count = 0;
            for i in (index + 1)..nodes.len() {
                if nodes[i].depth > parent_depth { remove_count += 1; } else { break; }
            }
            nodes.drain(index + 1..index + 1 + remove_count);
        }
        self.update_preview();
    }

    // --- PREVIEW LOGIC ---
    fn update_preview(&mut self) {
        if self.preview_pinned { return; }
        self.preview_text.clear();
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            let target_path = fs::canonicalize(&node.path).unwrap_or(node.path.clone());
            let is_symlink = node.path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
            if is_symlink {
                if let Ok(target) = fs::read_link(&node.path) {
                    let sym_info = format!("\x1b[1;35m\u{f481}  Symlink -> {}\x1b[0m\n\x1b[90m────────────────\x1b[0m\n", target.display());
                    self.preview_text.extend(sym_info.as_bytes());
                }
            }
            if let Ok(rel) = node.path.strip_prefix(&self.confy_dir) {
                let rel_str = rel.to_string_lossy().to_string();
                if let Some(note) = self.state_data.notes.get(&rel_str) {
                    let note_info = format!("\x1b[1;33m📝  Note: {}\x1b[0m\n\x1b[90m────────────────\x1b[0m\n", note);
                    self.preview_text.extend(note_info.as_bytes());
                }
            }
            if target_path.is_dir() {
                let mut output = format!("\x1b[1;36m\u{f115}  Directory Contents\x1b[0m\n\x1b[90m────────────────\x1b[0m\n");
                if let Ok(entries) = fs::read_dir(&target_path) {
                    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
                    paths.sort_by_key(|p| (!p.is_dir(), p.file_name().unwrap_or_default().to_os_string()));
                    for p in paths {
                        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                        let icon = if p.is_dir() { "\u{f115}" } else { get_icon(&p) };
                        output.push_str(&format!("{} {}\n", icon, name));
                    }
                }
                self.preview_text.extend(output.as_bytes()); return;
            }
            let (tcols, trows) = crossterm::terminal::size().unwrap_or((80, 24));
            let p_w = ((tcols as f32) * 0.6).max(10.0) as u16;
            let p_h = ((trows as f32) * 0.7).max(4.0) as u16;
            let size_arg = format!("{}x{}", p_w, p_h);

            let content_text = match detect_kind(&target_path) {
                PreviewKind::Image => {
                    let out = Command::new("chafa").arg("--format=symbols").arg("--view-size").arg(&size_arg)
                        .arg(&target_path).stdout(Stdio::piped()).stderr(Stdio::null()).output();
                    let mut output = format!("\x1b[1;36m\u{f1c5}  Image Info\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\x1b[33m{}\x1b[0m\n", get_file_info(&target_path));
                    if let Ok(out) = out {
                        if out.status.success() && !out.stdout.is_empty() {
                            output.push_str("\n\x1b[90m─── Preview ───\x1b[0m\n");
                            output.push_str(&String::from_utf8_lossy(&out.stdout));
                        }
                    }
                    output.into_bytes()
                }
                PreviewKind::Video => {
                    let info = get_video_meta(&target_path);
                    let mut output = format!("\x1b[1;36m\u{f03d}  Video Info\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\x1b[33m{}\x1b[0m\n", info);
                    if let Some(thumb) = extract_video_thumbnail_cached(&target_path) {
                        let out = Command::new("chafa").arg("--format=symbols").arg("--view-size").arg(&size_arg)
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
                PreviewKind::Text => {
                    let out = Command::new("bat").arg("--color=always").arg("--style=plain")
                        .arg("--paging=never").arg("--theme=ansi").arg(&target_path).stdout(Stdio::piped()).stderr(Stdio::piped()).output();
                    if let Ok(out) = out {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        if stdout.contains("[bat warning]") || stderr.contains("[bat warning]") {
                            "\x1b[1;33m\u{f071}  Binary File\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\n\x1b[90mPreview not available for binary files.\x1b[0m".to_string().into_bytes()
                        } else if out.status.success() { out.stdout }
                        else { read_small_fallback(&target_path) }
                    } else { read_small_fallback(&target_path) }
                }
            };
            self.preview_text.extend(content_text);
        }}
    }

    fn check_fs_changes(&mut self) {
        if let Some(time) = self.last_status_time {
            if !self.status_sticky && SystemTime::now().duration_since(time).unwrap_or_default().as_secs() >= 1 {
                self.status.clear(); self.last_status_time = None;
            }
        }
        let mut max_dir_mtime = self.last_dir_mtime; let mut need_refresh = false;
        let expanded_paths: Vec<PathBuf> = self.nodes.iter()
            .filter(|n| n.is_dir && n.expanded).map(|n| n.path.clone()).collect();
        let paths_to_check = std::iter::once(self.confy_dir.clone()).chain(expanded_paths.into_iter());
        for path in paths_to_check {
            if let Ok(meta) = fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    if Some(mtime) > max_dir_mtime { max_dir_mtime = Some(mtime); need_refresh = true; }
                }
            }
        }
        if need_refresh { self.last_dir_mtime = max_dir_mtime; self.refresh_items(); return; }
        if !self.preview_pinned {
            if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
                let target_path = fs::canonicalize(&node.path).unwrap_or(node.path.clone());
                if let Ok(meta) = fs::metadata(&target_path) {
                    if let Ok(mtime) = meta.modified() {
                        if Some(mtime) != self.last_file_mtime { self.update_preview(); }
                    }
                }
            }}
        }
    }

    // --- FILE OPERATIONS ---
    fn begin_add(&mut self) { self.input_mode = InputMode::AddingPath; self.input.clear(); self.add_source_path.clear(); self.status.clear(); }

    fn submit_add_step(&mut self) {
        match self.input_mode {
            InputMode::AddingPath => {
                let p = self.input.trim().to_string();
                if p.is_empty() { self.set_status("Path cannot be empty"); return; }
                let source_path = expand_tilde(&p);
                if !source_path.exists() { self.set_status(format!("Path not found: {}", p)); return; }
                self.add_source_path = source_path.to_string_lossy().to_string();
                self.input.clear(); self.input_mode = InputMode::AddingAlias;
            }
            InputMode::AddingAlias => {
                let alias = self.input.trim();
                if alias.is_empty() { self.set_status("Alias cannot be empty"); return; }
                let source = PathBuf::from(&self.add_source_path);
                let dest = self.confy_dir.join(alias);
                if !is_path_safe(&self.confy_dir, &dest) { self.set_status("Error: Cannot escape ~/.confy"); return; }
                if dest.exists() { self.set_status(format!("Alias already exists: {}", alias)); return; }
                if let Some(parent) = dest.parent() {
                    if let Err(e) = fs::create_dir_all(parent) { self.set_status(format!("Failed to create folder: {}", e)); return; }
                }
                match symlink(&source, &dest) {
                    Ok(_) => {
                        self.set_status(format!("Added: {} -> {}", alias, source.display()));
                        self.input.clear(); self.add_source_path.clear(); self.input_mode = InputMode::Normal;
                        self.refresh_items(); self.take_snapshot_and_cleanup();
                    }
                    Err(e) => self.set_status(format!("Failed to add link: {}", e)),
                }
            }
            _ => {}
        }
    }

    fn cut_item(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            self.cut_source = Some(node.path.clone()); self.yank_source = None;
            self.set_sticky_status(format!("Cut: {}. Press 'p' to paste. (Esc=cancel)", node.path.file_name().unwrap_or_default().to_string_lossy()));
        }}
    }

    fn yank_item(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            self.yank_source = Some(node.path.clone()); self.cut_source = None;
            self.set_sticky_status(format!("Yanked: {}. Press 'p' to paste. (Esc=cancel)", node.path.file_name().unwrap_or_default().to_string_lossy()));
        }}
    }

    fn paste_item(&mut self) {
        let src_opt = self.cut_source.clone().or(self.yank_source.clone());
        if let Some(src) = src_opt {
            let is_cut = self.cut_source.is_some();
            let src_abs = fs::canonicalize(&src).unwrap_or(src.clone());
            let dest_dir = if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
                if node.is_dir { node.path } else { node.path.parent().unwrap_or(&self.confy_dir).to_path_buf() }
            } else { self.confy_dir.clone() } } else { self.confy_dir.clone() };
            let dest_dir_abs = fs::canonicalize(&dest_dir).unwrap_or(dest_dir.clone());
            if dest_dir_abs.starts_with(&src_abs) {
                self.set_status("Cannot paste inside itself"); self.cut_source = None; self.yank_source = None; return;
            }
            let file_name = src.file_name().unwrap_or_default();
            let dest_path = dest_dir.join(file_name);
            if dest_path.exists() { self.set_status("Paste failed: Name already exists"); }
            else {
                let result = if is_cut { fs::rename(&src, &dest_path) }
                else if src.is_dir() { Command::new("cp").arg("-r").arg(&src).arg(&dest_path).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|_| ()) }
                else { fs::copy(&src, &dest_path).map(|_| ()) };
                match result {
                    Ok(_) => {
                        self.set_status(format!("Pasted to {}", dest_dir.display()));
                        self.cut_source = None; self.yank_source = None; self.refresh_items();
                    },
                    Err(e) => self.set_status(format!("Paste failed: {}", e)),
                }
            }
        }
    }

    fn begin_add_folder(&mut self) { self.input_mode = InputMode::AddingDir; self.input.clear(); self.status.clear(); }

    fn submit_add_folder(&mut self) {
        let name = self.input.trim();
        if name.is_empty() || name.contains('/') { self.set_status("Invalid name"); return; }
        let dest_dir = self.confy_dir.clone(); let dest_path = dest_dir.join(name);
        if !is_path_safe(&self.confy_dir, &dest_path) { self.set_status("Error: Cannot escape ~/.confy"); return; }
        if dest_path.exists() { self.set_status("Folder already exists"); return; }
        match fs::create_dir(&dest_path) {
            Ok(_) => self.set_status(format!("Created folder: {}", name)),
            Err(e) => self.set_status(format!("Failed to create folder: {}", e)),
        }
        self.input_mode = InputMode::Normal; self.input.clear(); self.refresh_items();
    }

    fn begin_rename(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            self.add_source_path = node.path.to_string_lossy().to_string();
            self.input = node.path.file_name().unwrap_or_default().to_string_lossy().to_string();
            self.input_mode = InputMode::Renaming; self.status.clear();
        }}
    }

    fn submit_rename(&mut self) {
        let new_name = self.input.trim();
        if new_name.is_empty() { self.cancel_input(); return; }
        let old_path = PathBuf::from(&self.add_source_path);
        let parent = old_path.parent().unwrap_or(&self.confy_dir);
        let new_path = parent.join(new_name);
        if !is_path_safe(&self.confy_dir, &new_path) { self.set_status("Error: Cannot escape ~/.confy"); return; }
        if new_path == old_path { self.cancel_input(); return; }
        if new_path.exists() { self.set_status("Name already exists"); return; }
        if let Some(new_parent) = new_path.parent() {
            if let Err(e) = fs::create_dir_all(new_parent) { self.set_status(format!("Failed to create folder: {}", e)); return; }
        }
        match fs::rename(&old_path, &new_path) {
            Ok(_) => self.set_status(format!("Renamed to {}", new_name)),
            Err(e) => self.set_status(format!("Rename failed: {}", e)),
        }
        self.input_mode = InputMode::Normal; self.input.clear(); self.add_source_path.clear(); self.refresh_items();
    }

    fn cancel_input(&mut self) {
        self.input_mode = InputMode::Normal; self.input.clear(); self.add_source_path.clear(); self.status.clear();
        self.pending_delete = None; self.cut_source = None; self.yank_source = None; self.sudo_target_file = None; self.status_sticky = false;
        self.deploy_target = None; self.deploy_pass = None;
    }

    fn start_search(&mut self) {
        self.input_mode = InputMode::Search; self.input.clear(); self.apply_filter();
        self.state.select(Some(0)); self.update_preview();
    }

    fn mark_or_delete(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            if let Some(cut) = &self.cut_source { if cut == &node.path { self.cut_source = None; self.status_sticky = false; } }
            if let Some(yank) = &self.yank_source { if yank == &node.path { self.yank_source = None; self.status_sticky = false; } }
            if let Some(pending) = &self.pending_delete {
                if pending == &node.path {
                    let is_symlink = node.path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
                    let result = if is_symlink { fs::remove_file(&node.path) }
                    else if node.is_dir { fs::remove_dir_all(&node.path) }
                    else { fs::remove_file(&node.path) };
                    match result {
                        Ok(_) => {
                            self.set_status(format!("Deleted: {}", node.path.file_name().unwrap_or_default().to_string_lossy()));
                            self.pending_delete = None; self.refresh_items();
                            let len = self.current_nodes().len();
                            if len > 0 { self.state.select(Some(i.min(len - 1))); }
                            self.update_preview();
                        }
                        Err(e) => { self.set_status(format!("Failed to delete: {}", e)); self.pending_delete = None; }
                    }
                } else {
                    self.pending_delete = Some(node.path.clone());
                    self.set_sticky_status(format!("Press 'd' again to delete {}. (Esc=cancel)", node.path.file_name().unwrap_or_default().to_string_lossy()));
                }
            } else {
                self.pending_delete = Some(node.path.clone());
                self.set_sticky_status(format!("Press 'd' again to delete {}. (Esc=cancel)", node.path.file_name().unwrap_or_default().to_string_lossy()));
            }
        }}
    }

    fn launch_fzf(&mut self) {
        let current_input = self.input.clone();
        let expanded_input = expand_tilde(&current_input);
        let search_dir = if current_input.is_empty() { dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")) }
        else if expanded_input.is_dir() { expanded_input.clone() }
        else { expanded_input.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))) };
        let cmd = format!("fd . {} --type f --type d 2>/dev/null || find {} 2>/dev/null", search_dir.display(), search_dir.display());
        disable_raw_mode().ok(); execute!(std::io::stdout(), LeaveAlternateScreen).ok(); std::io::stdout().flush().ok();
        let out = Command::new("sh").arg("-c").arg(format!("{} | fzf --prompt='Complete> ' --header='Searching in: {}'", cmd, search_dir.display()))
            .stdin(Stdio::inherit()).stdout(Stdio::piped()).stderr(Stdio::null()).output();
        execute!(std::io::stdout(), EnterAlternateScreen).ok(); enable_raw_mode().ok();
        if let Ok(out) = out { if out.status.success() {
            let selected = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !selected.is_empty() { self.input = selected; }
        }} else { self.set_status("Failed to launch fzf. Is it installed?"); }
    }

    fn next(&mut self) {
        self.pending_delete = None; self.preview_pinned = false;
        let len = self.current_nodes().len(); if len == 0 { return; }
        let i = match self.state.selected() { Some(i) => if i >= len - 1 { 0 } else { i + 1 }, None => 0 };
        self.state.select(Some(i)); self.update_preview();
    }

    fn previous(&mut self) {
        self.pending_delete = None; self.preview_pinned = false;
        let len = self.current_nodes().len(); if len == 0 { return; }
        let i = match self.state.selected() { Some(i) => if i == 0 { len - 1 } else { i - 1 }, None => 0 };
        self.state.select(Some(i)); self.update_preview();
    }

    fn edit_selected(&mut self) -> bool {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            let target_path = fs::canonicalize(&node.path).unwrap_or(node.path.clone());

            if matches!(detect_kind(&target_path), PreviewKind::Image | PreviewKind::Video) {
                let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                let _ = Command::new(opener).arg(&target_path).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
                return false;
            }

            let needs_sudo = match fs::OpenOptions::new().write(true).open(&target_path) {
                Ok(_) => false, Err(e) => e.kind() == std::io::ErrorKind::PermissionDenied,
            };

            let editor = if let Some(e) = &self.selected_editor { e.clone() }
            else {
                match select_editor() {
                    Ok(e) => { self.selected_editor = Some(e.clone()); e }
                    Err(_) => { self.set_status("Editor selection cancelled."); return false; }
                }
            };

            if needs_sudo {
                self.sudo_target_file = Some(target_path.clone());
                if let Some(pass) = &self.saved_sudo_pass {
                    if validate_sudo(pass) {
                        open_editor_sudo(&target_path, &editor, pass);
                        self.take_snapshot_and_cleanup(); return false;
                    } else { self.saved_sudo_pass = None; }
                }
                self.selected_editor = Some(editor);
                self.input_mode = InputMode::SudoPrompt; self.input.clear();
                self.set_status("Enter Sudo Password (Empty=View, Esc=cancel)");
                return false;
            }

            self.add_to_recent(&node.path);
            open_editor(&target_path, &editor);
            self.take_snapshot_and_cleanup();
        }}
        false
    }

    fn submit_sudo_password(&mut self) {
        if let Some(file) = self.sudo_target_file.clone() {
            let pass = self.input.clone();
            if pass.is_empty() {
                if let Some(editor) = &self.selected_editor { open_editor(&file, editor); }
                self.set_status("Opened in view-only mode.");
                self.cancel_input(); self.take_snapshot_and_cleanup(); return;
            }
            if validate_sudo(&pass) {
                if let Some(editor) = &self.selected_editor {
                    let success = open_editor_sudo(&file, editor, &pass);
                    if success { self.set_status("File edited successfully."); }
                }
                self.cancel_input(); self.take_snapshot_and_cleanup();
            } else { self.set_status("Wrong password, try again. (Esc=cancel)"); self.input.clear(); }
        }
    }

    fn execute_shell(&mut self) {
        if let Some(i) = self.state.selected() { if let Some(node) = self.current_nodes().get(i).cloned() {
            let cmd = self.input.trim();
            if !cmd.is_empty() {
                let full_cmd = format!("{} \"{}\"", cmd, node.path.display());
                let out = Command::new("sh").arg("-c").arg(&full_cmd).stdout(Stdio::piped()).stderr(Stdio::piped()).output();
                if let Ok(out) = out {
                    let mut output_str = String::new();
                    output_str.push_str("\x1b[1;36m\u{f120}  Command Output\x1b[0m\n\x1b[90m────────────────\x1b[0m\n");
                    if !out.stdout.is_empty() { output_str.push_str(&String::from_utf8_lossy(&out.stdout)); }
                    if !out.stderr.is_empty() {
                        if !out.stdout.is_empty() { output_str.push('\n'); }
                        output_str.push_str("\x1b[31m"); output_str.push_str(&String::from_utf8_lossy(&out.stderr)); output_str.push_str("\x1b[0m");
                    }
                    if out.stdout.is_empty() && out.stderr.is_empty() { output_str.push_str("(Command executed successfully, no output)\n"); }
                    output_str.push_str("\n\x1b[90m─── (Press Esc to return to preview) ───\x1b[0m\n");
                    self.preview_text = output_str.into_bytes(); self.preview_pinned = true;
                    self.set_status(format!("Executed: {} (Esc to return)", full_cmd));
                } else { self.set_status("Failed to execute command"); }
            }
        }}
        self.input.clear(); self.input_mode = InputMode::Normal;
    }
}

// --- HELPER FUNCTIONS ---
fn is_path_safe(base: &Path, target: &Path) -> bool {
    let canonical_base = fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let parent = target.parent().unwrap_or(Path::new("/"));
    if let Ok(canonical_parent) = fs::canonicalize(parent) {
        let canonical_target = canonical_parent.join(target.file_name().unwrap_or_default());
        return canonical_target.starts_with(&canonical_base);
    }
    false
}

fn select_editor() -> Result<String, Box<dyn std::error::Error>> {
    let home = dirs::home_dir().expect("No home dir");
    let editor_file = home.join(".confy_editor");
    if let Ok(editor) = fs::read_to_string(&editor_file) {
        let e = editor.trim().to_string();
        if !e.is_empty() { return Ok(e); }
    }
    if let Ok(editor) = env::var("EDITOR") {
        let e = editor.trim().to_string();
        if !e.is_empty() { return Ok(e); }
    }

    disable_raw_mode().ok(); execute!(std::io::stdout(), LeaveAlternateScreen).ok(); std::io::stdout().flush().ok();
    let known_editors = ["nvim", "vim", "vi", "nano", "code", "codium", "zed", "emacs", "micro", "kak", "hx", "neovim"];
    let mut available = Vec::new();
    for e in &known_editors {
        let status = Command::new("sh").arg("-c").arg(format!("command -v {}", e)).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
        if status.success() { available.push(e.to_string()); }
    }
    if available.is_empty() {
        execute!(std::io::stdout(), EnterAlternateScreen).ok(); enable_raw_mode().ok();
        return Err("No editors found on system.".into());
    }
    let list = available.join("\n");
    let out = Command::new("sh").arg("-c").arg(format!("echo \"{}\" | fzf --prompt='Select Editor> '", list))
        .stdin(Stdio::inherit()).stdout(Stdio::piped()).stderr(Stdio::null()).output()?;
    execute!(std::io::stdout(), EnterAlternateScreen).ok(); enable_raw_mode().ok();
    if !out.status.success() { return Err("Editor selection cancelled.".into()); }
    let selected = String::from_utf8_lossy(&out.stdout).trim().to_string();

    disable_raw_mode().ok(); execute!(std::io::stdout(), LeaveAlternateScreen).ok();
    let save_out = Command::new("sh").arg("-c").arg("echo -e 'Yes\nNo' | fzf --prompt='Save as default? '")
        .stdin(Stdio::inherit()).stdout(Stdio::piped()).stderr(Stdio::null()).output()?;
    execute!(std::io::stdout(), EnterAlternateScreen).ok(); enable_raw_mode().ok();
    if String::from_utf8_lossy(&save_out.stdout).trim() == "Yes" { fs::write(&editor_file, &selected)?; }
    Ok(selected)
}

fn open_editor(file: &Path, editor: &str) {
    disable_raw_mode().ok(); execute!(std::io::stdout(), LeaveAlternateScreen).ok(); std::io::stdout().flush().ok();
    let _ = Command::new(editor).arg(file).status();
    std::io::stdout().flush().ok(); execute!(std::io::stdout(), EnterAlternateScreen).ok(); enable_raw_mode().ok();
}

fn validate_sudo(pass: &str) -> bool {
    let mut child = match Command::new("sudo").arg("-S").arg("-v")
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() { Ok(c) => c, Err(_) => return false };
    if let Some(mut stdin) = child.stdin.take() { let _ = stdin.write_all(format!("{}\n", pass).as_bytes()); }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

fn open_editor_sudo(file: &Path, editor: &str, pass: &str) -> bool {
    disable_raw_mode().ok(); execute!(std::io::stdout(), LeaveAlternateScreen).ok(); std::io::stdout().flush().ok();
    let mut child = match Command::new("sudo").arg("-S").arg("-p").arg("").arg(editor).arg(file)
        .stdin(Stdio::piped()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).spawn() { Ok(c) => c, Err(_) => return false };
    if let Some(mut stdin) = child.stdin.take() { let _ = stdin.write_all(format!("{}\n", pass).as_bytes()); }
    let success = child.wait().map(|s| s.success()).unwrap_or(false);
    std::io::stdout().flush().ok(); execute!(std::io::stdout(), EnterAlternateScreen).ok(); enable_raw_mode().ok();
    success
}

fn sort_nodes(nodes: &mut [Node]) { nodes.sort_by_key(|n| (!n.is_dir, n.path.file_name().unwrap_or_default().to_os_string())); }

fn search_recursive(node: &Node, query: &str, filtered: &mut Vec<Node>) {
    let name = node.path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    if name.contains(query) { filtered.push(node.clone()); }
    if node.is_dir {
        if let Ok(entries) = fs::read_dir(&node.path) {
            let mut children: Vec<Node> = entries.flatten().map(|e| {
                let p = e.path(); let is_dir = fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
                Node { path: p, depth: node.depth + 1, is_dir, expanded: false }
            }).collect();
            sort_nodes(&mut children);
            for child in children { search_recursive(&child, query, filtered); }
        }
    }
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(stripped) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() { return home.join(stripped); }
    } else if p == "~" {
        if let Some(home) = dirs::home_dir() { return home; }
    }
    PathBuf::from(p)
}

fn highlight_name(name: &str, query: &str, accent: Style, normal: Style) -> Vec<Span<'static>> {
    if query.is_empty() { return vec![Span::styled(name.to_string(), normal)]; }
    let lname = name.to_lowercase(); let lquery = query.to_lowercase();
    if let Some(start_in_lname) = lname.find(&lquery) {
        let start_char_idx = lname[..start_in_lname].chars().count();
        let match_len_chars = lquery.chars().count();
        let mut byte_start = name.len();
        for (char_idx, (b, _ch)) in name.char_indices().enumerate() {
            if char_idx == start_char_idx { byte_start = b; break; }
        }
        if byte_start == name.len() && start_char_idx == 0 { byte_start = 0; }
        let mut byte_end = name.len(); let mut seen = 0usize;
        for (b, ch) in name[byte_start..].char_indices() {
            seen += 1; byte_end = byte_start + b + ch.len_utf8();
            if seen >= match_len_chars { break; }
        }
        if byte_end < byte_start { byte_end = name.len(); }
        vec![
            Span::styled(name[..byte_start].to_string(), normal),
            Span::styled(name[byte_start..byte_end].to_string(), accent.add_modifier(Modifier::BOLD)),
            Span::styled(name[byte_end..].to_string(), normal),
        ]
    } else { vec![Span::styled(name.to_string(), normal)] }
}

fn read_small_fallback(path: &Path) -> Vec<u8> {
    if let Ok(mut file) = File::open(path) {
        let mut buffer = vec![0; 4096];
        if let Ok(n) = file.read(&mut buffer) {
            buffer.truncate(n);
            if buffer.is_empty() { return "\x1b[90m(File is empty)\x1b[0m\n".to_string().into_bytes(); }
            if buffer.contains(&0) { return "\x1b[1;33m\u{f071}  Binary File\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\n\x1b[90mPreview not available for binary files.\x1b[0m".to_string().into_bytes(); }
            return buffer;
        }
    }
    "\x1b[31mUnable to read file (broken symlink or permissions?)\x1b[0m\n".to_string().into_bytes()
}

fn get_file_info(path: &Path) -> String {
    let out = Command::new("file").arg("--brief").arg(path).stdout(Stdio::piped()).stderr(Stdio::null()).output();
    if let Ok(out) = out { return String::from_utf8_lossy(&out.stdout).trim().to_string(); }
    "Unknown".to_string()
}

fn get_video_meta(path: &Path) -> String {
    let out = Command::new("ffprobe").arg("-v").arg("error")
        .arg("-show_entries").arg("format=duration,size:stream=codec_name,width,height")
        .arg("-of").arg("default=noprint_wrappers=1").arg(path).stdout(Stdio::piped()).stderr(Stdio::null()).output();
    if let Ok(out) = out { if out.status.success() { return String::from_utf8_lossy(&out.stdout).to_string(); } }
    "Metadata unavailable (is ffprobe installed?)".to_string()
}

fn extract_video_thumbnail_cached(path: &Path) -> Option<PathBuf> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let stem = path.file_name()?.to_string_lossy();
    let cache_dir = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("confy").join("thumbs");
    let _ = fs::create_dir_all(&cache_dir);
    let thumb_path = cache_dir.join(format!("{}_{}.png", stem, mtime));
    if thumb_path.exists() { return Some(thumb_path); }
    let status = Command::new("ffmpeg").arg("-y").arg("-i").arg(path)
        .arg("-vframes").arg("1").arg("-vf").arg("scale=960:-1").arg(&thumb_path)
        .stdout(Stdio::null()).stderr(Stdio::null()).status().ok()?;
    if status.success() { Some(thumb_path) } else { None }
}

fn format_size(size: u64) -> String {
    if size < 1024 { format!("{}B", size) }
    else if size < 1024 * 1024 { format!("{:.1}K", size as f64 / 1024.0) }
    else { format!("{:.1}M", size as f64 / (1024.0 * 1024.0)) }
}

fn get_icon(path: &Path) -> &str {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    if file_name == "dockerfile" { return "\u{e7b0}"; }
    if file_name == ".gitignore" || file_name == ".env" { return "\u{f46a}"; }
    if file_name.contains(".lock") { return "\u{f023}"; }
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return match ext.to_lowercase().as_str() {
            "rs" => "\u{e7a8}", "toml" => "\u{e615}", "json" => "\u{e60b}", "md" => "\u{e609}",
            "yaml" | "yml" => "\u{e615}", "lua" => "\u{e620}", "js" | "jsx" => "\u{e74e}",
            "ts" | "tsx" => "\u{e628}", "py" => "\u{e73c}", "sh" | "bash" => "\u{e795}",
            "txt" => "\u{f0f6}", "conf" | "config" => "\u{e615}", "c" | "h" => "\u{e61e}",
            "cpp" | "hpp" => "\u{e61e}", "go" => "\u{e627}", "java" => "\u{e256}",
            "html" => "\u{f13b}", "css" => "\u{e749}", "xml" => "\u{f121}", "log" => "\u{f02d}",
            "kdl" => "\u{e615}",
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => "\u{f1c5}",
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "flv" => "\u{f03d}",
            _ => "\u{f15b}",
        };
    }
    "\u{f15b}"
}

fn hash_file(path: &Path) -> std::io::Result<u64> {
    let mut file = File::open(path)?;
    let mut hasher = DefaultHasher::new();
    let mut buffer = [0u8; 1024 * 4];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        std::hash::Hash::hash_slice(&buffer[..n], &mut hasher);
    }
    Ok(std::hash::Hasher::finish(&hasher))
}

// --- DEPLOYMENT FUNCTIONS ---
fn create_archive(files: &[(String, PathBuf)], password: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = dirs::home_dir().expect("No home dir");
    let temp_dir = std::env::temp_dir().join("confy_archive");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir)?;
    let mut manifest = Manifest { files: vec![] };
    for (alias, path) in files {
        let dest = temp_dir.join(alias);
        fs::copy(path, &dest)?;
        let target_str = path.to_string_lossy().replace(&home.to_string_lossy().to_string(), "~");
        manifest.files.push(ManifestEntry { alias: alias.clone(), target: target_str });
    }
    let manifest_str = serde_json::to_string_pretty(&manifest)?;
    fs::write(temp_dir.join("manifest.json"), manifest_str)?;

    let out = Command::new("date").arg("+%Y%m%d_%H%M%S").stdout(Stdio::piped()).stderr(Stdio::null()).output()?;
    let timestamp = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let archive_name = format!("confy_backup_{}.zip", timestamp);
    let archive_path = home.join(".confy").join(archive_name);

    let mut cmd = Command::new("zip");
    cmd.arg("-r").arg(&archive_path).arg(".").current_dir(&temp_dir);
    if !password.is_empty() { cmd.arg("-P").arg(password); }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let status = cmd.status()?;

    if !status.success() { return Err("Failed to create zip archive".into()); }
    fs::remove_dir_all(&temp_dir)?;
    Ok(archive_path)
}

fn deploy_archive(archive: &Path, apply: bool, password: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    if !archive.exists() { return Err(format!("Archive not found: {}", archive.display()).into()); }
    let home = dirs::home_dir().expect("No home dir");
    let confy_dir = home.join(".confy");
    let temp_dir = std::env::temp_dir().join("confy_deploy");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir)?;

    let mut cmd = Command::new("unzip");
    cmd.arg("-o");
    // The Freeze Fix: Always pass a password. If the archive is unencrypted, unzip ignores it.
    // If it is encrypted and the password is wrong/missing, it fails instantly instead of hanging.
    let pass_to_use = password.unwrap_or("confy_dummy_pass_123");
    cmd.arg("-P").arg(pass_to_use);

    cmd.arg(archive).arg("-d").arg(&temp_dir);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let out = cmd.output()?;

    if !out.status.success() {
        let err_str = String::from_utf8_lossy(&out.stderr).to_lowercase();
        if err_str.contains("password") || err_str.contains("incorrect password") {
            return Err("PASSWORD_REQUIRED".into());
        }
        return Err(format!("Failed to extract archive: {}", err_str).into());
    }

    let manifest_path = temp_dir.join("manifest.json");
    if !manifest_path.exists() { return Err("manifest.json not found in archive!".into()); }
    let manifest_str = fs::read_to_string(&manifest_path)?;
    let manifest: Manifest = serde_json::from_str(&manifest_str)?;

    let mut output = format!("\x1b[1;36m\u{f019}  Deployment Plan {}\x1b[0m\n\x1b[90m────────────────\x1b[0m\n", if apply { "[APPLY]" } else { "[DRY RUN]" });

    for entry in &manifest.files {
        let target_str = entry.target.replace("~", &home.to_string_lossy());
        let target_path = PathBuf::from(&target_str);
        let alias_path = confy_dir.join(&entry.alias);
        let source_path = temp_dir.join(&entry.alias);

        output.push_str(&format!("\n\x1b[1mFile:\x1b[0m {}\n", entry.alias));
        output.push_str(&format!("\x1b[90mTarget:\x1b[0m {}\n", target_path.display()));

        if target_path.exists() {
            let target_hash = hash_file(&target_path).unwrap_or(0);
            let source_hash = hash_file(&source_path).unwrap_or(1);
            if target_hash == source_hash {
                output.push_str("\x1b[90m[SKIP] Identical content. No backup needed.\x1b[0m\n");
                continue;
            }
            output.push_str("\x1b[33m[UPDATE] Content differs.\x1b[0m\n");
            if apply {
                let out = Command::new("date").arg("+%Y%m%d_%H%M%S").stdout(Stdio::piped()).stderr(Stdio::null()).output()?;
                let ts = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let backup_path = format!("{}.bak_{}", target_path.display(), ts);
                fs::rename(&target_path, &backup_path)?;
                output.push_str(&format!("\x1b[90m[+] Backed up to {}\x1b[0m\n", backup_path));
            }
        } else {
            output.push_str("\x1b[32m[NEW] Will be created.\x1b[0m\n");
        }

        if apply {
            if let Some(parent) = target_path.parent() { fs::create_dir_all(parent)?; }
            fs::rename(&source_path, &target_path)?;
            if alias_path.exists() { fs::remove_file(&alias_path)?; }
            symlink(&target_path, &alias_path)?;
            output.push_str("\x1b[32m[✓] Deployed & Linked\x1b[0m\n");
        }
    }

    if !apply {
        output.push_str("\n\x1b[1;33m⚠️ Dry run complete. Run with '--apply' to execute.\x1b[0m\n");
        output.push_str("\n\x1b[90m─── (Press Esc to return to preview) ───\x1b[0m\n");
    } else {
        output.push_str("\n\x1b[1;32m✅ Deployment successful!\x1b[0m\n");
        fs::remove_dir_all(&temp_dir)?;
    }
    Ok(output)
}

fn generate_diff(hist_path: &Path, current_path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let out = Command::new("diff").arg("--color=always").arg("-u").arg(hist_path).arg(current_path)
        .stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;
    let mut output = String::new();
    if out.stdout.is_empty() && out.stderr.is_empty() {
        output.push_str("\x1b[32mFiles are identical. No differences found.\x1b[0m\n");
    } else {
        output.push_str(&String::from_utf8_lossy(&out.stdout));
    }
    Ok(output)
}

fn git_export() -> Result<String, Box<dyn std::error::Error>> {
    let home = dirs::home_dir().expect("No home dir");
    let confy_dir = home.join(".confy");
    Command::new("git").arg("init").current_dir(&confy_dir).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
    Command::new("git").arg("add").arg("-A").current_dir(&confy_dir).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
    let out = Command::new("git").arg("commit").arg("-m").arg("Confy snapshot export")
        .current_dir(&confy_dir).stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;
    if out.status.success() { Ok("✅ Git export successful. Committed to ~/.confy.".to_string()) }
    else {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("nothing to commit") { Ok("✅ Nothing to commit. Git repository is up to date.".to_string()) }
        else { Err(format!("Git commit failed: {}", err).into()) }
    }
}

// --- CLI & MAIN LOOP ---
fn run_tui() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut link_path = None; let mut link_name = None; let mut open_target = None; let mut version_limit = 4;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-l" => { if i + 1 < args.len() { link_path = Some(args[i + 1].clone()); i += 1; } }
            "-n" => { if i + 1 < args.len() { link_name = Some(args[i + 1].clone()); i += 1; } }
            "-o" => { if i + 1 < args.len() { open_target = Some(args[i + 1].clone()); i += 1; } }
            "-v" => { if i + 1 < args.len() { if let Ok(v) = args[i + 1].parse::<usize>() { version_limit = v; } i += 1; } }
            "-s" => {
                let pass = if i + 1 < args.len() { args[i + 1].clone() } else { String::new() };
                let home = dirs::home_dir().expect("Could not find home directory");
                let pass_file = home.join(".confy_sudo");
                if pass.is_empty() { let _ = fs::remove_file(&pass_file); println!("Deleted saved sudo password."); }
                else {
                    print!("WARNING: Saving sudo password in plaintext is dangerous. Continue? (y/N): ");
                    std::io::stdout().flush().unwrap();
                    let mut confirm = String::new(); std::io::stdin().read_line(&mut confirm).unwrap();
                    if confirm.trim().to_lowercase() == "y" {
                        if fs::write(&pass_file, &pass).is_ok() { let _ = fs::set_permissions(&pass_file, PermissionsExt::from_mode(0o600)); println!("Saved sudo password."); }
                        else { println!("Failed to save password."); }
                    } else { println!("Aborted."); }
                }
                std::process::exit(0);
            }
            "-h" | "--help" | "-help" => {
                println!("Usage: confy [options]\n  (no args)             Launch TUI\n  -l <path> -n <name>   Create a symlink (creates parent folders)\n  -o <file/folder>      Open file or expand folder in TUI\n  -e <file/folder>      Open file/folder in selected editor (--tui to force prompt)\n  -v <number>           Set version limit (default 4)\n  -s <password>         Save sudo password (DANGEROUS)\n  -s \"\"                Delete saved sudo password\n  -d <archive>          Deploy archive (Dry Run)\n  -d <archive> --apply  Deploy archive (Execute)\n  doctor                Run system health check\n  discover              Discover and link common dotfiles\n  git-export            Export ~/.confy to a git repo\n  -h, --help            Show this help menu");
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    if let Some(p) = link_path {
        let source_path = expand_tilde(&p);
        if !source_path.exists() { eprintln!("Error: Path not found: {}", p); std::process::exit(1); }
        let name = link_name.unwrap_or_else(|| source_path.file_name().unwrap_or_default().to_string_lossy().to_string());
        let home = dirs::home_dir().expect("Could not find home directory");
        let dest = home.join(".confy").join(&name);
        if let Some(parent) = dest.parent() { if let Err(e) = fs::create_dir_all(parent) { eprintln!("Failed to create folder: {}", e); std::process::exit(1); } }
        if dest.exists() { let _ = fs::remove_file(&dest); }
        match symlink(&source_path, &dest) {
            Ok(_) => println!("Successfully linked {} -> {}", name, source_path.display()),
            Err(e) => eprintln!("Failed to link: {}", e),
        }
        std::process::exit(0);
    }

    let cli_target = if let Some(t) = open_target {
        let home = dirs::home_dir().expect("Could not find home directory");
        let confy_dir = home.join(".confy");
        let target_path = expand_tilde(&t);
        let resolved_path = if target_path.is_absolute() && target_path.exists() { target_path } else { confy_dir.join(&t) };
        if !resolved_path.exists() { eprintln!("Error: Target not found: {}", t); std::process::exit(1); }
        if resolved_path.is_file() {
            if matches!(detect_kind(&resolved_path), PreviewKind::Image | PreviewKind::Video) {
                let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                let _ = Command::new(opener).arg(&resolved_path).stdout(Stdio::null()).stderr(Stdio::null()).status();
            } else { open_editor(&resolved_path, &select_editor().unwrap_or_else(|_| "vi".into())); }
            std::process::exit(0);
        }
        Some(resolved_path.file_name().unwrap_or_default().to_string_lossy().to_string())
    } else { None };

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(version_limit);
    let theme = Theme::new();

    if let Some(open_name) = &cli_target {
        if let Some(idx) = app.nodes.iter().position(|n| n.path.file_name().unwrap_or_default().to_string_lossy() == *open_name) {
            app.state.select(Some(idx));
            if app.nodes.get(idx).map(|n| n.is_dir).unwrap_or(false) {
                app.expand_node(idx);
                if app.nodes.len() > idx + 1 { app.state.select(Some(idx + 1)); app.update_preview(); }
            }
        }
    }

    loop {
        terminal.draw(|f| {
            let main_layout = Layout::default().direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref()).split(f.area());
            let chunks = Layout::default().direction(Direction::Horizontal)
                .constraints([Constraint::Length(45), Constraint::Min(45)].as_ref()).split(main_layout[0]);

            let display_nodes = app.current_nodes();
            let items: Vec<ListItem> = display_nodes.iter().map(|node| {
                let name = node.path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let size = fs::metadata(&node.path).map(|m| format_size(m.len())).unwrap_or_default();
                let is_selected = app.selected_nodes.contains(&node.path);
                let is_bookmarked = if let Ok(rel) = node.path.strip_prefix(&app.confy_dir) { app.state_data.bookmarks.contains(&rel.to_string_lossy().to_string()) } else { false };
                let icon = if is_selected { "✓" } else if is_bookmarked { "★" }
                else if node.is_dir { if node.expanded { "\u{f07c}" } else { "\u{f07b}" } } else { get_icon(&node.path) };
                let indent = "  ".repeat(node.depth);
                let name_spans: Vec<Span> = if app.input_mode == InputMode::Search && !app.input.is_empty() {
                    highlight_name(&name, &app.input, Style::default().fg(theme.accent), Style::default().fg(theme.text))
                } else { vec![Span::styled(name, Style::default().fg(theme.text))] };
                let mut spans_vec = Vec::new();
                spans_vec.push(Span::raw(format!("{}{} ", indent, icon)));
                spans_vec.extend(name_spans.into_iter());
                spans_vec.push(Span::raw(" "));
                spans_vec.push(Span::styled(format!("{:>7}", size), Style::default().fg(theme.muted)));
                ListItem::new(Line::from(spans_vec))
            }).collect();

            let list_title = if app.input_mode == InputMode::Search { format!(" Search: {} ", app.input) }
            else { format!(" ~/.confy ({} items) ", display_nodes.len()) };
            let list = List::new(items).block(
                Block::default().borders(Borders::ALL).border_type(BorderType::Thick)
                    .border_style(Style::default().fg(theme.border))
                    .title(Span::styled(list_title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text)))
            ).highlight_symbol("▶ ").highlight_style(theme.highlight);
            f.render_stateful_widget(list, chunks[0], &mut app.state);

            let preview_title = if app.input_mode == InputMode::VersionSelect { " Version History (Enter=Restore, d=Diff, Esc=Cancel) " }
            else if app.preview_pinned { " Preview (Esc=return) " } else { " Preview " };
            let preview_block = Block::default().borders(Borders::ALL).border_type(BorderType::Thick)
                .border_style(Style::default().fg(theme.border))
                .title(Span::styled(preview_title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
            f.render_widget(preview_block.clone(), chunks[1]);
            let inner_area = chunks[1].inner(Margin::new(1, 1));

            if app.input_mode == InputMode::VersionSelect {
                let mut items: Vec<ListItem> = Vec::new();
                for (i, (date, _)) in app.available_versions.iter().enumerate() {
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled(format!(" {} ", date), Style::default().fg(theme.text)),
                        Span::styled("●", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                    ])));
                    if i < app.available_versions.len() - 1 { items.push(ListItem::new(Line::from(vec![ Span::raw("       │") ]))); }
                }
                let version_list = List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight);
                f.render_stateful_widget(version_list, inner_area, &mut app.version_state);
            } else {
                let preview_text = app.preview_text.as_slice().into_text().unwrap_or_default();
                let preview = Paragraph::new(preview_text).alignment(Alignment::Left).wrap(Wrap { trim: false });
                f.render_widget(preview, inner_area);
            }

            if app.input_mode == InputMode::Help {
                let area = centered_rect(65, 60, f.area());
                f.render_widget(Clear, area);
                let block = Block::default().borders(Borders::ALL).border_type(BorderType::Thick)
                    .border_style(Style::default().fg(theme.accent))
                    .title(Span::styled(" ฅ^•ﻌ•^ฅ Help Menu (Esc to close) ", Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
                let help_text = vec![
                    Line::from(vec![Span::styled(" [j/k] or [↑/↓] ", Style::default().fg(theme.accent)), Span::raw(" Navigate")]),
                    Line::from(vec![Span::styled(" [Enter]       ", Style::default().fg(theme.accent)), Span::raw("  Edit File/Folder (Selects Editor)")]),
                    Line::from(vec![Span::styled(" [Right/Left]  ", Style::default().fg(theme.accent)), Span::raw("  Expand/Collapse Folder")]),
                    Line::from(vec![Span::styled(" [Space]       ", Style::default().fg(theme.accent)), Span::raw("  Select file for archiving")]),
                    Line::from(vec![Span::styled(" [V]           ", Style::default().fg(theme.accent)), Span::raw("  Visual Select Mode (hold & move)")]),
                    Line::from(vec![Span::styled(" [z]           ", Style::default().fg(theme.accent)), Span::raw("  Archive selected files (zip)")]),
                    Line::from(vec![Span::styled(" [D] on .zip   ", Style::default().fg(theme.accent)), Span::raw("  Deploy archive (Dry Run)")]),
                    Line::from(vec![Span::styled(" [a]           ", Style::default().fg(theme.accent)), Span::raw("  Add Symlink (Tab=fzf complete)")]),
                    Line::from(vec![Span::styled(" [f]           ", Style::default().fg(theme.accent)), Span::raw("  Create Folder")]),
                    Line::from(vec![Span::styled(" [c] / [p]     ", Style::default().fg(theme.accent)), Span::raw("  Cut / Paste")]),
                    Line::from(vec![Span::styled(" [y] / [p]     ", Style::default().fg(theme.accent)), Span::raw("  Yank (Copy) / Paste")]),
                    Line::from(vec![Span::styled(" [d]           ", Style::default().fg(theme.accent)), Span::raw("  Delete (press twice)")]),
                    Line::from(vec![Span::styled(" [r]           ", Style::default().fg(theme.accent)), Span::raw("  Rename (supports folders)")]),
                    Line::from(vec![Span::styled(" [v]           ", Style::default().fg(theme.accent)), Span::raw("  View File Versions")]),
                    Line::from(vec![Span::styled(" [!]           ", Style::default().fg(theme.accent)), Span::raw("  Shell Command on file/folder")]),
                    Line::from(vec![Span::styled(" [b]           ", Style::default().fg(theme.accent)), Span::raw("  Toggle Bookmark")]),
                    Line::from(vec![Span::styled(" [n]           ", Style::default().fg(theme.accent)), Span::raw("  Add/Edit Note for file")]),
                    Line::from(vec![Span::styled(" [R]           ", Style::default().fg(theme.accent)), Span::raw("  Jump to Recent File (MRU)")]),
                    Line::from(vec![Span::styled(" [/]           ", Style::default().fg(theme.accent)), Span::raw("  Search")]),
                    Line::from(vec![Span::styled(" [.]           ", Style::default().fg(theme.accent)), Span::raw("  Toggle Hidden Files")]),
                    Line::from(vec![Span::styled(" [q]           ", Style::default().fg(theme.accent)), Span::raw("  Quit")]),
                ];
                f.render_widget(Paragraph::new(help_text).block(block).alignment(Alignment::Left), area);
            }

            // --- SIMPLE STATIC BOTTOM BAR ---
            let (title, input_text) = match app.input_mode {
                InputMode::Normal => {
                    let cat = if app.cut_source.is_some() { "ฅ(^•x•^)✂️" }
                    else if app.yank_source.is_some() { "ฅ(^•ω•^)📋" }
                    else { "ฅ^•ﻌ•^ฅ" };
                    let text = if app.status.is_empty() { format!(" {} [?] Help | [j/k] Nav | [Enter] Edit | [←/→] Expand | [q] Quit ", cat) }
                    else { format!(" {} {}", cat, app.status) };
                    (" Status ", text)
                }
                InputMode::AddingPath => (" Add Path (Tab=complete, Enter=next, Esc=cancel) ", if app.status.is_empty() { app.input.clone() } else { app.status.clone() }),
                InputMode::AddingAlias => (" Add Alias (Path=folder/sub, Enter=save, Esc=cancel) ", if app.status.is_empty() { app.input.clone() } else { app.status.clone() }),
                InputMode::AddingDir => (" Folder Name (Enter=save, Esc=cancel) ", if app.status.is_empty() { app.input.clone() } else { app.status.clone() }),
                InputMode::AddingNote => (" Add Note (Enter=save, Esc=cancel) ", if app.status.is_empty() { app.input.clone() } else { app.status.clone() }),
                InputMode::ArchivePrompt => (" 🔒 enter passwd (esc=cancel) ", "*".repeat(app.input.len())),
                InputMode::Renaming => (" Rename (Path=folder/sub, Enter=save, Esc=cancel) ", app.input.clone()),
                InputMode::Search => (" Search (Enter=Open, Esc=cancel) ", app.input.clone()),
                InputMode::VersionSelect => (" Version Mode (Enter=Restore, d=Diff, Esc=Cancel) ", "".to_string()),
                InputMode::SudoPrompt => (" 🔒 Sudo Password (Empty=View, Enter=submit, Esc=cancel) ", "*".repeat(app.input.len())),
                InputMode::Shell => (" sh> (Enter=run, Esc=cancel) ", app.input.clone()),
                InputMode::Help => (" Help (Esc=close) ", "".to_string()),
                InputMode::VisualSelect => (" Visual Select (j/k to select, z to archive) ", "".to_string()),
                InputMode::DeployConfirm => (" Deploy (y=apply, n=cancel) ", "".to_string()),
            };
            let input_block = Block::default().borders(Borders::ALL).border_type(BorderType::Thick)
                .border_style(Style::default().fg(theme.border))
                .title(Span::styled(title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
            f.render_widget(Paragraph::new(input_text).block(input_block), main_layout[1]);
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press { continue; }
                    match app.input_mode {
                        InputMode::Normal => {
                            if !app.status.is_empty() && !app.status_sticky && key.code != KeyCode::Char('q') && key.code != KeyCode::Esc {
                                app.status.clear(); app.last_status_time = None;
                            }
                            match key.code {
                                KeyCode::Esc => {
                                    if app.preview_pinned { app.preview_pinned = false; app.set_status("Preview restored."); app.update_preview(); }
                                    else if app.cut_source.is_some() || app.yank_source.is_some() || app.pending_delete.is_some() {
                                        app.cut_source = None; app.yank_source = None; app.pending_delete = None; app.status_sticky = false;
                                        app.set_status("Operation cancelled.");
                                    }
                                }
                                KeyCode::Char('q') => break,
                                KeyCode::Down | KeyCode::Char('j') => app.next(),
                                KeyCode::Up | KeyCode::Char('k') => app.previous(),
                                KeyCode::Enter => { app.edit_selected(); terminal.clear()?; app.update_preview(); }
                                KeyCode::Right => { if let Some(i) = app.state.selected() { if let Some(node) = app.current_nodes().get(i).cloned() { if node.is_dir && !node.expanded { app.expand_node(i); } } } }
                                KeyCode::Left => { if let Some(i) = app.state.selected() { if let Some(node) = app.current_nodes().get(i).cloned() { if node.is_dir && node.expanded { app.collapse_node(i); } } } }
                                KeyCode::Char('a') => app.begin_add(),
                                KeyCode::Char('f') => app.begin_add_folder(),
                                KeyCode::Char('c') => app.cut_item(),
                                KeyCode::Char('y') => app.yank_item(),
                                KeyCode::Char('p') => app.paste_item(),
                                KeyCode::Char('v') => app.enter_version_mode(),
                                KeyCode::Char('d') => app.mark_or_delete(),
                                KeyCode::Char('D') => app.try_deploy(),
                                KeyCode::Char('r') => app.begin_rename(),
                                KeyCode::Char('b') => app.toggle_bookmark(),
                                KeyCode::Char('n') => app.begin_add_note(),
                                KeyCode::Char('R') => app.jump_to_recent(),
                                KeyCode::Char(' ') => app.toggle_selection(),
                                KeyCode::Char('V') => { app.input_mode = InputMode::VisualSelect; app.set_status("Visual Select: Move j/k to select, z to archive, Esc to cancel."); }
                                KeyCode::Char('z') => {
                                    if app.selected_nodes.is_empty() { app.set_status("No files selected."); }
                                    else { app.input_mode = InputMode::ArchivePrompt; app.deploy_target = None; app.input.clear(); }
                                }
                                KeyCode::Char('!') => { app.input_mode = InputMode::Shell; app.input.clear(); }
                                KeyCode::Char('/') => app.start_search(),
                                KeyCode::Char('?') => app.input_mode = InputMode::Help,
                                KeyCode::Char('.') => { app.show_hidden = !app.show_hidden; app.refresh_items(); }
                                _ => {}
                            }
                        }
                        InputMode::VisualSelect => {
                            match key.code {
                                KeyCode::Down | KeyCode::Char('j') => {
                                    app.next();
                                    if let Some(i) = app.state.selected() { if let Some(node) = app.current_nodes().get(i).cloned() {
                                        if app.selected_nodes.contains(&node.path) { app.selected_nodes.remove(&node.path); } else { app.selected_nodes.insert(node.path.clone()); }
                                    }}
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    app.previous();
                                    if let Some(i) = app.state.selected() { if let Some(node) = app.current_nodes().get(i).cloned() {
                                        if app.selected_nodes.contains(&node.path) { app.selected_nodes.remove(&node.path); } else { app.selected_nodes.insert(node.path.clone()); }
                                    }}
                                }
                                KeyCode::Esc => { app.input_mode = InputMode::Normal; app.selected_nodes.clear(); app.set_status("Visual Select cancelled."); }
                                KeyCode::Char('V') => { app.input_mode = InputMode::Normal; app.set_status("Exited visual select."); }
                                KeyCode::Char('z') => { if app.selected_nodes.is_empty() { app.set_status("No files selected."); } else { app.input_mode = InputMode::ArchivePrompt; app.deploy_target = None; app.input.clear(); } }
                                KeyCode::Char(' ') => app.toggle_selection(),
                                _ => {}
                            }
                        }
                        InputMode::AddingPath => match key.code {
                            KeyCode::Enter => app.submit_add_step(),
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Tab => { app.launch_fzf(); terminal.clear()?; }
                            KeyCode::Backspace => { if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; } else { app.input.pop(); } }
                            KeyCode::Char(c) => { if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; } app.input.push(c); }
                            _ => {}
                        },
                        InputMode::AddingAlias => match key.code {
                            KeyCode::Enter => app.submit_add_step(),
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Backspace => { if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; } else { app.input.pop(); } }
                            KeyCode::Char(c) => { if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; } app.input.push(c); }
                            _ => {}
                        },
                        InputMode::AddingDir => match key.code {
                            KeyCode::Enter => app.submit_add_folder(),
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Backspace => { if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; } else { app.input.pop(); } }
                            KeyCode::Char(c) => { if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; } app.input.push(c); }
                            _ => {}
                        },
                        InputMode::AddingNote => match key.code {
                            KeyCode::Enter => app.submit_note(),
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Backspace => { if !app.status.is_empty() { app.status.clear(); } else { app.input.pop(); } }
                            KeyCode::Char(c) => { if !app.status.is_empty() { app.status.clear(); } app.input.push(c); }
                            _ => {}
                        },
                        InputMode::ArchivePrompt => match key.code {
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Enter => {
                                let pass = app.input.clone();
                                if let Some(target_zip) = &app.deploy_target.clone() {
                                    app.preview_pinned = true; app.preview_text.clear();
                                    match deploy_archive(target_zip, false, Some(&pass)) {
                                        Ok(output) => {
                                            app.preview_text = output.into_bytes();
                                            app.deploy_pass = Some(pass);
                                            app.input_mode = InputMode::DeployConfirm;
                                            app.set_sticky_status("Dry run complete. Press 'y' to apply, 'n' to cancel.");
                                        }
                                        Err(e) => {
                                            app.set_status(format!("Deploy failed: {}", e));
                                            app.preview_pinned = false;
                                        }
                                    }
                                } else {
                                    app.archive_selected(&pass);
                                }
                                app.input.clear();
                            }
                            KeyCode::Backspace => { app.input.pop(); }
                            KeyCode::Char(c) => { app.input.push(c); }
                            _ => {}
                        },
                        InputMode::DeployConfirm => match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Some(path) = &app.deploy_target.clone() {
                                    let pass = app.deploy_pass.clone();
                                    app.preview_text.clear();
                                    match deploy_archive(path, true, pass.as_deref()) {
                                        Ok(output) => {
                                            app.preview_text = output.into_bytes();
                                            app.set_sticky_status("Deployment applied! (Esc to return)");
                                        }
                                        Err(e) => {
                                            app.set_status(format!("Apply failed: {}", e));
                                        }
                                    }
                                }
                                app.input_mode = InputMode::Normal;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                app.input_mode = InputMode::Normal;
                                app.set_status("Deploy cancelled.");
                            }
                            _ => {}
                        },
                        InputMode::Renaming => match key.code {
                            KeyCode::Enter => app.submit_rename(),
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Backspace => { app.input.pop(); }
                            KeyCode::Char(c) => { app.input.push(c); }
                            _ => {}
                        },
                        InputMode::Search => {
                            let input_changed = match key.code {
                                KeyCode::Esc => { app.cancel_input(); false }
                                KeyCode::Enter => { app.edit_selected(); terminal.clear()?; app.cancel_input(); false }
                                KeyCode::Down => { app.next(); false }
                                KeyCode::Up => { app.previous(); false }
                                KeyCode::Backspace => { app.input.pop(); true }
                                KeyCode::Char(c) => { app.input.push(c); true }
                                _ => false,
                            };
                            if input_changed { app.apply_filter(); app.state.select(Some(0)); app.update_preview(); }
                        },
                        InputMode::VersionSelect => match key.code {
                            KeyCode::Esc => app.exit_version_mode(),
                            KeyCode::Enter => { app.restore_version(); terminal.clear()?; }
                            KeyCode::Down | KeyCode::Char('j') => app.next_version(),
                            KeyCode::Up | KeyCode::Char('k') => app.previous_version(),
                            KeyCode::Char('d') => app.show_diff(),
                            _ => {}
                        },
                        InputMode::SudoPrompt => match key.code {
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Enter => { app.submit_sudo_password(); if app.input_mode == InputMode::Normal { terminal.clear()?; app.update_preview(); } }
                            KeyCode::Backspace => { app.input.pop(); app.status.clear(); }
                            KeyCode::Char(c) => { app.input.push(c); app.status.clear(); }
                            _ => {}
                        },
                        InputMode::Shell => match key.code {
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Enter => { app.execute_shell(); terminal.clear()?; }
                            KeyCode::Backspace => { app.input.pop(); }
                            KeyCode::Char(c) => { app.input.push(c); }
                            _ => {}
                        },
                        InputMode::Help => match key.code {
                            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => app.input_mode = InputMode::Normal,
                            _ => {}
                        },
                    }
                }
                Event::Mouse(mouse) => {
                    if app.input_mode == InputMode::Normal || app.input_mode == InputMode::VisualSelect {
                        match mouse.kind { MouseEventKind::ScrollDown => app.next(), MouseEventKind::ScrollUp => app.previous(), _ => {} }
                    }
                }
                _ => {}
            }
        }
        app.check_fs_changes();
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), crossterm::event::DisableMouseCapture, LeaveAlternateScreen)?;
    Ok(())
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Percentage((100 - percent_y) / 2), Constraint::Percentage(percent_y), Constraint::Percentage((100 - percent_y) / 2)].as_ref())
        .split(r);
    Layout::default().direction(Direction::Horizontal)
        .constraints([Constraint::Percentage((100 - percent_x) / 2), Constraint::Percentage(percent_x), Constraint::Percentage((100 - percent_x) / 2)].as_ref())
        .split(popup_layout[1])[1]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    // --- CLI: Edit Mode (-e) ---
    if args.len() > 1 && (args[1] == "-e" || args[1] == "--edit") {
        let path = if args.len() > 2 && !args[2].starts_with('-') { Some(PathBuf::from(&args[2])) } else { None };
        let force_tui = args.iter().any(|a| a == "--tui");
        if force_tui { let _ = fs::remove_file(dirs::home_dir().unwrap().join(".confy_editor")); }
        let editor = select_editor()?;
        if let Some(p) = path { open_editor(&p, &editor); } else { eprintln!("Usage: confy -e <file/folder> [--tui]"); }
        return Ok(());
    }

    // --- CLI: Open Mode (-o) ---
    if args.len() > 1 && (args[1] == "-o" || args[1] == "--open") {
        let path = if args.len() > 2 { PathBuf::from(&args[2]) } else { dirs::home_dir().unwrap().join(".confy") };
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        Command::new(opener).arg(&path).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
        return Ok(());
    }

    // --- CLI: Deploy Mode (-d) ---
    if args.len() > 1 && (args[1] == "-d" || args[1] == "--deploy") {
        if args.len() < 3 { eprintln!("Usage: confy -d <archive.zip> [--apply] [--pass <password>]"); std::process::exit(1); }
        let archive_path = PathBuf::from(&args[2]);
        let apply = args.iter().any(|a| a == "--apply");
        let password = if let Some(idx) = args.iter().position(|a| a == "--pass") { if idx + 1 < args.len() { Some(args[idx + 1].clone()) } else { None } } else { None };
        match deploy_archive(&archive_path, apply, password.as_deref()) {
            Ok(output) => println!("{}", output),
            Err(e) => eprintln!("Deploy failed: {}", e),
        }
        return Ok(());
    }

    // --- CLI: Doctor & Discover & Git ---
    if args.iter().any(|a| a == "doctor") {
        println!("🩺 Running Confy Doctor...");
        let home = dirs::home_dir().expect("No home dir");
        let confy_dir = home.join(".confy");
        println!("\nDependencies:");
        let deps = ["bat", "chafa", "fzf", "ffmpeg", "zip", "unzip", "diff"];
        for dep in &deps {
            let status = Command::new("sh").arg("-c").arg(format!("command -v {}", dep)).stdout(Stdio::null()).stderr(Stdio::null()).status();
            if status.map(|s| s.success()).unwrap_or(false) { println!("  [✓] {} installed", dep); } else { println!("  [✗] {} MISSING", dep); }
        }
        println!("\nSymlinks:");
        if let Ok(entries) = fs::read_dir(&confy_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_symlink = path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
                if is_symlink { if fs::metadata(&path).is_err() { println!("  [✗] BROKEN: {}", path.display()); } else { println!("  [✓] OK: {}", path.display()); } }
            }
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "discover") {
        let home = dirs::home_dir().expect("No home dir");
        let confy_dir = home.join(".confy");
        fs::create_dir_all(&confy_dir)?;
        let known_paths = [
            "~/.bashrc", "~/.zshrc", "~/.config/fish/config.fish", "~/.gitconfig", "~/.tmux.conf", "~/.config/starship.toml", "~/.config/fastfetch",
            "~/.config/kitty", "~/.config/alacritty", "~/.config/wezterm", "~/.config/ghostty", "~/.config/nvim", "~/.config/helix",
            "~/.config/hypr", "~/.config/sway", "~/.config/niri", "~/.config/waybar", "~/.config/rofi", "~/.config/dunst"
        ];
        println!("🔍 Discovering famous dotfiles...");
        for path_str in &known_paths {
            let expanded = if path_str.starts_with("~/") { home.join(&path_str[2..]) } else { PathBuf::from(path_str) };
            if expanded.exists() {
                let alias = expanded.file_name().unwrap().to_string_lossy().to_string();
                let dest = confy_dir.join(&alias);
                if !dest.exists() { symlink(&expanded, &dest)?; println!("  [+] Linked {} -> {}", alias, expanded.display()); }
                else { println!("  [✓] Already linked: {}", alias); }
            }
        }
        println!("✅ Discovery complete.");
        return Ok(());
    }
    if args.iter().any(|a| a == "git-export") {
        match git_export() {
            Ok(msg) => println!("{}", msg),
            Err(e) => eprintln!("Git export failed: {}", e),
        }
        return Ok(());
    }

    // --- TUI Mode ---
    run_tui()?;
    Ok(())
}
