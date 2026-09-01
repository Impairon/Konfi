use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use zip::write::FileOptions;

use crate::config::{AppSettings, ConfyHooks, HooksData, Manifest, ManifestEntry, atomic_write};
use crate::error::{ConfyError, Result};

pub struct TempDirGuard(pub PathBuf);
impl Drop for TempDirGuard { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

struct TempFileGuard(PathBuf);
impl Drop for TempFileGuard { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }

// ============ Path safety ============

pub fn is_path_safe(base: &Path, target: &Path) -> bool {
    if target.components().any(|c| matches!(c, std::path::Component::ParentDir)) { return false; }
    let cb = fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let ta = if target.is_absolute() { target.to_path_buf() } else { cb.join(target) };
    let mut existing = ta.clone();
    let mut suffix = Vec::new();
    while !existing.exists() {
        match existing.file_name() { Some(name) => suffix.push(name.to_os_string()), None => return false }
        if !existing.pop() { return false; }
    }
    let resolved = match fs::canonicalize(&existing) { Ok(p) => p, Err(_) => return false };
    if !resolved.starts_with(&cb) { return false; }
    suffix.iter().rev().fold(resolved, |p, part| p.join(part)).starts_with(&cb)
}

// ponytail: replaces 16+ scattered path_name() calls
pub fn path_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

// ponytail: replaces 25+ scattered .to_string_lossy().to_string() calls
pub fn path_to_string(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

pub fn path_exists(p: &Path) -> bool {
    p.exists() || p.symlink_metadata().is_ok()
}

pub fn ensure_parent(p: &Path) -> std::io::Result<()> {
    if let Some(parent) = p.parent() { fs::create_dir_all(parent)? } Ok(())
}

pub fn load_json<T: serde::de::DeserializeOwned>(p: &Path) -> Option<T> {
    fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok())
}

pub fn load_json_or<T: serde::de::DeserializeOwned + Default>(p: &Path) -> T {
    load_json(p).unwrap_or_default()
}

pub fn remove_entry(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || meta.is_file() { fs::remove_file(path) }
    else if meta.is_dir() { fs::remove_dir_all(path) }
    else { fs::remove_file(path) }
}

/// Pure-Rust PATH lookup (no shell spawn, no injection). Handles "code -w" style values.
pub fn command_exists(cmd: &str) -> bool {
    let Some(first) = cmd.split_whitespace().next() else { return false };
    if first.contains('/') { return fs::metadata(first).map(|m| m.is_file()).unwrap_or(false); }
    let Some(paths) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&paths).any(|d| {
        match fs::metadata(d.join(first)) {
            Ok(m) if m.is_file() => {
                #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; m.permissions().mode() & 0o111 != 0 }
                #[cfg(not(unix))] { true }
            }
            _ => false,
        }
    })
}

// ============ Trash ============

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrashMetadata { pub original_path: PathBuf, pub deleted_at: u64, pub kind: String }

fn trash_metadata_path(trash_path: &Path) -> PathBuf {
    let mut v = trash_path.as_os_str().to_os_string(); v.push(".json"); PathBuf::from(v)
}

pub fn save_trash_metadata(trash_path: &Path, original_path: &Path) -> Result<()> {
    let kind = match fs::symlink_metadata(original_path) {
        Ok(m) if m.file_type().is_symlink() => "symlink",
        Ok(m) if m.is_dir() => "directory",
        Ok(_) => "file",
        Err(_) => "unknown",
    };
    let metadata = TrashMetadata { original_path: original_path.to_path_buf(), deleted_at: now_secs(), kind: kind.into() };
    let path = trash_metadata_path(trash_path);
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&metadata)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn load_trash_metadata(trash_path: &Path) -> Option<TrashMetadata> {
    fs::read_to_string(trash_metadata_path(trash_path)).ok().and_then(|s| serde_json::from_str(&s).ok())
}
pub fn remove_trash_metadata(trash_path: &Path) { let _ = fs::remove_file(trash_metadata_path(trash_path)); }
pub fn now_secs() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }
pub fn now_nanos() -> u128 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() }
pub fn now_millis() -> u128 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() }

pub fn trash_deleted_at(trash_path: &Path) -> u64 {
    if let Some(meta) = load_trash_metadata(trash_path) { return meta.deleted_at; }
    fs::symlink_metadata(trash_path).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn trash_time_left(trash_path: &Path, retention_days: u32) -> Option<i64> {
    if retention_days == 0 { return None; }
    Some(trash_deleted_at(trash_path) as i64 + retention_days as i64 * 86_400 - now_secs() as i64)
}

pub fn format_time_left(secs: i64) -> String {
    if secs <= 0 { return "expired".into(); }
    let days = secs / 86_400; if days > 0 { return format!("{}d left", days); }
    let hours = secs / 3_600; if hours > 0 { return format!("{}h left", hours); }
    format!("{}m left", (secs / 60).max(1))
}

// ============ Misc helpers ============

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(s) = p.strip_prefix("~/") { if let Some(h) = dirs::home_dir() { return h.join(s); } }
    else if p == "~" { if let Some(h) = dirs::home_dir() { return h; } }
    PathBuf::from(p)
}

pub fn timestamp_now() -> String {
    let secs = now_secs();
    let (y, m, d) = days_to_ymd(secs / 86400);
    format!("{:04}{:02}{:02}_{:02}{:02}{:02}", y, m, d, (secs % 86400) / 3600, (secs % 3600) / 60, secs % 60)
}

pub fn timestamp_from_secs(secs: u64) -> String {
    let (y, m, d) = days_to_ymd(secs / 86400);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, m, d, (secs % 86400) / 3600, (secs % 3600) / 60)
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let mut y = 1970u64; let mut d = days;
    loop { let leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0); let yd = if leap { 366 } else { 365 }; if d < yd { break; } d -= yd; y += 1; }
    let leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
    let mdays = [31u64, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 1u64;
    for &md in &mdays { if d < md { break; } d -= md; m += 1; }
    (y, m, d + 1)
}

pub fn hash_file(path: &Path) -> std::io::Result<u64> {
    use std::hash::{DefaultHasher, Hasher};
    let mut f = fs::File::open(path)?;
    let mut h = DefaultHasher::new();
    let mut b = [0u8; 4096];
    loop { let n = f.read(&mut b)?; if n == 0 { break; } h.write(&b[..n]); }
    Ok(h.finish())
}

