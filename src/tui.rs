use std::path::PathBuf;

use ansi_to_tui::IntoText;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::app::*;
use crate::config::{AppSettings, KeyBind};
use crate::ops;

type Tui = Terminal<CrosstermBackend<std::io::Stdout>>;

pub struct TuiGuard;
impl Drop for TuiGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    }
}

// ============ Theme ============

pub struct Theme {
    pub border: Color, pub text: Color, pub muted: Color, pub accent: Color, pub highlight: Style,
    pub dir_color: Color, pub sym_color: Color, pub exec_color: Color,
    pub config_color: Color, pub modified_color: Color, pub broken_color: Color,
}

fn hex_to_color(h: &str) -> Option<Color> {
    let h = h.trim_start_matches('#');
    let h = if h.len() == 3 {
        let mut s = String::with_capacity(6);
        for c in h.chars() { s.push(c); s.push(c); }
        s
    } else { h.to_string() };
    let b = h.as_bytes();
    if b.len() == 6 && b.iter().all(|c| c.is_ascii_hexdigit()) {
        Some(Color::Rgb(u8::from_str_radix(&h[0..2], 16).ok()?, u8::from_str_radix(&h[2..4], 16).ok()?, u8::from_str_radix(&h[4..6], 16).ok()?))
    } else { None }
}

/// Perceptual luminance (YIQ approximation) — decides which palette to use.
fn luminance(c: Color) -> f32 {
    match c {
        Color::Rgb(r, g, b) => (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0,
        Color::White => 1.0, Color::Gray => 0.66, Color::DarkGray => 0.33, Color::Black => 0.0,
        _ => 0.5,
    }
}

struct FilePalette { dir: Color, sym: Color, exec: Color, config: Color, modified: Color, broken: Color, muted: Color }

/// Classic ls-style hues, light + saturated — pops on near-black backgrounds.
fn dark_palette() -> FilePalette {
    FilePalette {
        dir:      Color::Rgb(137, 180, 250), // blue
        sym:      Color::Rgb(240, 165, 245), // bright magenta
        exec:     Color::Rgb(166, 227, 161), // green
        config:   Color::Rgb(250, 227, 176), // yellow
        modified: Color::Rgb(255, 183, 135), // orange
        broken:   Color::Rgb(255, 110, 130), // red
        muted:    Color::Rgb(145, 150, 175), // clearly dimmer than text, still readable
    }
}

/// Same hues, dark + saturated — for light backgrounds (latte).
fn light_palette() -> FilePalette {
    FilePalette {
        dir:      Color::Rgb(30, 100, 235),
        sym:      Color::Rgb(150, 60, 220),
        exec:     Color::Rgb(45, 140, 35),
        config:   Color::Rgb(178, 116, 0),
        modified: Color::Rgb(210, 90, 25),
        broken:   Color::Rgb(210, 35, 65),
        muted:    Color::Rgb(100, 104, 128),
    }
}

impl Theme {
    pub fn new(s: &AppSettings) -> Self {
        let tc = s.color_mode.is_truecolor();
        if let Some(ct) = s.custom_themes.get(&s.theme_name) {
            if let (Some(fg), Some(_), Some(ac)) = (hex_to_color(&ct.foreground), hex_to_color(&ct.background), hex_to_color(&ct.accent)) {
                return Theme::build(fg, ac, tc);
            }
        }
        if (s.theme_name == "custom" || s.theme_name == "auto") && s.custom_theme_path.is_some() {
            if let Some(path) = &s.custom_theme_path {
                if let Some(t) = Self::from_file(path) { return t; }
            }
        }
        if s.theme_name == "auto" { if let Some(t) = Self::from_terminal() { return t; } }
        let (fg, ac) = match s.theme_name.as_str() {
            "latte" => (Color::Rgb(76, 79, 105), Color::Rgb(30, 102, 245)),
            "nord" => (Color::Rgb(216, 222, 233), Color::Rgb(136, 192, 208)),
            "dracula" => (Color::Rgb(248, 248, 242), Color::Rgb(189, 147, 249)),
            "gruvbox" => (Color::Rgb(235, 219, 178), Color::Rgb(250, 189, 47)),
            "tokyonight" => (Color::Rgb(192, 202, 245), Color::Rgb(125, 207, 255)),
            _ => (Color::Rgb(205, 214, 244), Color::Rgb(137, 180, 250)),
        };
        Theme::build(fg, ac, tc)
    }

    fn from_file(p: &str) -> Option<Self> {
        let tcol = crate::config::ThemeColors::from_file(std::path::Path::new(p))?;
        let fg = hex_to_color(&tcol.foreground)?;
        let ac = hex_to_color(&tcol.accent).unwrap_or(fg);
        Some(Self::build(fg, ac, true))
    }

    fn from_terminal() -> Option<Self> {
        let home = dirs::home_dir()?;
        for cand in &[".config/kitty/kitty.conf", ".config/ghostty/config", ".config/wezterm/wezterm.lua"] {
            let p = home.join(cand);
            if !p.exists() { continue; }
            let txt = std::fs::read_to_string(&p).ok()?;
            let (mut fg, mut bg) = (None, None);
            for line in txt.lines() {
                let l = line.trim();
                if let Some(rest) = l.strip_prefix("foreground") { fg = rest.trim_start_matches([' ', '=', '"', '\'']).split_whitespace().next().and_then(hex_to_color); }
                if let Some(rest) = l.strip_prefix("background") { bg = rest.trim_start_matches([' ', '=', '"', '\'']).split_whitespace().next().and_then(hex_to_color); }
            }
            if let (Some(fg), Some(_)) = (fg, bg) { return Some(Self::build(fg, fg, true)); }
        }
        None
    }

    fn build(fg: Color, ac: Color, tc: bool) -> Self {
        // Light foreground ⇒ dark-background theme (true for every built-in
        // and custom theme, so we never need the background color).
        let dark = luminance(fg) > 0.5;

        if !tc {
            // 16-color fallback: BRIGHT ANSI variants only — base Blue/Green
            // are too dark to read on dark terminals.
            let (text, accent_c) = if dark { (Color::White, Color::Cyan) } else { (Color::Black, Color::Blue) };
            let (d, s, e, c) = if dark {
                (Color::LightBlue, Color::LightMagenta, Color::LightGreen, Color::LightYellow)
            } else {
                (Color::Blue, Color::Magenta, Color::Green, Color::Cyan)
            };
            return Self {
                border: accent_c, text, muted: Color::Gray, accent: accent_c,
                highlight: Style::default().fg(text)
                    .bg(if dark { Color::DarkGray } else { Color::Gray })
                    .add_modifier(Modifier::BOLD),
                dir_color: d, sym_color: s, exec_color: e, config_color: c,
                modified_color: if dark { Color::LightRed } else { Color::Red },
                broken_color: Color::Red,
            };
        }

        let p = if dark { dark_palette() } else { light_palette() };
        Self {
            border: ac, text: fg, muted: p.muted, accent: ac,
            // highlight bg adapts: dark bg on dark themes, light bg on latte
            highlight: if dark {
                Style::default().fg(fg).bg(Color::Rgb(62, 62, 88)).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg).bg(Color::Rgb(214, 219, 238)).add_modifier(Modifier::BOLD)
            },
            dir_color: p.dir, sym_color: p.sym, exec_color: p.exec,
            config_color: p.config, modified_color: p.modified, broken_color: p.broken,
        }
    }
}// ============ layout helpers ============

pub fn centered_rect(px: u16, py: u16, r: Rect) -> Rect {
    let popup = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Percentage((100 - py) / 2), Constraint::Percentage(py), Constraint::Percentage((100 - py) / 2)]).split(r);
    Layout::default().direction(Direction::Horizontal)
        .constraints([Constraint::Percentage((100 - px) / 2), Constraint::Percentage(px), Constraint::Percentage((100 - px) / 2)]).split(popup[1])[1]
}

/// Char-boundary-safe match highlighting (no more slicing panics on unicode names).
pub fn highlight_name(name: &str, query: &str, accent: Style, normal: Style) -> Vec<Span<'static>> {
    let whole = || vec![Span::styled(name.to_string(), normal)];
    if query.is_empty() { return whole(); }
    let ln: Vec<char> = name.to_lowercase().chars().collect();
    let lq: Vec<char> = query.to_lowercase().chars().collect();
    if lq.is_empty() || ln.len() < lq.len() { return whole(); }
    let start = (0..=ln.len() - lq.len()).find(|&i| ln[i..i + lq.len()] == lq[..]);
    match start {
        None => whole(),
        Some(s) => {
            let chars: Vec<char> = name.chars().collect();
            let pre: String = chars[..s].iter().collect();
            let mid: String = chars[s..s + lq.len()].iter().collect();
            let post: String = chars[s + lq.len()..].iter().collect();
            vec![Span::styled(pre, normal), Span::styled(mid, accent.add_modifier(Modifier::BOLD)), Span::styled(post, normal)]
        }
    }
}

pub fn key_event_to_string(key: crossterm::event::KeyEvent) -> String {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let main = match key.code {
        KeyCode::Char(c) => {
            if c == ' ' { "Space".to_string() }
            else if ctrl || alt { c.to_lowercase().next().unwrap_or(c).to_string() }
            else { c.to_string() }
        }
        KeyCode::Enter => "Enter".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "BackTab".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::F(n) => format!("F{}", n),
        _ => return String::new(),
    };
    if ctrl { format!("Ctrl+{}", main) }
    else if alt { format!("Alt+{}", main) }
    else { main }
}

// ============ main loop ============

