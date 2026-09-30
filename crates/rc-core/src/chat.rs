//! Chat pipeline wire types — OpenAI Chat Completions shapes used between
//! the frontend, `/api/chat/completions`, and backend providers, plus the
//! unified streaming-delta vocabulary both provider adapters map into.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// OpenAI chat message. `content` stays a raw Value because OpenAI allows a
/// string OR an array of content parts; M1 sends/receives strings, later
/// milestones add multimodal parts without a breaking change.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// assistant tool_calls (OpenAI shape)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Value>>,
    /// tool role result payload (OpenAI: content string + tool_call_id)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// provider-specific reasoning fields preserved verbatim
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Incoming `/api/chat/completions` body. open-webui accepts an arbitrary
/// dict and pops its own fields; we mirror that with typed knowns +
/// `extra` passthrough for provider params (temperature, top_p, tools…).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatCompletionForm {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// open-webui per-request params (temperature, top_p, stop, seed, …)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    /// open-webui orchestration metadata (all popped before provider call)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_tasks: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Unified provider stream delta — both the OpenAI SSE adapter and the
/// Ollama ndjson adapter map into this vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamDelta {
    Content {
        content: String,
    },
    Reasoning {
        content: String,
    },
    ToolCall {
        index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        arguments_delta: String,
    },
    Usage {
        prompt_tokens: u64,
        completion_tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_tokens: Option<u64>,
    },
    Done {
        finish_reason: Option<String>,
    },
}

/// OR-style output item — the internal persisted message shape
/// (open-webui "output" list: message/reasoning/function_call/function_call_output).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputItem {
    /// OR item type: `message` | `reasoning` | `function_call` |
    /// `function_call_output`
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// `message`: text content; `function_call_output`: result content
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    /// `reasoning`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Value>,
    /// `function_call`: {name, arguments, call_id}
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// open-webui marks reasoning items with role assistant
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Accumulates a provider stream into output items (the persisted assistant
/// `output` list) and the flat message content.
#[derive(Debug, Default, Clone)]
pub struct OutputAccumulator {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCallAccum>,
    pub finish_reason: Option<String>,
    pub usage: Option<(u64, u64, Option<u64>)>,
}

#[derive(Debug, Clone)]
pub struct ToolCallAccum {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl OutputAccumulator {
    pub fn push(&mut self, delta: &StreamDelta) {
        match delta {
            StreamDelta::Content { content } => self.content.push_str(content),
            StreamDelta::Reasoning { content } => self.reasoning.push_str(content),
            StreamDelta::ToolCall {
                index,
                id,
                name,
                arguments_delta,
            } => {
                if let Some(slot) = self.tool_calls.iter_mut().find(|t| t.index == *index) {
                    if let Some(id) = id {
                        slot.id = id.clone();
                    }
                    if let Some(name) = name {
                        slot.name = name.clone();
                    }
                    slot.arguments.push_str(arguments_delta);
                } else {
                    self.tool_calls.push(ToolCallAccum {
                        index: *index,
                        id: id.clone().unwrap_or_default(),
                        name: name.clone().unwrap_or_default(),
                        arguments: arguments_delta.clone(),
                    });
                }
            }
            StreamDelta::Usage {
                prompt_tokens,
                completion_tokens,
                total_tokens,
            } => {
                self.usage = Some((*prompt_tokens, *completion_tokens, *total_tokens));
            }
            StreamDelta::Done { finish_reason } => {
                if let Some(reason) = finish_reason {
                    self.finish_reason = Some(reason.clone());
                }
            }
        }
    }

    /// OR-aligned output items: reasoning (when present) + assistant message
    /// + function_call items (when present).
    pub fn into_output_items(self) -> Vec<OutputItem> {
        let mut items = Vec::new();
        if !self.reasoning.is_empty() {
            items.push(OutputItem {
                kind: "reasoning".to_string(),
                id: Some(format!("reasoning_{}", uuid4_short())),
                role: Some("assistant".to_string()),
                content: None,
                summary: Some(Value::String(self.reasoning)),
                name: None,
                arguments: None,
                call_id: None,
                status: None,
                extra: Default::default(),
            });
        }
        if !self.content.is_empty() || self.tool_calls.is_empty() {
            items.push(OutputItem {
                kind: "message".to_string(),
                id: Some(format!("msg_{}", uuid4_short())),
                role: Some("assistant".to_string()),
                content: Some(Value::String(self.content.clone())),
                summary: None,
                name: None,
                arguments: None,
                call_id: None,
                status: Some("completed".to_string()),
                extra: Default::default(),
            });
        }
        for call in &self.tool_calls {
            items.push(OutputItem {
                kind: "function_call".to_string(),
                id: Some(format!("fc_{}", uuid4_short())),
                role: None,
                content: None,
                summary: None,
                name: Some(call.name.clone()),
                arguments: Some(call.arguments.clone()),
                call_id: Some(call.id.clone()),
                status: Some("completed".to_string()),
                extra: Default::default(),
            });
        }
        items
    }

    /// OpenAI assistant message (role/content/tool_calls) — used to continue
    /// a tool-call conversation loop.
    pub fn assistant_message(&self) -> ChatMessage {
        let tool_calls = if self.tool_calls.is_empty() {
            None
        } else {
            Some(
                self.tool_calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": {"name": c.name, "arguments": c.arguments},
                            "index": c.index,
                        })
                    })
                    .collect(),
            )
        };
        ChatMessage {
            role: "assistant".to_string(),
            content: Value::String(self.content.clone()),
            name: None,
            tool_calls,
            tool_call_id: None,
            reasoning_content: if self.reasoning.is_empty() {
                None
            } else {
                Some(self.reasoning.clone())
            },
            extra: Default::default(),
        }
    }
}