pub fn format_size(s: u64) -> String {
    const KB: u64 = 1024; const MB: u64 = KB * 1024; const GB: u64 = MB * 1024;
    if s < KB { format!("{}B", s) } else if s < MB { format!("{:.1}K", s as f64 / KB as f64) }
    else if s < GB { format!("{:.1}M", s as f64 / MB as f64) } else { format!("{:.1}G", s as f64 / GB as f64) }
}

pub fn get_icon(p: &Path) -> &'static str {
    let fn_ = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    if fn_ == "dockerfile" { return "\u{e7b0}"; }
    if fn_ == ".gitignore" || fn_ == ".env" { return "\u{f46a}"; }
    if fn_.contains(".lock") { return "\u{f023}"; }
    if let Some(e) = p.extension().and_then(|e| e.to_str()) {
        return match e.to_lowercase().as_str() {
            "rs"=>"\u{e7a8}","toml"=>"\u{e615}","json"=>"\u{e60b}","md"=>"\u{e609}","yaml"|"yml"=>"\u{e615}",
            "lua"=>"\u{e620}","js"|"jsx"=>"\u{e74e}","ts"|"tsx"=>"\u{e628}","py"=>"\u{e73c}",
            "sh"|"bash"=>"\u{e795}","txt"=>"\u{f0f6}","conf"|"config"=>"\u{e615}","c"|"h"=>"\u{e61e}",
            "cpp"|"hpp"=>"\u{e61e}","go"=>"\u{e627}","java"=>"\u{e256}","html"=>"\u{f13b}",
            "css"=>"\u{e749}","xml"=>"\u{f121}","log"=>"\u{f02d}","kdl"=>"\u{e615}",
            "png"|"jpg"|"jpeg"|"gif"|"webp"|"bmp"=>"\u{f1c5}",
            "mp4"|"mkv"|"webm"|"avi"|"mov"|"flv"=>"\u{f03d}", _ => "\u{f15b}",
        };
    }
    "\u{f15b}"
}

pub fn get_file_info(p: &Path) -> String {
    Command::new("file").arg("--brief").arg(p).stdout(Stdio::piped()).stderr(Stdio::null()).output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|_| "Unknown".into())
}

pub fn get_video_meta(p: &Path) -> String {
    match Command::new("ffprobe").arg("-v").arg("error").arg("-show-entries")
        .arg("format=duration,size:stream=codec_name,width,height").arg("-of").arg("default=noprint_wrappers=1")
        .arg(p).stdout(Stdio::piped()).stderr(Stdio::null()).output()
    { Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(), _ => "Metadata unavailable".into() }
}

pub fn extract_video_thumbnail_cached(p: &Path) -> Option<PathBuf> {
    let m = fs::metadata(p).ok()?;
    let mt = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let stem = p.file_name()?.to_string_lossy();
    let cd = dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("confy").join("thumbs");
    let _ = fs::create_dir_all(&cd);
    let tp = cd.join(format!("{}_{}.png", stem, mt));
    if tp.exists() { return Some(tp); }
    let st = Command::new("ffmpeg").arg("-y").arg("-i").arg(p).arg("-vframes").arg("1")
        .arg("-vf").arg("scale=960:-1").arg(&tp).stdout(Stdio::null()).stderr(Stdio::null()).status().ok()?;
    if st.success() { Some(tp) } else { None }
}

pub enum PreviewKind { Text, Image, Video }
pub fn detect_kind(p: &Path) -> PreviewKind {
    if let Some(e) = p.extension().and_then(|e| e.to_str()) {
        return match e.to_lowercase().as_str() {
            "png"|"jpg"|"jpeg"|"webp"|"gif"|"bmp" => PreviewKind::Image,
            "mp4"|"mkv"|"webm"|"mov"|"avi"|"flv" => PreviewKind::Video,
            _ => PreviewKind::Text,
        };
    }
    PreviewKind::Text
}

pub fn read_small_fallback(p: &Path) -> Vec<u8> {
    if let Ok(mut f) = fs::File::open(p) {
        let mut b = vec![0u8; 4096];
        if let Ok(n) = f.read(&mut b) {
            b.truncate(n);
            if b.is_empty() { return "\x1b[90m(empty)\x1b[0m\n".into(); }
            if b.contains(&0) { return "\x1b[1;33m\u{f071} Binary\x1b[0m".into(); }
            return b;
        }
    }
    "\x1b[31mUnable to read\x1b[0m\n".into()
}

// ============ Terminal suspend / editors ============

pub fn disable_raw_and_leave_alt() {
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
    let _ = std::io::stdout().flush();
}
pub fn restore_raw_and_enter_alt() {
    let _ = std::io::stdout().flush();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen);
    let _ = crossterm::terminal::enable_raw_mode();
}

/// Supports EDITOR values with args, e.g. "code -w".
fn editor_cmd(ed: &str, f: &Path) -> Command {
    let mut it = ed.split_whitespace();
    let mut c = Command::new(it.next().unwrap_or("vi"));
    for a in it { c.arg(a); }
    c.arg(f);
    c
}

pub fn open_editor(f: &Path, ed: &str) -> bool {
    disable_raw_and_leave_alt();
    let r = editor_cmd(ed, f).status();
    restore_raw_and_enter_alt();
    r.map(|s| s.success()).unwrap_or(false)
}

pub fn validate_sudo(pass: &str) -> bool {
    let mut c = match Command::new("sudo").arg("-S").arg("-v").stdin(Stdio::piped())
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn() { Ok(c) => c, Err(_) => return false };
    if let Some(mut s) = c.stdin.take() { let _ = s.write_all(format!("{}\n", pass).as_bytes()); }
    c.wait().map(|s| s.success()).unwrap_or(false)
}

fn file_signature(p: &Path) -> Option<(u64, u128)> {
    let m = fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some((m.len(), t))
}

fn watch_child<F: FnMut()>(child: &mut std::process::Child, f: &Path, interval: u64, mut on_save: F) -> bool {
    let mut last = file_signature(f);
    let ok = loop {
        match child.try_wait() { Ok(Some(st)) => break st.success(), Ok(None) => {}, Err(_) => break false }
        std::thread::sleep(Duration::from_millis(interval));
        let cur = file_signature(f);
        if cur.is_some() && cur != last { last = cur; on_save(); }
    };
    let final_sig = file_signature(f);
    if final_sig.is_some() && final_sig != last { on_save(); }
    ok
}