pub fn run_tui(confy_dir: PathBuf, cli_edit_target: Option<PathBuf>) -> anyhow::Result<()> {
    let mut app = App::new(confy_dir)?;
    let mut theme = Theme::new(&app.state_data.settings);
    if app.state_data.settings.start_in_filter_mode { app.start_search(); }
    if let Some(t) = &cli_edit_target { app.enter_editor_select(Some(t.clone())); }

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    enable_raw_mode()?;
    let _guard = TuiGuard;
    let mut stdout = std::io::stdout();
    if app.state_data.settings.enable_mouse { execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?; }
    else { execute!(stdout, EnterAlternateScreen)?; }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|f| draw(f, &mut app, &theme))?;
        match event::poll(std::time::Duration::from_millis(100)) {
            Ok(true) => match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press { continue; }
                    if handle_key(&mut app, &mut terminal, key) { break; }
                }
                Event::Mouse(m) => handle_mouse(&mut app, m),
                _ => {}
            },
            Ok(false) => {}
            Err(e) => { tracing::error!(?e, "event poll error"); break; }
        }
        app.check_fs_changes();
        app.detect_external_changes();
        if app.theme_dirty {
            let new_theme = Theme::new(&app.state_data.settings);
            theme = new_theme;
            app.theme_dirty = false;
        }
        if app.needs_clear { let _ = terminal.clear(); app.needs_clear = false; }
    }
    Ok(())
}

fn handle_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    if !app.state_data.settings.enable_mouse { return; }
    if !matches!(app.input_mode, InputMode::Normal | InputMode::VisualSelect) { return; }
    let area = app.file_list_area;
    let in_list = |row: u16, col: u16| row >= area.y + 1 && row < area.y + area.height && col >= area.x && col < area.x + area.width;
    match m.kind {
        MouseEventKind::ScrollDown => app.next(),
        MouseEventKind::ScrollUp => app.previous(),
        MouseEventKind::Down(MouseButton::Left) => {
            if in_list(m.row, m.column) {
                let item_idx = (m.row - area.y - 1) as usize;
                if item_idx < app.current_nodes().len() {
                    let path = app.current_nodes()[item_idx].path.clone();
                    let now = std::time::Instant::now();
                    app.state.select(Some(item_idx));
                    app.update_preview();
                    if app.last_mouse_click.as_ref().is_some_and(|(at, previous)| previous == &path && now.duration_since(*at) <= std::time::Duration::from_millis(400)) {
                        if app.selected_nodes.contains(&path) { app.selected_nodes.remove(&path); }
                        else { app.selected_nodes.insert(path.clone()); }
                        app.last_mouse_click = None;
                    } else {
                        app.last_mouse_click = Some((now, path));
                    }
                    app.is_dragging = true;
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if app.is_dragging && in_list(m.row, m.column) {
                let item_idx = (m.row - area.y - 1) as usize;
                if item_idx < app.current_nodes().len() {
                    app.state.select(Some(item_idx));
                    app.update_preview();
                    let path = app.current_nodes()[item_idx].path.clone();
                    if app.selected_nodes.contains(&path) { app.selected_nodes.remove(&path); }
                    else { app.selected_nodes.insert(path); }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => app.is_dragging = false,
        _ => {}
    }
}

// ============ drawing ============

fn draw(f: &mut Frame, app: &mut App, theme: &Theme) {
    let main = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)]).split(f.size());
    let clamped_lw = app.state_data.settings.list_width.min(f.size().width.saturating_sub(50)).max(10);
    let chunks = Layout::default().direction(Direction::Horizontal)
        .constraints([Constraint::Length(clamped_lw), Constraint::Min(45)]).split(main[0]);
    draw_file_list(f, app, theme, chunks[0]);
    draw_preview(f, app, theme, chunks[1]);

    match app.input_mode {
        InputMode::Help => draw_help(f, app, theme),
        InputMode::Info => draw_info(f, app, theme),
        InputMode::SelectEditor => draw_editor_select(f, app, theme),
        InputMode::Settings => draw_settings(f, app, theme),
        InputMode::HooksMenu => draw_hooks(f, app, theme),
        InputMode::RootMenu => draw_roots(f, app, theme),
        InputMode::TrashManagement => draw_trash(f, app, theme),
        InputMode::ServicesMenu => draw_services(f, app, theme),
        InputMode::KeybindMenu => draw_keybinds(f, app, theme),
        InputMode::SettingsInput => draw_input(f, app, theme, " Enter value (Enter=save, Esc=back): "),
        InputMode::HookPathInput => draw_input(f, app, theme, " Hook: /path/to/script or !command (Tab=complete) "),
        InputMode::AddingRoot => draw_input(f, app, theme, " Root path (Tab=complete, Enter=add): "),
        InputMode::ChmodPrompt => draw_input(f, app, theme, " Permissions (e.g. +x or 644, Enter=save): "),
        InputMode::QuickMove => draw_input(f, app, theme, " Move to path (Tab=complete, Enter=move): "),
        InputMode::AddCustomBind => draw_input(f, app, theme, " !command or /path (Enter=next, Esc=back): "),
        InputMode::AddingPath => draw_input(f, app, theme, " Add Path (Tab=fzf, Enter=next, Esc=cancel) "),
        InputMode::AddingAlias => draw_input(f, app, theme, " Add Alias (Enter=save, Esc=cancel) "),
        InputMode::AddingDir => draw_input(f, app, theme, " Folder Name (Enter=save, Esc=cancel) "),
        InputMode::AddingNote => draw_input(f, app, theme, " Add Note (Enter=save, Esc=cancel) "),
        InputMode::AddingTag => draw_input(f, app, theme, " Add Tag (Enter=save, Esc=cancel) "),
        InputMode::Renaming => draw_input(f, app, theme, " Rename (Enter=save, Esc=cancel) "),
        InputMode::SudoPrompt => draw_input(f, app, theme, " 🔒 Sudo Password (Ctrl+H show/hide, Enter=submit) "),
        InputMode::Shell => draw_input(f, app, theme, " sh> ({f}=file, !alias, Tab=complete, Enter=run, exit=close) "),
        InputMode::AddingThemePath => draw_input(f, app, theme, " Theme file path (Tab=fzf, Enter=next) "),
        InputMode::AddingThemeName => draw_input(f, app, theme, " Theme name (Enter=save, Esc=cancel) "),
        _ => {}
    }

    let (title, text) = status_bar(app);
    let ib = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    f.render_widget(Paragraph::new(text).block(ib), main[1]);

    if matches!(app.input_mode,
        InputMode::AddingPath | InputMode::AddingAlias | InputMode::AddingDir | InputMode::AddingNote
        | InputMode::AddingTag | InputMode::Renaming | InputMode::Search | InputMode::SudoPrompt
        | InputMode::Shell | InputMode::SettingsInput | InputMode::HookPathInput
        | InputMode::AddingRoot | InputMode::AddingThemePath | InputMode::AddingThemeName
        | InputMode::ChmodPrompt | InputMode::QuickMove | InputMode::AddCustomBind) {
        let a = main[1];
        let visible = app.input.chars().count().min((a.width as usize).saturating_sub(2)) as u16;
        f.set_cursor(a.x + 1 + visible, a.y + 1);
    }
}

fn draw_file_list(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let display = app.current_nodes();
    let items: Vec<ListItem> = display.iter().map(|node| {
        let name = ops::path_name(&node.path);
        let size_str = if app.state_data.settings.show_sizes { node.size.map(|s| ops::format_size(s)).unwrap_or_default() } else { String::new() };
        let is_sel = app.selected_nodes.contains(&node.path);
        let is_book = node.path.strip_prefix(&app.confy_dir).map(|r| app.state_data.bookmarks.contains(&ops::path_to_string(&r))).unwrap_or(false);
        let is_modified = app.modified_cache.contains(&node.path);

        let name_color = if node.broken_symlink { Color::Red }
            else if node.is_symlink { theme.sym_color }
            else if node.is_dir { theme.dir_color }
            else if node.is_executable { theme.exec_color }
            else if name.ends_with(".conf") || name.ends_with(".toml") || name.ends_with(".json") || name.ends_with(".yaml") || name.ends_with(".yml") { theme.config_color }
            else { theme.text };

        let icon = if is_sel { "✓" } else if is_book { "★" }
            else if node.is_dir { if node.expanded { "\u{f07c}" } else { "\u{f07b}" } }
            else { ops::get_icon(&node.path) };
        let indent = "  ".repeat(node.depth);
        let ns = if app.input_mode == InputMode::Search && !app.input.is_empty() {
            highlight_name(&name, &app.input, Style::default().fg(theme.accent), Style::default().fg(name_color))
        } else { vec![Span::styled(name, Style::default().fg(name_color))] };
        let mut spans = vec![Span::raw(format!("{}{} ", indent, icon))];
        spans.extend(ns);
        if node.broken_symlink { spans.push(Span::styled(" ●", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))); }
        else if node.has_broken_descendant { spans.push(Span::styled(" ●", Style::default().fg(Color::Red))); }
        if is_modified { spans.push(Span::styled(" ●", Style::default().fg(theme.modified_color))); }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(format!("{:>7}", size_str), Style::default().fg(theme.muted)));
        ListItem::new(Line::from(spans))
    }).collect();
    let dir_name = ops::path_name(&app.confy_dir);
    let list_title = if app.input_mode == InputMode::Search { format!(" Search: {} ", app.input) }
        else if app.bookmarks_only { format!(" {} (Bookmarks: {}) ", dir_name, display.len()) }
        else if app.host_filter { format!(" {} (Host filter: {}) ", dir_name, display.len()) }
        else if app.jump_list { format!(" {} (Jump list: {}) ", dir_name, display.len()) }
        else { format!(" {} ({} items) ", dir_name, display.len()) };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .title(Span::styled(list_title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text))))
        .highlight_symbol("▶ ").highlight_style(theme.highlight);
    f.render_stateful_widget(list, area, &mut app.state);
    app.file_list_area = area;
}

