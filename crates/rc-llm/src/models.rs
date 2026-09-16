//! Model DTOs — shapes returned by `/api/models` (open-webui
//! `utils/models.py::fetch_ollama_models` / `fetch_openai_models`).
//! Free-form extra fields are preserved via `flatten` since the frontend
//! reads model-specific payloads (e.g. the raw ollama tag entry).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One entry of the merged `/api/models` data array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "object", default = "default_object")]
    pub object: String,
    #[serde(default)]
    pub created: i64,
    /// `ollama` | `openai`
    pub owned_by: String,
    /// raw ollama tag entry (models list item), verbatim
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ollama: Option<OllamaTagModel>,
    /// set when the backend reports a loaded model (`expires_at` present)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loaded: Option<bool>,
    /// `local` | `external`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<String>,
    /// index of the originating OpenAI-compatible connection (wire name `urlIdx`)
    #[serde(rename = "urlIdx", skip_serializing_if = "Option::is_none")]
    pub url_idx: Option<usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, serde_json::Value>,
    /// preserved unknown fields (forward compatibility)
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_object() -> String {
    "model".to_string()
}

/// Raw Ollama `/api/tags` list item (subset preserved verbatim through
/// `extra`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaTagModel {
    pub name: String,
    /// full namespaced id (`hf.co/...` etc.) — `model` field
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // 覆盖矩阵：
    // ✅ ollama 条目序列化含 object=model、owned_by、raw ollama 载荷
    // ✅ openai 条目（urlIdx，无 ollama 字段时省略）
    // ✅ 未知字段经 flatten 保留（前端私有字段不丢）
    // ✅ 反序列化容忍缺省字段
    // ⛔ 刻意不覆盖：workspace model 覆盖（M3）

    #[test]
    fn ollama_item_roundtrip() {
        let item = ModelInfo {
            id: "llama3:8b".into(),
            name: "llama3:8b".into(),
            object: default_object(),
            created: 0,
            owned_by: "ollama".into(),
            ollama: Some(OllamaTagModel {
                name: "llama3:8b".into(),
                model: "llama3:8b".into(),
                expires_at: Some("2026-09-16T00:00:00Z".into()),
                connection_type: None,
                extra: Default::default(),
            }),
            loaded: Some(true),
            connection_type: Some("local".into()),
            url_idx: None,
            tags: Default::default(),
            extra: Default::default(),
        };
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["object"], "model");
        assert_eq!(v["owned_by"], "ollama");
        assert_eq!(v["ollama"]["name"], "llama3:8b");
        assert_eq!(v["loaded"], true);
        let back: ModelInfo = serde_json::from_value(v).unwrap();
        assert_eq!(back.id, "llama3:8b");
    }

    #[test]
    fn openai_item_preserves_unknown_fields() {
        let raw = json!({
            "id": "gpt-x", "name": "gpt-x", "object": "model", "created": 123,
            "owned_by": "openai", "urlIdx": 1,
            "custom_vendor_field": {"a": 1}
        });
        let parsed: ModelInfo = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.url_idx, Some(1));
        assert_eq!(parsed.extra["custom_vendor_field"]["a"], 1);
        let out = serde_json::to_value(&parsed).unwrap();
        assert_eq!(out["custom_vendor_field"]["a"], 1);
    }

    use serde_json::json;
}
