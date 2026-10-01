//! OpenAI-compatible chat adapter — wraps `openai-interface` (DECISIONS
//! D-011) and maps its typed chunks into the unified [`StreamDelta`]
//! vocabulary. Multi-backend via per-request base_url (ollama's `/v1`,
//! llama.cpp, OpenRouter, …).

use crate::registry::now_epoch;
use futures::{Stream, StreamExt};
use openai_interface::OapiError;
use openai_interface::chat::create::request::{
    Message as OaiMessage, MessageContent, RequestBody, StreamOptions,
};
use openai_interface::chat::create::response::streaming::ChatCompletionChunk;
use openai_interface::rest::post::PostStream;
use openai_interface::rest::{RequestOptions, default_client};
use rc_core::chat::{ChatCompletionForm, ChatMessage, StreamDelta};
use rc_core::{Error, Result};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::pin::Pin;
use std::task::{Context, Poll};

/// One OpenAI-compatible backend target.
#[derive(Debug, Clone)]
pub struct ChatTarget {
    pub base_url: String,
    pub api_key: Option<String>,
    /// extra headers (OpenRouter HTTP-Referer/X-Title, Azure api-key, …)
    pub extra_headers: BTreeMap<String, String>,
}

/// Scalar params we forward as typed RequestBody fields; anything else in
/// `params`/`extra` goes into `extra_body_map` (openai-interface flatten).
const TYPED_PARAM_KEYS: &[&str] = &[
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "seed",
    "stop",
    "presence_penalty",
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "top_logprobs",
    "n",
];

fn content_to_message_content(content: &Value) -> MessageContent {
    match content {
        Value::String(s) => MessageContent::Text(s.clone()),
        Value::Array(parts) => {
            // M1: join text parts; multimodal parts arrive in M2.
            let text = rc_core::chat::content_text(content);
            if parts.iter().any(|p| p.get("image_url").is_some()) {
                tracing::debug!("dropping non-text content parts (multimodal lands in M2)");
            }
            let _ = parts;
            MessageContent::Text(text)
        }
        _ => MessageContent::Text(String::new()),
    }
}

fn to_oai_message(msg: &ChatMessage) -> OaiMessage {
    match msg.role.as_str() {
        "system" => OaiMessage::System(openai_interface::chat::create::request::SystemMessage {
            content: content_to_message_content(&msg.content),
            name: msg.name.clone(),
        }),
        "assistant" => {
            OaiMessage::Assistant(openai_interface::chat::create::request::AssistantMessage {
                content: Some(rc_core::chat::content_text(&msg.content)),
                // Tool-call replay arrives with the M6 tool loop.
                tool_calls: None,
                reasoning_content: msg.reasoning_content.clone(),
                ..Default::default()
            })
        }
        "tool" => OaiMessage::Tool(openai_interface::chat::create::request::ToolMessage {
            content: MessageContent::Text(rc_core::chat::content_text(&msg.content)),
            tool_call_id: msg.tool_call_id.clone().unwrap_or_default(),
        }),
        // user + anything unknown → user
        _ => OaiMessage::User(openai_interface::chat::create::request::UserMessage {
            content: content_to_message_content(&msg.content),
            name: msg.name.clone(),
        }),
    }
}