fn draw_preview(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let title = if app.input_mode == InputMode::VersionSelect { " Versions (Enter=Restore, d=Diff, Esc=Cancel) " }
        else if app.preview_pinned { " Preview (Esc=return) " }
        else { " Preview " };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    f.render_widget(block, area);
    let inner = area.inner(&Margin::new(1, 1));
    if app.input_mode == InputMode::VersionSelect {
        let mut items: Vec<ListItem> = Vec::new();
        let current_file = app.current_rel_path.as_ref().map(|rp| app.confy_dir.join(rp));
        let current_hash = current_file.and_then(|p| ops::hash_file(&p).ok());
        let matching_idx = current_hash.and_then(|ch| app.available_versions.iter().position(|(_, hp)| ops::hash_file(hp).ok() == Some(ch)));
        for (i, (date, _)) in app.available_versions.iter().enumerate() {
            let marker = if Some(i) == matching_idx { "★" } else { " " };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!(" {} {} ", marker, date), Style::default().fg(theme.text)),
                Span::styled("●", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            ])));
            if i < app.available_versions.len() - 1 { items.push(ListItem::new(Line::from(vec![Span::raw("           │")]))); }
        }
        f.render_stateful_widget(List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight), inner, &mut app.version_state);
    } else {
        let text = app.preview_text.as_slice().into_text().unwrap_or_default();
        f.render_widget(Paragraph::new(text).alignment(Alignment::Left).wrap(Wrap { trim: false }), inner);
    }
}

fn draw_info(f: &mut Frame, app: &App, theme: &Theme) {
    let area = centered_rect(60, 50, f.size());
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" 📋 File Info (Esc=close) ", Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let mut lines: Vec<Line> = Vec::new();
    if let Some(node) = app.selected_node_ref() {
        let path = &node.path;
        let name = ops::path_name(path);
        lines.push(Line::from(vec![Span::styled(format!("  {}", name), Style::default().fg(theme.text).add_modifier(Modifier::BOLD))]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled("  Path:       ", Style::default().fg(theme.muted)), Span::styled(path.display().to_string(), Style::default().fg(theme.text))]));
        let is_sym = path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
        if is_sym {
            lines.push(Line::from(vec![Span::styled("  Type:       ", Style::default().fg(theme.muted)), Span::styled("Symlink", Style::default().fg(theme.text))]));
            if let Ok(target) = std::fs::read_link(path) {
                lines.push(Line::from(vec![Span::styled("  Target:     ", Style::default().fg(theme.muted)), Span::styled(target.display().to_string(), Style::default().fg(theme.text))]));
            }
            if std::fs::metadata(path).is_err() {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled("  ✗ BROKEN SYMLINK: target missing", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))]));
            }
        } else {
            lines.push(Line::from(vec![Span::styled("  Type:       ", Style::default().fg(theme.muted)), Span::styled(if node.is_dir { "Directory" } else { "File" }, Style::default().fg(theme.text))]));
        }
        if let Some(size) = node.size { lines.push(Line::from(vec![Span::styled("  Size:       ", Style::default().fg(theme.muted)), Span::styled(ops::format_size(size), Style::default().fg(theme.text))])); }
        if let Ok(meta) = std::fs::metadata(path) {
            use std::os::unix::fs::PermissionsExt;
            let perms = meta.permissions().mode();
            lines.push(Line::from(vec![Span::styled("  Perms:      ", Style::default().fg(theme.muted)), Span::styled(format!("{:o}", perms & 0o777), Style::default().fg(theme.text))]));
            if let Ok(mtime) = meta.modified() {
                if let Ok(d) = mtime.duration_since(std::time::UNIX_EPOCH) {
                    lines.push(Line::from(vec![Span::styled("  Modified:   ", Style::default().fg(theme.muted)), Span::styled(ops::timestamp_from_secs(d.as_secs()), Style::default().fg(theme.text))]));
                }
            }
        }
        if let Ok(rel) = path.strip_prefix(&app.confy_dir) {
            let rs = ops::path_to_string(&rel);
            if let Some(note) = app.state_data.notes.get(&rs) {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled(format!("  📝 Note: {}", note), Style::default().fg(Color::Yellow))]));
            }
            if let Some(tags) = app.state_data.tags.get(&rs) {
                if !tags.is_empty() {
                    let ts = tags.iter().cloned().collect::<Vec<_>>().join(", ");
                    lines.push(Line::from(vec![Span::styled(format!("  🏷️  Tags: {}", ts), Style::default().fg(Color::LightBlue))]));
                }
            }
            if app.state_data.bookmarks.contains(&rs) {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled("  ★ Bookmarked", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))]));
            }
        }
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from("  No file selected"));
    }
    lines.push(Line::from(""));
    f.render_widget(Paragraph::new(lines).block(block).alignment(Alignment::Left), area);
}

fn draw_trash(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = centered_rect(80,60 , f.size());
    f.render_widget(Clear, area);
    let days = app.state_data.settings.trash_retention_days;
    let title = format!(" Trash — retention {}d ", days);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let items: Vec<ListItem> = app.trash_items.iter().map(|p| {
        let name = ops::path_name(&p);
        let name: String = name.chars().take(34).collect();
        let left = ops::trash_time_left(p, days);
        let (label, color) = match left {
            Some(d) if d <= 0 => ("due next cleanup".to_string(), Color::Red),
            Some(d) if d <= 86_400 => (ops::format_time_left(d), Color::Yellow),
            Some(d) => (ops::format_time_left(d), theme.muted),
            None => ("kept forever".to_string(), theme.muted),
        };
        ListItem::new(Line::from(vec![
            Span::styled(format!("{:<34}", name), Style::default().fg(theme.text)),
            Span::styled(format!(" {}", label), Style::default().fg(color)),
        ]))
    }).collect();
    let inner = block.inner(area);
    let layout = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    f.render_stateful_widget(List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight), layout[0], &mut app.trash_state);
    f.render_widget(Paragraph::new(" r restore · d delete permanently (press twice) · Esc back ").style(Style::default().fg(theme.muted)), layout[1]);
    f.render_widget(block, area);
}

fn draw_services(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = centered_rect(60, 60, f.size());
    f.render_widget(Clear, area);
    let scope = if app.service_user_scope { "User" } else { "System" };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(format!(" {} Services ", scope), Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let services = app.filtered_services();
    let mut visible_state = ratatui::widgets::ListState::default();
    if let Some(selected) = app.service_state.selected() {
        let selected_name = app.available_services.get(selected).map(|item| item.0.clone());
        if let Some(visible) = services.iter().position(|service| Some(service.0.clone()) == selected_name) { visible_state.select(Some(visible)); }
    }
    let items: Vec<ListItem> = services.iter().map(|(name, state, desc)| {
        ListItem::new(Line::from(vec![
            Span::styled(format!("{:<30}", name), Style::default().fg(theme.text)),
            Span::styled(format!("{:<15}", state), Style::default().fg(theme.accent)),
            Span::styled(desc.clone(), Style::default().fg(theme.muted)),
        ]))
    }).collect();
    let inner = block.inner(area);
    let layout = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    f.render_stateful_widget(List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight), layout[0], &mut visible_state);
    let footer = if app.service_filter.is_empty() { " a add unit · d delete · / search · u user · y system · r restart · s stop · Esc back".into() } else { format!(" /{}  Enter search · Esc clear · a add · d delete", app.service_filter) };
    f.render_widget(Paragraph::new(footer).style(Style::default().fg(theme.muted)), layout[1]);
    f.render_widget(block, area);
}

// ============ keybind menu ============

/// Built-ins + any customs present in the keymap (deduped).
fn keybind_items(app: &App) -> Vec<KeyBind> {
    let mut items = KeyBind::all_variants();
    for (_, kb) in app.keymap.iter() {
        if let KeyBind::Custom(_) = kb {
            if !items.contains(kb) { items.push(kb.clone()); }
        }
    }
    items
}

fn all_keys_for(app: &App, kb: &KeyBind) -> String {
    let mut ks: Vec<String> = app.keymap.iter().filter(|(_, v)| *v == kb).map(|(k, _)| k.clone()).collect();
    ks.sort();
    if ks.is_empty() { "unbound".into() } else { ks.join("/") }
}

fn draw_key_picker(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = centered_rect(65, 60, f.size());
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" Key Picker ", Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let inner = block.inner(area);
    let store = crate::secrets::load_key_store(&app.confy_dir);
    let keys = if app.key_picker_group { store.generated } else { store.shared };
    let items: Vec<ListItem> = keys.iter().map(|k| {
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {} ", if app.key_picker_group { "g" } else { "s" }), Style::default().fg(theme.accent)),
            Span::styled(k.name.clone(), Style::default().fg(theme.text)),
        ]))
    }).collect();
    let list_area = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    f.render_stateful_widget(List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight), list_area[0], &mut app.key_picker_state);
    f.render_widget(Paragraph::new(" s/shared · g/generated · Enter copy · Esc close ").style(Style::default().fg(theme.muted)), list_area[1]);
    f.render_widget(block, area);
}

