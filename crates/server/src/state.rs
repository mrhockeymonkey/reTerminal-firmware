//! The server's state: the screen template, the values filled into it, and
//! the rendered document the device receives.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use screen_spec::{content_hash, ParseError, MAX_JSON_BYTES, MAX_TEXT};

use crate::template::{self, TemplateError, Values};

/// Served when there is no state file yet.
pub const DEFAULT_SPEC: &str = include_str!("../../screen-spec/samples/kitchen.json");

/// Name of the values file, kept in the same directory as the state file.
pub const VALUES_FILE: &str = "values.json";

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
    /// The template's placeholders could not be filled.
    Template(TemplateError),
    /// The values document was malformed.
    BadValues(String),
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
            SetError::Template(e) => write!(f, "{e}"),
            SetError::BadValues(e) => write!(f, "{e}"),
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

/// Fills `template` with `values` and validates the result as the device would.
fn render(template: &str, values: &Values) -> Result<Current, SetError> {
    let json = template::render(template, values).map_err(SetError::Template)?;
    validate(json.as_bytes())?;
    Ok(Current::new(json))
}

struct Inner {
    /// The layout as stored, placeholders and all.
    template: String,
    /// What the placeholders are filled with.
    values: Values,
    /// `template` rendered with `values`: what the device receives.
    current: Current,
}

/// Shared application state.
pub struct AppState {
    inner: RwLock<Inner>,
    state_file: Option<PathBuf>,
    values_file: Option<PathBuf>,
}

impl AppState {
    /// Loads the template from `state_file` and the values from a
    /// `values.json` next to it. A missing or invalid template falls back to
    /// [`DEFAULT_SPEC`]; missing or invalid values fall back to empty ones.
    /// `None` keeps state in memory.
    pub fn load(state_file: Option<PathBuf>) -> io::Result<Self> {
        let values_file = state_file
            .as_ref()
            .map(|path| path.with_file_name(VALUES_FILE));
        let values = match &values_file {
            Some(path) if path.exists() => match Values::from_json(&std::fs::read(path)?) {
                Ok(values) => {
                    tracing::info!("loaded values from {}", path.display());
                    values
                }
                Err(e) => {
                    tracing::warn!("ignoring {}: {e}; using empty values", path.display());
                    Values::default()
                }
            },
            _ => Values::default(),
        };
        let template = match &state_file {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(path)?;
                match render(&text, &values) {
                    Ok(_) => {
                        tracing::info!("loaded screen template from {}", path.display());
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
        let current = render(&template, &values).expect("validated above or built-in");
        Ok(AppState {
            inner: RwLock::new(Inner {
                template,
                values,
                current,
            }),
            state_file,
            values_file,
        })
    }

    /// In-memory state seeded with `json` as the template (tests).
    #[cfg(test)]
    pub fn in_memory(json: &str) -> Self {
        let values = Values::default();
        AppState {
            inner: RwLock::new(Inner {
                template: json.to_owned(),
                current: render(json, &values).expect("valid test template"),
                values,
            }),
            state_file: None,
            values_file: None,
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The current rendered document.
    pub fn current(&self) -> Current {
        self.read().current.clone()
    }

    /// The stored template.
    pub fn template(&self) -> String {
        self.read().template.clone()
    }

    /// The current values.
    pub fn values(&self) -> Values {
        self.read().values.clone()
    }

    /// Validates (rendered with the current values), persists, then
    /// publishes a new template.
    pub fn set_template(&self, template: String) -> Result<Current, SetError> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let next = render(&template, &inner.values)?;
        if let Some(path) = &self.state_file {
            write_atomically(path, template.as_bytes()).map_err(SetError::Io)?;
        }
        inner.template = template;
        inner.current = next.clone();
        Ok(next)
    }

    /// Validates (rendered into the current template), persists, then
    /// publishes new values.
    pub fn set_values(&self, json: &[u8]) -> Result<Current, SetError> {
        let values = Values::from_json(json).map_err(SetError::BadValues)?;
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let next = render(&inner.template, &values)?;
        if let Some(path) = &self.values_file {
            write_atomically(path, values.to_json().as_bytes()).map_err(SetError::Io)?;
        }
        inner.values = values;
        inner.current = next.clone();
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
