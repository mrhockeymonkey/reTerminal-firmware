//! The one piece of state: the current screen spec document.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use screen_spec::{content_hash, ParseError, MAX_JSON_BYTES, MAX_TEXT};

/// Served when there is no state file yet.
pub const DEFAULT_SPEC: &str = include_str!("../../screen-spec/samples/kitchen.json");

/// A validated document plus the metadata `GET /screen` exposes.
#[derive(Clone, Debug)]
pub struct Current {
    /// The exact bytes the device will receive.
    pub json: String,
    /// Quoted hex FNV-1a of `json`; also what the device compares against.
    pub etag: String,
}

impl Current {
    fn new(json: String) -> Self {
        let etag = format!("\"{:016x}\"", content_hash(json.as_bytes()));
        Current { json, etag }
    }
}

/// Why a document was not accepted.
#[derive(Debug)]
pub enum SetError {
    /// Larger than the device can buffer.
    TooLarge(usize),
    /// Rejected by the shared `screen-spec` parser.
    Invalid(ParseError),
    /// Persisting to the state file failed (the in-memory state is unchanged).
    Io(io::Error),
}

impl fmt::Display for SetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetError::TooLarge(n) => {
                write!(f, "document is {n} bytes; the limit is {MAX_JSON_BYTES}")
            }
            SetError::Invalid(e) => write!(f, "{e}"),
            SetError::Io(e) => write!(f, "could not persist state file: {e}"),
        }
    }
}

impl std::error::Error for SetError {}

/// Validates exactly as the device does.
pub fn validate(json: &[u8]) -> Result<(), SetError> {
    if json.len() > MAX_JSON_BYTES {
        return Err(SetError::TooLarge(json.len()));
    }
    let mut scratch = [0u8; MAX_TEXT];
    screen_spec::parse(json, &mut scratch)
        .map(|_| ())
        .map_err(SetError::Invalid)
}

/// Shared application state.
pub struct AppState {
    current: RwLock<Current>,
    state_file: Option<PathBuf>,
}

impl AppState {
    /// Loads the document from `state_file` if it exists and is valid,
    /// otherwise starts with [`DEFAULT_SPEC`]. `None` keeps state in memory.
    pub fn load(state_file: Option<PathBuf>) -> io::Result<Self> {
        let json = match &state_file {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(path)?;
                match validate(text.as_bytes()) {
                    Ok(()) => {
                        tracing::info!("loaded screen spec from {}", path.display());
                        text
                    }
                    Err(e) => {
                        tracing::warn!(
                            "ignoring {}: {e}; using the built-in sample",
                            path.display()
                        );
                        DEFAULT_SPEC.to_owned()
                    }
                }
            }
            Some(path) => {
                tracing::info!("no {} yet; using the built-in sample", path.display());
                DEFAULT_SPEC.to_owned()
            }
            None => DEFAULT_SPEC.to_owned(),
        };
        Ok(AppState {
            current: RwLock::new(Current::new(json)),
            state_file,
        })
    }

    /// In-memory state seeded with `json` (tests).
    #[cfg(test)]
    pub fn in_memory(json: &str) -> Self {
        AppState {
            current: RwLock::new(Current::new(json.to_owned())),
            state_file: None,
        }
    }

    /// The current document.
    pub fn current(&self) -> Current {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Validates, persists, then publishes a new document.
    pub fn set(&self, json: String) -> Result<Current, SetError> {
        validate(json.as_bytes())?;
        if let Some(path) = &self.state_file {
            write_atomically(path, json.as_bytes()).map_err(SetError::Io)?;
        }
        let next = Current::new(json);
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = next.clone();
        Ok(next)
    }
}

/// Write to a sibling temp file and rename, so a crash mid-write never
/// leaves a truncated state file behind.
fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("screen.json"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