fn handle_key_picker(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Esc => { app.input_mode = InputMode::Normal; }
        KeyCode::Char('s') => { app.key_picker_group = false; app.key_picker_state.select(Some(0)); }
        KeyCode::Char('g') => { app.key_picker_group = true; app.key_picker_state.select(Some(0)); }
        KeyCode::Enter => {
            let store = crate::secrets::load_key_store(&app.confy_dir);
            let keys = if app.key_picker_group { store.generated } else { store.shared };
            if let Some(i) = app.key_picker_state.selected() { if let Some(k) = keys.get(i) { ops::copy_to_clipboard(&k.key); app.set_status(format!("Copied {} to clipboard.", k.name)); } }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let store = crate::secrets::load_key_store(&app.confy_dir);
            let n = if app.key_picker_group { store.generated.len() } else { store.shared.len() };
            if n > 0 {
                let cur = app.key_picker_state.selected().unwrap_or(0);
                app.key_picker_state.select(Some((cur + 1).min(n - 1)));
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let store = crate::secrets::load_key_store(&app.confy_dir);
            let n = if app.key_picker_group { store.generated.len() } else { store.shared.len() };
            if n > 0 {
                let cur = app.key_picker_state.selected().unwrap_or(0);
                app.key_picker_state.select(Some(cur.saturating_sub(1).min(n - 1)));
            }
        }
        _ => {}
    }
}

fn draw_keybinds(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = centered_rect(65, 70, f.size());
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" Keybinds ", Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let rev: std::collections::HashMap<&String, &String> = app.shell_aliases.iter().map(|(n, c)| (c, n)).collect();
    let items: Vec<ListItem> = keybind_items(app).iter().map(|kb| {
        let key_str = all_keys_for(app, kb);
        let desc = match kb {
            KeyBind::Custom(cmd) => {
                if let Some(p) = cmd.strip_prefix("edit:") { format!("📝 Open {}", p) }
                else { match rev.get(cmd) { Some(n) => format!("⚡ {}", n), None => format!("⚡ {}", cmd) } }
            }
            other => other.description(),
        };
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {:<32}: ", desc), Style::default().fg(theme.muted)),
            Span::styled(key_str, Style::default().fg(theme.accent)),
        ]))
    }).collect();
    let inner = block.inner(area);
    let layout = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    f.render_stateful_widget(List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight), layout[0], &mut app.keybind_state);
    f.render_widget(Paragraph::new(" Space rebind · a add · r reset · R factory reset · u undo · Esc back ").style(Style::default().fg(theme.muted)), layout[1]);
    f.render_widget(block, area);
}

fn status_bar(app: &App) -> (&'static str, String) {
    let cat = if app.cut_source.is_some() { "ฅ(^•x•^)✂️" } else if app.yank_source.is_some() { "ฅ(^•ω•^)📋" } else { "ฅ^•ﻌ•^ฅ" };
    match app.input_mode {
        InputMode::Normal => {
            let mut filters = String::new();
            if app.host_filter { filters.push_str(" [F:host]"); }
            if app.bookmarks_only { filters.push_str(" [B:★]"); }
            if app.jump_list { filters.push_str(" [jump]"); }
            (" Status ", if app.status.is_empty() {
                format!(" {} {}{} [?] Help | [j/k] Nav | [Enter] Edit | [u] Undo | [q] Quit ", cat, filters, "")
            } else { format!(" {} {} {}", cat, filters, app.status) })
        }
        InputMode::AddingPath => (" Add Path (Tab=fzf, Enter=next, Esc=cancel) ", if app.status.is_empty() { app.input.clone() } else { app.status.clone() }),
        InputMode::AddingAlias => (" Add Alias (Enter=save, Esc=cancel) ", if app.status.is_empty() { app.input.clone() } else { app.status.clone() }),
        InputMode::AddingDir => (" Folder Name (Enter=save, Esc=cancel) ", if app.status.is_empty() { app.input.clone() } else { app.status.clone() }),
        InputMode::AddingNote => (" Add Note (Enter=save, Esc=cancel) ", app.input.clone()),
        InputMode::AddingTag => (" Add Tag (Enter=save, Esc=cancel) ", app.input.clone()),
        InputMode::Renaming => (" Rename (Enter=save, Esc=cancel) ", app.input.clone()),
        InputMode::Search => (" Search (Enter=Open, Esc=cancel) ", app.input.clone()),
        InputMode::VersionSelect => (" Version (Enter=Restore, d=Diff, Esc=Cancel) ", String::new()),
        InputMode::SudoPrompt => (" 🔒 Sudo (Ctrl+H, Enter=submit, Esc=cancel) ", if app.hide_password { "*".repeat(app.input.chars().count()) } else { app.input.clone() }),
        InputMode::Shell => (" sh> ({f}=file, !alias, Tab=complete, Enter=run, exit=close, Esc=cancel) ", app.input.clone()),
        InputMode::Help => (" Help (↑↓ line, ←→ page, g/Home top, End bottom, Esc close) ", String::new()),
        InputMode::VisualSelect => (" Visual Select ", "j/k move+toggle | Space toggle | Esc finish".into()),
        InputMode::DeployConfirm => (" Deploy (e=browse, y=apply, n=cancel) ", String::new()),
        InputMode::SelectEditor => (" Select Editor ([d]=Default, [Enter]=Use once, [Esc]=Cancel) ", String::new()),
        InputMode::Settings => (" Settings ([Enter] Change, [H/M/L] Jump, [Esc] Close) ", String::new()),
        InputMode::SettingsInput => (" Settings Input (Enter=save, Esc=back) ", app.input.clone()),
        InputMode::HooksMenu => (" Hooks ([Enter]=Edit, [t]=Test, [d]=Clear, [Esc]=Back) ", String::new()),
        InputMode::HookPathInput => (" Hook: /path or !command (Enter=save, Esc=back) ", app.input.clone()),
        InputMode::RootMenu => (" Roots ([Enter]=Switch, [a]=Add, [d]=Del, [Esc]=Close) ", String::new()),
        InputMode::AddingRoot => (" Root path (Tab=complete, Enter=add) ", app.input.clone()),
        InputMode::AddingThemePath => (" Theme file path (Tab=fzf, Enter=next, Esc=cancel) ", if app.status.is_empty() { app.input.clone() } else { app.status.clone() }),
        InputMode::AddingThemeName => (" Theme name (Enter=save, Esc=cancel) ", app.input.clone()),
        InputMode::Info => (" File Info (Esc=close) ", String::new()),
        InputMode::TrashManagement => (" Trash ([r]estore, [d]elete, [Esc]=Back) ", String::new()),
        InputMode::ChmodPrompt => (" Permissions (e.g. +x or 644, Enter=save, Esc=cancel) ", app.input.clone()),
        InputMode::QuickMove => (" Move to path (Tab=complete, Enter=move, Esc=cancel) ", app.input.clone()),
        InputMode::ServicesMenu => (" Services ([u]ser/[y]stem, [r]estart, [s]top, [Esc]) ", String::new()),
        InputMode::KeybindMenu => (" Keybinds ([Space]=Rebind, [a]=Add, [r]=Reset, [R]=Factory, [u]=Undo, [Esc]) ", String::new()),
        InputMode::KeybindCapture => (" Press a key to bind... (Esc=cancel) ", String::new()),
        InputMode::AddCustomBind => (" !command or /path (Enter=next, Esc=back) ", app.input.clone()),
        InputMode::KeyPicker => (" Key Picker (s/shared · g/generated · Enter copy · Esc close) ", String::new()),
    }
}

// ============ help (dynamic, adaptive pagination) ============

fn truncate_chars(s: &str, n: usize) -> String {
    if s.chars().count() > n { s.chars().take(n).collect::<String>() + "…" } else { s.to_string() }
}

