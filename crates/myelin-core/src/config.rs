use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Deliberately empty for now. Ingestion sources, the capture-worthiness
/// filter, promotion thresholds, and embeddings policy land here once the
/// extraction pipeline design settles — see README.md for the current
/// design sketch.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {}

impl Config {
    /// Missing config file is not an error — defaults apply.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Config::default());
        }

        let raw = std::fs::read_to_string(path).map_err(|source| Error::ConfigRead {
            path: path.to_path_buf(),
            source,
        })?;

        Ok(toml::from_str(&raw)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::ConfigRead {
                path: path.to_path_buf(),
                source,
            })?;
        }
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(path, raw).map_err(|source| Error::ConfigRead {
            path: path.to_path_buf(),
            source,
        })
    }
}
