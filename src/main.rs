mod app;
mod config;
mod error;
mod ops;
mod secrets;
mod tui;

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::ConfyState;

#[derive(Parser)]
#[command(name = "confy", version, about = "A TUI dotfile manager")]
struct Cli {
    #[arg(short = 'r', long, global = true, num_args = 0..=1, default_missing_value = "", help = "Set root directory; omit the value to list saved roots")]
    root: Option<Option<String>>,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    Tui,
    #[command(short_flag = 'e')]
    Edit { #[arg(help = "File or directory to edit")] path: Option<String>, #[arg(short = 't', long, help = "Open the TUI editor")] tui: bool },
    #[command(short_flag = 'o')]
    Open { #[arg(help = "Path to open")] path: Option<String> },
    #[command(short_flag = 'l')]
    Link { #[arg(help = "Source path to link")] path: String, #[arg(short = 'n', long = "name", help = "Name inside the Confy root")] name: Option<String> },
    #[command(short_flag = 'd')]
    Deploy { #[arg(help = "Archive to inspect or deploy")] archive: String, #[arg(short = 'a', long, help = "Apply the deployment; otherwise show a dry run")] apply: bool, #[arg(long, help = "Legacy archive password option (unsupported)")] pass: Option<String>, #[arg(short = 'H', long, help = "Only deploy entries for this host")] host: Option<String>, #[arg(short = 's', long, help = "Explicitly allow archive scripts to run")] allow_scripts: bool },
    #[command(short_flag = 'b')]
    Rollback { archive: String },
    Doctor,
    Discover,
    #[command(short_flag = 'i')]
    Import { dir: String },
    #[command(short_flag = 'c')]
    Clone { #[arg(help = "Git repository URL or local repository path")] source: String, #[arg(help = "Optional relative destination under the selected root")] destination: Option<String> },
    GitExport,
    GitRemote { url: String },
    #[command(short_flag = 'p')]
    GitPush { #[arg(help = "Optional paths to stage; omit for the normal full push")] paths: Vec<String> },
    Secret {
        #[command(subcommand)]
        action: SecretCmd,
    },
}

#[derive(Subcommand)]
enum SecretCmd {
    #[command(short_flag = 'g', about = "Generate an age keypair and store the private identity under the selected root")]
    Generate { #[arg(help = "Key label, for example laptop")] label: String },
    #[command(short_flag = 'e', about = "Encrypt a file with configured age recipients by default")]
    Encrypt { #[arg(help = "Plaintext file under the selected root")] path: String, #[arg(short = 'R', long, help = "Recipient key; repeat for multiple recipients")] recipient: Vec<String>, #[arg(long, help = "Use a hidden passphrase prompt instead of age recipients")] passphrase: bool, #[arg(short = 'o', long, help = "Output .age path")] output: Option<String> },
    #[command(short_flag = 'd', about = "Decrypt an age file using local identities by default")]
    Decrypt { #[arg(help = "Encrypted file under the selected root")] path: String, #[arg(long, help = "Use a hidden passphrase prompt")] passphrase: bool, #[arg(short = 'o', long, help = "Plaintext output path")] output: Option<String> },
    #[command(short_flag = 's', about = "Show encrypted and plaintext secret status")]
    Status { #[arg(long, help = "Compatibility option; global --root selects the scan root")] root: Option<String> },
    #[command(short_flag = 'c', about = "Scan plaintext files for likely secrets")]
    Scan { #[arg(long, help = "Compatibility option; global --root selects the scan root")] root: Option<String> },
}

fn run_secret_command(confy_dir: &Path, action: &SecretCmd) -> anyhow::Result<()> {
    let root_path = |raw: &str| -> anyhow::Result<PathBuf> {
        let path = PathBuf::from(raw);
        if path.components().any(|component| matches!(component, std::path::Component::ParentDir)) {
            return Err(anyhow::anyhow!("secret paths cannot contain '..'"));
        }
        let path = if path.is_absolute() { path } else { confy_dir.join(path) };
        if !path.starts_with(confy_dir) {
            return Err(anyhow::anyhow!("secret path must stay under the selected root: {}", confy_dir.display()));
        }
        Ok(path)
    };
    let read_passphrase = |requested: bool, prompt: &str| -> anyhow::Result<Option<String>> {
        if requested { Ok(Some(rpassword::prompt_password(prompt)?)) } else { Ok(None) }
    };
    match action {
        SecretCmd::Generate { label } => {
            let mut manager = crate::secrets::SecretsManager::load(confy_dir);
            let pubkey = manager.generate_keypair(label)?;
            println!("✅ Generated age keypair for {}", label);
            println!("Public recipient: {}", pubkey);
            Ok(())
        }
        SecretCmd::Encrypt { path, recipient, passphrase, output } => {
            let manager = crate::secrets::SecretsManager::load(confy_dir);
            let target = root_path(path)?;
            let recipients = if recipient.is_empty() { manager.cfg.recipients.iter().map(|r| r.key.clone()).collect() } else { recipient.clone() };
            let passphrase = read_passphrase(*passphrase, "Passphrase: ")?;
            if passphrase.is_some() {
                let plain = std::fs::read(&target)?;
                let encrypted = crate::secrets::encrypt_with_passphrase(&plain, passphrase.as_deref().unwrap())?;
                let out = output.as_deref().map(root_path).transpose()?.unwrap_or_else(|| target.with_extension("age"));
                crate::secrets::write_private_file(&out, &encrypted)?;
                println!("✅ Wrote {}", out.display());
            } else {
                let plain = std::fs::read(&target)?;
                let encrypted = crate::secrets::encrypt_with_recipients(&plain, &recipients)?;
                let out = output.as_deref().map(root_path).transpose()?.unwrap_or_else(|| target.with_extension("age"));
                crate::secrets::write_private_file(&out, &encrypted)?;
                println!("✅ Wrote {}", out.display());
            }
            Ok(())
        }
        SecretCmd::Decrypt { path, passphrase, output } => {
            let manager = crate::secrets::SecretsManager::load(confy_dir);
            let encrypted = root_path(path)?;
            let bytes = std::fs::read(&encrypted)?;
            let passphrase = read_passphrase(*passphrase, "Passphrase: ")?;
            let plain = if let Some(pass) = passphrase.as_deref() {
                crate::secrets::decrypt_with_passphrase(&bytes, pass)?
            } else {
                let identities = manager.identities_raw().into_iter().map(|(_, v)| v.to_string()).collect::<Vec<_>>();
                crate::secrets::decrypt_with_identities(&bytes, &identities)?
            };
            let out = output.as_deref().map(root_path).transpose()?.unwrap_or_else(|| encrypted.with_extension("txt"));
            crate::secrets::write_private_file(&out, &plain)?;
            println!("✅ Wrote {}", out.display());
            Ok(())
        }
        SecretCmd::Status { root: _ } => {
            let root = confy_dir;
            let manager = crate::secrets::SecretsManager::load(root);
            for item in crate::secrets::scan_status(root, &manager.cfg) {
                let label = match item.kind {
                    crate::secrets::StatusKind::Encrypted => "encrypted",
                    crate::secrets::StatusKind::Plaintext => "plaintext",
                    crate::secrets::StatusKind::EncryptedNoRule => "encrypted-no-rule",
                };
                println!("{}\t{}", label, item.rel);
            }
            Ok(())
        }
        SecretCmd::Scan { root: _ } => {
            let root = confy_dir;
            let findings = crate::secrets::scan_plaintext(root);
            if findings.is_empty() {
                println!("No likely plaintext secrets found.");
            } else {
                for finding in findings {
                    println!("{}:{} {} {}", finding.path, finding.line, finding.kind, finding.preview);
                }
            }
            Ok(())
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if matches!(&cli.root, Some(None)) {
        let default_root = dirs::home_dir().map(|h| h.join(".confy")).ok_or_else(|| anyhow::anyhow!("No home dir"))?;
        let roots = crate::config::load_roots(&default_root.join(".assets/.roots.json"));
        if roots.is_empty() { println!("No saved roots. Default: {}", default_root.display()); }
        else { for root in roots { println!("{}", root.display()); } }
        return Ok(());
    }
    let confy_dir = match &cli.root {
        Some(Some(p)) => ops::expand_tilde(p),
        Some(None) => unreachable!("bare --root is handled above"),
        None => dirs::home_dir().map(|h| h.join(".confy")).ok_or_else(|| anyhow::anyhow!("No home dir"))?,
    };
    init_tracing(&confy_dir);
    match &cli.command {
        None | Some(Cmd::Tui) => { tui::run_tui(confy_dir, None)?; }
        Some(Cmd::Edit { path, tui }) => {
            let p = path.as_ref().map(|p| ops::expand_tilde(p));
            if *tui { tui::run_tui(confy_dir, p)?; }
            else {
                let editor = std::fs::read_to_string(confy_dir.join(".assets/.state.json")).ok()
                    .and_then(|s| serde_json::from_str::<ConfyState>(&s).ok())
                    .and_then(|st| st.settings.default_editor)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "vi".into());
                match p {
                    Some(p) => { ops::open_editor(&p, &editor); }
                    None => eprintln!("Usage: confy edit <file> [--tui]"),
                }
            }
        }
        Some(Cmd::Open { path }) => {
            let p = path.as_ref().map(|p| ops::expand_tilde(p)).unwrap_or_else(|| confy_dir.clone());
            let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
            Command::new(opener).arg(&p).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
        }
        Some(Cmd::Link { path, name }) => {
            let src = ops::expand_tilde(path);
            if !src.exists() { eprintln!("Not found: {}", path); std::process::exit(1); }
            let n = name.clone().unwrap_or_else(|| ops::path_name(&src));
            let d = confy_dir.join(&n);
            ops::ensure_parent(&d)?;
            if d.symlink_metadata().is_ok() { let _ = std::fs::remove_file(&d); }
            match std::os::unix::fs::symlink(&src, &d) {
                Ok(_) => println!("Linked {} -> {}", n, src.display()),
                Err(e) => eprintln!("Failed: {}", e),
            }
        }
        Some(Cmd::Deploy { archive, apply, pass, host, allow_scripts }) => {
            let st = ConfyState::load(&confy_dir.join(".assets/.state.json"));
            match ops::deploy_archive(&confy_dir, &PathBuf::from(archive), *apply, pass.as_deref(), host.as_deref(), *allow_scripts, &st.settings) {
                Ok(sum) => println!("{}", sum.output),
                Err(e) => { eprintln!("Deploy failed: {}", e); std::process::exit(1); }
            }
        }
        Some(Cmd::Rollback { archive }) => {
            let nm = ops::path_name(&PathBuf::from(&archive));
            let dd = confy_dir.join(".assets/.deployments");
            let home = dirs::home_dir().unwrap_or_default();
            if let Ok(entries) = std::fs::read_dir(&dd) {
                let mut deps: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
                deps.sort();
                if let Some(dep) = deps.iter().rev().find(|d| d.file_name().unwrap_or_default().to_string_lossy().starts_with(&nm)) {
                    println!("⏪ Rolling back {}...", nm);
                    for line in std::fs::read_to_string(dep.join("rollback_manifest.txt")).unwrap_or_default().lines() {
                        let parts: Vec<&str> = line.splitn(2, '\t').collect();
                        if parts.len() == 2 {
                            let alias = parts[0];
                            let target = Path::new(parts[1]);
                            // hard guards: no traversal, never touch / or $HOME
                            if alias.is_empty() || alias.contains("..") { continue; }
                            if target == Path::new("/") || target == home { continue; }
                            let bak = dep.join(alias);
                            if bak.exists() && bak.starts_with(&dep) {
                                let _ = crate::ops::remove_entry(target);
                                if std::fs::rename(&bak, target).is_err() {
                                    eprintln!("  [!] failed to restore {}", target.display());
                                }
                            }
                        }
                    }
                    for line in std::fs::read_to_string(dep.join("created_manifest.txt")).unwrap_or_default().lines() {
                        let p = Path::new(line.trim());
                        if p == Path::new("/") || p == home { continue; }
                        if ops::path_exists(&p) {
                            let _ = crate::ops::remove_entry(p);
                        }
                    }
                    let _ = std::fs::remove_dir_all(dep);
                    println!("✅ Rollback complete.");
                    return Ok(());
                }
            }
            eprintln!("No deployment found for {}", nm);
            std::process::exit(1);
        }
        Some(Cmd::Doctor) => run_doctor(&confy_dir),
        Some(Cmd::Discover) => run_discover(&confy_dir),
        Some(Cmd::Import { dir }) => run_import(&confy_dir, dir),
        Some(Cmd::Clone { source, destination }) => {
            let destination = destination.as_ref().map(|raw| {
                let path = PathBuf::from(raw);
                if path.is_absolute() || path.components().any(|component| matches!(component, std::path::Component::ParentDir)) {
                    return Err(anyhow::anyhow!("clone destination must be relative under the selected root"));
                }
                Ok(confy_dir.join(path))
            }).transpose()?;
            let source = ops::expand_tilde(source).to_string_lossy().into_owned();
            let cloned = ops::git_clone(&confy_dir, &source, destination.as_deref())?;
            println!("Cloned into {}", cloned.display());
        }
        Some(Cmd::GitExport) => match ops::git_export(&confy_dir) { Ok(m) => println!("{}", m), Err(e) => { eprintln!("{}", e); std::process::exit(1); } },
        Some(Cmd::GitRemote { url }) => match ops::git_remote(&confy_dir, url) { Ok(m) => println!("{}", m), Err(e) => { eprintln!("{}", e); std::process::exit(1); } },
        Some(Cmd::GitPush { paths }) => {
            let mut selected = Vec::with_capacity(paths.len());
            for raw in paths {
                let path = PathBuf::from(raw);
                if path.components().any(|component| matches!(component, std::path::Component::ParentDir)) {
                    return Err(anyhow::anyhow!("git-push paths cannot contain '..': {}", raw));
                }
                let path = if path.is_absolute() { path } else { confy_dir.join(path) };
                if !path.starts_with(&confy_dir) {
                    return Err(anyhow::anyhow!("git-push path must stay under the selected root: {}", raw));
                }
                if !ops::path_exists(&path) {
                    return Err(anyhow::anyhow!("git-push path not found: {}", raw));
                }
                selected.push(path);
            }
            match ops::git_push(&confy_dir, &selected) { Ok(m) => println!("{}", m), Err(e) => { eprintln!("{}", e); std::process::exit(1); } }
        }
        Some(Cmd::Secret { action }) => run_secret_command(&confy_dir, action)?,
    }
    tracing::info!("=== confy session ended ===");
    Ok(())
}

fn init_tracing(cd: &Path) {
    let _ = std::fs::create_dir_all(cd.join(".assets"));
    let lp = cd.join(".assets").join("confy.log");
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&lp) {
        let _ = tracing_subscriber::fmt()
            .with_writer(f)
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .with_target(false)
            .with_timer(tracing_subscriber::fmt::time::SystemTime)
            .try_init();
    }
    tracing::info!("=== confy session started ===");
}

fn run_doctor(cd: &Path) {
    println!("🩺 Running Confy Doctor...\nDependencies:");
    for d in &["bat", "chafa", "fzf", "ffmpeg", "diff", "git"] {
        println!("  [{}] {}", if ops::command_exists(d) { "✓" } else { "✗" }, d);
    }
    println!("\nSymlinks:");
    if let Ok(entries) = std::fs::read_dir(cd) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_sym = path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);
            if is_sym {
                if std::fs::metadata(&path).is_err() { println!("  [✗] BROKEN: {}", path.display()); }
                else { println!("  [✓] OK: {}", path.display()); }
            }
        }
    }
    check_hooks(cd);
    check_version(cd);
}

fn check_version(cd: &Path) {
    println!("\nVersion:");
    let current = env!("CARGO_PKG_VERSION");
    println!("  Current: {}", current);
    let settings = ConfyState::load(&cd.join(".assets/.state.json")).settings;
    if !settings.check_updates_in_doctor {
        println!("  Update check disabled in settings.");
        return;
    }
    let cache_path = cd.join(".assets/.last_version_check.json");
    let mut do_check = true;
    if let Ok(s) = std::fs::read_to_string(&cache_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
            if let Some(checked_at) = v.get("checked_at").and_then(|t| t.as_u64()) {
                let now = ops::now_secs();
                if now.saturating_sub(checked_at) < 86_400 {
                    do_check = false;
                    if let Some(latest) = v.get("latest_version").and_then(|v| v.as_str()) {
                        if latest != current { println!("  \x1b[33mUpdate available!\x1b[0m Latest: {}", latest); }
                        else { println!("  \x1b[32mUp to date.\x1b[0m"); }
                    }
                }
            }
        }
    }
    if do_check {
        if let Some(latest) = ops::get_latest_version() {
            let now = ops::now_secs();
            let json = serde_json::json!({ "checked_at": now, "latest_version": latest });
            let _ = std::fs::write(&cache_path, serde_json::to_vec_pretty(&json).unwrap_or_default());
            if latest != current { println!("  \x1b[33mUpdate available!\x1b[0m Latest: {}", latest); }
            else { println!("  \x1b[32mUp to date.\x1b[0m"); }
        } else { println!("  Could not fetch latest version (network issue)."); }
    }
}

fn check_hooks(cd: &Path) {
    println!("\nHooks:");
    let mut total = 0usize;
    let mut bad = 0usize;
    let mut report = |label: String, value: &str| {
        total += 1;
        match ops::validate_hook(value) {
            Ok(kind) => println!("  [✓] {} -> {}", label, kind),
            Err(e) => { bad += 1; println!("  [✗] {} -> {}", label, e); }
        }
    };

    let hooks_file = cd.join(".assets").join(".hooks.json");
    if let Some(map) = std::fs::read_to_string(&hooks_file).ok()
        .and_then(|s| serde_json::from_str::<std::collections::HashMap<String, String>>(&s).ok())
    {
        let mut keys: Vec<_> = map.keys().cloned().collect();
        keys.sort();
        for k in keys { let v = map[&k].clone(); report(format!("global {}", k), &v); }
    }

    let state_file = cd.join(".assets").join(".state.json");
    if let Some(state) = std::fs::read_to_string(&state_file).ok()
        .and_then(|s| serde_json::from_str::<ConfyState>(&s).ok())
    {
        let mut objs: Vec<_> = state.object_hooks.keys().cloned().collect();
        objs.sort();
        for obj in objs {
            let map = state.object_hooks[&obj].clone();
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            for k in keys { let v = map[&k].clone(); report(format!("{} {}", obj, k), &v); }
        }
    }

    if total == 0 { println!("  (none configured)"); }
    else if bad == 0 { println!("  {} hook(s), all runnable.", total); }
    else { println!("  {} of {} hook(s) are broken.", bad, total); }
}

fn run_discover(cd: &Path) {
    let Some(home) = dirs::home_dir() else { eprintln!("No home dir"); return; };
    let _ = std::fs::create_dir_all(cd);
    let known = ["~/.bashrc", "~/.zshrc", "~/.gitconfig", "~/.tmux.conf", "~/.config/starship.toml",
        "~/.config/kitty", "~/.config/alacritty", "~/.config/wezterm", "~/.config/ghostty",
        "~/.config/nvim", "~/.config/helix", "~/.config/hypr", "~/.config/sway",
        "~/.config/waybar", "~/.config/rofi", "~/.config/dunst"];
    println!("🔍 Discovering dotfiles...");
    for ps in &known {
        let exp = if let Some(s) = ps.strip_prefix("~/") { home.join(s) } else { PathBuf::from(ps) };
        if exp.exists() {
            let alias = ops::path_name(&exp);
            let dest = cd.join(&alias);
            if dest.symlink_metadata().is_err() {
                match std::os::unix::fs::symlink(&exp, &dest) {
                    Ok(_) => println!("  [+] Linked {}", alias),
                    Err(e) => println!("  [✗] {}: {}", alias, e),
                }
            } else { println!("  [✓] Already linked: {}", alias); }
        }
    }
    println!("✅ Discovery complete.");
}

fn run_import(cd: &Path, src: &str) {
    let sd = PathBuf::from(src);
    if !sd.is_dir() { eprintln!("Not a directory."); std::process::exit(1); }
    println!("📥 Importing from {}...", sd.display());
    if let Ok(entries) = std::fs::read_dir(&sd) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let alias = ops::path_name(&p);
                let dest = cd.join(&alias);
                if dest.symlink_metadata().is_err() {
                    match std::os::unix::fs::symlink(&p, &dest) {
                        Ok(_) => println!("  [+] Linked {}", alias),
                        Err(e) => println!("  [✗] {}: {}", alias, e),
                    }
                } else { println!("  [✓] Already exists: {}", alias); }
            }
        }
    }
    println!("✅ Import complete.");
}