/// Builds the typed openai-interface request from our form.
pub fn build_request_body(form: &ChatCompletionForm) -> Result<RequestBody> {
    let mut body = RequestBody {
        messages: form.messages.iter().map(to_oai_message).collect(),
        model: form.model.clone(),
        stream: Some(true),
        stream_options: Some(StreamOptions {
            include_usage: true,
        }),
        ..Default::default()
    };

    // provider params: typed keys from form.extra first, then form.params
    let mut extra_body: serde_json::Map<String, Value> = Default::default();
    let mut params_sources: Vec<Value> =
        vec![Value::Object(form.extra.clone().into_iter().collect())];
    if let Some(params @ Value::Object(_)) = &form.params {
        params_sources.push(params.clone());
    }
    for source in &params_sources {
        if let Some(map) = source.as_object() {
            for (key, value) in map {
                if key == "params" || key == "stop" && value.is_null() {
                    continue;
                }
                if TYPED_PARAM_KEYS.contains(&key.as_str()) {
                    match key.as_str() {
                        "temperature" => body.temperature = value.as_f64().map(|v| v as f32),
                        "top_p" => body.top_p = value.as_f64().map(|v| v as f32),
                        "max_tokens" => body.max_tokens = value.as_u64().map(|v| v as u32),
                        "max_completion_tokens" => {
                            body.max_completion_tokens = value.as_u64().map(|v| v as u32)
                        }
                        "seed" => body.seed = value.as_i64(),
                        "presence_penalty" => {
                            body.presence_penalty = value.as_f64().map(|v| v as f32)
                        }
                        "frequency_penalty" => {
                            body.frequency_penalty = value.as_f64().map(|v| v as f32)
                        }
                        "n" => body.n = value.as_u64().map(|v| v as u32),
                        "logprobs" => body.logprobs = value.as_bool(),
                        "stop" => {
                            extra_body.insert("stop".to_string(), value.clone());
                        }
                        "logit_bias" => {
                            body.logit_bias = serde_json::from_value(value.clone()).ok()
                        }
                        "top_logprobs" => body.top_logprobs = value.as_u64().map(|v| v as u32),
                        _ => {}
                    }
                } else {
                    extra_body.insert(key.clone(), value.clone());
                }
            }
        }
    }
    body.extra_body_map = Some(extra_body);
    Ok(body)
}

fn options_for(target: &ChatTarget) -> RequestOptions {
    let mut options = match &target.api_key {
        Some(key) => RequestOptions::bearer(key.clone()),
        None => RequestOptions::new(),
    };
    for (name, value) in &target.extra_headers {
        if let (Ok(name), Ok(value)) = (axum_extra_name(name), value.parse()) {
            options.extra_headers.insert(name, value);
        }
    }
    options
}

// openai-interface re-exports reqwest; we construct a header name via the
// same reqwest version (dependency unification guarantees identical types).
fn axum_extra_name(name: &str) -> Result<reqwest::header::HeaderName> {
    reqwest::header::HeaderName::from_bytes(name.as_bytes())
        .map_err(|e| Error::BadRequest(format!("invalid header name {name}: {e}")))
}

/// Adapter stream: openai-interface SSE chunks → unified deltas. A `Done`
/// delta is synthesized at stream end from the last observed finish_reason.
pub struct OpenAiDeltaStream {
    inner: Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, OapiError>> + Send>>,
    last_finish_reason: Option<String>,
    finished: bool,
    pending: std::collections::VecDeque<StreamDelta>,
}

