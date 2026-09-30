//! Realtime event frames — open-webui socket.io `events` payload shapes,
//! transported over our native WebSocket as `{"event": "events", "data": …}`
//! (see docs/COMPATIBILITY.md §6).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One socket frame sent to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsFrame {
    pub event: String,
    pub data: Value,
}

impl WsFrame {
    /// Chat pipeline event: `{event: "events", data: {chat_id, message_id, data}}`.
    pub fn chat_event(chat_id: &str, message_id: Option<&str>, data: Value) -> Self {
        Self {
            event: "events".to_string(),
            data: serde_json::json!({
                "chat_id": chat_id,
                "message_id": message_id,
                "data": data,
            }),
        }
    }
}

/// Typed builders for the event payloads the frontend consumes
/// (open-webui `Chat.svelte::chatEventHandler` parity).
pub mod event_data {
    use serde_json::{Value, json};

    /// `chat:message:delta` — append text.
    pub fn message_delta(content: &str) -> Value {
        json!({"type": "chat:message:delta", "data": {"content": content}})
    }

    /// `chat:message` (replace) — final full content.
    pub fn message_replace(content: &str) -> Value {
        json!({"type": "chat:message", "data": {"content": content}})
    }

    /// `status` — one statusHistory entry.
    pub fn status(status: Value) -> Value {
        json!({"type": "status", "data": status})
    }

    /// `chat:message:error`.
    pub fn message_error(error: Value) -> Value {
        json!({"type": "chat:message:error", "data": {"error": error}})
    }

    /// `chat:message:files`.
    pub fn message_files(files: Value) -> Value {
        json!({"type": "chat:message:files", "data": {"files": files}})
    }

    /// `chat:active` — generation activity marker (true at start, false at end).
    pub fn chat_active(active: bool) -> Value {
        json!({"type": "chat:active", "data": {"active": active}})
    }

    /// `chat:title` — data IS the title string (frontend `chatTitle.set(data)`).
    pub fn chat_title(title: &str) -> Value {
        json!({"type": "chat:title", "data": title})
    }

    /// `chat:message` final message object replacement including done flag —
    /// emitted after persistence so the client reloads exact state.
    pub fn message_done(content: &str, output: Value, usage: Option<Value>) -> Value {
        json!({
            "type": "chat:message",
            "data": {
                "content": content,
                "done": true,
                "output": output,
                "usage": usage,
            }
        })
    }
}

/// `POST /api/chat/completions` response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionEnvelope {
    pub status: bool,
    pub task_ids: Vec<String>,
    pub chat_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 覆盖矩阵：
    // ✅ WsFrame 形状：{event: "events", data: {chat_id, message_id, data}}
    // ✅ message_id=None 序列化为显式 null（键存在）
    // ✅ 各 event_data 构造器的 type 字段与载荷键名（前端消费契约），
    //    含 message_files；chat:title 的 data 是字符串本身
    // ✅ envelope 序列化
    // ⛔ 刻意不覆盖：citation/embeds 等富类型（M2+）

    #[test]
    fn frame_shape_matches_open_webui() {
        let frame = WsFrame::chat_event("c1", Some("m1"), event_data::message_delta("hi"));
        let v = serde_json::to_value(&frame).unwrap();
        assert_eq!(v["event"], json!("events"));
        assert_eq!(v["data"]["chat_id"], json!("c1"));
        assert_eq!(v["data"]["message_id"], json!("m1"));
        assert_eq!(v["data"]["data"]["type"], json!("chat:message:delta"));
        assert_eq!(v["data"]["data"]["data"]["content"], json!("hi"));
    }

    #[test]
    fn event_payload_types() {
        let cases = [
            (event_data::message_replace("x"), "chat:message"),
            (event_data::status(json!({"done": true})), "status"),
            (
                event_data::message_error(json!({"detail": "e"})),
                "chat:message:error",
            ),
            (event_data::chat_active(false), "chat:active"),
            (event_data::chat_title("T"), "chat:title"),
            (
                event_data::message_done("x", json!([]), None),
                "chat:message",
            ),
        ];
        for (payload, expected_type) in cases {
            assert_eq!(payload["type"], json!(expected_type));
        }
        let done = event_data::message_done(
            "content",
            json!([{"type": "message"}]),
            Some(json!({"count": 3})),
        );
        assert_eq!(done["data"]["done"], json!(true));
        assert_eq!(done["data"]["usage"]["count"], json!(3));
    }

    #[test]
    fn message_files_payload_shape() {
        let payload = event_data::message_files(json!([{"id": "f1", "name": "a.txt"}]));
        assert_eq!(payload["type"], json!("chat:message:files"));
        assert_eq!(payload["data"]["files"][0]["id"], json!("f1"));
    }

    #[test]
    fn chat_event_without_message_id_serializes_null() {
        // frontend contract: chat-level events (title/active) carry an
        // explicit null message_id, not a missing key
        let frame = WsFrame::chat_event("c1", None, event_data::chat_title("T"));
        let v = serde_json::to_value(&frame).unwrap();
        assert_eq!(v["data"]["message_id"], json!(null));
        assert!(v["data"].get("message_id").is_some());
    }

    #[test]
    fn title_payload_is_the_string_itself() {
        // chat:title data is the raw title string (frontend chatTitle.set(data))
        let payload = event_data::chat_title("Greetings");
        assert_eq!(payload["data"], json!("Greetings"));
    }
}
