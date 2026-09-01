use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use age::secrecy::{ExposeSecret, SecretString};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::config::atomic_write;
use crate::error::{ConfyError, Result};

fn default_true() -> bool {
    true
}

pub fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    if path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return Err(ConfyError::InvalidInput(format!("refusing to write through symlink: {}", path.display())));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let mut current = parent;
        while let Some(existing) = current.ancestors().find(|candidate| candidate.exists()) {
            if existing.symlink_metadata()?.file_type().is_symlink() {
                return Err(ConfyError::InvalidInput(format!("refusing to write through symlinked parent: {}", existing.display())));
            }
            current = existing.parent().unwrap_or(existing);
            if existing.parent().is_none() { break; }
        }
    }
    let temp = path.with_file_name(format!(".{}.tmp-{}-{}", path.file_name().and_then(|name| name.to_str()).unwrap_or("secret"), std::process::id(), now_nanos()));
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(&temp)?
    };
    #[cfg(not(unix))]
    let mut file = fs::OpenOptions::new().create_new(true).write(true).open(&temp)?;
    let result = (|| -> Result<()> {
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&temp); }
    result?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecretRule {
    pub glob: String,
    #[serde(default = "default_true")] pub required: bool,
    #[serde(default)] pub backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecretRecipient {
    pub label: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecretsConfig {
    #[serde(default)] pub rules: Vec<SecretRule>,
    #[serde(default)] pub recipients: Vec<SecretRecipient>,
    #[serde(default = "default_true")] pub block_plaintext_commits: bool,
    #[serde(default = "default_true")] pub deploy_decrypt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Encrypted,
    Plaintext,
    EncryptedNoRule,
}

#[derive(Debug, Clone)]
pub struct StatusItem {
    pub rel: String,
    pub kind: StatusKind,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub path: String,
    pub line: usize,
    pub kind: String,
    pub preview: String,
}

pub fn ensure_root_layout(root: &Path) -> Result<()> {
    let assets = root.join(".assets");
    let keys = assets.join(".keys");
    let tmp = assets.join(".tmp");
    fs::create_dir_all(&keys)?;
    fs::create_dir_all(&tmp)?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        let mut p = fs::metadata(&keys)?.permissions();
        p.set_mode(0o700);
        fs::set_permissions(&keys, p.clone())?;
        let mut tp = fs::metadata(&tmp)?.permissions();
        tp.set_mode(0o700);
        fs::set_permissions(&tmp, tp)?;
    }

    let gitignore = root.join(".gitignore");
    let lines = if gitignore.exists() { fs::read_to_string(&gitignore).unwrap_or_default() } else { String::new() };
    let needed = [
        ".assets/.keys/",
        ".assets/.tmp/",
        ".assets/.secrets.json",
        ".assets/.state.json",
    ];
    let mut set: HashSet<String> = lines.lines().map(str::trim).filter(|s| !s.is_empty() && !s.starts_with('#')).map(str::to_string).collect();
    for entry in needed {
        set.insert(entry.to_string());
    }
    let mut next = lines;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    for entry in set.into_iter() {
        if !next.lines().any(|line| line.trim() == entry) {
            next.push_str(&entry);
            next.push('\n');
        }
    }
    if !next.is_empty() {
        fs::write(&gitignore, next)?;
    }
    Ok(())
}

pub fn compile_rules(rules: &[SecretRule]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for rule in rules {
        builder.add(Glob::new(&rule.glob).map_err(|e| ConfyError::InvalidInput(format!("bad secret glob '{}': {}", rule.glob, e)))?);
    }
    Ok(builder.build().map_err(|e| ConfyError::InvalidInput(format!("bad secret rule set: {}", e)))?)
}

pub fn matches(globs: &GlobSet, rel: &str) -> bool {
    globs.is_match(rel) || rel.strip_suffix(".age").is_some_and(|s| globs.is_match(s))
}

pub fn scan_status(root: &Path, cfg: &SecretsConfig) -> Vec<StatusItem> {
    let Ok(globs) = compile_rules(&cfg.rules) else { return Vec::new(); };
    let mut items = Vec::new();
    for path in walked_files(root) {
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let required = cfg.rules.iter().any(|rule| {
            if !rule.required { return false; }
            Glob::new(&rule.glob).ok().is_some_and(|g| {
                g.compile_matcher().is_match(rel.as_str()) || g.compile_matcher().is_match(unencrypted_if_age(rel.as_str()))
            })
        });
        if rel.ends_with(".age") {
            let unencrypted = rel.strip_suffix(".age").unwrap_or(&rel);
            let required = cfg.rules.iter().any(|rule| rule.required && Glob::new(&rule.glob).ok().is_some_and(|g| g.compile_matcher().is_match(unencrypted)));
            if matches(&globs, &rel) {
                items.push(StatusItem { rel, kind: StatusKind::Encrypted, required });
            } else {
                items.push(StatusItem { rel, kind: StatusKind::EncryptedNoRule, required: false });
            }
        } else if matches(&globs, &rel) {
            items.push(StatusItem { rel, kind: StatusKind::Plaintext, required });
        }
    }
    items
}

