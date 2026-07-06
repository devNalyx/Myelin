use serde::Deserialize;
use std::time::Duration;

/// A minimal OpenAI-compatible `/v1/embeddings` client — the same shape
/// NexusContext's real, working embeddings client uses (Ollama, LM
/// Studio, vLLM, llama.cpp server all speak this). Constructing one is
/// the caller's job, gated on `myelin_core::Config::embeddings_policy()`
/// being `Allowed` — this type has no opinion on policy, just the HTTP call.
pub struct EmbeddingsClient {
    endpoint: String,
    model: String,
    api_key: Option<String>,
    agent: ureq::Agent,
}

#[derive(Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl EmbeddingsClient {
    pub fn new(
        endpoint: String,
        model: String,
        api_key: Option<String>,
        timeout_secs: u64,
    ) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(timeout_secs))
            .build();
        Self {
            endpoint,
            model,
            api_key,
            agent,
        }
    }

    /// Embeds `text`, returning the first (and only requested) vector.
    /// Callers should treat any error as "fall back to token-overlap" -
    /// this is an enhancement, never load-bearing for the daemon to work.
    pub fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let url = format!("{}/embeddings", self.endpoint.trim_end_matches('/'));
        let mut request = self.agent.post(&url);
        if let Some(key) = self.api_key.as_deref().filter(|k| !k.is_empty()) {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }

        let body = serde_json::json!({ "model": self.model, "input": text });
        let response: EmbeddingsResponse = request.send_json(body)?.into_json()?;
        response
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| anyhow::anyhow!("embeddings response had no data"))
    }
}

/// Cosine similarity, 0.0 for mismatched/empty/zero-norm vectors rather
/// than NaN or a panic — callers compare this against the same
/// 0.0-1.0 `similarity_threshold` used for Jaccard, so it needs to fail
/// closed (never match) rather than propagate an error into a hot path.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors_have_similarity_one() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors_have_similarity_zero() {
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
    }

    #[test]
    fn mismatched_lengths_and_empty_vectors_are_zero_not_a_panic() {
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }
}