impl Stream for OpenAiDeltaStream {
    type Item = Result<StreamDelta>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }
        loop {
            match self.inner.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    let mut deltas = Vec::new();
                    for choice in &chunk.choices {
                        if let Some(reason) = &choice.finish_reason {
                            self.last_finish_reason = Some(reason.as_str().to_string());
                        }
                        let delta = &choice.delta;
                        if let Some(content) = &delta.content
                            && !content.is_empty()
                        {
                            deltas.push(StreamDelta::Content {
                                content: content.clone(),
                            });
                        }
                        // openai-interface 0.12: the field is unconditional under the `reasoning`
                        if let Some(reasoning) = &delta.reasoning_content
                            && !reasoning.is_empty()
                        {
                            deltas.push(StreamDelta::Reasoning {
                                content: reasoning.clone(),
                            });
                        }
                        if let Some(tool_calls) = &delta.tool_calls {
                            for call in tool_calls {
                                deltas.push(StreamDelta::ToolCall {
                                    index: call.index as usize,
                                    id: call.id.clone(),
                                    name: call.function.as_ref().and_then(|f| f.name.clone()),
                                    arguments_delta: call
                                        .function
                                        .as_ref()
                                        .and_then(|f| f.arguments.clone())
                                        .unwrap_or_default(),
                                });
                            }
                        }
                    }
                    if let Some(usage) = &chunk.usage {
                        deltas.push(StreamDelta::Usage {
                            prompt_tokens: usage.prompt_tokens,
                            completion_tokens: usage.completion_tokens,
                            total_tokens: Some(usage.total_tokens),
                        });
                    }
                    if !deltas.is_empty() {
                        // hand deltas out one at a time, buffering the rest
                        for delta in deltas.into_iter().rev() {
                            self.pending.push_back(delta);
                        }
                    }
                    if let Some(next) = self.pending.pop_front() {
                        return Poll::Ready(Some(Ok(next)));
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(Error::Internal(format!(
                        "provider stream error: {e}"
                    )))));
                }
                Poll::Ready(None) => {
                    self.finished = true;
                    return Poll::Ready(Some(Ok(StreamDelta::Done {
                        finish_reason: self.last_finish_reason.clone(),
                    })));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Streaming chat against one OpenAI-compatible backend.
pub async fn stream_chat(
    target: ChatTarget,
    form: &ChatCompletionForm,
) -> Result<OpenAiDeltaStream> {
    let body = build_request_body(form)?;
    let options = options_for(&target);
    let client = default_client();
    let stream = body
        .get_stream_response(&client, &target.base_url, &options)
        .await
        .map_err(|e| Error::Internal(format!("provider request failed: {e}")))?;
    Ok(OpenAiDeltaStream {
        inner: Box::pin(stream),
        last_finish_reason: None,
        finished: false,
        pending: Default::default(),
    })
}

/// Non-streaming chat (open-webui `stream=false` callers): collect the
/// stream and shape a standard OpenAI chat.completion response.
pub async fn complete(target: ChatTarget, form: &ChatCompletionForm) -> Result<Value> {
    let mut deltas = stream_chat(target, form).await?;
    let mut acc = rc_core::chat::OutputAccumulator::default();
    while let Some(delta) = deltas.next().await {
        acc.push(&delta?);
    }
    Ok(json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
        "object": "chat.completion",
        "created": now_epoch(),
        "model": form.model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": acc.content,
            },
            "finish_reason": acc.finish_reason.unwrap_or_else(|| "stop".to_string()),
        }],
        "usage": acc.usage.map(|(p, c, t)| json!({
            "prompt_tokens": p, "completion_tokens": c,
            "total_tokens": t.unwrap_or(p + c),
        })).unwrap_or(json!(null)),
    }))
}