#[allow(non_snake_case)]
#[allow(non_snake_case)]
pub fn build_help_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let k = |kb: &KeyBind| all_keys_for(app, kb);
    let hdr = |t: &str| Line::from(vec![Span::styled(format!(" {}", t), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))]);
    let kl = |keys: &str, desc: &str| Line::from(vec![
        Span::styled(format!(" {:<16}", keys), Style::default().fg(theme.muted)),
        Span::styled(desc.to_string(), Style::default().fg(theme.text)),
    ]);
    let note = |t: &str| Line::from(vec![Span::styled(format!(" {}", t), Style::default().fg(theme.modified_color))]);

    let mut out: Vec<Line> = vec![];
    out.push(note("⟳ External edits auto-versioned"));
    out.push(hdr("NAVIGATION"));
    out.push(kl(&format!("{} {}", k(&KeyBind::NextItem), k(&KeyBind::PrevItem)), "Move down / up"));
    out.push(kl(&format!("{} {} {}", k(&KeyBind::JumpTop), k(&KeyBind::JumpMid), k(&KeyBind::JumpBot)), "Top / mid / bottom"));
    out.push(kl(&format!("{} {}", k(&KeyBind::HalfPageDown), k(&KeyBind::HalfPageUp)), "Half page down / up"));
    out.push(kl(&k(&KeyBind::GotoPrefix), "Goto prefix (gg)"));
    out.push(kl(&k(&KeyBind::JumpList), "Jump list (bm+recent)"));
    out.push(kl(&k(&KeyBind::EditFile), "Edit file / open dir"));
    out.push(kl(&format!("{} {}", k(&KeyBind::Expand), k(&KeyBind::Collapse)), "Expand / collapse"));
    out.push(kl(&k(&KeyBind::FileInfo), "File info"));
    out.push(kl("v", "Versions / restore"));
    out.push(kl("y", "Copy path to clipboard"));
    out.push(kl("Mouse", "click select · dbl toggle"));

    out.push(hdr("FILES"));
    out.push(kl(&k(&KeyBind::AddSymlink), "Add symlink (Tab=fzf)"));
    out.push(kl(&k(&KeyBind::AddFolder), "Create folder"));
    out.push(kl(&k(&KeyBind::Rename), "Rename / move in root"));
    out.push(kl(&k(&KeyBind::Delete), "Delete → trash (press 2×)"));
    out.push(kl(&format!("{} {}", k(&KeyBind::Cut), k(&KeyBind::Paste)), "Cut / paste"));
    out.push(kl(&k(&KeyBind::Chmod), "Permissions (chmod)"));
    out.push(kl(&k(&KeyBind::QuickMove), "Quick move to path"));
    out.push(kl(&k(&KeyBind::Shell), "Shell ({f}, !alias)"));
    out.push(kl(&format!("{} {}", k(&KeyBind::ToggleHidden), k(&KeyBind::FilterBookmarks)), "Hidden / bookmarks filter"));
    out.push(kl(&k(&KeyBind::Undo), "Undo delete/rename/move"));

    out.push(hdr("ARCHIVE / DEPLOY"));
    out.push(kl(&format!("{} {}", k(&KeyBind::ToggleSelection), k(&KeyBind::VisualSelect)), "Select / visual mode"));
    out.push(kl(&k(&KeyBind::Archive), "Archive selected"));
    out.push(kl("D (on .zip)", "Deploy (3-way merge)"));

    out.push(hdr("HOOKS & STATE"));
    out.push(kl(&k(&KeyBind::OpenHooks), "Per-object hooks"));
    out.push(kl(&k(&KeyBind::OpenSettings), "Settings"));
    out.push(kl(&format!("{} {} {}", k(&KeyBind::OpenRoots), k(&KeyBind::ToggleHostFilter), k(&KeyBind::ToggleBookmark)), "Roots/host/bookmark"));
    out.push(kl(&format!("{} {}", k(&KeyBind::AddNote), k(&KeyBind::AddHostTag)), "Note / host tag"));
    out.push(kl(&k(&KeyBind::GitPush), "Git push (auto-commit)"));
    out.push(kl(&k(&KeyBind::Search), "Search"));
    out.push(kl(&format!("{} {}", k(&KeyBind::SelectEditor), k(&KeyBind::OpenServices)), "Editor / services"));
    out.push(kl(&k(&KeyBind::Quit), "Quit"));

    let customs: Vec<(String, String)> = app.keymap.iter()
        .filter_map(|(key, v)| if let KeyBind::Custom(c) = v { Some((key.clone(), c.clone())) } else { None })
        .collect();
    if !customs.is_empty() {
        out.push(hdr("CUSTOM KEYBINDS"));
        let rev: std::collections::HashMap<&String, &String> = app.shell_aliases.iter().map(|(n, c)| (c, n)).collect();
        for (key, cmd) in customs {
            let body = if let Some(p) = cmd.strip_prefix("edit:") { format!("📝 {}", truncate_chars(p, 24)) }
                else { match rev.get(&cmd) { Some(n) => format!("⚡ {}", n), None => format!("⚡ {}", truncate_chars(&cmd, 24)) } };
            out.push(Line::from(vec![
                Span::styled(format!(" {:<16}", key), Style::default().fg(theme.muted)),
                Span::styled(body, Style::default().fg(theme.text)),
            ]));
        }
    }

    if !app.shell_aliases.is_empty() {
        let bound: std::collections::HashSet<&String> = app.keymap.values()
            .filter_map(|v| if let KeyBind::Custom(c) = v { Some(c) } else { None }).collect();
        out.push(hdr("SHELL ALIASES (!name in ! prompt)"));
        for (n, c) in &app.shell_aliases {
            let st = if bound.contains(c) { "bound" } else { "unbound" };
            out.push(Line::from(vec![
                Span::styled(format!(" {:<16}", n), Style::default().fg(theme.muted)),
                Span::styled(truncate_chars(c, 26), Style::default().fg(theme.text)),
                Span::styled(format!(" [{}]", st), Style::default().fg(theme.config_color)),
            ]));
        }
    }

    let mut active: Vec<String> = app.hooks.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(hk, _)| hk.clone()).collect();
    if !active.is_empty() {
        active.sort();
        out.push(hdr("ACTIVE GLOBAL HOOKS"));
        for hk in active {
            let v = app.hooks.get(&hk).cloned().unwrap_or_default();
            out.push(Line::from(vec![
                Span::styled(format!(" {:<16}", hk), Style::default().fg(theme.muted)),
                Span::styled(truncate_chars(&v, 28), Style::default().fg(theme.accent)),
            ]));
        }
    }
    out
}

fn draw_help(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = centered_rect(80, 80, f.size()); // was (80, 90) — bigger box, fewer pages
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" Confy Help ", Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let inner = block.inner(area);

    let all = build_help_lines(app, theme);
    let total = all.len();
    if total == 0 { f.render_widget(block, area); return; }

    let vert = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    let (body, footer) = (vert[0], vert[1]);

    let rows = (body.height as usize).max(1);
    // two columns only when wide enough AND content would overflow one page
    let two_col = body.width >= 100 && total > rows;
    let capacity = rows * if two_col { 2 } else { 1 };

    app.help_scroll = app.help_scroll.min(total - 1);
    let pages = (total + capacity - 1) / capacity;
    let page = (app.help_scroll / capacity).min(pages - 1);
    app.help_page_len = capacity; app.help_total = total; app.help_page = page;

    let start = page * capacity;
    let visible = &all[start..(start + capacity).min(total)];

    if two_col {
        let half = (visible.len() + 1) / 2;
        let (left, right) = visible.split_at(half);
        let cols = Layout::default().direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)]).split(body);
        f.render_widget(Paragraph::new(left.to_vec()).wrap(Wrap { trim: false }), cols[0]);
        f.render_widget(Paragraph::new(right.to_vec()).wrap(Wrap { trim: false }), cols[1]);
    } else {
        f.render_widget(Paragraph::new(visible.to_vec()).wrap(Wrap { trim: false }), body);
    }

    let footer_line = if pages > 1 {
        Line::from(vec![
            Span::styled(format!(" Page {}/{}  ", page + 1, pages), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled("↑↓ line · ←→ page · g/Home top · End bottom · Esc close", Style::default().fg(theme.muted)),
        ])
    } else {
        Line::from(Span::styled(" Esc to close ", Style::default().fg(theme.muted)))
    };
    f.render_widget(Paragraph::new(footer_line), footer);
    f.render_widget(block, area);
}

fn draw_editor_select(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = centered_rect(40, 50, f.size());
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" Select Editor ", Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let items: Vec<ListItem> = app.available_editors.iter().map(|e| {
        let star = if app.selected_editor.as_deref() == Some(e.as_str()) { "★ " } else { "  " };
        ListItem::new(Line::from(vec![Span::raw(star), Span::styled(e.clone(), Style::default().fg(theme.text))]))
    }).collect();
    let inner = block.inner(area);
    let editor_layout = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    f.render_stateful_widget(List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight), editor_layout[0], &mut app.editor_state);
    f.render_widget(Paragraph::new(" d set default · Enter use once · Esc close ").style(Style::default().fg(theme.muted)), editor_layout[1]);
    f.render_widget(block, area);
}

fn draw_settings(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = centered_rect(60, 65, f.size());
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" Settings ", Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let s = &app.state_data.settings;
    let on = |b: bool| -> &'static str { if b { "[ON]" } else { "[OFF]" } };
    let mut items: Vec<ListItem> = Vec::new();
    let mut action_rows: Vec<usize> = Vec::with_capacity(SETTINGS_MENU.len());
    let mut last_cat = "";
    for &action in SETTINGS_MENU {
        let cat = action.category();
        if cat != last_cat {
            items.push(ListItem::new(Line::from(vec![Span::styled(format!("--- {} ---", cat), Style::default().fg(theme.muted).add_modifier(Modifier::BOLD))])));
            last_cat = cat;
        }
        let (label, value) = match action {
            SettingAction::CycleTheme => ("Theme", s.theme_name.clone()),
            SettingAction::AddCustomTheme => ("Add Custom Theme", format!("({} added)", s.custom_themes.len())),
            SettingAction::CycleColorMode => ("Color Mode", s.color_mode.label().into()),
            SettingAction::ToggleFolders => ("Folders First", on(s.folders_first).into()),
            SettingAction::ToggleSizes => ("Show Sizes", on(s.show_sizes).into()),
            SettingAction::ToggleHidden => ("Show Hidden", on(s.show_hidden_by_default).into()),
            SettingAction::ToggleSymlinks => ("Show Symlinks", on(s.show_symlinks_in_list).into()),
            SettingAction::InputListWidth => ("List Width", format!("[{}]", s.list_width)),
            SettingAction::SelectEditor => ("Default Editor", s.default_editor.clone().unwrap_or_else(|| "None".into())),
            SettingAction::ToggleVersioning => ("Versioning", on(s.enable_versioning).into()),
            SettingAction::InputVersionLimit => ("Version Limit", format!("[{}]", s.version_limit)),
            SettingAction::CycleDeployMode => ("Default Deploy", s.default_deploy_mode.label().into()),
            SettingAction::CycleBackupBehavior => ("Backup Behavior", s.backup_behavior.label().into()),
            SettingAction::CycleOverwrite => ("Confirm Overwrite", s.confirm_overwrite_deploy.label().into()),
            SettingAction::ToggleConfirmDel => ("Confirm Deletes", on(s.confirm_delete).into()),
            SettingAction::ToggleConfirmMove => ("Confirm Moves", on(s.confirm_moves).into()),
            SettingAction::ToggleMouse => ("Mouse", on(s.enable_mouse).into()),
            SettingAction::CycleSearchMode => ("Search Mode", s.search_mode.label().into()),
            SettingAction::ToggleHooks => ("Hooks Enabled", on(s.enable_hooks).into()),
            SettingAction::EditGlobalHooks => ("Edit Global Hooks", "Open Menu".into()),
            SettingAction::EditKeybinds => ("Edit Keybinds", "Open Menu".into()),
            SettingAction::ToggleTerminalWindow => ("New Terminal Window", on(s.open_terminal_in_new_window).into()),
            SettingAction::OpenTrash => ("Open Trash", "TUI Manager".into()),
            SettingAction::CycleTrashRetention => {
                let d = s.trash_retention_days;
                ("Trash Retention", if d == 0 { "Never".into() } else { format!("{}d", d) })
            }
        };
        items.push(ListItem::new(Line::from(vec![
            Span::raw(format!(" {:<20}: ", label)),
            Span::styled(value, Style::default().fg(theme.accent)),
        ])));
        action_rows.push(items.len() - 1);
    }
    let inner = block.inner(area);
    let setting_layout = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    let mut display_state = ratatui::widgets::ListState::default();
    if let Some(action_index) = app.settings_state.selected() {
        if let Some(&row) = action_rows.get(action_index) { display_state.select(Some(row)); }
    }
    f.render_stateful_widget(List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight), setting_layout[0], &mut display_state);
    f.render_widget(Paragraph::new(" Enter change · H/M/L jump · Esc close ").style(Style::default().fg(theme.muted)), setting_layout[1]);
    f.render_widget(block, area);
}

