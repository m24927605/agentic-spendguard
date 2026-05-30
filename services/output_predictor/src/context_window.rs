//! Phase B skeleton — model_context_window.toml loader.
//!
//! Spec ref output-predictor-service-spec-v1alpha1.md §3.2.
//!
//! The full TOML file is populated in Phase C; this module compiles the
//! loader + lookup so the server can boot. Missing TOML returns an empty
//! table — lookups return None → caller falls back to the configured
//! `unknown_model_context_window` default (spec §3.3).

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Deserialize)]
struct TomlFile {
    #[serde(default)]
    entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    model: String,
    context_window: i64,
}

/// Lookup table for `model → context_window`. Cheap to clone via Arc;
/// Arc-wrapped at construction in main.rs.
pub struct ContextWindowTable {
    table: HashMap<String, i64>,
}

impl ContextWindowTable {
    /// Load from a TOML file path. Missing file is non-fatal — the
    /// service still boots with an empty table (lookups return None and
    /// callers fall back to the configured unknown_model default).
    pub fn load_from_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let text = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "model_context_window.toml missing; falling back to empty table — \
                     unknown_model_context_window default applies for every model"
                );
                return Self {
                    table: HashMap::new(),
                };
            }
        };
        let parsed: TomlFile = match toml::from_str(&text) {
            Ok(f) => f,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "model_context_window.toml parse error; falling back to empty table"
                );
                return Self {
                    table: HashMap::new(),
                };
            }
        };
        let table = parsed
            .entries
            .into_iter()
            .map(|e| (e.model, e.context_window))
            .collect();
        Self { table }
    }

    /// Empty table — for tests and fallback boots.
    pub fn empty() -> Self {
        Self {
            table: HashMap::new(),
        }
    }

    /// Look up the context window for a model. None when not in table;
    /// caller falls back to the configured unknown_model default.
    pub fn lookup(&self, model: &str) -> Option<i64> {
        self.table.get(model).copied()
    }

    /// Test helper — number of entries loaded.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.table.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn empty_table_returns_none() {
        let table = ContextWindowTable::empty();
        assert_eq!(table.lookup("gpt-4o"), None);
    }

    #[test]
    fn load_from_valid_toml() {
        let mut f = NamedTempFile::new().expect("tempfile");
        writeln!(
            f,
            r#"
[[entries]]
model = "gpt-4o"
context_window = 128000

[[entries]]
model = "claude-3-5-sonnet-20240620"
context_window = 200000
"#
        )
        .expect("write");
        let table = ContextWindowTable::load_from_path(f.path());
        assert_eq!(table.lookup("gpt-4o"), Some(128_000));
        assert_eq!(table.lookup("claude-3-5-sonnet-20240620"), Some(200_000));
        assert_eq!(table.lookup("unknown-model"), None);
    }

    #[test]
    fn missing_file_falls_back_to_empty() {
        let table = ContextWindowTable::load_from_path("/tmp/does-not-exist-nope-nope.toml");
        assert_eq!(table.lookup("gpt-4o"), None);
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn malformed_toml_falls_back_to_empty() {
        let mut f = NamedTempFile::new().expect("tempfile");
        writeln!(f, "this is not toml [[[").expect("write");
        let table = ContextWindowTable::load_from_path(f.path());
        assert_eq!(table.lookup("gpt-4o"), None);
        assert_eq!(table.len(), 0);
    }
}