// silence unused warnings for types referenced only behind features

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use serde_json::json;

    // 覆盖矩阵：
    // ✅ build_request_body：消息角色映射（system/user/assistant/tool）、
    //    typed 参数（temperature/seed…）、未知参数进 extra_body、params 合并
    // ✅ 流式适配：content/reasoning/tool_calls 分片、末尾 usage chunk
    //    （choices: []）、Done{finish_reason} 合成、SSE 被任意字节边界
    //    切碎后仍正确解析（跨 TCP 分块——openai-interface 升级最敏感点）、
    //    乱序/交错 tool index 聚合、空 delta 跳过
    // ✅ 非流式 complete：聚合为 OpenAI chat.completion 形状
    // ✅ 错误路径：上游非 200 → Err(provider request failed)；bearer 与
    //    extra_headers 实际发出（mock 捕获断言）

    #[test]
    fn builds_request_body_with_roles_and_params() {
        let form: ChatCompletionForm = serde_json::from_value(json!({
            "model": "qwen3:8b",
            "messages": [
                {"role": "system", "content": "be nice"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
                {"role": "tool", "content": "21°", "tool_call_id": "call_1"}
            ],
            "temperature": 0.5,
            "some_vendor_thing": {"a": 1},
            "params": {"seed": 42, "stop": ["END"]}
        }))
        .unwrap();

        let body = build_request_body(&form).unwrap();
        assert_eq!(body.model, "qwen3:8b");
        assert_eq!(body.messages.len(), 4);
        assert_eq!(body.temperature, Some(0.5));
        assert_eq!(body.seed, Some(42));
        let extra = body.extra_body_map.unwrap();
        assert_eq!(extra["some_vendor_thing"]["a"], json!(1));
        assert_eq!(extra["stop"], json!(["END"]));
    }

    #[tokio::test]
    async fn streams_and_maps_chunks() {
        // SSE wire: two content chunks, one reasoning, one tool_call split
        // across three fragments, empty-choices usage chunk, [DONE].
        let sse_lines = [
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"reasoning_content":"thin"},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c9","function":{"name":"f","arguments":"{\"x\""}}]},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":1}"}}]},"finish_reason":"tool_calls"}]}"#,
            // vLLM-style usage-only chunk: `"choices": null` (0.12 tolerance fix)
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":null,"usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}}"#,
            "data: [DONE]",
        ];
        let body = format!("{}\n\n", sse_lines.join("\n\n"));
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(move || {
                let body = body.clone();
                async move {
                    axum::http::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from(body))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let form: ChatCompletionForm = serde_json::from_value(json!({
            "model": "m", "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap();
        let target = ChatTarget {
            base_url: format!("http://{addr}"),
            api_key: None,
            extra_headers: Default::default(),
        };

        let mut stream = stream_chat(target, &form).await.unwrap();
        let mut acc = rc_core::chat::OutputAccumulator::default();
        let mut deltas = Vec::new();
        while let Some(delta) = stream.next().await {
            let delta = delta.unwrap();
            acc.push(&delta);
            deltas.push(delta);
        }
        assert_eq!(acc.content, "Hello");
        assert_eq!(acc.reasoning, "thin");
        assert_eq!(acc.tool_calls[0].arguments, "{\"x\":1}");
        assert_eq!(acc.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(acc.usage, Some((3, 5, Some(8))));
        assert!(deltas.iter().any(
            |d| matches!(d, StreamDelta::Done { finish_reason: Some(fr) } if fr == "tool_calls")
        ));

        // full pipeline: accumulator → output items
        let items = acc.into_output_items();
        let kinds: Vec<&str> = items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, vec!["reasoning", "message", "function_call"]);
    }

    #[tokio::test]
    async fn complete_returns_openai_shape() {
        let sse = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(|| {
                let sse = sse.to_string();
                async move {
                    axum::http::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from(sse))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let form: ChatCompletionForm = serde_json::from_value(json!({
            "model": "m", "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap();
        let target = ChatTarget {
            base_url: format!("http://{addr}"),
            api_key: None,
            extra_headers: Default::default(),
        };
        let response = complete(target, &form).await.unwrap();
        assert_eq!(response["choices"][0]["message"]["content"], json!("ok"));
        assert_eq!(response["choices"][0]["finish_reason"], json!("stop"));
        assert!(response["id"].as_str().unwrap().starts_with("chatcmpl-"));
    }

    fn form() -> ChatCompletionForm {
        serde_json::from_value(json!({
            "model": "m", "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap()
    }

    /// Drives `stream_chat` against `base_url` and returns (deltas, accumulator).
    async fn collect(base_url: String) -> (Vec<StreamDelta>, rc_core::chat::OutputAccumulator) {
        let target = ChatTarget {
            base_url,
            api_key: None,
            extra_headers: Default::default(),
        };
        let mut stream = stream_chat(target, &form()).await.unwrap();
        let mut acc = rc_core::chat::OutputAccumulator::default();
        let mut deltas = Vec::new();
        while let Some(delta) = stream.next().await {
            let delta = delta.unwrap();
            acc.push(&delta);
            deltas.push(delta);
        }
        (deltas, acc)
    }

    #[tokio::test]
    async fn streams_sse_split_across_tcp_chunks() {
        // the same wire format as streams_and_maps_chunks, but the body is
        // sliced every 7 bytes — data: lines break mid-JSON and mid-"data:".
        // Regression fence for openai-interface upgrades (D-011).
        let sse_lines = [
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"reasoning_content":"why"},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":"stop"}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":null,"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
            "data: [DONE]",
        ];
        let body = format!("{}\n\n", sse_lines.join("\n\n"));
        let body_bytes = bytes::Bytes::from_owner(body);
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(move || {
                let body_bytes = body_bytes.clone();
                async move {
                    let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = body_bytes
                        .chunks(7)
                        .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
                        .collect();
                    axum::http::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from_stream(futures::stream::iter(chunks)))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let (deltas, acc) = collect(format!("http://{addr}")).await;
        assert_eq!(acc.content, "Hello");
        assert_eq!(acc.reasoning, "why");
        assert_eq!(acc.finish_reason.as_deref(), Some("stop"));
        assert_eq!(acc.usage, Some((1, 2, Some(3))));
        assert!(
            deltas.iter().any(
                |d| matches!(d, StreamDelta::Done { finish_reason: Some(fr) } if fr == "stop")
            )
        );
    }

    #[tokio::test]
    async fn parallel_tool_calls_aggregate_out_of_order() {
        // index 1 starts and finishes before index 0 continues; the two
        // argument streams must not interleave
        let sse_lines = [
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"f1","function":{"name":"fn_b","arguments":"{\"b\""}}]},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"f0","function":{"name":"fn_a","arguments":"{\"a\""}}]},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":":1}"}}]},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":2}"}}]},"finish_reason":"tool_calls"}]}"#,
            "data: [DONE]",
        ];
        let body = format!("{}\n\n", sse_lines.join("\n\n"));
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(move || {
                let body = body.clone();
                async move {
                    axum::http::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from(body))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let (_, acc) = collect(format!("http://{addr}")).await;
        assert_eq!(acc.tool_calls.len(), 2);
        // accumulator preserves first-seen order: index 1 arrived first
        assert_eq!(acc.tool_calls[0].index, 1);
        assert_eq!(acc.tool_calls[0].arguments, "{\"b\":1}");
        assert_eq!(acc.tool_calls[1].index, 0);
        assert_eq!(acc.tool_calls[1].arguments, "{\"a\":2}");
        assert_eq!(acc.finish_reason.as_deref(), Some("tool_calls"));
    }

    #[tokio::test]
    async fn empty_delta_chunks_are_skipped() {
        let sse_lines = [
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":""},"finish_reason":null}]}"#,
            r#"data: {"id":"c1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "data: [DONE]",
        ];
        let body = format!("{}\n\n", sse_lines.join("\n\n"));
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(move || {
                let body = body.clone();
                async move {
                    axum::http::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from(body))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let (deltas, acc) = collect(format!("http://{addr}")).await;
        assert_eq!(acc.content, "ok");
        let contents: Vec<_> = deltas
            .iter()
            .filter(|d| matches!(d, StreamDelta::Content { .. }))
            .collect();
        assert_eq!(
            contents.len(),
            1,
            "empty deltas must be dropped: {deltas:?}"
        );
    }

    #[tokio::test]
    async fn upstream_500_maps_to_internal_error() {
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(|| async {
                axum::http::Response::builder()
                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .body(axum::body::Body::from("boom"))
                    .unwrap()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let target = ChatTarget {
            base_url: format!("http://{addr}"),
            api_key: None,
            extra_headers: Default::default(),
        };
        let err = match stream_chat(target, &form()).await {
            Err(e) => e,
            Ok(_) => panic!("expected the upstream 500 to fail the request"),
        };
        let msg = err.to_string();
        assert!(msg.contains("provider request failed"), "{msg}");
    }

    #[tokio::test]
    async fn bearer_and_extra_headers_are_sent() {
        // the mock captures the headers openai-interface actually puts on the
        // wire, then answers with a minimal SSE so stream_chat completes
        let captured: std::sync::Arc<tokio::sync::Mutex<Option<(String, String, String)>>> =
            Default::default();
        let captured_captured = captured.clone();
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(move |headers: axum::http::HeaderMap| {
                let captured = captured_captured.clone();
                async move {
                    let auth = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let title = headers
                        .get("x-title")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let referer = headers
                        .get("http-referer")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    *captured.lock().await = Some((auth, title, referer));
                    axum::http::Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(axum::body::Body::from("data: [DONE]\n\n"))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let target = ChatTarget {
            base_url: format!("http://{addr}"),
            api_key: Some("sk-x".into()),
            extra_headers: [
                ("X-Title".to_string(), "Rusty".to_string()),
                ("HTTP-Referer".to_string(), "https://rusty".to_string()),
            ]
            .into_iter()
            .collect(),
        };
        let mut stream = stream_chat(target, &form()).await.unwrap();
        while let Some(delta) = stream.next().await {
            delta.unwrap();
        }
        let (auth, title, referer) = captured.lock().await.clone().unwrap();
        assert_eq!(auth, "Bearer sk-x");
        assert_eq!(
            title, "Rusty",
            "extra headers reach the wire (name case-insensitive)"
        );
        assert_eq!(referer, "https://rusty");
    }
}