pub fn open_editor_watched<F: FnMut()>(f: &Path, ed: &str, interval: u64, on_save: F) -> bool {
    disable_raw_and_leave_alt();
    let mut child = match editor_cmd(ed, f).spawn() { Ok(c) => c, Err(_) => { restore_raw_and_enter_alt(); return false; } };
    let ok = watch_child(&mut child, f, interval, on_save);
    restore_raw_and_enter_alt();
    ok
}

pub fn open_editor_sudo_watched<F: FnMut()>(f: &Path, ed: &str, pass: &str, interval: u64, on_save: F) -> bool {
    disable_raw_and_leave_alt();
    let mut c = Command::new("sudo");
    c.arg("-S").arg("-p").arg("");
    let mut it = ed.split_whitespace();
    c.arg(it.next().unwrap_or("vi"));
    for a in it { c.arg(a); }
    c.arg(f).stdin(Stdio::piped()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    let mut child = match c.spawn() { Ok(c) => c, Err(_) => { restore_raw_and_enter_alt(); return false; } };
    if let Some(mut s) = child.stdin.take() { let _ = s.write_all(format!("{}\n", pass).as_bytes()); }
    let ok = watch_child(&mut child, f, interval, on_save);
    restore_raw_and_enter_alt();
    ok
}

// ============ Hooks ============

#[derive(Debug, Clone, PartialEq)]
pub enum HookSpec { Command(String), Script(PathBuf) }

pub fn parse_hook(value: &str) -> Option<HookSpec> {
    let v = value.trim();
    if v.is_empty() { return None; }
    if let Some(cmd) = v.strip_prefix('!') {
        let cmd = cmd.trim();
        if cmd.is_empty() { return None; }
        return Some(HookSpec::Command(cmd.to_string()));
    }
    Some(HookSpec::Script(expand_tilde(v)))
}

pub fn validate_hook(value: &str) -> std::result::Result<String, String> {
    match parse_hook(value) {
        None => Err("empty hook".into()),
        Some(HookSpec::Command(c)) => Ok(format!("command: {}", c)),
        Some(HookSpec::Script(p)) => {
            let meta = match fs::metadata(&p) { Ok(m) => m, Err(_) => return Err(format!("script not found: {}", p.display())) };
            if meta.is_dir() { return Err(format!("not a script (directory): {}", p.display())); }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o111 == 0 { return Err(format!("script not executable — run: chmod +x {}", p.display())); }
            }
            Ok(format!("script: {}", p.display()))
        }
    }
}

/// Cached — hostname lookups were spawning a process per hook run / filter toggle.
pub fn hostname() -> String {
    static H: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        Command::new("hostname").output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    }).clone()
}

#[derive(Debug, Clone)]
pub struct HookContext { pub confy_dir: PathBuf, pub file: PathBuf, pub alias: String, pub op: String, pub tags: String, pub hostname: String, pub success: bool }

impl HookContext {
    pub fn new(confy_dir: &Path, file: &Path, op: &str, success: bool, tags: &HashMap<String, HashSet<String>>) -> Self {
        let alias = path_name(&file);
        let ts = tags.get(&alias).cloned().unwrap_or_default().into_iter().collect::<Vec<_>>().join(",");
        Self { confy_dir: confy_dir.to_path_buf(), file: file.to_path_buf(), alias, op: op.to_string(), tags: ts, hostname: hostname(), success }
    }
}

#[derive(Debug, Clone)]
pub struct HookRun { pub ok: bool, pub detail: String }

const HOOK_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs a hook with: piped output drained in threads (no pipe deadlock),
/// a hard 30s timeout with kill (no UI hang), UTF-8-safe detail truncation.
pub fn run_hook_value(value: &str, ctx: &HookContext) -> HookRun {
    let spec = match parse_hook(value) { Some(s) => s, None => return HookRun { ok: false, detail: "empty hook".into() } };
    if let Err(e) = validate_hook(value) { tracing::warn!(hook = value, reason = %e, "hook skipped"); return HookRun { ok: false, detail: e }; }
    let (label, mut cmd) = match &spec {
        HookSpec::Command(c) => { let mut k = Command::new("sh"); k.arg("-c").arg(c); (format!("!{}", c), k) }
        HookSpec::Script(p) => (p.display().to_string(), Command::new(p)),
    };
    tracing::info!(hook = %label, file = ?ctx.file, op = %ctx.op, "running hook");
    cmd.current_dir(&ctx.confy_dir)
        .env("CONFY_OPERATION", &ctx.op).env("CONFY_FILE", &ctx.file)
        .env("CONFY_ALIAS", &ctx.alias).env("CONFY_TAGS", &ctx.tags)
        .env("CONFY_ROOT", &ctx.confy_dir).env("CONFY_HOSTNAME", &ctx.hostname)
        .env("CONFY_SUCCESS", if ctx.success { "true" } else { "false" })
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => { tracing::warn!(hook = %label, error = ?e, "hook failed to start"); return HookRun { ok: false, detail: format!("failed to start: {}", e) }; }
    };
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();
    let t_out = std::thread::spawn(move || { let mut b = Vec::new(); if let Some(mut s) = out_pipe { let _ = s.read_to_end(&mut b); } b });
    let t_err = std::thread::spawn(move || { let mut b = Vec::new(); if let Some(mut s) = err_pipe { let _ = s.read_to_end(&mut b); } b });

    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if start.elapsed() > HOOK_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    tracing::warn!(hook = %label, "hook timed out after 30s");
                    return HookRun { ok: false, detail: "timed out after 30s".into() };
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return HookRun { ok: false, detail: format!("wait failed: {}", e) },
        }
    };
    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();

    let ok = status.success();
    let mut detail = String::from_utf8_lossy(&stdout).trim().to_string();
    if !ok { let err = String::from_utf8_lossy(&stderr).trim().to_string(); if !err.is_empty() { detail = err; } }
    if detail.chars().count() > 200 { detail = detail.chars().take(200).collect::<String>() + "…"; }
    if detail.is_empty() { detail = if ok { "ok".into() } else { format!("exit {}", status.code().unwrap_or(-1)) }; }
    tracing::info!(hook = %label, ok = ok, detail = %detail, "hook finished");
    HookRun { ok, detail }
}

pub fn run_hook(hooks: &ConfyHooks, settings: &AppSettings, confy_dir: &Path, name: &str, file: &Path, op: &str, success: bool, tags: &HashMap<String, HashSet<String>>) -> Option<HookRun> {
    if !settings.enable_hooks { return None; }
    let value = hooks.get(name)?.clone();
    if value.trim().is_empty() { return None; }
    let ctx = HookContext::new(confy_dir, file, op, success, tags);
    Some(run_hook_value(&value, &ctx))
}

