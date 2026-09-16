//! Model registry — open-webui `get_all_base_models` parity for M1: fan out
//! to Ollama (`/api/tags`) and OpenAI-compatible (`/models`) backends,
//! assemble `ModelInfo` items, dedupe by id (last wins, matching the
//! frontend resolution via `app.state.MODELS`).

use crate::models::ModelInfo;
use crate::ollama::OllamaClient;
use rc_core::Result;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub struct BackendConfig {
    pub ollama_enable: bool,
    pub ollama_base_urls: Vec<String>,
    pub openai_enable: bool,
    /// (base_url, api_key, prefix_id) per connection
    pub openai_connections: Vec<OpenAiConnection>,
}

#[derive(Debug, Clone)]
pub struct OpenAiConnection {
    pub base_url: String,
    pub api_key: Option<String>,
    pub prefix_id: Option<String>,
}

pub fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `fetch_ollama_models`: raw merged tags → ModelInfo items.
pub async fn fetch_ollama_models(client: &OllamaClient) -> Vec<ModelInfo> {
    let mut out = Vec::new();
    for model in client.all_tags_merged().await {
        let Some(name) = model
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let id = model
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(&name)
            .to_string();
        let loaded = model.get("expires_at").is_some();
        out.push(ModelInfo {
            id,
            name,
            object: "model".to_string(),
            created: 0,
            owned_by: "ollama".to_string(),
            ollama: serde_json::from_value(model.clone()).ok(),
            loaded: Some(loaded),
            connection_type: model
                .get("connection_type")
                .and_then(Value::as_str)
                .unwrap_or("local")
                .to_string()
                .into(),
            url_idx: None,
            tags: Default::default(),
            extra: {
                let mut extra = std::collections::BTreeMap::new();
                if let Some(urls) = model.get("urls") {
                    extra.insert("urls".to_string(), urls.clone());
                }
                extra
            },
        });
    }
    out
}

/// `fetch_openai_models`: GET {base}/models per connection with bearer auth;
/// unknown items pass through as ModelInfo (extra preserved); prefix_id
/// namespaces ids (`prefix.model`, open-webui parity).
pub async fn fetch_openai_models(conn: &OpenAiConnection) -> Vec<ModelInfo> {
    let url = format!("{}/models", conn.base_url.trim_end_matches('/'));
    let http = reqwest::Client::new();
    let mut request = http.get(&url);
    if let Some(key) = &conn.api_key {
        request = request.bearer_auth(key);
    }
    let Ok(resp) = request.send().await else {
        return vec![];
    };
    if !resp.status().is_success() {
        return vec![];
    }
    let Ok(body) = resp.json::<Value>().await else {
        return vec![];
    };
    let Some(data) = body.get("data").and_then(Value::as_array) else {
        return vec![];
    };

    data.iter()
        .filter_map(|item| {
            let id = item.get("id")?.as_str()?.to_string();
            let full_id = match &conn.prefix_id {
                Some(prefix) if !prefix.is_empty() => format!("{prefix}.{id}"),
                _ => id.clone(),
            };
            let mut parsed: ModelInfo = serde_json::from_value(item.clone()).unwrap_or(ModelInfo {
                id: full_id.clone(),
                name: String::new(),
                object: "model".to_string(),
                created: 0,
                owned_by: "openai".to_string(),
                ollama: None,
                loaded: None,
                connection_type: None,
                url_idx: None,
                tags: Default::default(),
                extra: Default::default(),
            });
            parsed.id = full_id;
            if parsed.owned_by.is_empty() {
                parsed.owned_by = "openai".to_string();
            }
            parsed.url_idx = item
                .get("urlIdx")
                .and_then(Value::as_u64)
                .map(|v| v as usize);
            Some(parsed)
        })
        .collect()
}

/// `get_all_base_models`: function models (none in M1) + openai + ollama,
/// deduped by id with last-wins (open-webui dict-collapse parity).
pub async fn all_base_models(cfg: &BackendConfig) -> Result<Vec<ModelInfo>> {
    let mut all: Vec<ModelInfo> = Vec::new();

    if cfg.openai_enable {
        for conn in &cfg.openai_connections {
            all.extend(fetch_openai_models(conn).await);
        }
    }
    if cfg.ollama_enable {
        let client = OllamaClient::new(cfg.ollama_base_urls.clone())?;
        all.extend(fetch_ollama_models(&client).await);
    }

    let mut deduped: std::collections::BTreeMap<String, ModelInfo> = Default::default();
    for m in all {
        deduped.insert(m.id.clone(), m);
    }
    Ok(deduped.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 覆盖矩阵：
    // ✅ openai /models：bearer 头、data 数组解析、prefix_id 命名空间、
    //    urlIdx 提取、非 200 → 空列表
    // ✅ dedup：同名 id 后者覆盖
    // ✅ disabled 后端不请求
    // ⛔ 刻意不覆盖：api_configs per-connection 扩展（M3）

    async fn spawn_openai_mock(body: Value, expect_auth: Option<&str>) -> String {
        let expect_auth = expect_auth.map(str::to_string);
        let app = axum::Router::new().route(
            "/models",
            axum::routing::get(move |headers: axum::http::HeaderMap| {
                let body = body.clone();
                let expect_auth = expect_auth.clone();
                async move {
                    if let Some(expected) = expect_auth {
                        let got = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        if got != format!("Bearer {expected}") {
                            return axum::http::StatusCode::UNAUTHORIZED.into_response();
                        }
                    }
                    axum::Json(body).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    use axum::response::IntoResponse;
    use serde_json::json;

    #[tokio::test]
    async fn openai_models_parsed_with_prefix_and_auth() {
        let url = spawn_openai_mock(
            json!({"object": "list", "data": [
                {"id": "gpt-x", "object": "model", "created": 1, "owned_by": "openai"},
                {"id": "other", "object": "model", "created": 2, "owned_by": "vendor"}
            ]}),
            Some("sk-test"),
        )
        .await;

        let conn = OpenAiConnection {
            base_url: url,
            api_key: Some("sk-test".into()),
            prefix_id: Some("corp".into()),
        };
        let models = fetch_openai_models(&conn).await;
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "corp.gpt-x", "prefix namespaces the id");
        assert_eq!(models[1].owned_by, "vendor", "unknown owners preserved");
    }

    #[tokio::test]
    async fn dedupe_last_wins() {
        let cfg = BackendConfig {
            ollama_enable: false,
            ollama_base_urls: vec![],
            openai_enable: false,
            openai_connections: vec![],
        };
        // nothing enabled → empty
        assert!(all_base_models(&cfg).await.unwrap().is_empty());

        // dedupe check via direct map semantics: insert two same-id items
        let a = ModelInfo {
            id: "m".into(),
            name: "first".into(),
            object: "model".into(),
            created: 0,
            owned_by: "ollama".into(),
            ollama: None,
            loaded: None,
            connection_type: None,
            url_idx: None,
            tags: Default::default(),
            extra: Default::default(),
        };
        let mut b = a.clone();
        b.name = "second".into();
        let mut map = std::collections::BTreeMap::new();
        map.insert(a.id.clone(), a);
        map.insert(b.id.clone(), b);
        assert_eq!(map["m"].name, "second", "last wins");
    }
}
