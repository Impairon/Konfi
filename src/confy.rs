use ansi_to_tui::IntoText;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

// --- THEME ---
// Detects terminal capabilities (Truecolor vs ANSI 256) to use Catppuccin Mocha colors
// if supported, gracefully falling back to standard ANSI colors.
struct Theme {
    border: Color,
    text: Color,
    muted: Color,
    accent: Color,
    highlight: Style,
}

impl Theme {
    fn new() -> Self {
        let term = env::var("TERM").unwrap_or_default().to_lowercase();
        let colorterm = env::var("COLORTERM").unwrap_or_default().to_lowercase();
        let truecolor = colorterm.contains("truecolor") || colorterm.contains("24bit");

        if truecolor {
            Self {
                border: Color::Rgb(69, 71, 90),
                text: Color::Rgb(205, 214, 244),
                muted: Color::Rgb(108, 112, 134),
                accent: Color::Rgb(137, 180, 250),
                highlight: Style::default()
                    .fg(Color::Rgb(205, 214, 244))
                    .bg(Color::Rgb(49, 50, 68))
                    .add_modifier(Modifier::BOLD),
            }
        } else if term.contains("256color") {
            Self {
                border: Color::DarkGray,
                text: Color::White,
                muted: Color::Gray,
                accent: Color::Cyan,
                highlight: Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD),
            }
        } else {
            Self {
                border: Color::Gray,
                text: Color::White,
                muted: Color::DarkGray,
                accent: Color::Blue,
                highlight: Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            }
        }
    }
}

// --- DATA STRUCTURES ---

// Represents a single file or folder in our tree view.
#[derive(Clone)]
struct Node {
    path: PathBuf,
    depth: usize,    // How far indented this should be (0 for root, 1 for child, etc.)
    is_dir: bool,
    expanded: bool,  // Only applies to directories
}

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
enum InputMode { Normal, AddingPath, AddingAlias, Renaming, Search }

struct App {
    // The flat list of visible nodes. If a folder is expanded, its children are inserted
    // right after it in this Vec. If collapsed, they are removed.
    nodes: Vec<Node>,
    filtered_nodes: Vec<Node>,
    state: ListState,
    preview_text: Vec<u8>,
    input_mode: InputMode,
    input: String,
    add_source_path: String,
    status: String,
    configz_dir: PathBuf,
    last_dir_mtime: Option<SystemTime>,
    last_file_mtime: Option<SystemTime>,
    pending_delete: Option<PathBuf>,
    last_status_time: Option<SystemTime>,
}

impl App {
    fn new() -> Self {
        let home = dirs::home_dir().expect("Could not find home directory");
        let configz_dir = home.join(".configz");
        fs::create_dir_all(&configz_dir).expect("Failed to create ~/.configz");

        let mut state = ListState::default();
        state.select(Some(0));

        let mut app = App {
            nodes: Vec::new(),
            filtered_nodes: Vec::new(),
            state,
            preview_text: Vec::new(),
            input_mode: InputMode::Normal,
            input: String::new(),
            add_source_path: String::new(),
            status: String::new(),
            configz_dir,
            last_dir_mtime: None,
            last_file_mtime: None,
            pending_delete: None,
            last_status_time: None,
        };
        app.refresh_items();
        app
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.last_status_time = Some(SystemTime::now());
    }

