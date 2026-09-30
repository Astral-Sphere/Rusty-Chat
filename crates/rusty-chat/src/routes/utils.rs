//! `POST /api/v1/utils/highlight` — server-side code highlighting
//! (DECISIONS D-007/D-013): tree-sitter + vendored zed queries produce span
//! lists the frontend backfills into finished code blocks.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use rc_highlight::Span;
use serde_json::{Value, json};

use crate::extract::VerifiedUser;
use crate::state::AppState;

/// Request: `{ "code": "…", "language": "rust" }`.
/// Response: `{ "language", "unsupported", "spans": [[start, end, "name"], …] }`
/// — spans as compact triplets to keep the payload small. Byte offsets.
pub async fn highlight(
    State(_app): State<AppState>,
    VerifiedUser(_user): VerifiedUser,
    Json(form): Json<Value>,
) -> Response {
    let code = form.get("code").and_then(Value::as_str).unwrap_or_default();
    let language = form
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let (unsupported, spans) = match rc_highlight::highlight(code, language) {
        Some(spans) => (false, spans),
        None => (true, Vec::<Span>::new()),
    };
    let spans: Vec<Value> = spans
        .iter()
        .map(|s| json!([s.start, s.end, s.name]))
        .collect();

    Json(json!({
        "language": language,
        "unsupported": unsupported,
        "spans": spans,
    }))
    .into_response()
}

/// `GET /api/v1/utils/highlight/languages` — diagnostics: which languages
/// compiled and which queries failed (all failures are asset/grammar drift).
pub async fn languages(
    State(_app): State<AppState>,
    VerifiedUser(_user): VerifiedUser,
) -> Response {
    Json(json!({
        "languages": rc_highlight::languages(),
        "errors": rc_highlight::language_errors()
            .into_iter()
            .map(|(lang, err)| json!({"language": lang, "error": err}))
            .collect::<Vec<_>>(),
    }))
    .into_response()
}