fn unencrypted_if_age(s: &str) -> &str {
    s.strip_suffix(".age").unwrap_or(s)
}

fn walked_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(root) else { return out; };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name == ".git" || name == ".assets" { continue; }
        if p.is_dir() {
            out.extend(walked_files(&p));
        } else {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn walk_plaintext_candidates(root: &Path) -> Vec<PathBuf> {
    walked_files(root)
        .into_iter()
        .filter(|p| !p.to_string_lossy().ends_with(".age"))
        .collect()
}

fn secret_patterns() -> Vec<(&'static str, Regex)> {
    vec![
        ("aws-key", Regex::new(r"(AKIA|ASIA)[0-9A-Z]{16}").unwrap()),
        ("ssh-private-key", Regex::new(r"-----BEGIN (?:RSA |OPENSSH |EC |DSA )?PRIVATE KEY-----").unwrap()),
        ("github-token", Regex::new(r"gh[pousr]_[A-Za-z0-9]{36,}").unwrap()),
        ("google-api-key", Regex::new(r"AIza[0-9A-Za-z\-_]{35}").unwrap()),
        ("assigned-secret", Regex::new(r#"(?i)\b(passwo?rd|passwd|secret|api[_-]?key|token|credential)s?\b\s*[:=]\s*[\"']([^\"']{8,})[\"']"#).unwrap()),
    ]
}

pub fn scan_plaintext(root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    for path in walk_plaintext_candidates(root) {
        let rel = path.strip_prefix(root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        let Ok(bytes) = fs::read(&path) else { continue; };
        if bytes.is_empty() || bytes.iter().any(|b| *b == 0) || bytes.len() > 2 * 1024 * 1024 { continue; }
        let text = String::from_utf8_lossy(&bytes);
        for (idx, line) in text.lines().enumerate() {
            for (kind, regex) in secret_patterns() {
                if let Some(caps) = regex.captures(line) {
                    let preview = caps.get(2).map(|m| {
                        let prefix: String = m.as_str().chars().take(2).collect();
                        format!("{} = \"{}***\"", kind, prefix)
                    }).unwrap_or_else(|| format!("{} pattern matched", kind));
                    out.push(Finding { path: rel.clone(), line: idx + 1, kind: kind.to_string(), preview });
                    break;
                }
            }
        }
    }
    out
}

pub fn check_git_blockers(root: &Path) -> Result<()> {
    let cfg = load_config(root);
    if !cfg.block_plaintext_commits { return Ok(()); }
    let blocked: Vec<String> = scan_status(root, &cfg)
        .into_iter()
        .filter(|item| matches!(item.kind, StatusKind::Plaintext) && item.required)
        .map(|item| item.rel)
        .collect();
    if !blocked.is_empty() {
        return Err(ConfyError::Git(format!("blocked plaintext required secret(s): {}\nset CONFY_ALLOW_SECRETS=1 to override", blocked.join(", "))));
    }
    let findings = scan_plaintext(root);
    if !findings.is_empty() {
        let joined = findings.iter().take(5).map(|f| format!("{}:{}", f.path, f.line)).collect::<Vec<_>>().join(", ");
        return Err(ConfyError::Git(format!("possible secrets in plaintext: {}\nset CONFY_ALLOW_SECRETS=1 to override", joined)));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SecretsManager {
    pub keys_dir: PathBuf,
    pub cfg_path: PathBuf,
    pub cfg: SecretsConfig,
}

impl SecretsManager {
    pub fn load(root: &Path) -> Self {
        let cfg_path = root.join(".assets/.secrets.json");
        let cfg = load_config(root);
        let manager = Self {
            keys_dir: root.join(".assets/.keys"),
            cfg_path,
            cfg,
        };
        let _ = ensure_root_layout(root);
        manager
    }

    pub fn save(&self) -> Result<()> {
        atomic_write(&self.cfg_path, &serde_json::to_vec_pretty(&self.cfg)?)?;
        Ok(())
    }

    pub fn generate_keypair(&mut self, label: &str) -> Result<String> {
        let (secret, public) = generate_age_keypair();
        let key_path = self.key_path(label);
        let _ = fs::create_dir_all(&self.keys_dir);
        let secret = Zeroizing::new(secret);
        write_private_file(&key_path, secret.as_bytes())?;
        if !self.cfg.recipients.iter().any(|r| r.key == public) {
            self.cfg.recipients.push(SecretRecipient { label: label.to_string(), key: public.clone() });
            self.save()?;
        }
        Ok(public)
    }

    pub fn list_identities(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&self.keys_dir) else { return out; };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(true) { continue; }
            let label = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let Ok(secret) = fs::read_to_string(&p) else { continue; };
            if validate_identity(&secret).is_some() {
                out.push((label, secret));
            }
        }
        out
    }

    fn key_path(&self, label: &str) -> PathBuf {
        let safe: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        self.keys_dir.join(format!("{}.key", safe))
    }

}

fn now_nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
}

pub fn load_config(root: &Path) -> SecretsConfig {
    let cfg_path = root.join(".assets/.secrets.json");
    let raw = fs::read_to_string(&cfg_path).ok();
    raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default()
}

pub fn generate_age_keypair() -> (String, String) {
    let id = age::x25519::Identity::generate();
    let secret = id.to_string();
    let public = id.to_public().to_string();
    (secret.expose_secret().to_string(), public.to_string())
}

pub fn validate_identity(s: &str) -> Option<String> {
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Ok(identity) = age::x25519::Identity::from_str(line) {
            return Some(identity.to_public().to_string());
        }
    }
    None
}

pub fn encrypt_with_recipients(plaintext: &[u8], recipients: &[String]) -> Result<Vec<u8>> {
    let mut valid = Vec::new();
    for recipient in recipients {
        let s = recipient.trim();
        if s.is_empty() || s.starts_with('#') { continue; }
        let rec = age::x25519::Recipient::from_str(s).map_err(|e| ConfyError::InvalidInput(format!("bad age recipient: {}", e)))?;
        valid.push(rec);
    }
    if valid.is_empty() {
        return Err(ConfyError::InvalidInput("no valid age recipients configured".into()));
    }
    let enc = age::Encryptor::with_recipients(valid.iter().map(|r| r as &dyn age::Recipient)).map_err(|e| ConfyError::Crypto(format!("encrypt: {}", e)))?;
    let mut out = Vec::new();
    let mut writer = enc.wrap_output(&mut out).map_err(|e| ConfyError::Crypto(format!("wrap_output: {}", e)))?;
    writer.write_all(plaintext)?;
    writer.finish().map_err(|e| ConfyError::Crypto(format!("finish: {}", e)))?;
    Ok(out)
}

pub fn encrypt_with_passphrase(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let pass = SecretString::new(passphrase.to_owned().into());
    let enc = age::Encryptor::with_user_passphrase(pass);
    let mut out = Vec::new();
    let mut writer = enc.wrap_output(&mut out).map_err(|e| ConfyError::Crypto(format!("pass encrypt: {}", e)))?;
    writer.write_all(plaintext)?;
    writer.finish().map_err(|e| ConfyError::Crypto(format!("pass finish: {}", e)))?;
    Ok(out)
}

pub fn decrypt_with_identities(ciphertext: &[u8], identities: &[String]) -> Result<Vec<u8>> {
    let mut ids = Vec::new();
    for s in identities {
        for line in s.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let id = age::x25519::Identity::from_str(line).map_err(|e| ConfyError::InvalidInput(format!("bad age identity: {}", e)))?;
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(ConfyError::InvalidInput("no age identities available".into()));
    }
    let decryptor = age::Decryptor::new(ciphertext).map_err(|e| ConfyError::Crypto(format!("decrypt: {}", e)))?;
    if decryptor.is_scrypt() {
        return Err(ConfyError::Crypto("encrypted with passphrase, not private keys".into()));
    }
    let mut reader = decryptor.decrypt(ids.iter().map(|i| i as &dyn age::Identity)).map_err(|_| ConfyError::Crypto("no matching age identity for encrypted file".into()))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

pub fn decrypt_with_passphrase(ciphertext: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let decryptor = age::Decryptor::new(ciphertext).map_err(|e| ConfyError::Crypto(format!("decrypt: {}", e)))?;
    if !decryptor.is_scrypt() {
        return Err(ConfyError::Crypto("encrypted with recipients, not a passphrase".into()));
    }
    let identity = age::scrypt::Identity::new(SecretString::new(passphrase.to_owned().into()));
    let mut reader = decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity)).map_err(|_| ConfyError::Crypto("wrong passphrase".into()))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn round_trip_key_encryption_works() {
        let root = std::env::temp_dir().join(format!("confy-secrets-test-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root.join(".assets/.keys")).unwrap();
        let mut manager = SecretsManager::load(&root);
        let public = manager.generate_keypair("test-user").unwrap();
        let plain = root.join("config.yaml");
        fs::write(&plain, "api_key: test\n").unwrap();
        let enc = encrypt_with_recipients(b"api_key: test\n", &[public.clone()]).unwrap();
        assert!(enc.starts_with(b"age-encryption.org/v1"));
        let identity = manager.list_identities().into_iter().map(|(_, value)| value).collect::<Vec<_>>();
        let plain = decrypt_with_identities(&enc, &identity).unwrap();
        assert_eq!(plain, b"api_key: test\n");
        let _ = fs::remove_dir_all(&root);
        assert!(!public.trim().is_empty());
    }

    #[test]
    fn round_trip_passphrase_encryption_works() {
        let ciphertext = encrypt_with_passphrase(b"private value", "test passphrase").unwrap();
        assert_eq!(decrypt_with_passphrase(&ciphertext, "test passphrase").unwrap(), b"private value");
        assert!(decrypt_with_passphrase(&ciphertext, "wrong").is_err());
    }
}