    // Reads ~/.configz and builds the top-level (depth 0) nodes
    fn refresh_items(&mut self) {
        // 1. Save which directories were currently expanded
        let mut expanded_paths = std::collections::HashSet::new();
        for node in &self.nodes {
            if node.is_dir && node.expanded {
                expanded_paths.insert(node.path.clone());
            }
        }

        // 2. Rebuild the root nodes from scratch
        self.nodes = match fs::read_dir(&self.configz_dir) {
            Ok(dir) => {
                let mut roots: Vec<Node> = dir.filter_map(|e| e.ok())
                    .map(|e| {
                        let path = e.path();
                        let is_dir = fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
                        Node { path, depth: 0, is_dir, expanded: false }
                    })
                    .collect();
                sort_nodes(&mut roots);
                roots
            }
            Err(_) => Vec::new(),
        };

        // 3. Re-expand the directories that were open before the refresh
        let mut i = 0;
        while i < self.nodes.len() {
            if self.nodes[i].is_dir && expanded_paths.contains(&self.nodes[i].path) {
                let path = self.nodes[i].path.clone();
                let depth = self.nodes[i].depth + 1;
                self.nodes[i].expanded = true;

                if let Ok(entries) = fs::read_dir(&path) {
                    let mut children: Vec<Node> = entries.flatten().map(|e| {
                        let p = e.path();
                        let is_dir = fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
                        Node { path: p, depth, is_dir, expanded: false }
                    }).collect();
                    sort_nodes(&mut children);

                    for (offset, child) in children.into_iter().enumerate() {
                        self.nodes.insert(i + 1 + offset, child);
                    }
                }
            }
            i += 1;
        }

        self.apply_filter();

        let current = self.state.selected().unwrap_or(0);
        if !self.filtered_nodes.is_empty() {
            self.state.select(Some(current.min(self.filtered_nodes.len() - 1)));
        } else {
            self.state.select(None);
        }

        // Calculate the maximum mtime across the root AND all expanded folders
        // to prevent infinite refresh loops when a subdirectory changes.
        let mut max_dir_mtime = None;
        if let Ok(meta) = fs::metadata(&self.configz_dir) {
            max_dir_mtime = meta.modified().ok();
        }
        for node in &self.nodes {
            if node.is_dir && node.expanded {
                if let Ok(meta) = fs::metadata(&node.path) {
                    if let Ok(mtime) = meta.modified() {
                        if Some(mtime) > max_dir_mtime {
                            max_dir_mtime = Some(mtime);
                        }
                    }
                }
            }
        }
        self.last_dir_mtime = max_dir_mtime;

        self.update_preview();
    }

    // Search logic: recursively searches the entire tree for files matching the query
    fn apply_filter(&mut self) {
        self.pending_delete = None;
        if self.input.is_empty() {
            self.filtered_nodes = self.nodes.clone();
        } else {
            let query = self.input.to_lowercase();
            let mut filtered = Vec::new();
            for node in &self.nodes {
                search_recursive(node, &query, &mut filtered);
            }
            self.filtered_nodes = filtered;
        }
    }

    fn current_nodes(&self) -> &Vec<Node> {
        if self.input_mode == InputMode::Search { &self.filtered_nodes } else { &self.nodes }
    }

    fn current_nodes_mut(&mut self) -> &mut Vec<Node> {
        if self.input_mode == InputMode::Search { &mut self.filtered_nodes } else { &mut self.nodes }
    }

    // --- TREE EXPANSION LOGIC ---
    fn toggle_expand(&mut self) {
        if self.input_mode != InputMode::Normal { return; }
        let current_nodes = self.current_nodes().clone();
        if let Some(i) = self.state.selected() {
            if let Some(node) = current_nodes.get(i) {
                if node.is_dir {
                    if node.expanded { self.collapse_node(i); }
                    else { self.expand_node(i); }
                }
            }
        }
    }

    fn expand_node(&mut self, index: usize) {
        let nodes = self.current_nodes_mut();
        if let Some(node) = nodes.get_mut(index) {
            node.expanded = true;
            let path = node.path.clone();
            let depth = node.depth + 1;

            if let Ok(entries) = fs::read_dir(&path) {
                let mut children: Vec<Node> = entries.flatten().map(|e| {
                    let p = e.path();
                    let is_dir = fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
                    Node { path: p, depth, is_dir, expanded: false }
                }).collect();
                sort_nodes(&mut children);

                for (offset, child) in children.into_iter().enumerate() {
                    nodes.insert(index + 1 + offset, child);
                }
            }
        }
        self.update_preview();
    }

