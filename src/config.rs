use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::Result;

// ============ Keybinds ============

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
pub enum KeyBind {
    Quit, NextItem, PrevItem, HalfPageDown, HalfPageUp, JumpTop, JumpMid, JumpBot,
    Expand, Collapse, AddSymlink, AddFolder, Rename, Delete, Cut, Paste, Chmod,
    QuickMove, ToggleBookmark, FilterBookmarks, ToggleHidden, AddNote, AddHostTag,
    OpenSettings, OpenHooks, OpenRoots, SelectEditor, Undo, FileInfo, GitPush,
    ToggleSelection, VisualSelect, Archive, Shell, Search, Help, ToggleHostFilter,
    OpenServices, GotoPrefix, EditFile, JumpList, ClearScreen,
    Custom(String),
}

impl KeyBind {
    pub fn description(&self) -> String {
        match self {
            KeyBind::Quit => "Quit".into(), KeyBind::NextItem => "Next item".into(),
            KeyBind::PrevItem => "Previous item".into(), KeyBind::HalfPageDown => "Half page down".into(),
            KeyBind::HalfPageUp => "Half page up".into(), KeyBind::JumpTop => "Jump to top".into(),
            KeyBind::JumpMid => "Jump to middle".into(), KeyBind::JumpBot => "Jump to bottom".into(),
            KeyBind::Expand => "Expand folder".into(), KeyBind::Collapse => "Collapse folder".into(),
            KeyBind::AddSymlink => "Add symlink".into(), KeyBind::AddFolder => "Create folder".into(),
            KeyBind::Rename => "Rename".into(), KeyBind::Delete => "Delete".into(),
            KeyBind::Cut => "Cut".into(), KeyBind::Paste => "Paste".into(),
            KeyBind::Chmod => "Permissions (chmod)".into(), KeyBind::QuickMove => "Quick move".into(),
            KeyBind::ToggleBookmark => "Toggle bookmark".into(), KeyBind::FilterBookmarks => "Bookmarks only".into(),
            KeyBind::ToggleHidden => "Toggle hidden files".into(), KeyBind::AddNote => "Add note".into(),
            KeyBind::AddHostTag => "Add host tag".into(), KeyBind::OpenSettings => "Settings".into(),
            KeyBind::OpenHooks => "Object hooks".into(), KeyBind::OpenRoots => "Root menu".into(),
            KeyBind::SelectEditor => "Select editor".into(), KeyBind::Undo => "Undo".into(),
            KeyBind::FileInfo => "File info".into(), KeyBind::GitPush => "Git push".into(),
            KeyBind::ToggleSelection => "Toggle selection".into(), KeyBind::VisualSelect => "Visual select mode".into(),
            KeyBind::Archive => "Archive selected".into(), KeyBind::Shell => "Shell command".into(),
            KeyBind::Search => "Search".into(), KeyBind::Help => "Help".into(),
            KeyBind::ToggleHostFilter => "Filter by hostname".into(), KeyBind::OpenServices => "Systemd services".into(),
            KeyBind::GotoPrefix => "Goto prefix (gg)".into(), KeyBind::EditFile => "Edit file / Open dir".into(),
            KeyBind::JumpList => "Jump list".into(), KeyBind::ClearScreen => "Clear screen".into(),
            KeyBind::Custom(c) => {
                if let Some(p) = c.strip_prefix("edit:") { format!("Open {}", p) } else { format!("Run: {}", c) }
            }
        }
    }

    pub fn all_variants() -> Vec<KeyBind> {
        vec![
            KeyBind::Quit, KeyBind::NextItem, KeyBind::PrevItem, KeyBind::HalfPageDown, KeyBind::HalfPageUp,
            KeyBind::JumpTop, KeyBind::JumpMid, KeyBind::JumpBot, KeyBind::Expand, KeyBind::Collapse,
            KeyBind::AddSymlink, KeyBind::AddFolder, KeyBind::Rename, KeyBind::Delete, KeyBind::Cut,
            KeyBind::Paste, KeyBind::Chmod, KeyBind::QuickMove, KeyBind::ToggleBookmark,
            KeyBind::FilterBookmarks, KeyBind::ToggleHidden, KeyBind::AddNote, KeyBind::AddHostTag,
            KeyBind::OpenSettings, KeyBind::OpenHooks, KeyBind::OpenRoots, KeyBind::SelectEditor,
            KeyBind::Undo, KeyBind::FileInfo, KeyBind::GitPush, KeyBind::ToggleSelection,
            KeyBind::VisualSelect, KeyBind::Archive, KeyBind::Shell, KeyBind::Search, KeyBind::Help,
            KeyBind::ToggleHostFilter, KeyBind::OpenServices, KeyBind::GotoPrefix, KeyBind::EditFile,
            KeyBind::JumpList, KeyBind::ClearScreen,
        ]
    }
}