fn draw_input(f: &mut Frame, app: &App, theme: &Theme, title: &str) {
    let area = centered_rect(70, 7, f.size());
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    f.render_widget(Paragraph::new(app.input.clone()).block(block), area);
}

fn draw_hooks(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = centered_rect(66, 50, f.size());
    f.render_widget(Clear, area);
    let is_object = app.hooks_target_path.is_some();
    let hook_list = if is_object { OBJECT_HOOK_LIST } else { HOOK_LIST };
    let title = if is_object { format!(" Object Hooks: {} ", app.hooks_target_path.as_deref().unwrap_or("?")) } else { " Global Hooks ".into() };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(title, Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let items: Vec<ListItem> = hook_list.iter().map(|h| {
        let raw = app.current_hook_value(h);
        let (mark, mark_style, value, value_style) = if raw.trim().is_empty() {
            ("·", Style::default().fg(theme.muted), "None".to_string(), Style::default().fg(theme.muted))
        } else {
            match ops::validate_hook(&raw) {
                Ok(_) => ("✓", Style::default().fg(Color::Green), raw.clone(), Style::default().fg(theme.text)),
                Err(e) => ("✗", Style::default().fg(Color::Red), format!("{}  ({})", raw, e), Style::default().fg(Color::Red)),
            }
        };
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {} ", mark), mark_style),
            Span::styled(format!("{:<18}: ", h), Style::default().fg(theme.muted)),
            Span::styled(value, value_style),
        ]))
    }).collect();
    let inner = block.inner(area);
    let layout = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    f.render_stateful_widget(List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight), layout[0], &mut app.hooks_state);
    f.render_widget(Paragraph::new(" Enter edit · t test · d clear · Esc back ").style(Style::default().fg(theme.muted)), layout[1]);
    f.render_widget(block, area);
}

fn draw_roots(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = centered_rect(50, 50, f.size());
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" Roots ", Style::default().add_modifier(Modifier::BOLD).fg(theme.text)));
    let items: Vec<ListItem> = app.roots.iter().map(|r| {
        let pfx = if r == &app.confy_dir { "★ " } else { "  " };
        ListItem::new(Line::from(vec![Span::raw(pfx), Span::styled(r.display().to_string(), Style::default().fg(theme.text))]))
    }).collect();
    let inner = block.inner(area);
    let layout = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
    f.render_stateful_widget(List::new(items).highlight_symbol("▶ ").highlight_style(theme.highlight), layout[0], &mut app.root_state);
    f.render_widget(Paragraph::new(" Enter switch · a add · d delete · Esc close ").style(Style::default().fg(theme.muted)), layout[1]);
    f.render_widget(block, area);
}

// ============ key dispatch ============

fn handle_key(app: &mut App, terminal: &mut Tui, key: event::KeyEvent) -> bool {
    // Ctrl+C: hard quit in every mode
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        tracing::info!("Ctrl+C, exiting");
        return true;
    }
    // 'g' prefix (Normal mode only)
    if app.pending_g {
        app.pending_g = false;
        if app.input_mode == InputMode::Normal && key.code == KeyCode::Char('g') { app.jump_top(); }
        return false;
    }
    match app.input_mode {
        InputMode::Normal => handle_normal(app, terminal, key),
        InputMode::Settings => { handle_settings(app, key); false }
        InputMode::SettingsInput => { handle_settings_input(app, key); false }
        InputMode::HooksMenu => { handle_hooks_menu(app, key); false }
        InputMode::HookPathInput => { handle_hook_path_input(app, key); false }
        InputMode::RootMenu => { handle_root_menu(app, terminal, key); false }
        InputMode::AddingRoot => { handle_adding_root(app, key); false }
        InputMode::SelectEditor => { handle_editor_select_mode(app, key); false }
        InputMode::VisualSelect => { handle_visual_select(app, key); false }
        InputMode::AddingPath | InputMode::AddingAlias | InputMode::AddingDir
        | InputMode::AddingNote | InputMode::AddingTag => { handle_text_input(app, key); false }
        InputMode::Renaming => { handle_renaming(app, key); false }
        InputMode::Search => { handle_search(app, key); false }
        InputMode::VersionSelect => { handle_version_select(app, key); false }
        InputMode::SudoPrompt => { handle_sudo_prompt(app, key); false }
        InputMode::Shell => { handle_shell(app, key); false }
        InputMode::AddingThemePath => { handle_theme_path(app, key); false }
        InputMode::AddingThemeName => { handle_theme_name(app, key); false }
        InputMode::Info => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('i') | KeyCode::Char('q') | KeyCode::Enter) { app.input_mode = InputMode::Normal; }
            false
        }
        InputMode::Help => { handle_help(app, key); false }
        InputMode::TrashManagement => { handle_trash(app, key); false }
        InputMode::ChmodPrompt => { handle_chmod_prompt(app, key); false }
        InputMode::QuickMove => { handle_quick_move(app, key); false }
        InputMode::ServicesMenu => { handle_services_menu(app, key); false }
        InputMode::KeybindMenu => { handle_keybind_menu(app, key); false }
        InputMode::KeybindCapture => { handle_keybind_capture(app, key); false }
        InputMode::AddCustomBind => { handle_add_custom_bind(app, key); false }
        InputMode::KeyPicker => { handle_key_picker(app, key); false }
        InputMode::DeployConfirm => { handle_deploy_confirm(app, key); false }
    }
}

fn handle_normal(app: &mut App, _terminal: &mut Tui, key: event::KeyEvent) -> bool {
    // Esc: clear ALL transient state before anything else
    if key.code == KeyCode::Esc {
        let had = app.cut_source.is_some() || app.yank_source.is_some() || app.pending_delete.is_some()
            || app.jump_list || app.bookmarks_only || app.host_filter || app.preview_pinned;
        app.cut_source = None; app.yank_source = None; app.pending_delete = None;
        if app.jump_list || app.bookmarks_only || app.host_filter {
            app.jump_list = false; app.bookmarks_only = false; app.host_filter = false;
            app.refresh_items();
        }
        if app.preview_pinned { app.preview_pinned = false; app.update_preview(); }
        if had { app.set_status("Cancelled."); }
        return false;
    }

    let ks = key_event_to_string(key);

    // Hard-wired reserved keys — these survive ANY keybinds.json damage:
    // v = versions (your restore hatch), D = deploy, y = copy path.
    match ks.as_str() {
        "v" => { app.enter_version_mode(); return false; }
        "D" => { app.start_deploy(); return false; }
        "y" => { app.yank_item(); return false; }
        _ => {}
    }

    let Some(bind) = app.keymap.get(&ks).cloned() else { return false };
    match bind {
        KeyBind::Quit => return true,
        KeyBind::GotoPrefix => app.pending_g = true,
        KeyBind::NextItem => app.next(),
        KeyBind::PrevItem => app.previous(),
        KeyBind::HalfPageDown => app.half_down(),
        KeyBind::HalfPageUp => app.half_up(),
        KeyBind::JumpTop => app.jump_top(),
        KeyBind::JumpMid => app.jump_mid(),
        KeyBind::JumpBot => app.jump_bot(),
        KeyBind::Expand => app.expand_selected(),
        KeyBind::Collapse => app.collapse_selected(),
        KeyBind::EditFile => app.edit_selected(),
        KeyBind::AddSymlink => app.begin_add(),
        KeyBind::AddFolder => app.begin_add_folder(),
        KeyBind::Rename => app.begin_rename(),
        KeyBind::Delete => app.request_delete(),
        KeyBind::Cut => app.cut_item(),
        KeyBind::Paste => app.paste_item(),
        KeyBind::Chmod => app.begin_chmod(),
        KeyBind::QuickMove => app.begin_quick_move(),
        KeyBind::ToggleBookmark => app.toggle_bookmark(),
        KeyBind::FilterBookmarks => {
            app.bookmarks_only = !app.bookmarks_only;
            if app.bookmarks_only { app.jump_list = false; }
            app.refresh_items();
            app.set_status(if app.bookmarks_only { "Bookmarks only (Esc=clear)." } else { "Showing all." });
        }
        KeyBind::ToggleHidden => { app.show_hidden = !app.show_hidden; app.refresh_items(); }
        KeyBind::AddNote => app.begin_add_note(),
        KeyBind::AddHostTag => app.begin_add_tag("host"),
        KeyBind::OpenSettings => { app.settings_state.select(Some(0)); app.input_mode = InputMode::Settings; }
        KeyBind::OpenHooks => app.enter_object_hooks(),
        KeyBind::OpenRoots => {
            let idx = app.roots.iter().position(|r| r == &app.confy_dir).unwrap_or(0);
            app.root_state.select(Some(idx));
            app.input_mode = InputMode::RootMenu;
        }
        KeyBind::SelectEditor => app.enter_editor_select(None),
        KeyBind::Undo => app.undo(),
        KeyBind::FileInfo => app.input_mode = InputMode::Info,
        KeyBind::GitPush => {
            ops::disable_raw_and_leave_alt();
            let r = ops::git_push(&app.confy_dir, &[]);
            ops::restore_raw_and_enter_alt();
            app.needs_clear = true;
            match r { Ok(m) => app.set_status(m), Err(e) => app.set_status(format!("Push failed: {}", e)) }
        }
        KeyBind::ToggleSelection => app.toggle_selection(),
        KeyBind::VisualSelect => app.input_mode = InputMode::VisualSelect,
        KeyBind::Archive => app.archive_selected(),
        KeyBind::Shell => { app.input_mode = InputMode::Shell; app.input.clear(); }
        KeyBind::Search => app.start_search(),
        KeyBind::Help => { app.input_mode = InputMode::Help; app.help_scroll = 0; }
        KeyBind::ToggleHostFilter => app.toggle_host_filter(),
        KeyBind::OpenServices => app.enter_services_menu(),
        KeyBind::JumpList => app.toggle_jump_list(),
        KeyBind::ClearScreen => app.needs_clear = true,
        KeyBind::ToggleTerminalPreview => app.toggle_preview_terminal(),
        KeyBind::Custom(cmd) => app.run_custom_bind(&cmd),
    }
    false
}

