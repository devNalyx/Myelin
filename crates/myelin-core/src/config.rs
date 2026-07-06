use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::Path;

/// Ingestion sources and the capture-worthiness filter land here once the
/// extraction pipeline design settles — see README.md.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub promotion: PromotionConfig,
    #[serde(default)]
    pub atrophy: AtrophyConfig,
    #[serde(default)]
    pub embeddings: EmbeddingsConfig,
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

/// A skill nobody's invoked (or used since) in this long is flagged
/// `stale` by `list_skills` — informational only, nothing acts on it yet.
/// The default (30 days) is a guess, same caveat as `PromotionConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtrophyConfig {
    #[serde(default = "default_stale_after_secs")]
    pub stale_after_secs: i64,
}

fn default_stale_after_secs() -> i64 {
    30 * 24 * 3600
}

impl Default for AtrophyConfig {
    fn default() -> Self {
        Self {
            stale_after_secs: default_stale_after_secs(),
        }
    }
}

/// Optional upgrade over token-overlap similarity matching. Off by
/// default; ported near-verbatim from NexusContext's identical
/// `EmbeddingsConfig`/policy, since the "optional, policy-gated
/// OpenAI-compatible endpoint" concern is exactly the same regardless of
/// what's being embedded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    #[serde(default)]
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub allow_remote: bool,
}

fn default_timeout_secs() -> u64 {
    30
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            model: None,
            api_key: None,
            timeout_secs: default_timeout_secs(),
            allow_remote: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingsPolicy {
    /// Endpoint or model (or both) aren't filled in — nothing to turn on.
    NotConfigured,
    /// Endpoint and model are filled in, but the feature switch is off.
    Disabled,
    Allowed,
    /// Configured and enabled, but points off-box and `allow_remote` isn't set.
    RemoteBlocked,
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

    pub fn embeddings_policy(&self) -> EmbeddingsPolicy {
        let (Some(endpoint), Some(model)) = (&self.embeddings.endpoint, &self.embeddings.model)
        else {
            return EmbeddingsPolicy::NotConfigured;
        };
        if endpoint.trim().is_empty() || model.trim().is_empty() {
            return EmbeddingsPolicy::NotConfigured;
        }
        if !self.embeddings.enabled {
            return EmbeddingsPolicy::Disabled;
        }
        if self.embeddings.allow_remote || is_loopback_or_private(endpoint) {
            EmbeddingsPolicy::Allowed
        } else {
            EmbeddingsPolicy::RemoteBlocked
        }
    }
}

fn extract_host(endpoint: &str) -> Option<&str> {
    let without_scheme = endpoint.split("://").nth(1).unwrap_or(endpoint);
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host = host_port.split(':').next().unwrap_or(host_port);
    (!host.is_empty()).then_some(host)
}

fn is_loopback_or_private(endpoint: &str) -> bool {
    let Some(host) = extract_host(endpoint) else {
        return false;
    };
    if host == "localhost" {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback() || v4.is_private(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_and_private_hosts_are_recognized() {
        assert!(is_loopback_or_private("http://localhost:11434/v1"));
        assert!(is_loopback_or_private("http://127.0.0.1:11434/v1"));
        assert!(is_loopback_or_private("http://192.168.1.50:11434/v1"));
        assert!(is_loopback_or_private("http://10.0.0.5:11434/v1"));
    }

    #[test]
    fn public_hosts_are_not_loopback_or_private() {
        assert!(!is_loopback_or_private("https://api.example.com/v1"));
        assert!(!is_loopback_or_private("http://8.8.8.8/v1"));
    }

    fn config_with_embeddings(
        endpoint: Option<&str>,
        model: Option<&str>,
        enabled: bool,
        allow_remote: bool,
    ) -> Config {
        Config {
            embeddings: EmbeddingsConfig {
                enabled,
                endpoint: endpoint.map(str::to_string),
                model: model.map(str::to_string),
                api_key: None,
                timeout_secs: default_timeout_secs(),
                allow_remote,
            },
            ..Config::default()
        }
    }

    #[test]
    fn policy_is_not_configured_without_endpoint_or_model() {
        assert_eq!(
            config_with_embeddings(None, None, true, false).embeddings_policy(),
            EmbeddingsPolicy::NotConfigured
        );
        assert_eq!(
            config_with_embeddings(Some("http://localhost:11434/v1"), None, true, false)
                .embeddings_policy(),
            EmbeddingsPolicy::NotConfigured
        );
    }

    #[test]
    fn policy_is_disabled_when_configured_but_not_enabled() {
        assert_eq!(
            config_with_embeddings(
                Some("http://localhost:11434/v1"),
                Some("nomic-embed-text"),
                false,
                false
            )
            .embeddings_policy(),
            EmbeddingsPolicy::Disabled
        );
    }

    #[test]
    fn policy_is_allowed_for_enabled_loopback_endpoint() {
        assert_eq!(
            config_with_embeddings(
                Some("http://localhost:11434/v1"),
                Some("nomic-embed-text"),
                true,
                false
            )
            .embeddings_policy(),
            EmbeddingsPolicy::Allowed
        );
    }

    #[test]
    fn policy_is_remote_blocked_without_allow_remote() {
        assert_eq!(
            config_with_embeddings(
                Some("http://100.120.200.220:11434/v1"),
                Some("nomic-embed-text"),
                true,
                false
            )
            .embeddings_policy(),
            EmbeddingsPolicy::RemoteBlocked
        );
        assert_eq!(
            config_with_embeddings(
                Some("http://100.120.200.220:11434/v1"),
                Some("nomic-embed-text"),
                true,
                true
            )
            .embeddings_policy(),
            EmbeddingsPolicy::Allowed
        );
    }
}
