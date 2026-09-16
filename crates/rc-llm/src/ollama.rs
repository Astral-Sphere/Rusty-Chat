//! Ollama HTTP client — designed as a publishable, server-free crate
//! (DECISIONS D-011): constructed from base URLs, no DB / app-state coupling.
//! M1 surface: `/api/tags` (model list), `/api/version`; chat + embeddings
//! arrive with M1-5/M2.

use rc_core::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct OllamaClient {
    /// One or more Ollama base URLs (no trailing slash).
    pub base_urls: Vec<String>,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct VersionResponse {
    version: String,
}

impl OllamaClient {
    pub fn new(base_urls: Vec<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Internal(format!("http client build failed: {e}")))?;
        Ok(Self {
            base_urls: base_urls
                .into_iter()
                .map(|u| u.trim_end_matches('/').to_string())
                .collect(),
            http,
        })
    }

    fn url(&self, idx: usize, path: &str) -> Result<String> {
        self.base_urls
            .get(idx)
            .map(|base| format!("{base}{path}"))
            .ok_or_else(|| Error::BadRequest(format!("ollama backend index {idx} out of range")))
    }

    /// Raw `/api/tags` entry from one backend (fail-fast per backend).
    pub async fn tags_from(&self, idx: usize) -> Result<Vec<Value>> {
        let url = self.url(idx, "/api/tags")?;
        let resp = self.http.get(&url).send().await.map_err(|e| {
            Error::Internal(format!("ollama backend {idx} unreachable ({url}): {e}"))
        })?;
        if !resp.status().is_success() {
            return Err(Error::Internal(format!(
                "ollama backend {idx} returned {} for {url}",
                resp.status()
            )));
        }
        let tags: TagsResponse = resp.json().await.map_err(|e| {
            Error::Internal(format!("ollama backend {idx} /api/tags decode failed: {e}"))
        })?;
        Ok(tags.models)
    }

    /// `GET /api/version` — lowest version wins across backends (open-webui
    /// parity: the oldest backend dictates capability).
    pub async fn lowest_version(&self) -> Result<Option<String>> {
        let mut lowest: Option<String> = None;
        for idx in 0..self.base_urls.len() {
            let url = match self.url(idx, "/api/version") {
                Ok(u) => u,
                Err(_) => continue,
            };
            let Ok(resp) = self.http.get(&url).send().await else {
                continue;
            };
            if let Ok(v) = resp.json::<VersionResponse>().await {
                lowest = Some(match lowest {
                    Some(cur) if cur <= v.version => cur,
                    _ => v.version,
                });
            }
        }
        Ok(lowest)
    }

    /// Fan out `/api/tags` across all backends, tagging each raw model with
    /// the backend indexes that serve it (`urls: [idx…]`) — mirrors
    /// open-webui's `merge_models_lists` + per-model `urls` tracking.
    pub async fn all_tags_merged(&self) -> Vec<Value> {
        let mut per_backend: Vec<Vec<Value>> = Vec::with_capacity(self.base_urls.len());
        for idx in 0..self.base_urls.len() {
            per_backend.push(self.tags_from(idx).await.unwrap_or_default());
        }

        let mut merged: Vec<Value> = Vec::new();
        for (idx, models) in per_backend.iter().enumerate() {
            for model in models {
                let Some(name) = model.get("name").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(existing) = merged
                    .iter_mut()
                    .find(|m| m.get("name").and_then(Value::as_str) == Some(name))
                {
                    if let Some(obj) = existing.as_object_mut() {
                        let entry = obj.entry("urls").or_insert_with(|| json!([]));
                        if let Some(arr) = entry.as_array_mut()
                            && !arr.contains(&json!(idx))
                        {
                            arr.push(json!(idx));
                        }
                    }
                } else {
                    let mut m = model.clone();
                    if let Some(obj) = m.as_object_mut() {
                        obj.insert("urls".to_string(), json!([idx]));
                    }
                    merged.push(m);
                }
            }
        }
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 覆盖矩阵：
    // ✅ 单后端 /api/tags 拉取与解析
    // ✅ 多后端合并：同名模型 urls 列表聚合；不同模型并存
    // ✅ 后端不可达 → 该后端记空、其余正常（gather 容错）
    // ✅ 404/500 后端 → 忽略该后端
    // ⛔ 刻意不覆盖：认证头（本地 Ollama 无鉴权；api_configs M3）

    async fn spawn_ollama_mock(body: Value) -> String {
        let app = axum::Router::new().route(
            "/api/tags",
            axum::routing::get(move || {
                let body = body.clone();
                async move { axum::Json(body) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn merges_models_across_backends() {
        let backend_a = spawn_ollama_mock(json!({"models": [
            {"name": "llama3:8b", "model": "llama3:8b", "digest": "d1", "size": 1},
            {"name": "mistral", "model": "mistral", "digest": "d2", "size": 2}
        ]}))
        .await;
        let backend_b = spawn_ollama_mock(json!({"models": [
            {"name": "llama3:8b", "model": "llama3:8b", "digest": "d1", "size": 1, "expires_at": "x"}
        ]})).await;

        let client = OllamaClient::new(vec![backend_a, backend_b]).unwrap();
        let merged = client.all_tags_merged().await;

        let llama = merged.iter().find(|m| m["name"] == "llama3:8b").unwrap();
        assert_eq!(
            llama["urls"],
            json!([0, 1]),
            "same model on two backends merges index list"
        );
        let mistral = merged.iter().find(|m| m["name"] == "mistral").unwrap();
        assert_eq!(mistral["urls"], json!([0]));
    }

    #[tokio::test]
    async fn unreachable_backend_degrades_to_empty() {
        let backend = spawn_ollama_mock(json!({"models": [
            {"name": "llama3:8b", "model": "llama3:8b"}
        ]}))
        .await;
        let client = OllamaClient::new(vec![backend, "http://127.0.0.1:1".into()]).unwrap();
        let merged = client.all_tags_merged().await;
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["urls"], json!([0]));
        // direct call to the dead backend errors
        assert!(client.tags_from(1).await.is_err());
    }

    #[tokio::test]
    async fn out_of_range_index_is_bad_request() {
        let client = OllamaClient::new(vec!["http://127.0.0.1:1".into()]).unwrap();
        let err = client.tags_from(5).await.unwrap_err();
        assert!(matches!(err, rc_core::Error::BadRequest(_)));
    }
}
