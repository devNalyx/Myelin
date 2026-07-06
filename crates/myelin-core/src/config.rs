use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Ingestion sources, the capture-worthiness filter, and embeddings policy
/// land here once the extraction pipeline design settles — see README.md.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub promotion: PromotionConfig,
}

/// Tunable knobs for the warmup-queue -> skill promotion logic. Defaults
/// are guesses (see README's open risks), not measured values — this
/// exists so retuning them doesn't require a rebuild.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionConfig {
    /// Reps a candidate needs (with no high-stakes signal) before it
    /// auto-promotes.
    #[serde(default = "default_reps")]
    pub reps: i64,
    /// Token-overlap (Jaccard) threshold for "this observation is the same
    /// procedure as that candidate". 0.0-1.0.
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f64,
}

fn default_reps() -> i64 {
    3
}

fn default_similarity_threshold() -> f64 {
    0.4
}

impl Default for PromotionConfig {
    fn default() -> Self {
        Self {
            reps: default_reps(),
            similarity_threshold: default_similarity_threshold(),
        }
    }
}

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