// ---- text input modes ----

fn push_char(app: &mut App, c: char) {
    if c.is_control() { return; }
    app.input.push(c);
}

fn handle_text_input(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.submit_add_step(),
        KeyCode::Esc => app.cancel_input(),
        KeyCode::Tab => {
            if app.input_mode == InputMode::AddingPath {
                let src = "find \"$HOME\" -maxdepth 4 -type f -not -path '*/.git/*' -not -path '*/.cache/*' -not -path '*/node_modules/*' 2>/dev/null | head -n 5000";
                if let Some(p) = ops::fzf_pick("path> ", src) { app.input = p; }
            }
        }
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_renaming(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.submit_rename(),
        KeyCode::Esc => app.cancel_input(),
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_chmod_prompt(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.submit_chmod(),
        KeyCode::Esc => app.cancel_input(),
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_quick_move(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.submit_quick_move(),
        KeyCode::Esc => app.cancel_input(),
        KeyCode::Tab => complete_path_input(app),
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_sudo_prompt(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.submit_sudo_password(),
        KeyCode::Esc => app.cancel_input(),
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => app.hide_password = !app.hide_password,
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_shell(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.run_shell_input(std::time::Duration::from_secs(600)),
        KeyCode::Esc => { app.input_mode = InputMode::Normal; app.input.clear(); app.preview_pinned = false; app.update_preview(); }
        KeyCode::Tab => complete_path_input(app),
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => {
            if app.input == "exit" || app.input == "quit" {
                app.input_mode = InputMode::Normal;
                app.preview_pinned = false;
                app.input.clear();
                app.update_preview();
            } else {
                push_char(app, c);
            }
        }
        _ => {}
    }
}

fn handle_deploy_confirm(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Char('e') => app.browse_archive(),
        KeyCode::Char('y') => app.apply_deploy(),
        KeyCode::Char('n') | KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.deploy_target = None;
            app.preview_pinned = false;
            app.update_preview();
        }
        _ => {}
    }
}

fn handle_search(app: &mut App, key: event::KeyEvent) {
    if app.return_mode == InputMode::ServicesMenu {
        match key.code {
            KeyCode::Esc => { app.service_filter.clear(); app.input.clear(); app.input_mode = InputMode::ServicesMenu; }
            KeyCode::Enter => { app.service_filter = app.input.clone(); app.input.clear(); app.input_mode = InputMode::ServicesMenu; }
            KeyCode::Backspace => { app.input.pop(); }
            KeyCode::Char(c) if key.modifiers.is_empty() => app.input.push(c),
            _ => {}
        }
        return;
    }
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input.clear();
            app.apply_filter();
            app.state.select(Some(0));
            app.update_preview();
        }
        KeyCode::Enter => {
            app.input_mode = InputMode::Normal;
            app.input.clear();
            app.apply_filter();
            app.edit_selected();
        }
        KeyCode::Down | KeyCode::Char('j') => app.next(),
        KeyCode::Up | KeyCode::Char('k') => app.previous(),
        KeyCode::Backspace => { app.input.pop(); app.apply_filter(); app.update_preview(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => { push_char(app, c); app.apply_filter(); app.update_preview(); }
        _ => {}
    }
}

fn handle_version_select(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Esc => app.exit_version_mode(),
        KeyCode::Enter => app.restore_version(),
        KeyCode::Char('d') => app.show_diff(),
        KeyCode::Char('j') | KeyCode::Down => app.next_version(),
        KeyCode::Char('k') | KeyCode::Up => app.previous_version(),
        _ => {}
    }
}

fn handle_theme_path(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.submit_theme_path(),
        KeyCode::Esc => app.cancel_input(),
        KeyCode::Tab => {
            let src = "find \"$HOME/.config\" -maxdepth 3 \\( -name '*.conf' -o -name '*.toml' -o -name '*.json' -o -name '*.css' -o -name '*.lua' -o -name '*.yml' -o -name '*.yaml' \\) -type f 2>/dev/null | head -n 2000";
            if let Some(p) = ops::fzf_pick("theme> ", src) { app.input = p; }
        }
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_theme_name(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.submit_theme_name(),
        KeyCode::Esc => app.cancel_input(),
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_help(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.input_mode = InputMode::Normal,
        KeyCode::Up | KeyCode::Char('k') => app.help_scroll = app.help_scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => {
            if app.help_total > 0 { app.help_scroll = (app.help_scroll + 1).min(app.help_total - 1); }
        }
        KeyCode::Left | KeyCode::Char('h') => app.help_scroll = app.help_scroll.saturating_sub(app.help_page_len.max(1)),
        KeyCode::Right | KeyCode::Char('l') => {
            if app.help_total > 0 { app.help_scroll = (app.help_scroll + app.help_page_len.max(1)).min(app.help_total - 1); }
        }
        KeyCode::Home | KeyCode::Char('g') => app.help_scroll = 0,
        KeyCode::End => { if app.help_total > 0 { app.help_scroll = app.help_total - 1; } }
        _ => {}
    }
}

fn handle_visual_select(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => { app.toggle_selection(); app.next(); }
        KeyCode::Char('k') | KeyCode::Up => { app.previous(); app.toggle_selection(); }
        KeyCode::Char(' ') | KeyCode::Enter => app.toggle_selection(),
        KeyCode::Esc | KeyCode::Char('v') | KeyCode::Char('V') => app.input_mode = InputMode::Normal,
        _ => {}
    }
}

fn handle_editor_select_mode(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.submit_editor_select(false),
        KeyCode::Char('d') => app.submit_editor_select(true),
        KeyCode::Char('j') | KeyCode::Down => {
            let n = app.available_editors.len();
            if n > 0 { let i = (app.editor_state.selected().unwrap_or(0) + 1).min(n - 1); app.editor_state.select(Some(i)); }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let i = app.editor_state.selected().unwrap_or(0).saturating_sub(1);
            app.editor_state.select(Some(i));
        }
        KeyCode::Esc => {
            app.tui_edit_target = None;
            app.return_from_menu();
        }
        _ => {}
    }
}