// ============ Versioning ============

pub fn take_snapshot(confy_dir: &Path, history_dir: &Path, hooks: &ConfyHooks, settings: &AppSettings, tags: &HashMap<String, HashSet<String>>) -> std::io::Result<()> {
    if !settings.enable_versioning { return Ok(()); }
    let limit = settings.version_limit.max(1);
    let ts = format!("{}_{}", timestamp_now(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos());
    let sd = history_dir.join(&ts);
    let mut changed = false;
    if let Ok(entries) = fs::read_dir(confy_dir) {
        for e in entries.flatten() {
            let p = e.path();
            let fname = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            // skip dirs, dotfiles, and confy's own backup zips (they'd bloat history)
            if p.is_dir() || fname.starts_with('.')
                || p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false) { continue; }
            if let Ok(rel) = p.strip_prefix(confy_dir) {
                if has_file_changed(history_dir, &p, rel) {
                    if !changed {
                        run_hook(hooks, settings, confy_dir, "pre_save_version", &p, "save_version", false, tags);
                        let _ = fs::create_dir_all(&sd);
                        changed = true;
                    }
                    let dest = sd.join(rel);
                    ensure_parent(&dest)?;
                    fs::copy(&p, &dest)?;
                    run_hook(hooks, settings, confy_dir, "post_save_version", &p, "save_version", true, tags);
                }
            }
        }
    }
    if let Ok(entries) = fs::read_dir(history_dir) {
        let mut s: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        s.sort();
        if s.len() > limit { for old in s.iter().take(s.len() - limit) { let _ = fs::remove_dir_all(old); } }
    }
    Ok(())
}

pub fn has_file_changed(hd: &Path, cp: &Path, rp: &Path) -> bool {
    let mut latest: Option<PathBuf> = None;
    if let Ok(entries) = fs::read_dir(hd) {
        let mut s: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        s.sort();
        for snap in s.iter().rev() { let hf = snap.join(rp); if hf.exists() { latest = Some(hf); break; } }
    }
    match latest { None => true, Some(h) => hash_file(cp).unwrap_or(0) != hash_file(&h).unwrap_or(1) }
}

// ============ Archive ============

fn render_clone_progress(line: &str) {
    let percent = line.split_whitespace().find_map(|part| part.strip_suffix('%').and_then(|value| value.parse::<u8>().ok()));
    let phase = if line.contains("Receiving objects") { "Receiving" }
        else if line.contains("Resolving deltas") { "Resolving" }
        else if line.contains("Compressing objects") { "Compressing" }
        else { return };
    let percent = percent.unwrap_or(0).min(100);
    let width = 28usize;
    let filled = width * percent as usize / 100;
    eprint!("\r{} [{:<width$}] {:>3}%", phase, "=".repeat(filled), percent, width = width);
}

