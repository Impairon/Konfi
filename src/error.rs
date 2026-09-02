use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ConfyError>;

#[derive(Error, Debug)]
pub enum ConfyError {
    #[error("path traversal: {0}")] PathTraversal(PathBuf),
    #[error("deploy: {0}")] Deploy(String),
    #[error("git: {0}")] Git(String),
    #[error("crypto: {0}")] Crypto(String),
    #[error("invalid input: {0}")] InvalidInput(String),
    #[error("not found: {0}")] NotFound(String),
    #[error("io: {source}")] Io { #[from] source: std::io::Error },
    #[error("json: {source}")] Json { #[from] source: serde_json::Error },
    #[error("zip: {source}")] Zip { #[from] source: zip::result::ZipError },
}