fn handle_settings(app: &mut App, key: event::KeyEvent) {
    let n = SETTINGS_MENU.len();
    match key.code {
        KeyCode::Esc => app.input_mode = InputMode::Normal,
        KeyCode::Enter => {
            if let Some(i) = app.settings_state.selected() {
                if let Some(&a) = SETTINGS_MENU.get(i) { app.execute_setting(a); }
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if n > 0 { let i = (app.settings_state.selected().unwrap_or(0) + 1).min(n - 1); app.settings_state.select(Some(i)); }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let i = app.settings_state.selected().unwrap_or(0).saturating_sub(1);
            app.settings_state.select(Some(i));
        }
        KeyCode::Char('H') => app.settings_state.select(Some(0)),
        KeyCode::Char('M') => app.settings_state.select(Some(n / 2)),
        KeyCode::Char('L') => app.settings_state.select(Some(n.saturating_sub(1))),
        _ => {}
    }
}

fn handle_settings_input(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            if app.settings_input_target == "service_name" { app.add_service_job(); }
            else { app.submit_settings_input(); }
        }
        KeyCode::Esc => { app.input.clear(); app.input_mode = InputMode::Settings; }
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if c.is_ascii_digit() && key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_hooks_menu(app: &mut App, key: event::KeyEvent) {
    let list = if app.hooks_target_path.is_some() { OBJECT_HOOK_LIST } else { HOOK_LIST };
    let n = list.len();
    match key.code {
        KeyCode::Esc => {
            app.hooks_target_path = None;
            app.return_from_menu();
        }
        KeyCode::Enter => {
            if let Some(i) = app.hooks_state.selected() {
                if let Some(&h) = list.get(i) {
                    app.hook_edit_type = h.to_string();
                    app.input = app.current_hook_value(h);
                    app.input_mode = InputMode::HookPathInput;
                }
            }
        }
        KeyCode::Char('t') => {
            if let Some(i) = app.hooks_state.selected() {
                if let Some(&h) = list.get(i) { app.test_hook_value(h); }
            }
        }
        KeyCode::Char('d') => {
            if let Some(i) = app.hooks_state.selected() {
                if let Some(&h) = list.get(i) { app.clear_hook_value(h); }
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if n > 0 { let i = (app.hooks_state.selected().unwrap_or(0) + 1).min(n - 1); app.hooks_state.select(Some(i)); }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let i = app.hooks_state.selected().unwrap_or(0).saturating_sub(1);
            app.hooks_state.select(Some(i));
        }
        _ => {}
    }
}

fn handle_hook_path_input(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => { let v = app.input.clone(); app.set_hook_value(v); }
        KeyCode::Esc => { app.input.clear(); app.input_mode = InputMode::HooksMenu; }
        KeyCode::Tab => complete_path_input(app),
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_root_menu(app: &mut App, terminal: &mut Tui, key: event::KeyEvent) {
    match key.code {
        KeyCode::Esc => app.input_mode = InputMode::Normal,
        KeyCode::Enter => {
            if let Some(i) = app.root_state.selected() {
                if let Some(r) = app.roots.get(i).cloned() {
                    if r == app.confy_dir { app.set_status("Already active."); return; }
                    if app.switch_root(r) {
                        let _ = terminal.clear();
                        app.needs_clear = true;
                        app.set_status("Root switched.");
                    }
                }
            }
        }
        KeyCode::Char('a') => { app.input_mode = InputMode::AddingRoot; app.input.clear(); }
        KeyCode::Char('d') => app.remove_root(),
        KeyCode::Char('j') | KeyCode::Down => {
            let n = app.roots.len();
            if n > 0 { let i = (app.root_state.selected().unwrap_or(0) + 1).min(n - 1); app.root_state.select(Some(i)); }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let i = app.root_state.selected().unwrap_or(0).saturating_sub(1);
            app.root_state.select(Some(i));
        }
        _ => {}
    }
}

fn handle_adding_root(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.add_root(),
        KeyCode::Esc => { app.input.clear(); app.input_mode = InputMode::RootMenu; }
        KeyCode::Tab => complete_path_input(app),
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

fn handle_trash(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Esc => app.return_from_menu(),
        KeyCode::Char('r') => app.restore_trash_item(),
        KeyCode::Char('d') => {
            // permanent delete → double-press, same mental model as normal delete
            let sel = app.trash_state.selected().and_then(|i| app.trash_items.get(i).cloned());
            let Some(p) = sel else { return };
            if app.pending_delete.as_ref() == Some(&p) {
                app.pending_delete = None;
                app.purge_trash_item();
            } else {
                app.pending_delete = Some(p.clone());
                let nm = p.file_name().unwrap_or_default().to_string_lossy();
                app.set_sticky_status(format!("PERMANENTLY delete {}? Press d again", nm));
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let n = app.trash_items.len();
            if n > 0 { let i = (app.trash_state.selected().unwrap_or(0) + 1).min(n - 1); app.trash_state.select(Some(i)); }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let i = app.trash_state.selected().unwrap_or(0).saturating_sub(1);
            app.trash_state.select(Some(i));
        }
        _ => {}
    }
}

fn handle_services_menu(app: &mut App, key: event::KeyEvent) {
    let visible_indices = || app.available_services.iter().enumerate().filter(|(_, (name, _, desc))| {
        app.service_filter.is_empty() || name.to_lowercase().contains(&app.service_filter.to_lowercase()) || desc.to_lowercase().contains(&app.service_filter.to_lowercase())
    }).map(|(index, _)| index).collect::<Vec<_>>();
    match key.code {
        KeyCode::Esc => { app.service_filter.clear(); app.input.clear(); app.input_mode = InputMode::Normal; }
        KeyCode::Char('a') => app.begin_add_service(),
        KeyCode::Char('/') => { app.input.clear(); app.input_mode = InputMode::Search; app.return_mode = InputMode::ServicesMenu; },
        KeyCode::Char('d') => app.delete_selected_service(),
        KeyCode::Char('u') => app.switch_service_scope(true),
        KeyCode::Char('y') => app.switch_service_scope(false),
        KeyCode::Char('r') => app.restart_selected_service(),
        KeyCode::Char('s') => app.stop_selected_service(),
        KeyCode::Char('j') | KeyCode::Down => {
            let visible = visible_indices();
            if let Some(position) = visible.iter().position(|index| Some(*index) == app.service_state.selected()) {
                if let Some(&index) = visible.get((position + 1).min(visible.len() - 1)) { app.service_state.select(Some(index)); }
            } else if let Some(&index) = visible.first() { app.service_state.select(Some(index)); }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let visible = visible_indices();
            if let Some(position) = visible.iter().position(|index| Some(*index) == app.service_state.selected()) {
                if let Some(&index) = visible.get(position.saturating_sub(1)) { app.service_state.select(Some(index)); }
            } else if let Some(&index) = visible.first() { app.service_state.select(Some(index)); }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.service_filter.push(c);
            let query = app.service_filter.to_lowercase();
            if let Some((index, _)) = app.available_services.iter().enumerate().find(|(_, (name, _, desc))| name.to_lowercase().contains(&query) || desc.to_lowercase().contains(&query)) {
                app.service_state.select(Some(index));
            }
        }
        _ => {}
    }
}

fn handle_keybind_menu(app: &mut App, key: event::KeyEvent) {
    let items = keybind_items(app);
    let n = items.len();
    let clamp = |app: &mut App, i: usize| { if n > 0 { app.keybind_state.select(Some(i.min(n - 1))); } };
    match key.code {
        KeyCode::Esc => { app.pending_reset = None; app.return_from_menu(); }
        KeyCode::Enter | KeyCode::Char(' ') => {
            app.pending_reset = None;
            if let Some(i) = app.keybind_state.selected() {
                if let Some(kb) = items.get(i).cloned() {
                    app.pending_keybind = Some(kb);
                    app.input_mode = InputMode::KeybindCapture;
                }
            }
        }
        KeyCode::Char('a') => { app.pending_reset = None; app.input_mode = InputMode::AddCustomBind; app.input.clear(); }
        KeyCode::Char('r') => {
            // reset binds, keep aliases — double-press confirm
            if app.pending_reset == Some(1) {
                app.pending_reset = None;
                app.reset_keymap(false);
            } else {
                app.pending_reset = Some(1);
                app.set_sticky_status("Reset keybinds (aliases kept)? Press r again to confirm");
            }
        }
        KeyCode::Char('R') => {
            // full factory reset — double-press confirm
            if app.pending_reset == Some(2) {
                app.pending_reset = None;
                app.reset_keymap(true);
            } else {
                app.pending_reset = Some(2);
                app.set_sticky_status("FACTORY RESET binds + aliases? Press R again to confirm");
            }
        }
        KeyCode::Char('u') => {
            app.pending_reset = None;
            if !app.restore_keymap_backup() { app.set_status("No pre-reset backup to restore."); }
        }
        KeyCode::Char('j') | KeyCode::Down => { app.pending_reset = None; clamp(app, app.keybind_state.selected().unwrap_or(0) + 1); }
        KeyCode::Char('k') | KeyCode::Up => { app.pending_reset = None; clamp(app, app.keybind_state.selected().unwrap_or(0).saturating_sub(1)); }
        _ => {}
    }
}

fn handle_keybind_capture(app: &mut App, key: event::KeyEvent) {
    if key.code == KeyCode::Esc {
        app.pending_keybind = None;
        app.input_mode = InputMode::KeybindMenu;
        return;
    }
    let ks = key_event_to_string(key);
    if ks.is_empty() { return; }
    // reserved keys (v/D/y/Esc/Ctrl+c) can never be shadowed by a bind
    if crate::config::is_reserved_key(&ks) {
        app.set_sticky_status(format!("[{}] is reserved and can't be bound", ks));
        return;
    }
    if let Some(kb) = app.pending_keybind.take() {
        // one bind per custom command: clear stale duplicates first
        if let KeyBind::Custom(ref cmd) = kb {
            let stale: Vec<String> = app.keymap.iter()
                .filter(|(_, v)| matches!(v, KeyBind::Custom(c) if c == cmd))
                .map(|(k, _)| k.clone()).collect();
            for k in stale { app.keymap.remove(&k); }
        }
        app.keymap.insert(ks.clone(), kb);
        app.save_keymap(); // aliases preserved by save
        app.set_status(format!("Bound to [{}]", ks));
    }
    app.input_mode = InputMode::KeybindMenu;
}

fn handle_add_custom_bind(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Enter => app.submit_custom_bind(),
        KeyCode::Esc => { app.input.clear(); app.input_mode = InputMode::KeybindMenu; }
        KeyCode::Tab => complete_path_input(app),
        KeyCode::Backspace => { app.input.pop(); }
        KeyCode::Char(c) if key.modifiers.is_empty() => push_char(app, c),
        _ => {}
    }
}

/// Tab-completion for path-ish inputs (shell, quickmove, hooks, roots...). Char-safe.
fn complete_path_input(app: &mut App) {
    let (dir_part, prefix) = match app.input.rfind('/') {
        Some(i) => (app.input[..=i].to_string(), app.input[i + 1..].to_string()),
        None => (String::new(), app.input.clone()),
    };
    let base = if dir_part.is_empty() { PathBuf::from(".") } else { ops::expand_tilde(&dir_part) };
    let Ok(rd) = std::fs::read_dir(&base) else { return };
    let mut matches: Vec<String> = rd.flatten()
        .map(|e| ops::path_to_string(&e.path()))
        .filter(|n| n.starts_with(&prefix))
        .collect();
    matches.sort();
    if matches.len() == 1 {
        let full = format!("{}{}", dir_part, matches[0]);
        let p = ops::expand_tilde(&full);
        app.input = if p.is_dir() { format!("{}/", full) } else { full };
    } else if matches.len() > 1 {
        let mut cp: String = matches[0].clone();
        for m in &matches[1..] {
            cp = cp.chars().zip(m.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a).collect();
        }
        if cp.chars().count() > prefix.chars().count() {
            app.input = format!("{}{}", dir_part, cp);
        } else {
            let list: String = matches.iter().map(|m| m.as_str()).collect::<Vec<_>>().join("  ");
            app.set_status(truncate_chars(&list, 80));
        }
    }
}
