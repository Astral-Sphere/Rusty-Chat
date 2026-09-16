//! `/api/models` + `/ollama/*` reverse proxy.

use axum::Json;
use axum::extract::{RawQuery, State};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use rc_llm::registry::{BackendConfig, OpenAiConnection, all_base_models};
use serde_json::{Value, json};

use crate::state::AppState;

/// Reads backend connection settings from the config engine.
pub(crate) async fn backend_config(app: &AppState) -> BackendConfig {
    let get = |key: &'static str| async move { app.config.get(key).await.ok().flatten() };
    let urls = |key: &'static str| async move {
        get(key)
            .await
            .and_then(|v| v.as_array().cloned())
            .map(|a| {
                a.into_iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };

    let openai_enable = get("openai.enable")
        .await
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let base_urls: Vec<String> = urls("openai.api_base_urls").await;
    let keys: Vec<String> = urls("openai.api_keys").await;
    let configs = get("openai.api_configs")
        .await
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let openai_connections = base_urls
        .iter()
        .enumerate()
        .map(|(i, url)| {
            let cfg = configs.get(i).cloned().unwrap_or(json!({}));
            let per_conn_key = cfg.get("key").and_then(Value::as_str).map(str::to_string);
            OpenAiConnection {
                base_url: url.clone(),
                // per-connection key wins, else the positional key list (OWU parity)
                api_key: per_conn_key
                    .or_else(|| keys.get(i).cloned())
                    .filter(|k| !k.is_empty()),
                prefix_id: cfg
                    .get("prefix_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }
        })
        .collect();

    let ollama_enable = get("ollama.enable")
        .await
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let ollama_base_urls: Vec<String> = urls("ollama.base_urls").await;
    BackendConfig {
        ollama_enable,
        ollama_base_urls,
        openai_enable,
        openai_connections,
    }
}

/// `GET /api/models` — merged base-model registry (`{data: [...]}`).
pub async fn get_models(State(app): State<AppState>, mut parts: Parts) -> Response {
    // VerifiedUser semantics without the extractor (config route needs both
    // state and parts; replicate the pending-role check).
    let user = match crate::extract::optional_user(&mut parts, &app).await {
        Some(u) => u,
        None => return (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
    };
    if user.role.as_deref() == Some("pending") {
        return (axum::http::StatusCode::UNAUTHORIZED, "pending").into_response();
    }

    let cfg = backend_config(&app).await;
    match all_base_models(&cfg).await {
        Ok(models) => Json(json!({ "data": models })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "detail": e.to_string() })),
        )
            .into_response(),
    }
}

/// `/ollama/{*path}` reverse proxy (M1: forwards to the FIRST configured
/// backend; per-model host routing arrives with api_configs in M3).
/// Streams request and response bodies both ways.
pub async fn ollama_proxy(
    State(app): State<AppState>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    RawQuery(query): RawQuery,
    body: axum::body::Bytes,
) -> Response {
    let _ = query;
    let enabled = app
        .config
        .get("ollama.enable")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let base_urls: Vec<String> = app
        .config
        .get("ollama.base_urls")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .map(|a| {
            a.into_iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if !enabled || base_urls.is_empty() {
        return (axum::http::StatusCode::NOT_FOUND, "ollama not configured").into_response();
    }

    let mut target = format!("{}{}", base_urls[0].trim_end_matches('/'), uri.path());
    if let Some(q) = query {
        target.push('?');
        target.push_str(&q);
    }

    let http = reqwest::Client::new();
    let mut request = http.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        &target,
    );
    request = request.header("content-type", "application/json");
    if !body.is_empty() {
        request = request.body(body.to_vec());
    }
    match request.send().await {
        Ok(upstream) => {
            let status = upstream.status();
            let headers = upstream.headers().clone();
            let stream = upstream.bytes_stream();
            let mut response = Response::builder().status(
                axum::http::StatusCode::from_u16(status.as_u16())
                    .unwrap_or(axum::http::StatusCode::BAD_GATEWAY),
            );
            for (name, value) in headers.iter() {
                if matches!(name.as_str(), "content-type" | "cache-control") {
                    response = response.header(name, value);
                }
            }
            response
                .body(axum::body::Body::from_stream(stream))
                .unwrap_or_else(|_| {
                    (axum::http::StatusCode::BAD_GATEWAY, "stream error").into_response()
                })
        }
        Err(e) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(json!({ "detail": format!("ollama unreachable: {e}") })),
        )
            .into_response(),
    }
}