pub fn git_clone(confy_dir: &Path, source: &str, destination: Option<&Path>) -> Result<PathBuf> {
    fs::create_dir_all(confy_dir)?;
    if let Some(path) = destination {
        if !path.starts_with(confy_dir) { return Err(ConfyError::PathTraversal(path.to_path_buf())); }
        if path_exists(&path) {
            return Err(ConfyError::InvalidInput(format!("clone destination already exists: {}", path.display())));
        }
    }
    let mut command = Command::new("git");
    command.arg("clone").arg("--progress").arg(source);
    if let Some(path) = destination { command.arg(path); }
    let mut child = command.current_dir(confy_dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()
        .map_err(|e| ConfyError::Git(format!("start clone: {}", e)))?;
    if let Some(stderr) = child.stderr.take() {
        for line in BufReader::new(stderr).lines() {
            let line = line.map_err(|e| ConfyError::Git(format!("read clone progress: {}", e)))?;
            if line.contains('%') { render_clone_progress(&line); }
            else if !line.trim().is_empty() { eprintln!("\n{}", line); }
        }
    }
    let status = child.wait()?;
    eprintln!();
    if !status.success() { return Err(ConfyError::Git(format!("clone failed with {}", status))); }
    Ok(destination.map(Path::to_path_buf).unwrap_or_else(|| confy_dir.join(source.rsplit('/').next().unwrap_or(source).trim_end_matches(".git"))))
}

fn add_dir_to_zip(zip: &mut zip::ZipWriter<fs::File>, base_path: &Path, alias: &str, options: FileOptions<()>) -> Result<()> {
    for entry in fs::read_dir(base_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.symlink_metadata()?.file_type().is_symlink() {
            return Err(ConfyError::InvalidInput(format!("archive refuses symlink: {}", path.display())));
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let zip_path = format!("{}/{}", alias, name);
        if path.is_dir() { zip.add_directory(&zip_path, options)?; add_dir_to_zip(zip, &path, &zip_path, options)?; }
        else { zip.start_file(&zip_path, options)?; let mut f = fs::File::open(&path)?; let mut buffer = Vec::new(); f.read_to_end(&mut buffer)?; zip.write_all(&buffer)?; }
    }
    Ok(())
}

pub fn create_archive(confy_dir: &Path, files: &[(String, PathBuf)], password: Option<&str>, hooks_data: &HooksData) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| ConfyError::NotFound("home dir".into()))?;
    if password.is_some_and(|p| !p.is_empty()) {
        return Err(ConfyError::InvalidInput("archive passwords are not supported yet; refusing to create an unencrypted archive".into()));
    }
    let ap = confy_dir.join(format!("confy_backup_{}.zip", now_millis()));
    let temp_ap = confy_dir.join(format!(".confy_backup_{}.zip.tmp", now_nanos()));
    let mut temp_guard = TempFileGuard(temp_ap.clone());
    let file = fs::File::create(&temp_ap)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = FileOptions::<()>::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(0o644);
    let mut manifest = Manifest { files: vec![] };

    for (alias, path) in files {
        if path.symlink_metadata()?.file_type().is_symlink() {
            return Err(ConfyError::InvalidInput(format!("archive refuses symlink: {}", path.display())));
        }
        if path.is_dir() { zip.add_directory(alias, options)?; add_dir_to_zip(&mut zip, path, alias, options)?; }
        else { zip.start_file(alias, options)?; let mut f = fs::File::open(path)?; let mut buffer = Vec::new(); f.read_to_end(&mut buffer)?; zip.write_all(&buffer)?; }
        let ts = path.strip_prefix(&home).map(|p| format!("~/{}", p.display())).unwrap_or_else(|_| path.display().to_string());
        manifest.files.push(ManifestEntry { alias: alias.clone(), target: ts, host: None, script: None });
    }

    if !hooks_data.is_empty() {
        zip.start_file("hooks.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(hooks_data)?)?;
        tracing::info!(files_with_hooks = hooks_data.len(), "hooks embedded in archive");
    }
    zip.start_file("manifest.json", options)?;
    zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    zip.finish()?;
    fs::rename(&temp_ap, &ap)?;
    temp_guard.0 = PathBuf::new();
    tracing::info!(archive = ?ap, files = files.len(), "archive created");
    Ok(ap)
}

// ============ Deploy ============

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeployIndex { pub last_deployed_hash: u64, pub last_deployed_at: String }

pub struct DeploySummary { pub output: String, pub new: u32, pub updated: u32, pub skipped: u32, pub conflicts: u32 }

pub fn load_deploy_index(p: &Path) -> HashMap<String, DeployIndex> { load_json_or(p) }
pub fn save_deploy_index(map: &HashMap<String, DeployIndex>, p: &Path) -> Result<()> {
    atomic_write(p, &serde_json::to_vec_pretty(map)?)
}

pub fn deploy_archive(confy_dir: &Path, archive: &Path, apply: bool, _password: Option<&str>, host: Option<&str>, allow_scripts: bool, _settings: &AppSettings) -> Result<DeploySummary> {
    if _password.is_some_and(|p| !p.is_empty()) {
        return Err(ConfyError::InvalidInput("encrypted archives are not supported yet; refusing to process a password".into()));
    }
    let file = fs::File::open(archive)?;
    let mut zip_archive = zip::ZipArchive::new(file)?;

    let td = confy_dir.join(".assets/.tmp").join(format!("deploy_{}_{}", now_nanos(), std::process::id()));
    let _g = TempDirGuard(td.clone());
    fs::create_dir_all(td.parent().ok_or_else(|| ConfyError::Deploy("invalid temporary directory".into()))?)?;
    fs::create_dir_all(&td)?;

    // zip-bomb guards: per-file and total caps, enforced while streaming
    const MAX_FILE: u64 = 512 * 1024 * 1024;
    const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
    let mut total: u64 = 0;
    for i in 0..zip_archive.len() {
        let mut zip_file = zip_archive.by_index(i)?;
        let name = zip_file.enclosed_name().ok_or_else(|| ConfyError::PathTraversal(zip_file.name().into()))?;
        let outpath = td.join(&name);
        if zip_file.is_dir() { fs::create_dir_all(&outpath)?; continue; }
        ensure_parent(&outpath)?;
        let mut outfile = fs::File::create(&outpath)?;
        let mut buf = [0u8; 65536];
        let mut written: u64 = 0;
        loop {
            let n = zip_file.read(&mut buf)?;
            if n == 0 { break; }
            outfile.write_all(&buf[..n])?;
            written += n as u64;
            if written > MAX_FILE { return Err(ConfyError::Deploy("member exceeded size limit (possible zip bomb)".into())); }
        }
        total += written;
        if total > MAX_TOTAL { return Err(ConfyError::Deploy("archive total size limit exceeded".into())); }
    }

    let manifest_path = td.join("manifest.json");
    let manifest: Manifest = if manifest_path.exists() {
        serde_json::from_str(&fs::read_to_string(manifest_path)?)?
    } else { return Err(ConfyError::Deploy("manifest not found".into())); };
    let hooks_path = td.join("hooks.json");
    let hooks_data: Option<HooksData> = if hooks_path.exists() { serde_json::from_str(&fs::read_to_string(hooks_path)?).ok() } else { None };

    let mut output = format!("\x1b[1;36m\u{f019}  Deployment Plan {}\x1b[0m\n\x1b[90m────────────────\x1b[0m\n", if apply { "[APPLY]" } else { "[DRY RUN]" });
    let home = dirs::home_dir().ok_or_else(|| ConfyError::NotFound("home dir".into()))?;
    let assets_root = confy_dir.join(".assets");
    let bd = confy_dir.join(".assets/.deployments").join(format!("{}_{}", 
        archive.file_name().unwrap_or_default().to_string_lossy(),
        now_millis()));
    if apply { fs::create_dir_all(&bd)?; }
    let mut sum = (0u32, 0u32, 0u32, 0u32); // new, updated, skipped, conflicts

    let index_path = confy_dir.join(".assets/.deploy_index.json");
    let mut deploy_index = load_deploy_index(&index_path);
    let base_dir = confy_dir.join(".assets/.deploy_base");

    for entry in &manifest.files {
        if let Some(h) = &entry.host {
            if let Some(current_host) = host { if h != current_host { continue; } }
        }
        if entry.alias.is_empty() || entry.alias.contains("..") {
            output.push_str(&format!("\n[ERROR] Bad alias: {}\n", entry.alias)); continue;
        }

        let tp: PathBuf = if let Some(rest) = entry.target.strip_prefix("~/") { home.join(rest) }
            else if entry.target == "~" { home.clone() }
            else if entry.target == "/" || entry.target.is_empty() {
                output.push_str(&format!("\n[ERROR] Unsafe target: {}\n", entry.target)); continue; }
            else if Path::new(&entry.target).is_absolute() { PathBuf::from(&entry.target) }
            else { output.push_str(&format!("\n[ERROR] Relative target: {}\n", entry.alias)); continue; };

        // Hard safety: never touch home root, confy root, ancestors of them, or confy internals
        if tp == home || home.starts_with(&tp)
            || tp == confy_dir || confy_dir.starts_with(&tp)
            || tp.starts_with(&assets_root)
            || tp.components().any(|c| matches!(c, std::path::Component::ParentDir))
            || !is_path_safe(&home, &tp) {
            output.push_str(&format!("\n[ERROR] Unsafe target: {}\n", entry.target)); continue;
        }

        let src_path = td.join(&entry.alias);
        if !src_path.exists() {
            output.push_str(&format!("\n[ERROR] Source not found in archive: {}\n", entry.alias)); continue;
        }
        output.push_str(&format!("\nFile: {}\n  Target: {}\n", entry.alias, tp.display()));
        if let Some(s) = &entry.script {
            output.push_str(&format!("  \x1b[35m[SCRIPT] {}run:\x1b[0m {}\n", if apply && allow_scripts { "" } else { "would not " }, s));
        }

        let tp_exists = path_exists(&tp);
        if tp_exists && tp.symlink_metadata()?.file_type().is_symlink() {
            output.push_str(&format!("\n[ERROR] Refusing to overwrite symlink target: {}\n", tp.display()));
            continue;
        }
        if tp_exists {
            let current_hash = hash_file(&tp).unwrap_or(0);
            if current_hash == hash_file(&src_path).unwrap_or(1) {
                output.push_str("  \x1b[90m[SKIP] Identical.\x1b[0m\n"); sum.2 += 1;
            } else {
                output.push_str("  \x1b[33m[UPDATE]\x1b[0m\n"); sum.1 += 1;
                if apply {
                    let bak = bd.join(&entry.alias);
                    ensure_parent(&bak)?;
                    let base_path = base_dir.join(&entry.alias);
                    let tp_is_regular = tp.is_file() && tp.symlink_metadata().map(|m| !m.file_type().is_symlink()).unwrap_or(false);
                    let mut merged = false; let mut conflicted = false; let mut attempted_merge = false;

                    // 3-way merge: local edits kept + archive changes applied.
                    // Local version is backed up FIRST, always, before any merge attempt.
                    if tp_is_regular && base_path.is_file() && src_path.is_file() {
                        fs::copy(&tp, &bak)?;
                        attempted_merge = true;
                        match Command::new("git").arg("merge-file").arg("-q").arg(&tp).arg(&base_path).arg(&src_path).status() {
                            Ok(st) if st.success() => merged = true,
                            Ok(st) if st.code().unwrap_or(0) > 0 => {
                                conflicted = true; sum.3 += 1;
                                let _ = fs::copy(&bak, bd.join(format!("{}.ours", entry.alias)));
                                let _ = fs::copy(&src_path, bd.join(format!("{}.theirs", entry.alias)));
                            }
                            _ => { /* git unavailable → fall through to replace path */ }
                        }
                    }

                    if !merged && !conflicted {
                        if !attempted_merge {
                            let is_dir_real = tp.is_dir() && tp.symlink_metadata().map(|m| !m.file_type().is_symlink()).unwrap_or(false);
                            if is_dir_real { copy_dir_recursive(&tp, &bak)?; }
                            else if fs::copy(&tp, &bak).is_err() { fs::rename(&tp, &bak)?; }
                        }
                        if !path_exists(&tp) { remove_entry(&tp)?; }
                        if let Some(parent) = tp.parent() { fs::create_dir_all(parent)?; }
                        if src_path.is_dir() { copy_dir_recursive(&src_path, &tp)?; }
                        else { fs::copy(&src_path, &tp)?; }
                        output.push_str("  \x1b[33m[REPLACED] previous version backed up\x1b[0m\n");
                    } else if merged {
                        output.push_str("  \x1b[34m[✓ MERGED] local edits kept + archive applied\x1b[0m\n");
                    } else {
                        output.push_str("  \x1b[31m[✗ CONFLICT] markers written into target\x1b[0m\n");
                        output.push_str(&format!("    ours:   {}.ours\n    theirs: {}.theirs\n",
                            bd.join(&entry.alias).display(), bd.join(&entry.alias).display()));
                    }

                    if tp.is_file() && !conflicted {
                        let _ = fs::create_dir_all(&base_dir);
                        let _ = fs::copy(&tp, base_dir.join(&entry.alias));
                    }
                    fs::OpenOptions::new().append(true).create(true).open(bd.join("rollback_manifest.txt"))?
                        .write_all(format!("{}\t{}\n", entry.alias, tp.display()).as_bytes())?;
                    deploy_index.insert(entry.alias.clone(), DeployIndex { last_deployed_hash: hash_file(&tp).unwrap_or(0), last_deployed_at: timestamp_now() });
                }
            }
        } else {
            output.push_str("  \x1b[32m[NEW]\x1b[0m\n"); sum.0 += 1;
            if apply {
                fs::OpenOptions::new().append(true).create(true).open(bd.join("created_manifest.txt"))?
                    .write_all(format!("{}\n", tp.display()).as_bytes())?;
                if let Some(parent) = tp.parent() { fs::create_dir_all(parent)?; }
                if src_path.is_dir() { copy_dir_recursive(&src_path, &tp)?; }
                else { fs::copy(&src_path, &tp)?; }
                deploy_index.insert(entry.alias.clone(), DeployIndex { last_deployed_hash: hash_file(&tp).unwrap_or(0), last_deployed_at: timestamp_now() });
                if tp.is_file() { let _ = fs::create_dir_all(&base_dir); let _ = fs::copy(&tp, base_dir.join(&entry.alias)); }
            }
        }

        if apply && allow_scripts {
            if let Some(script) = &entry.script {
                let _ = Command::new("sh").arg("-c").arg(script).current_dir(&home).stdin(Stdio::null()).status();
                output.push_str("  \x1b[32m[✓] Deployed + script ran\x1b[0m\n");
            } else {
                output.push_str("  \x1b[32m[✓] Deployed\x1b[0m\n");
            }
        } else if apply && entry.script.is_some() {
            output.push_str("  \x1b[33m[!] Deployed; script blocked (re-run with --allow-scripts)\x1b[0m\n");
        }
    }

    if apply { save_deploy_index(&deploy_index, &index_path)?; }
    if let Some(hd) = &hooks_data {
        if apply && !hd.is_empty() {
            fs::write(bd.join("deployed_hooks.json"), serde_json::to_string_pretty(hd)?)?;
            output.push_str("\n\x1b[1;35m📎 Deployed hooks saved to .assets/.deployments/ for review.\x1b[0m\n");
        }
    }
    output.push_str(&format!("\nSummary: {} New, {} Updated, {} Skipped, {} Conflicts\n", sum.0, sum.1, sum.2, sum.3));
    if apply { output.push_str("\n\x1b[1;32m✅ Deployed!\x1b[0m\n"); }
    else { output.push_str("\n\x1b[1;33m⚠️ Dry run. Press 'e' to browse archive, 'y' to apply.\x1b[0m\n"); }
    Ok(DeploySummary { output, new: sum.0, updated: sum.1, skipped: sum.2, conflicts: sum.3 })
}

pub fn list_archive_contents(archive: &Path) -> String {
    match fs::File::open(archive) {
        Ok(file) => match zip::ZipArchive::new(file) {
            Ok(mut arch) => {
                let mut s = String::from("\x1b[1;36m📦 Archive Contents & Scripts\x1b[0m\n\x1b[90m────────────────\x1b[0m\n");
                for i in 0..arch.len() {
                    if let Ok(mut file) = arch.by_index(i) {
                        let name = file.name().to_string();
                        s.push_str(&format!("\n\x1b[1;33m📄 File: {}\x1b[0m ({} bytes)\n", name, file.size()));
                        if name.ends_with(".sh") || name.ends_with(".json") || name.ends_with(".txt") {
                            let mut limited = (&mut file).take(64 * 1024); // cap reads: no RAM blowups
                            let mut content = String::new();
                            if limited.read_to_string(&mut content).is_ok() {
                                s.push_str("\x1b[90m─── Content ───\x1b[0m\n"); s.push_str(&content); s.push_str("\n\x1b[90m───────────────\x1b[0m\n");
                            }
                        }
                    }
                }
                s.push_str("\n\x1b[90m─── (Esc to return) ───\x1b[0m\n");
                s
            }
            Err(e) => format!("Failed to read archive: {}", e),
        },
        Err(_) => "Archive not found".into(),
    }
}

// ============ Git ============

pub fn git_export(cd: &Path) -> Result<String> {
    crate::secrets::ensure_root_layout(cd)?;
    if std::env::var_os("CONFY_ALLOW_SECRETS").is_none() {
        crate::secrets::check_git_blockers(cd)?;
    }
    let gi = cd.join(".gitignore");
    const REQ: &str = ".assets/.trash/\n.assets/.deployments/\n.assets/.keys/\n.assets/.tmp/\n.assets/.state.json\n";
    let cur = fs::read_to_string(&gi).unwrap_or_default();
    if !cur.contains(".assets/.trash") {
        let mut n = cur.clone();
        if !n.is_empty() && !n.ends_with('\n') { n.push('\n'); } // never merge lines
        fs::write(&gi, format!("{}{}", n, REQ))?;
    }
    let _ = Command::new("git").arg("init").current_dir(cd).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let _ = Command::new("git").arg("add").arg("-A").current_dir(cd).stdout(Stdio::null()).stderr(Stdio::null()).status();
    let out = Command::new("git").arg("commit").arg("-m").arg("Confy export").current_dir(cd).stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;
    if out.status.success() { Ok("✅ Exported.".into()) }
    else if String::from_utf8_lossy(&out.stderr).contains("nothing to commit") { Ok("✅ Up to date.".into()) }
    else { Err(ConfyError::Git(format!("commit: {}", String::from_utf8_lossy(&out.stderr)))) }
}

pub fn git_remote(cd: &Path, url: &str) -> Result<String> {
    if !cd.join(".git").exists() { let _ = Command::new("git").arg("init").current_dir(cd).stdout(Stdio::null()).stderr(Stdio::null()).status(); }
    let chk = Command::new("git").arg("remote").current_dir(cd).stdout(Stdio::piped()).stderr(Stdio::null()).output()?;
    if String::from_utf8_lossy(&chk.stdout).contains("origin") {
        let _ = Command::new("git").arg("remote").arg("set-url").arg("origin").arg(url).current_dir(cd).stdout(Stdio::null()).stderr(Stdio::null()).status();
        Ok(format!("✅ Remote updated: {}", url))
    } else {
        let _ = Command::new("git").arg("remote").arg("add").arg("origin").arg(url).current_dir(cd).stdout(Stdio::null()).stderr(Stdio::null()).status();
        Ok(format!("✅ Remote added: {}", url))
    }
}

pub fn git_push(cd: &Path, selected_paths: &[PathBuf]) -> Result<String> {
    if !cd.join(".git").exists() { return Err(ConfyError::Git("Not initialized".into())); }
    if std::env::var_os("CONFY_ALLOW_SECRETS").is_none() {
        crate::secrets::check_git_blockers(cd)?;
    }
    let chk = Command::new("git").arg("remote").current_dir(cd).stdout(Stdio::piped()).stderr(Stdio::null()).output()?;
    if !String::from_utf8_lossy(&chk.stdout).contains("origin") { return Err(ConfyError::Git("No remote".into())); }
    let mut status = Command::new("git");
    status.arg("status").arg("--porcelain");
    if !selected_paths.is_empty() { status.arg("--").args(selected_paths); }
    let st = status.current_dir(cd).stdout(Stdio::piped()).stderr(Stdio::null()).output()?;
    if !String::from_utf8_lossy(&st.stdout).trim().is_empty() {
        let mut add = Command::new("git");
        add.arg("add");
        if selected_paths.is_empty() { add.arg("-A"); } else { add.arg("--").args(selected_paths); }
        let add_status = add.current_dir(cd).stdout(Stdio::null()).stderr(Stdio::piped()).output()?;
        if !add_status.status.success() {
            return Err(ConfyError::Git(format!("add: {}", String::from_utf8_lossy(&add_status.stderr))));
        }
        let mut commit = Command::new("git");
        commit.arg("commit").arg("-m").arg(format!("Auto: {}", timestamp_now()));
        if !selected_paths.is_empty() { commit.arg("--").args(selected_paths); }
        let commit_status = commit.current_dir(cd).stdout(Stdio::null()).stderr(Stdio::piped()).output()?;
        if !commit_status.status.success() && !String::from_utf8_lossy(&commit_status.stderr).contains("nothing to commit") {
            return Err(ConfyError::Git(format!("commit: {}", String::from_utf8_lossy(&commit_status.stderr))));
        }
    }
    let po = Command::new("git").arg("push").arg("-u").arg("origin").arg("HEAD").current_dir(cd).stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;
    if po.status.success() { Ok("✅ Pushed.".into()) }
    else { Err(ConfyError::Git(format!("Push: {}", String::from_utf8_lossy(&po.stderr)))) }
}

pub fn generate_diff(h: &Path, c: &Path) -> Result<String> {
    match Command::new("diff").arg("--color=always").arg("-u").arg(h).arg(c).stdout(Stdio::piped()).stderr(Stdio::piped()).output() {
        Ok(out) => if out.stdout.is_empty() && out.stderr.is_empty() { Ok("\x1b[32mIdentical.\x1b[0m\n".into()) } else { Ok(String::from_utf8_lossy(&out.stdout).to_string()) },
        Err(_) => Ok("\x1b[33mdiff not installed; showing hashes:\x1b[0m\n".to_string()
            + &format!("{}: {}\n{}: {}\n", h.display(), hash_file(h).unwrap_or(0), c.display(), hash_file(c).unwrap_or(0))),
    }
}

// ============ Search / fs ============

pub fn name_matches(n: &str, q: &str, fuzzy: bool) -> bool {
    if fuzzy {
        let mut it = n.chars().peekable();
        for cn in q.chars() {
            loop { match it.next() { Some(h) if h == cn => break, Some(_) => continue, None => return false } }
        }
        true
    } else { n.contains(q) }
}

/// Copies a directory tree; symlinks are recreated as symlinks (not de-referenced).
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() { fs::create_dir_all(dst)?; }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());
        let is_sym = path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
        if is_sym {
            let target = fs::read_link(&path)?;
            #[cfg(unix)] { std::os::unix::fs::symlink(&target, &dest_path)?; }
            #[cfg(not(unix))] { fs::copy(&path, &dest_path)?; }
        }
        else if path.is_dir() { copy_dir_recursive(&path, &dest_path)?; }
        else { fs::copy(&path, &dest_path)?; }
    }
    Ok(())
}