pub type KeyMap = HashMap<String, KeyBind>;
pub type AliasMap = BTreeMap<String, String>; // "!name" -> command

pub struct KeymapConfig { pub map: KeyMap, pub aliases: AliasMap }

pub fn default_keymap() -> KeyMap {
    let mut m = KeyMap::new();
    m.insert("q".into(), KeyBind::Quit);
    m.insert("j".into(), KeyBind::NextItem);
    m.insert("k".into(), KeyBind::PrevItem);
    m.insert("Down".into(), KeyBind::NextItem);
    m.insert("Up".into(), KeyBind::PrevItem);
    m.insert("Ctrl+d".into(), KeyBind::HalfPageDown);
    m.insert("Ctrl+u".into(), KeyBind::HalfPageUp);
    m.insert("H".into(), KeyBind::JumpTop);
    m.insert("M".into(), KeyBind::JumpMid);
    m.insert("L".into(), KeyBind::JumpBot);
    m.insert("Right".into(), KeyBind::Expand);
    m.insert("Left".into(), KeyBind::Collapse);
    m.insert("a".into(), KeyBind::AddSymlink);
    m.insert("f".into(), KeyBind::AddFolder);
    m.insert("r".into(), KeyBind::Rename);
    m.insert("d".into(), KeyBind::Delete);
    m.insert("c".into(), KeyBind::Cut);
    m.insert("p".into(), KeyBind::Paste);
    m.insert("O".into(), KeyBind::Chmod);
    m.insert("m".into(), KeyBind::QuickMove);
    m.insert("b".into(), KeyBind::ToggleBookmark);
    m.insert("B".into(), KeyBind::FilterBookmarks);
    m.insert(".".into(), KeyBind::ToggleHidden);
    m.insert("n".into(), KeyBind::AddNote);
    m.insert("h".into(), KeyBind::AddHostTag);
    m.insert("s".into(), KeyBind::OpenSettings);
    m.insert("o".into(), KeyBind::OpenHooks);
    m.insert("R".into(), KeyBind::OpenRoots);
    m.insert("t".into(), KeyBind::SelectEditor);
    m.insert("u".into(), KeyBind::Undo);
    m.insert("i".into(), KeyBind::FileInfo);
    m.insert("P".into(), KeyBind::GitPush);
    m.insert("Space".into(), KeyBind::ToggleSelection);
    m.insert("V".into(), KeyBind::VisualSelect);
    m.insert("z".into(), KeyBind::Archive);
    m.insert("!".into(), KeyBind::Shell);
    m.insert("/".into(), KeyBind::Search);
    m.insert("?".into(), KeyBind::Help);
    m.insert("F".into(), KeyBind::ToggleHostFilter);
    m.insert("S".into(), KeyBind::OpenServices);
    m.insert("g".into(), KeyBind::GotoPrefix);
    m.insert("Enter".into(), KeyBind::EditFile);
    m.insert("Ctrl+g".into(), KeyBind::JumpList);
    m.insert("Ctrl+l".into(), KeyBind::ClearScreen);
    m
}

/// Reserved keys (hard-wired in the TUI): v = versions, D = deploy, y = copy path, Esc, Ctrl+C.
/// These guarantee restore/deploy/quit are always reachable regardless of keybinds.json.
pub fn is_reserved_key(k: &str) -> bool { matches!(k, "v" | "D" | "y" | "Esc" | "Ctrl+c") }

/// Guarantee the TUI always has navigation / help / quit bound, even with a mangled keybinds.json.
fn ensure_critical_binds(m: &mut KeyMap) {
    for (k, kb) in [("q", KeyBind::Quit), ("j", KeyBind::NextItem), ("k", KeyBind::PrevItem),
        ("Down", KeyBind::NextItem), ("Up", KeyBind::PrevItem), ("?", KeyBind::Help),
        ("Enter", KeyBind::EditFile), ("/", KeyBind::Search), ("!", KeyBind::Shell)] {
        m.entry(k.to_string()).or_insert(kb);
    }
}