    fn collapse_node(&mut self, index: usize) {
        let nodes = self.current_nodes_mut();
        if let Some(node) = nodes.get_mut(index) {
            node.expanded = false;
            let parent_depth = node.depth;

            let mut remove_count = 0;
            for i in (index + 1)..nodes.len() {
                if nodes[i].depth > parent_depth { remove_count += 1; }
                else { break; }
            }
            nodes.drain(index + 1..index + 1 + remove_count);
        }
        self.update_preview();
    }

    // --- PREVIEW LOGIC ---
    fn update_preview(&mut self) {
        self.preview_text.clear();
        if let Some(i) = self.state.selected() {
            if let Some(node) = self.current_nodes().get(i).cloned() {
                let target_path = fs::canonicalize(&node.path).unwrap_or(node.path.clone());

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
                    self.preview_text = output.into_bytes();
                    return;
                }

                let (tcols, trows) = crossterm::terminal::size().unwrap_or((80, 24));
                let p_w = ((tcols as f32) * 0.6).max(10.0) as u16;
                let p_h = ((trows as f32) * 0.7).max(4.0) as u16;
                let size_arg = format!("{}x{}", p_w, p_h);

                match detect_kind(&target_path) {
                    PreviewKind::Image => {
                        let out = std::process::Command::new("chafa")
                            .arg("--format=symbols").arg("--view-size").arg(&size_arg)
                            .arg(&target_path).output();
                        let mut output = format!("\x1b[1;36m\u{f1c5}  Image Info\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\x1b[33m{}\x1b[0m\n", get_file_info(&target_path));
                        if let Ok(out) = out {
                            if out.status.success() && !out.stdout.is_empty() {
                                output.push_str("\n\x1b[90m─── Preview ───\x1b[0m\n");
                                output.push_str(&String::from_utf8_lossy(&out.stdout));
                            }
                        }
                        self.preview_text = output.into_bytes();
                    }
                    PreviewKind::Video => {
                        let info = get_video_meta(&target_path);
                        let mut output = format!("\x1b[1;36m\u{f03d}  Video Info\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\x1b[33m{}\x1b[0m\n", info);
                        if let Some(thumb) = extract_video_thumbnail_cached(&target_path) {
                            let out = std::process::Command::new("chafa")
                                .arg("--format=symbols").arg("--view-size").arg(&size_arg)
                                .arg(&thumb).output();
                            if let Ok(out) = out {
                                if out.status.success() && !out.stdout.is_empty() {
                                    output.push_str("\n\x1b[90m─── Thumbnail ───\x1b[0m\n");
                                    output.push_str(&String::from_utf8_lossy(&out.stdout));
                                }
                            }
                        }
                        self.preview_text = output.into_bytes();
                    }
                    PreviewKind::Text => {
                        let out = std::process::Command::new("bat")
                            .arg("--color=always").arg("--style=plain")
                            .arg("--paging=never").arg("--theme=ansi")
                            .arg(&target_path).output();

                        if let Ok(out) = out {
                            let stdout = String::from_utf8_lossy(&out.stdout);
                            let stderr = String::from_utf8_lossy(&out.stderr);
                            if stdout.contains("[bat warning]") || stderr.contains("[bat warning]") {
                                self.preview_text = "\x1b[1;33m\u{f071}  Binary File\x1b[0m\n\x1b[90m────────────────\x1b[0m\n\n\x1b[90mPreview not available for binary files.\x1b[0m".to_string().into_bytes();
                            } else if out.status.success() {
                                self.preview_text = out.stdout;
                            } else {
                                self.preview_text = read_small_fallback(&target_path);
                            }
                        } else {
                            self.preview_text = read_small_fallback(&target_path);
                        }
                    }
                }
            }
        }
    }

    fn check_fs_changes(&mut self) {
        if let Some(time) = self.last_status_time {
            if SystemTime::now().duration_since(time).unwrap_or_default().as_secs() >= 1 {
                self.status.clear();
                self.last_status_time = None;
            }
        }

        let mut max_dir_mtime = self.last_dir_mtime;
        let mut need_refresh = false;

        let expanded_paths: Vec<PathBuf> = self.nodes.iter()
            .filter(|n| n.is_dir && n.expanded)
            .map(|n| n.path.clone())
            .collect();

        let paths_to_check = std::iter::once(self.configz_dir.clone())
            .chain(expanded_paths.into_iter());

        for path in paths_to_check {
            if let Ok(meta) = fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    if Some(mtime) > max_dir_mtime {
                        max_dir_mtime = Some(mtime);
                        need_refresh = true;
                    }
                }
            }
        }

        if need_refresh {
            self.last_dir_mtime = max_dir_mtime;
            self.refresh_items();
            return;
        }

        if let Some(i) = self.state.selected() {
            if let Some(node) = self.current_nodes().get(i).cloned() {
                let target_path = fs::canonicalize(&node.path).unwrap_or(node.path.clone());
                if let Ok(meta) = fs::metadata(&target_path) {
                    if let Ok(mtime) = meta.modified() {
                        if Some(mtime) != self.last_file_mtime {
                            self.update_preview();
                        }
                    }
                }
            }
        }
    }

    // --- ADD / RENAME / DELETE LOGIC ---
    fn begin_add(&mut self) {
        self.input_mode = InputMode::AddingPath;
        self.input.clear();
        self.add_source_path.clear();
        self.status.clear();
    }

    fn submit_add_step(&mut self) {
        match self.input_mode {
            InputMode::AddingPath => {
                let p = self.input.trim().to_string();
                if p.is_empty() { self.set_status("Path cannot be empty"); return; }
                let source_path = expand_tilde(&p);
                if !source_path.exists() { self.set_status(format!("Path not found: {}", p)); return; }
                self.add_source_path = source_path.to_string_lossy().to_string();
                self.input.clear();
                self.input_mode = InputMode::AddingAlias;
            }
            InputMode::AddingAlias => {
                let alias = self.input.trim();
                if alias.is_empty() { self.set_status("Alias cannot be empty"); return; }
                if alias.contains('/') { self.set_status("Alias cannot contain '/'"); return; }
                let source = PathBuf::from(&self.add_source_path);
                let dest = self.configz_dir.join(alias);
                if dest.exists() { self.set_status(format!("Alias already exists: {}", alias)); return; }
                match symlink(&source, &dest) {
                    Ok(_) => {
                        self.set_status(format!("Added: {} -> {}", alias, source.display()));
                        self.input.clear();
                        self.add_source_path.clear();
                        self.input_mode = InputMode::Normal;
                        self.refresh_items();
                    }
                    Err(e) => self.set_status(format!("Failed to add link: {}", e)),
                }
            }
            _ => {}
        }
    }

    fn begin_rename(&mut self) {
        if let Some(i) = self.state.selected() {
            if let Some(node) = self.current_nodes().get(i).cloned() {
                self.add_source_path = node.path.to_string_lossy().to_string();
                self.input = node.path.file_name().unwrap_or_default().to_string_lossy().to_string();
                self.input_mode = InputMode::Renaming;
                self.status.clear();
            }
        }
    }

    fn submit_rename(&mut self) {
        let new_name = self.input.trim();
        if new_name.is_empty() || new_name.contains('/') { self.set_status("Invalid name"); return; }
        let old_path = PathBuf::from(&self.add_source_path);
        let parent = old_path.parent().unwrap_or(&self.configz_dir);
        let new_path = parent.join(new_name);
        if new_path.exists() && new_path != old_path { self.set_status("Name already exists"); return; }
        match fs::rename(&old_path, &new_path) {
            Ok(_) => self.set_status(format!("Renamed to {}", new_name)),
            Err(e) => self.set_status(format!("Rename failed: {}", e)),
        }
        self.input_mode = InputMode::Normal;
        self.input.clear();
        self.add_source_path.clear();
        self.refresh_items();
    }

    fn cancel_input(&mut self) {
        self.input_mode = InputMode::Normal;
        self.input.clear();
        self.add_source_path.clear();
        self.status.clear();
        self.pending_delete = None;
    }

    fn start_search(&mut self) {
        self.input_mode = InputMode::Search;
        self.input.clear();
        self.apply_filter();
        self.state.select(Some(0));
        self.update_preview();
    }

    fn mark_or_delete(&mut self) {
        if let Some(i) = self.state.selected() {
            if let Some(node) = self.current_nodes().get(i).cloned() {
                if let Some(pending) = &self.pending_delete {
                    if pending == &node.path {
                        let is_symlink = node.path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
                        let result = if is_symlink {
                            fs::remove_file(&node.path)
                        } else if node.is_dir {
                            fs::remove_dir_all(&node.path)
                        } else {
                            fs::remove_file(&node.path)
                        };

                        match result {
                            Ok(_) => {
                                self.set_status(format!("Deleted: {}", node.path.file_name().unwrap_or_default().to_string_lossy()));
                                self.pending_delete = None;
                                self.refresh_items();
                                let len = self.current_nodes().len();
                                if len > 0 { self.state.select(Some(i.min(len - 1))); }
                                self.update_preview();
                            }
                            Err(e) => {
                                self.set_status(format!("Failed to delete: {}", e));
                                self.pending_delete = None;
                            }
                        }
                    } else {
                        self.pending_delete = Some(node.path.clone());
                        self.set_status(format!("Press 'd' again to delete {}", node.path.file_name().unwrap_or_default().to_string_lossy()));
                    }
                } else {
                    self.pending_delete = Some(node.path.clone());
                    self.set_status(format!("Press 'd' again to delete {}", node.path.file_name().unwrap_or_default().to_string_lossy()));
                }
            }
        }
    }

    fn launch_fzf(&mut self) {
        disable_raw_mode().ok();
        execute!(std::io::stdout(), LeaveAlternateScreen).ok();
        std::io::stdout().flush().ok();
        let cmd = "fd . ~ --type f 2>/dev/null || find ~ -type f 2>/dev/null";
        let out = std::process::Command::new("sh")
            .arg("-c").arg(format!("{} | fzf", cmd))
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::piped())
            .output();
        execute!(std::io::stdout(), EnterAlternateScreen).ok();
        enable_raw_mode().ok();
        if let Ok(out) = out {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() { self.input = path; }
            }
        } else { self.set_status("Failed to launch fzf. Is it installed?"); }
    }

    // --- NAVIGATION ---
    fn next(&mut self) {
        self.pending_delete = None;
        let len = self.current_nodes().len();
        if len == 0 { return; }
        let i = match self.state.selected() {
            Some(i) => if i >= len - 1 { 0 } else { i + 1 },
            None => 0,
        };
        self.state.select(Some(i));
        self.update_preview();
    }

    fn previous(&mut self) {
        self.pending_delete = None;
        let len = self.current_nodes().len();
        if len == 0 { return; }
        let i = match self.state.selected() {
            Some(i) => if i == 0 { len - 1 } else { i - 1 },
            None => 0,
        };
        self.state.select(Some(i));
        self.update_preview();
    }

    // Opens file/folder. Folders expand/collapse. Files open in editor/viewer.
    fn open_or_expand(&self) -> bool {
        if let Some(i) = self.state.selected() {
            if let Some(node) = self.current_nodes().get(i) {
                if node.is_dir { return true; }

                let target_path = fs::canonicalize(&node.path).unwrap_or_else(|_| node.path.clone());
                if matches!(detect_kind(&target_path), PreviewKind::Image | PreviewKind::Video) {
                    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                    let _ = std::process::Command::new(opener).arg(&target_path).spawn();
                    return false;
                }
                let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".into());
                disable_raw_mode().ok();
                execute!(std::io::stdout(), LeaveAlternateScreen).ok();
                std::io::stdout().flush().ok();
                let _ = std::process::Command::new(&editor).arg(&target_path).status();
                std::io::stdout().flush().ok();
                execute!(std::io::stdout(), EnterAlternateScreen).ok();
                enable_raw_mode().ok();
            }
        }
        false
    }
}