pub fn copy_to_clipboard(text: &str) {
    let cmd = if std::env::var("WAYLAND_DISPLAY").is_ok() { ("wl-copy", Vec::<&str>::new()) }
    else if std::env::var("DISPLAY").is_ok() { ("xclip", vec!["-selection", "clipboard"]) }
    else { return; };
    if let Ok(mut child) = Command::new(cmd.0).args(&cmd.1).stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() { let _ = stdin.write_all(text.as_bytes()); }
        let _ = child.wait();
    }
}

// ============ Shell ============

/// Blunt but effective seatbelt against obviously catastrophic commands.
pub fn is_command_dangerous(cmd: &str) -> bool {
    let c = cmd.trim().to_lowercase();
    let home = dirs::home_dir().map(|h| h.display().to_string().to_lowercase()).unwrap_or_default();
    let is_rm = c.starts_with("rm ") || c == "rm";
    let nuke_root = is_rm && (c.contains(" -rf /") || c.contains(" -fr /") || c.contains(" -rf /*") || c.contains(" -fr /*")
        || c.contains(" -rf ~") || c.contains(" -fr ~") || c.contains(" -rf ~/") || c.contains(" -fr ~/"));
    let nuke_home = is_rm && ((!home.is_empty() && c.contains(&format!(" -rf {}", home)))
        || c.contains(" -rf $home") || c.contains(" -fr $home"));
    nuke_root || nuke_home || c.contains("mkfs") || c.contains(" dd if=")
}