fn seed_example_aliases(a: &mut AliasMap) {
    a.entry("!reload".to_string()).or_insert_with(|| "systemctl --user restart waybar".into());
}

pub fn load_keymap(p: &Path) -> KeymapConfig {
    let raw = match std::fs::read_to_string(p) {
        Ok(s) => s,
        Err(_) => {
            let mut aliases = AliasMap::new();
            seed_example_aliases(&mut aliases);
            let cfg = KeymapConfig { map: default_keymap(), aliases: aliases.clone() };
            let _ = save_keymap(&cfg.map, &aliases, p);
            return cfg;
        }
    };

    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            // NEVER destroy user data: back the corrupt file up, fall back to defaults.
            let bak = p.with_file_name(format!("{}.corrupt.bak", p.file_name().unwrap_or_default().to_string_lossy()));
            let _ = std::fs::write(&bak, &raw);
            tracing::warn!(?p, ?bak, error = %e, "keybinds.json corrupt; backed up, using defaults");
            let mut aliases = AliasMap::new();
            seed_example_aliases(&mut aliases);
            let mut map = default_keymap();
            ensure_critical_binds(&mut map);
            let _ = save_keymap(&map, &aliases, p);
            return KeymapConfig { map, aliases };
        }
    };

    // Aliases in _aliases
    let mut aliases = AliasMap::new();
    for section in ["_aliases"] {
        if let Some(obj) = v.get(section).and_then(|a| a.as_object()) {
            for (k, val) in obj {
                if let Some(cmd) = val.as_str() {
                    let name = k.strip_prefix('!').unwrap_or(k).trim();
                    let cmd = cmd.trim();
                    if name.is_empty() || cmd.is_empty() { continue; }
                    aliases.insert(format!("!{}", name), cmd.to_string());
                }
            }
        }
    }

    let mut m = KeyMap::new();
    if let Some(obj) = v.as_object() {
        for (k, val) in obj {
            if k.starts_with('_') || k.is_empty() { continue; }
            let Some(s_val) = val.as_str() else { continue };
            let kb = match serde_json::from_value::<KeyBind>(serde_json::Value::String(s_val.to_string())) {
                Ok(kb) => Some(kb),
                Err(_) => {
                    let raw_val = s_val.trim();
                    if raw_val.is_empty() { None }
                    else if let Some(cmd) = raw_val.strip_prefix('!') {
                        // "!alias" resolves through the alias table; otherwise it's a literal command
                        let resolved = aliases.get(raw_val).cloned().unwrap_or_else(|| cmd.trim().to_string());
                        if resolved.is_empty() { None } else { Some(KeyBind::Custom(resolved)) }
                    } else if (raw_val.starts_with('/') || raw_val.starts_with("~/")) && !raw_val.contains(' ') {
                        Some(KeyBind::Custom(format!("edit:{}", raw_val))) // opens in editor
                    } else { None }
                }
            };
            if let Some(kb) = kb { if !is_reserved_key(k) { m.insert(k.clone(), kb); } }
        }
    }

    if m.is_empty() {
        let mut map = default_keymap();
        ensure_critical_binds(&mut map);
        let _ = save_keymap(&map, &aliases, p);
        return KeymapConfig { map, aliases };
    }
    ensure_critical_binds(&mut m);
    KeymapConfig { map: m, aliases }
}

pub fn save_keymap(map: &KeyMap, aliases: &AliasMap, p: &Path) -> Result<()> {
    let mut json_map = serde_json::Map::new();
    json_map.insert("_comment".into(), serde_json::json!(
        "Confy Keybinds. \"key\": \"Action\" | \"!command\" | \"/path\". Named commands go in _aliases (\"!reload\": \"...\"), bind with \"w\": \"!reload\". Reserved keys: v, D, y, Esc."
    ));
    json_map.insert("_aliases".into(), serde_json::Value::Object(
        aliases.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect()));
    // reverse map so a bound command saves back as its alias name (round-trip stable)
    let rev: HashMap<&String, &String> = aliases.iter().map(|(k, v)| (v, k)).collect();
    let mut keys: Vec<_> = map.iter().collect();
    keys.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in keys {
        let val = match v {
            KeyBind::Custom(cmd) => {
                if let Some(path) = cmd.strip_prefix("edit:") { path.to_string() }
                else if let Some(name) = rev.get(cmd) { name.to_string() }
                else { format!("!{}", cmd) }
            }
            other => serde_json::to_string(other).unwrap_or_default().trim_matches('"').to_string(),
        };
        json_map.insert(k.clone(), serde_json::Value::String(val));
    }
    atomic_write(p, &serde_json::to_vec_pretty(&serde_json::Value::Object(json_map))?)
}