fn uuid4_short() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

/// Extracts the flat text of a message content Value (string or parts array).
pub fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str).or_else(|| p.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

use std::collections::BTreeMap;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 覆盖矩阵：
    // ✅ Form 解析：已知字段 + extra passthrough（temperature 等参数不丢）
    // ✅ Accumulator：内容/reasoning 拼接、tool_calls 按分片聚合（含多
    //    index 交错）、usage/done 记录、Done{None} 不覆盖已有 finish_reason、
    //    id-only 工具分片、空累加器 → 单个空 message item
    // ✅ into_output_items：纯文本 → [message]；带 reasoning → [reasoning,
    //    message]；纯工具调用 → [function_call…]
    // ✅ assistant_message：tool_calls 组装回 OpenAI 形状 + reasoning_content
    // ✅ content_text：字符串/数组（typed parts + 裸字符串元素）/其他
    // ⛔ 刻意不覆盖：多模态 content parts（M2）

    #[test]
    fn form_keeps_provider_params() {
        let form: ChatCompletionForm = serde_json::from_value(json!({
            "model": "llama3",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true,
            "id": "assistant-msg-1",
            "parent_id": null,
            "chat_id": "c1",
            "temperature": 0.7,
            "top_p": 0.9,
            "params": {"stop": ["\n\n"]}
        }))
        .unwrap();
        assert_eq!(form.model, "llama3");
        assert!(form.stream.unwrap());
        assert_eq!(form.extra["temperature"], json!(0.7));
        assert_eq!(form.extra["top_p"], json!(0.9));
        assert_eq!(form.params.as_ref().unwrap()["stop"], json!(["\n\n"]));
        assert_eq!(form.id.as_deref(), Some("assistant-msg-1"));
        assert!(form.parent_id.is_none());
    }

    #[test]
    fn accumulator_joins_deltas() {
        let mut acc = OutputAccumulator::default();
        for delta in [
            StreamDelta::Content {
                content: "Hel".into(),
            },
            StreamDelta::Content {
                content: "lo".into(),
            },
            StreamDelta::Reasoning {
                content: "thin".into(),
            },
            StreamDelta::Reasoning {
                content: "king".into(),
            },
            StreamDelta::ToolCall {
                index: 0,
                id: Some("c1".into()),
                name: Some("get_weather".into()),
                arguments_delta: "{\"ci".into(),
            },
            StreamDelta::ToolCall {
                index: 0,
                id: None,
                name: None,
                arguments_delta: "ty\":\"SF\"}".into(),
            },
            StreamDelta::Usage {
                prompt_tokens: 5,
                completion_tokens: 7,
                total_tokens: Some(12),
            },
            StreamDelta::Done {
                finish_reason: Some("tool_calls".into()),
            },
        ] {
            acc.push(&delta);
        }
        assert_eq!(acc.content, "Hello");
        assert_eq!(acc.reasoning, "thinking");
        assert_eq!(acc.tool_calls[0].arguments, "{\"city\":\"SF\"}");
        assert_eq!(acc.finish_reason.as_deref(), Some("tool_calls"));

        let items = acc.clone().into_output_items();
        let kinds: Vec<&str> = items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, vec!["reasoning", "message", "function_call"]);

        let assistant = acc.assistant_message();
        // reasoning surfaces as reasoning_content for tool-loop continuation
        assert_eq!(
            assistant.reasoning_content.as_deref(),
            Some("thinking"),
            "{assistant:?}"
        );
        let calls = assistant.tool_calls.clone().unwrap();
        assert_eq!(calls[0]["function"]["name"], json!("get_weather"));
    }

    #[test]
    fn done_with_none_keeps_existing_finish_reason() {
        let mut acc = OutputAccumulator::default();
        acc.push(&StreamDelta::Done {
            finish_reason: Some("tool_calls".into()),
        });
        // vLLM/openai-interface may emit a trailing Done{None}; it must not
        // clobber the real stop reason
        acc.push(&StreamDelta::Done {
            finish_reason: None,
        });
        assert_eq!(acc.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn interleaved_tool_call_indices_merge_independently() {
        let mut acc = OutputAccumulator::default();
        for delta in [
            StreamDelta::ToolCall {
                index: 0,
                id: Some("call-0".into()),
                name: Some("fn_a".into()),
                arguments_delta: "{\"a\"".into(),
            },
            StreamDelta::ToolCall {
                index: 1,
                id: Some("call-1".into()),
                name: Some("fn_b".into()),
                arguments_delta: "{\"b\"".into(),
            },
            StreamDelta::ToolCall {
                index: 0,
                id: None,
                name: None,
                arguments_delta: ":1}".into(),
            },
            StreamDelta::ToolCall {
                index: 1,
                id: None,
                name: None,
                arguments_delta: ":2}".into(),
            },
        ] {
            acc.push(&delta);
        }
        assert_eq!(acc.tool_calls.len(), 2);
        assert_eq!(acc.tool_calls[0].arguments, "{\"a\":1}");
        assert_eq!(acc.tool_calls[1].arguments, "{\"b\":2}");
        assert_eq!(acc.tool_calls[1].id, "call-1");
    }

    #[test]
    fn empty_accumulator_produces_empty_message_item() {
        // a provider that streams nothing still yields one (empty) message
        let items = OutputAccumulator::default().into_output_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "message");
        assert_eq!(items[0].content, Some(json!("")));
    }

    #[test]
    fn tool_call_id_only_fragment_keeps_empty_name() {
        let mut acc = OutputAccumulator::default();
        acc.push(&StreamDelta::ToolCall {
            index: 0,
            id: Some("c1".into()),
            name: None,
            arguments_delta: String::new(),
        });
        assert_eq!(acc.tool_calls[0].id, "c1");
        assert_eq!(acc.tool_calls[0].name, "");
    }

    #[test]
    fn pure_tool_call_has_no_empty_message_item() {
        let mut acc = OutputAccumulator::default();
        acc.push(&StreamDelta::ToolCall {
            index: 0,
            id: Some("c".into()),
            name: Some("f".into()),
            arguments_delta: "{}".into(),
        });
        let items = acc.into_output_items();
        let kinds: Vec<&str> = items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, vec!["function_call"]);
    }

    #[test]
    fn content_text_variants() {
        assert_eq!(content_text(&json!("plain")), "plain");
        assert_eq!(
            content_text(&json!([{"type": "text", "text": "a"}, {"type": "text", "text": "b"}])),
            "ab"
        );
        // legacy providers put bare strings inside the parts array
        assert_eq!(
            content_text(&json!(["a", {"type": "text", "text": "b"}])),
            "ab"
        );
        assert_eq!(content_text(&json!(null)), "");
    }
}