/// Runs a command on the real terminal (TUI suspended) with a hard timeout.
pub fn run_shell_command(cmd: &str, timeout: Duration) -> (bool, Option<i32>) {
    disable_raw_and_leave_alt();
    println!("\x1b[1;36m$ {}\x1b[0m", cmd);
    let mut child = match Command::new("sh").arg("-c").arg(cmd).spawn() {
        Ok(c) => c,
        Err(e) => { eprintln!("Failed to run: {}", e); restore_raw_and_enter_alt(); return (false, None); }
    };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => { let r = (st.success(), st.code()); restore_raw_and_enter_alt(); return r; }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    restore_raw_and_enter_alt();
                    println!("\x1b[33m⏱ killed after {}s\x1b[0m", timeout.as_secs());
                    return (false, None);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => { restore_raw_and_enter_alt(); return (false, None); }
        }
    }
}

/// Interactive fzf picker fed by `source_cmd` output. Returns None if fzf missing/cancelled.
pub fn fzf_pick(prompt: &str, source_cmd: &str) -> Option<String> {
    disable_raw_and_leave_alt();
    let result = (|| {
        let mut finder = Command::new("sh").arg("-c").arg(source_cmd)
            .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        let mut list = String::new();
        if let Some(mut s) = finder.stdout.take() { let _ = s.read_to_string(&mut list); }
        let _ = finder.wait();
        let mut child = Command::new("fzf").arg("--prompt").arg(prompt).arg("--height").arg("40%")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().ok()?;
        if let Some(mut sin) = child.stdin.take() { let _ = sin.write_all(list.as_bytes()); }
        let mut out = String::new();
        if let Some(mut so) = child.stdout.take() { let _ = so.read_to_string(&mut out); }
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        let t = out.trim().to_string();
        if ok && !t.is_empty() { Some(t) } else { None }
    })();
    restore_raw_and_enter_alt();
    result
}