// ============ Settings enums ============

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum ColorMode { #[serde(rename="auto")] Auto, #[serde(rename="truecolor")] Truecolor, #[serde(rename="256")] Color256, #[serde(rename="mono")] Mono }
impl Default for ColorMode { fn default() -> Self { Self::Auto } }
impl ColorMode {
    pub fn next(self) -> Self { match self { Self::Auto=>Self::Truecolor, Self::Truecolor=>Self::Color256, Self::Color256=>Self::Mono, Self::Mono=>Self::Auto } }
    pub fn label(self) -> &'static str { match self { Self::Auto=>"auto", Self::Truecolor=>"truecolor", Self::Color256=>"256", Self::Mono=>"mono" } }
    pub fn is_truecolor(self) -> bool { match self { Self::Truecolor=>true, Self::Color256|Self::Mono=>false, Self::Auto=>std::env::var("COLORTERM").map(|v|v.contains("truecolor")||v.contains("24bit")).unwrap_or(false) } }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum DeployMode { #[serde(rename="dry-run")] DryRun, #[serde(rename="apply")] Apply }
impl Default for DeployMode { fn default() -> Self { Self::DryRun } }
impl DeployMode { pub fn next(self) -> Self { match self { Self::DryRun=>Self::Apply, Self::Apply=>Self::DryRun } } pub fn label(self) -> &'static str { match self { Self::DryRun=>"dry-run", Self::Apply=>"apply" } } pub fn is_apply(self) -> bool { matches!(self, Self::Apply) } }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum SearchMode { #[serde(rename="fuzzy")] Fuzzy, #[serde(rename="substring")] Substring }
impl Default for SearchMode { fn default() -> Self { Self::Fuzzy } }
impl SearchMode { pub fn next(self) -> Self { match self { Self::Fuzzy=>Self::Substring, Self::Substring=>Self::Fuzzy } } pub fn label(self) -> &'static str { match self { Self::Fuzzy=>"fuzzy", Self::Substring=>"substring" } } pub fn is_fuzzy(self) -> bool { matches!(self, Self::Fuzzy) } }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum BackupBehavior { #[serde(rename="always")] Always, #[serde(rename="if different")] IfDifferent, #[serde(rename="never")] Never }
impl Default for BackupBehavior { fn default() -> Self { Self::IfDifferent } }
impl BackupBehavior { pub fn next(self) -> Self { match self { Self::Always=>Self::IfDifferent, Self::IfDifferent=>Self::Never, Self::Never=>Self::Always } } pub fn label(self) -> &'static str { match self { Self::Always=>"always", Self::IfDifferent=>"if different", Self::Never=>"never" } } }

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum OverwriteMode { #[serde(rename="always")] Always, #[serde(rename="if different")] IfDifferent, #[serde(rename="never")] Never }
impl Default for OverwriteMode { fn default() -> Self { Self::Always } }
impl OverwriteMode { pub fn next(self) -> Self { match self { Self::Always=>Self::IfDifferent, Self::IfDifferent=>Self::Never, Self::Never=>Self::Always } } pub fn label(self) -> &'static str { match self { Self::Always=>"always", Self::IfDifferent=>"if different", Self::Never=>"never" } } }

// ============ Themes ============

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ThemeColors { pub foreground: String, pub background: String, pub accent: String }

impl ThemeColors {
    pub fn from_file(path: &Path) -> Option<Self> {
        let txt = std::fs::read_to_string(path).ok()?;
        if txt.trim_start().starts_with('{') { return Self::from_json(&txt); }
        let (mut fg, mut bg, mut ac) = (None, None, None);
        for line in txt.lines() {
            let l = line.split(';').next().unwrap_or("").trim();
            if l.starts_with('#') || l.starts_with("//") { continue; }
            let Some((key, value)) = l.split_once(|c: char| c == ':' || c == '=' || c.is_whitespace()) else { continue };
            let key = key.trim().to_ascii_lowercase();
            let color = extract_color(value);
            if fg.is_none() && matches!(key.as_str(), "foreground" | "fg" | "foreground-color" | "text") { fg = color.clone(); }
            if bg.is_none() && matches!(key.as_str(), "background" | "bg" | "background-color") { bg = color.clone(); }
            if ac.is_none() && matches!(key.as_str(), "accent" | "color4" | "primary" | "cursor") { ac = color; }
        }
        let fg = fg?;
        Some(Self { foreground: fg.clone(), background: bg.unwrap_or_else(|| fg.clone()), accent: ac.unwrap_or_else(|| fg.clone()) })
    }
    fn from_json(txt: &str) -> Option<Self> {
        let json: serde_json::Value = serde_json::from_str(txt).ok()?;
        if let Some(themes) = json.get("themes").and_then(|v| v.as_array()) {
            for theme in themes { if let Some(style) = theme.get("style") { if let Some(c) = Self::from_json_style(style) { return Some(c); } } }
        }
        Self::from_json_style(&json)
    }
    fn from_json_style(style: &serde_json::Value) -> Option<Self> {
        let fg = style.get("text").or_else(|| style.get("editor.foreground")).or_else(|| style.get("foreground")).and_then(|v| v.as_str()).map(|s| s.to_string());
        let bg = style.get("background").or_else(|| style.get("editor.background")).and_then(|v| v.as_str()).map(|s| s.to_string());
        let ac = style.get("text.accent").or_else(|| style.get("border.focused")).or_else(|| style.get("accent")).and_then(|v| v.as_str()).map(|s| s.to_string());
        let fg = fg?;
        Some(Self { foreground: fg.clone(), background: bg.unwrap_or_else(|| fg.clone()), accent: ac.unwrap_or_else(|| fg.clone()) })
    }
}

fn extract_color(rest: &str) -> Option<String> {
    let rest = rest.trim().trim_start_matches(['=', ':', ' ', '"', '\'']);
    let word = rest.split_whitespace().next()?.trim_matches([',', '"', '\'']);
    if word.starts_with('#') && word.len() >= 4 { return Some(word.to_string()); }
    if (word.len() == 6 || word.len() == 3) && word.chars().all(|c| c.is_ascii_hexdigit()) { return Some(format!("#{}", word)); }
    None
}

// ============ App settings ============

fn d_true() -> bool { true }
fn d_theme() -> String { "auto".into() }
fn d_vl() -> usize { 4 }
fn d_lw() -> u16 { 45 }
fn d_trash_days() -> u32 { 30 }
fn d_poll() -> u64 { 400 }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppSettings {
    #[serde(default = "d_theme")] pub theme_name: String,
    #[serde(default)] pub color_mode: ColorMode,
    #[serde(default)] pub custom_theme_path: Option<String>,
    #[serde(default)] pub custom_themes: HashMap<String, ThemeColors>,
    #[serde(default = "d_true")] pub folders_first: bool,
    #[serde(default = "d_true")] pub show_sizes: bool,
    #[serde(default)] pub show_hidden_by_default: bool,
    #[serde(default = "d_true")] pub show_symlinks_in_list: bool,
    #[serde(default = "d_true")] pub confirm_delete: bool,
    #[serde(default = "d_true")] pub confirm_moves: bool,
    #[serde(default = "d_true")] pub enable_versioning: bool,
    #[serde(default = "d_vl")] pub version_limit: usize,
    #[serde(default)] pub default_deploy_mode: DeployMode,
    #[serde(default)] pub backup_behavior: BackupBehavior,
    #[serde(default)] pub confirm_overwrite_deploy: OverwriteMode,
    #[serde(default = "d_true")] pub enable_hooks: bool,
    #[serde(default = "d_true")] pub enable_mouse: bool,
    #[serde(default)] pub search_mode: SearchMode,
    #[serde(default)] pub start_in_filter_mode: bool,
    #[serde(default = "d_lw")] pub list_width: u16,
    #[serde(default)] pub default_editor: Option<String>,
    #[serde(default = "d_trash_days")] pub trash_retention_days: u32,
    #[serde(default = "d_true")] pub post_edit_live: bool,
    #[serde(default = "d_poll")] pub post_edit_poll_interval_ms: u64,
    #[serde(default)] pub show_internal_debug: bool,
    #[serde(default = "d_true")] pub check_updates_in_doctor: bool,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self { theme_name: d_theme(), color_mode: ColorMode::default(), custom_theme_path: None,
            custom_themes: HashMap::new(), folders_first: true, show_sizes: true,
            show_hidden_by_default: false, show_symlinks_in_list: true, confirm_delete: true,
            confirm_moves: true, enable_versioning: true, version_limit: 4,
            default_deploy_mode: DeployMode::default(), backup_behavior: BackupBehavior::default(),
            confirm_overwrite_deploy: OverwriteMode::default(), enable_hooks: true,
            enable_mouse: true, search_mode: SearchMode::default(), start_in_filter_mode: false,
            list_width: d_lw(), default_editor: None, trash_retention_days: 30,
            post_edit_live: true, post_edit_poll_interval_ms: d_poll(), show_internal_debug: false,
            check_updates_in_doctor: true }
    }
}
impl AppSettings {
    pub fn validate(&self) -> Result<()> {
        if self.version_limit == 0 { return Err(crate::error::ConfyError::InvalidInput("version_limit must be > 0".into())); }
        if self.list_width < 10 { return Err(crate::error::ConfyError::InvalidInput("list_width must be >= 10".into())); }
        Ok(())
    }
}

// ============ State ============

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct ConfyState {
    #[serde(default)] pub bookmarks: HashSet<String>,
    #[serde(default)] pub notes: HashMap<String, String>,
    #[serde(default)] pub recent: VecDeque<String>,
    #[serde(default)] pub tags: HashMap<String, HashSet<String>>,
    #[serde(default)] pub settings: AppSettings,
    #[serde(default)] pub object_hooks: HashMap<String, HashMap<String, String>>,
}
impl ConfyState {
    /// Load state; falls back to the .bak if the main file is corrupt (durability).
    pub fn load(p: &Path) -> Self {
        if let Ok(s) = std::fs::read_to_string(p) {
            match serde_json::from_str::<ConfyState>(&s) {
                Ok(st) => return st,
                Err(e) => tracing::warn!(?p, error = %e, "state file corrupt; trying backup"),
            }
        }
        if let Ok(s) = std::fs::read_to_string(p.with_extension("json.bak")) {
            if let Ok(st) = serde_json::from_str(&s) { return st; }
        }
        Self::default()
    }
    pub fn save(&self, p: &Path) -> Result<()> {
        if let Ok(prev) = std::fs::read_to_string(p) {
            if serde_json::from_str::<ConfyState>(&prev).is_ok() {
                let _ = std::fs::write(p.with_extension("json.bak"), prev);
            }
        }
        atomic_write(p, &serde_json::to_vec_pretty(self)?)
    }
}

pub type ConfyHooks = HashMap<String, String>;
pub fn load_hooks(p: &Path) -> ConfyHooks {
    let s = match std::fs::read_to_string(p) { Ok(s) => s, Err(_) => return ConfyHooks::new() };
    if let Ok(map) = serde_json::from_str::<HashMap<String, Option<String>>>(&s) { return map.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect(); }
    serde_json::from_str::<HashMap<String, String>>(&s).unwrap_or_default()
}
pub fn save_hooks(hooks: &ConfyHooks, p: &Path) -> Result<()> { atomic_write(p, &serde_json::to_vec_pretty(hooks)?) }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ManifestEntry { pub alias: String, pub target: String, pub host: Option<String>, pub script: Option<String> }
#[derive(Serialize, Deserialize, Debug, Clone)] pub struct Manifest { pub files: Vec<ManifestEntry> }
pub type HooksData = HashMap<String, HashMap<String, String>>;

pub fn load_roots(p: &Path) -> Vec<PathBuf> { std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default() }
pub fn save_roots(r: &[PathBuf], p: &Path) -> Result<()> { atomic_write(p, &serde_json::to_vec_pretty(r)?) }

// ============ Atomic write (crash-safe) ============

static WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let seq = WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "data".into());
    // unique hidden tmp name in the same dir: no cross-writer collisions, rename stays atomic (same fs)
    let tmp = path.with_file_name(format!(".{}.{}.{}.tmp", name, std::process::id(), seq));
    let write = || -> Result<()> {
        use std::io::Write;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mode = std::fs::metadata(path).map(|m| m.permissions().mode()).unwrap_or(0o644);
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(&tmp)?;
            f.write_all(contents)?;
            f.sync_all()?;
        }
        #[cfg(not(unix))]
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(contents)?;
            f.sync_all()?;
        }
        Ok(())
    };
    match write() {
        Ok(()) => {
            std::fs::rename(&tmp, path)?;
            #[cfg(unix)]
            if let Some(parent) = path.parent() { if let Ok(d) = std::fs::File::open(parent) { let _ = d.sync_all(); } }
            Ok(())
        }
        Err(e) => { let _ = std::fs::remove_file(&tmp); Err(e.into()) }
    }
}