// --- HELPERS ---

// Shared sorting logic to keep folders on top and alphabetized
fn sort_nodes(nodes: &mut [Node]) {
    nodes.sort_by_key(|n| (!n.is_dir, n.path.file_name().unwrap_or_default().to_os_string()));
}

fn search_recursive(node: &Node, query: &str, filtered: &mut Vec<Node>) {
    let name = node.path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    if name.contains(query) { filtered.push(node.clone()); }
    if node.is_dir {
        if let Ok(entries) = fs::read_dir(&node.path) {
            let mut children: Vec<Node> = entries.flatten().map(|e| {
                let p = e.path();
                let is_dir = fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
                Node { path: p, depth: node.depth + 1, is_dir, expanded: false }
            }).collect();
            sort_nodes(&mut children);
            for child in children {
                search_recursive(&child, query, filtered);
            }
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
    let lname = name.to_lowercase();
    let lquery = query.to_lowercase();
    if let Some(start_in_lname) = lname.find(&lquery) {
        let start_char_idx = lname[..start_in_lname].chars().count();
        let match_len_chars = lquery.chars().count();
        let mut byte_start = name.len();
        for (char_idx, (b, _ch)) in name.char_indices().enumerate() {
            if char_idx == start_char_idx { byte_start = b; break; }
        }
        if byte_start == name.len() && start_char_idx == 0 { byte_start = 0; }
        let mut byte_end = name.len();
        let mut seen = 0usize;
        for (b, ch) in name[byte_start..].char_indices() {
            seen += 1;
            byte_end = byte_start + b + ch.len_utf8();
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
    let out = std::process::Command::new("file").arg("--brief").arg(path).output();
    if let Ok(out) = out { return String::from_utf8_lossy(&out.stdout).trim().to_string(); }
    "Unknown".to_string()
}

fn get_video_meta(path: &Path) -> String {
    let out = std::process::Command::new("ffprobe")
        .arg("-v").arg("error")
        .arg("-show_entries").arg("format=duration,size:stream=codec_name,width,height")
        .arg("-of").arg("default=noprint_wrappers=1")
        .arg(path).output();
    if let Ok(out) = out { if out.status.success() { return String::from_utf8_lossy(&out.stdout).to_string(); } }
    "Metadata unavailable (is ffprobe installed?)".to_string()
}

fn extract_video_thumbnail_cached(path: &Path) -> Option<PathBuf> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let stem = path.file_name()?.to_string_lossy();
    let cache_dir = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("configz").join("thumbs");
    let _ = fs::create_dir_all(&cache_dir);
    let thumb_path = cache_dir.join(format!("{}_{}.png", stem, mtime));
    if thumb_path.exists() { return Some(thumb_path); }
    let status = std::process::Command::new("ffmpeg")
        .arg("-y").arg("-i").arg(path)
        .arg("-vframes").arg("1")
        .arg("-vf").arg("scale=960:-1")
        .arg(&thumb_path).status().ok()?;
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

// --- CLI ARGUMENT PARSING ---
fn parse_cli_args() -> Option<String> {
    let args: Vec<String> = env::args().collect();
    let mut link_path = None;
    let mut link_name = None;
    let mut open_target = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-l" => { if i + 1 < args.len() { link_path = Some(args[i + 1].clone()); i += 1; } }
            "-n" => { if i + 1 < args.len() { link_name = Some(args[i + 1].clone()); i += 1; } }
            "-o" => { if i + 1 < args.len() { open_target = Some(args[i + 1].clone()); i += 1; } }
            _ => {}
        }
        i += 1;
    }

    if let Some(p) = link_path {
        let source_path = expand_tilde(&p);
        if !source_path.exists() {
            eprintln!("Error: Path not found: {}", p);
            std::process::exit(1);
        }
        let name = link_name.unwrap_or_else(|| {
            source_path.file_name().unwrap_or_default().to_string_lossy().to_string()
        });
        let home = dirs::home_dir().expect("Could not find home directory");
        let dest = home.join(".configz").join(&name);
        if dest.exists() { let _ = fs::remove_file(&dest); }
        match symlink(&source_path, &dest) {
            Ok(_) => println!("Successfully linked {} -> {}", name, source_path.display()),
            Err(e) => eprintln!("Failed to link: {}", e),
        }
        std::process::exit(0);
    }

    if let Some(t) = open_target {
        let home = dirs::home_dir().expect("Could not find home directory");
        let configz_dir = home.join(".configz");
        let target_path = expand_tilde(&t);
        let resolved_path = if target_path.is_absolute() && target_path.exists() { target_path } else { configz_dir.join(&t) };

        if !resolved_path.exists() {
            eprintln!("Error: Target not found: {}", t);
            std::process::exit(1);
        }

        if resolved_path.is_file() {
            if matches!(detect_kind(&resolved_path), PreviewKind::Image | PreviewKind::Video) {
                let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                let _ = std::process::Command::new(opener).arg(&resolved_path).status();
            } else {
                let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".into());
                let _ = std::process::Command::new(&editor).arg(&resolved_path).status();
            }
            std::process::exit(0);
        }

        return Some(resolved_path.file_name().unwrap_or_default().to_string_lossy().to_string());
    }

    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli_target = parse_cli_args();

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let theme = Theme::new();

    if let Some(open_name) = &cli_target {
        if let Some(idx) = app.nodes.iter().position(|n| n.path.file_name().unwrap_or_default().to_string_lossy() == *open_name) {
            app.state.select(Some(idx));
            if app.nodes.get(idx).map(|n| n.is_dir).unwrap_or(false) {
                app.expand_node(idx);
                if app.nodes.len() > idx + 1 {
                    app.state.select(Some(idx + 1));
                    app.update_preview();
                }
            }
        }
    }

    loop {
        terminal.draw(|f| {
            let main_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref())
                .split(f.area());

            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
                .split(main_layout[0]);

            let display_nodes = app.current_nodes();
            let items: Vec<ListItem> = display_nodes.iter().map(|node| {
                let name = node.path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let size = fs::metadata(&node.path).map(|m| format_size(m.len())).unwrap_or_default();
                let icon = if node.is_dir {
                    if node.expanded { "\u{f07c}" } else { "\u{f07b}" }
                } else { get_icon(&node.path) };

                let indent = "  ".repeat(node.depth);
                let name_spans: Vec<Span> = if app.input_mode == InputMode::Search && !app.input.is_empty() {
                    highlight_name(&name, &app.input, Style::default().fg(theme.accent), Style::default().fg(theme.text))
                } else {
                    vec![Span::styled(name, Style::default().fg(theme.text))]
                };

                let mut spans_vec = Vec::new();
                spans_vec.push(Span::raw(format!("{}{} ", indent, icon)));
                spans_vec.extend(name_spans.into_iter());
                spans_vec.push(Span::raw(" "));
                spans_vec.push(Span::styled(format!("{:>7}", size), Style::default().fg(theme.muted)));

                ListItem::new(Line::from(spans_vec))
            }).collect();

            let list_title = if app.input_mode == InputMode::Search {
                format!(" Search: {} ", app.input)
            } else {
                format!(" ~/.configz ({} items) ", display_nodes.len())
            };

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme.border))
                        .title(Span::styled(list_title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text)))
                )
                .highlight_symbol("▶ ")
                .highlight_style(theme.highlight);

            f.render_stateful_widget(list, chunks[0], &mut app.state);

            let preview_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border))
                .title(Span::styled(" Preview ", Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));

            f.render_widget(preview_block.clone(), chunks[1]);
            let inner_area = chunks[1].inner(Margin::new(1, 1));

            let preview_text = app.preview_text.as_slice().into_text().unwrap_or_default();
            let preview = Paragraph::new(preview_text)
                .alignment(Alignment::Left)
                .wrap(Wrap { trim: false });
            f.render_widget(preview, inner_area);

            let (title, input_text) = match app.input_mode {
                InputMode::Normal => {
                    let text = if app.status.is_empty() {
                        " [a] Add  [/] Search  [d] Delete  [r] Rename  [Enter] Expand/Open  [q] Quit ".to_string()
                    } else { format!(" {}", app.status) };
                    (" Help ", text)
                }
                InputMode::AddingPath => {
                    let title = if app.status.is_empty() { " Add Path (z=fzf, Enter=next, Esc=cancel) " } else { " Error (Esc to cancel) " };
                    let text = if app.status.is_empty() { app.input.clone() } else { app.status.clone() };
                    (title, text)
                }
                InputMode::AddingAlias => {
                    let title = if app.status.is_empty() { " Alias Name in ~/.configz (Enter=save) " } else { " Error (Esc to cancel) " };
                    let text = if app.status.is_empty() { app.input.clone() } else { app.status.clone() };
                    (title, text)
                }
                InputMode::Renaming => (" Rename (Enter=save, Esc=cancel) ", app.input.clone()),
                InputMode::Search => (" Search (Esc=cancel, Enter=Open) ", app.input.clone()),
            };

            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
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
                            if !app.status.is_empty() && key.code != KeyCode::Char('q') {
                                app.status.clear();
                                app.last_status_time = None;
                            }
                            match key.code {
                                KeyCode::Char('q') => break,
                                KeyCode::Down | KeyCode::Char('j') => app.next(),
                                KeyCode::Up | KeyCode::Char('k') => app.previous(),
                                KeyCode::Enter => {
                                    if app.open_or_expand() { app.toggle_expand(); }
                                    terminal.clear()?;
                                    app.update_preview();
                                }
                                KeyCode::Char('a') => app.begin_add(),
                                KeyCode::Char('d') => app.mark_or_delete(),
                                KeyCode::Char('r') => app.begin_rename(),
                                KeyCode::Char('/') => app.start_search(),
                                _ => {}
                            }
                        }
                        InputMode::AddingPath => match key.code {
                            KeyCode::Enter => app.submit_add_step(),
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Tab | KeyCode::Char('z') => {
                                app.launch_fzf();
                                terminal.clear()?;
                            }
                            KeyCode::Backspace => {
                                if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; }
                                else { app.input.pop(); }
                            }
                            KeyCode::Char(c) => {
                                if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; }
                                app.input.push(c);
                            }
                            _ => {}
                        },
                        InputMode::AddingAlias => match key.code {
                            KeyCode::Enter => app.submit_add_step(),
                            KeyCode::Esc => app.cancel_input(),
                            KeyCode::Backspace => {
                                if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; }
                                else { app.input.pop(); }
                            }
                            KeyCode::Char(c) => {
                                if !app.status.is_empty() { app.status.clear(); app.last_status_time = None; }
                                app.input.push(c);
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
                                KeyCode::Enter => {
                                    if app.open_or_expand() { app.toggle_expand(); }
                                    terminal.clear()?;
                                    app.cancel_input();
                                    false
                                }
                                KeyCode::Down => { app.next(); false }
                                KeyCode::Up => { app.previous(); false }
                                KeyCode::Backspace => { app.input.pop(); true }
                                KeyCode::Char(c) => { app.input.push(c); true }
                                _ => false,
                            };

                            if input_changed {
                                app.apply_filter();
                                app.state.select(Some(0));
                                app.update_preview();
                            }
                        },
                    }
                }
                Event::Mouse(mouse) => {
                    if app.input_mode == InputMode::Normal {
                        match mouse.kind {
                            MouseEventKind::ScrollDown => app.next(),
                            MouseEventKind::ScrollUp => app.previous(),
                            _ => {}
                        }
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