pub fn get_latest_version() -> Option<String> {
    let out = Command::new("curl").arg("-s").arg(format!("https://crates.io/api/v1/crates/{}", env!("CARGO_PKG_NAME")))
        .stdout(Stdio::piped()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() { return None; }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    json.get("crate").and_then(|c| c.get("max_stable_version")).and_then(|v| v.as_str()).map(|s| s.to_string())
}

// ============ Services ============

pub fn list_services(user: bool) -> Vec<(String, String, String)> {
    let mut cmd = Command::new("systemctl");
    if user { cmd.arg("--user"); }
    cmd.arg("list-units").arg("--type=service").arg("--all").arg("--no-pager").arg("--plain");
    match cmd.stdout(Stdio::piped()).stderr(Stdio::null()).output() {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).lines().skip(1).filter_map(|l| {
                let parts: Vec<&str> = l.split_whitespace().collect();
                if parts.len() >= 4 {
                    Some((parts[0].to_string(),
                        if parts[3] == "running" { "[r] running".to_string() } else { "[s] stopped".to_string() },
                        parts[4..].join(" ")))
                } else { None }
            }).collect()
        }
        _ => Vec::new(),
    }
}

pub fn restart_service(name: &str, user: bool) -> bool {
    let mut cmd = Command::new("systemctl");
    if user { cmd.arg("--user"); }
    cmd.arg("restart").arg(name).status().map(|s| s.success()).unwrap_or(false)
}

pub fn stop_service(name: &str, user: bool) -> bool {
    let mut cmd = Command::new("systemctl");
    if user { cmd.arg("--user"); }
    cmd.arg("stop").arg(name).status().map(|s| s.success()).unwrap_or(false)
}

pub fn daemon_reload(user: bool) -> bool {
    let mut cmd = Command::new("systemctl");
    if user { cmd.arg("--user"); }
    cmd.arg("daemon-reload").status().map(|s| s.success()).unwrap_or(false)
}

pub fn service_unit_path(name: &str, user: bool) -> PathBuf {
    if user { dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")) .join("systemd/user").join(name) }
    else { PathBuf::from("/etc/systemd/system").join(name) }
}
