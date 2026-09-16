//! Ollama native `/api/chat` adapter (ndjson streaming). Maps open-webui's
//! OpenAI→Ollama payload conversion (`utils/payload.py`) and response
//! normalization (`utils/response.py`) into the unified [`StreamDelta`]
//! vocabulary.

use crate::registry::now_epoch;
use futures::Stream;
use futures::StreamExt;
use rc_core::chat::{ChatCompletionForm, StreamDelta};
use rc_core::{Error, Result};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Streaming chat against one Ollama backend (`base_url`, no `/v1`).
pub async fn stream_chat(base_url: &str, form: &ChatCompletionForm) -> Result<OllamaDeltaStream> {
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));
    let payload = build_payload(form)?;
    let http = reqwest::Client::new();
    let response = http
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| Error::Internal(format!("ollama unreachable ({url}): {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Internal(format!(
            "ollama returned {status} for {url}: {}",
            truncate(&body, 300)
        )));
    }
    Ok(OllamaDeltaStream {
        inner: Box::pin(response.bytes_stream()),
        buffer: Vec::new(),
        finished: false,
        last_finish: None,
    })
}

/// open-webui `convert_payload_openai_to_ollama`: messages become native
/// (system/user/assistant with string content), known sampling params nest
/// under `options`.
pub fn build_payload(form: &ChatCompletionForm) -> Result<Value> {
    let messages: Vec<Value> = form
        .messages
        .iter()
        .map(|m| {
            let content = rc_core::chat::content_text(&m.content);
            let mut obj = json!({"role": m.role, "content": content});
            if let Some(reasoning) = &m.reasoning_content {
                obj["thinking"] = json!(reasoning);
            }
            obj
        })
        .collect();

    let mut options: Value = json!({});
    let mut sources: Vec<Value> = vec![Value::Object(form.extra.clone().into_iter().collect())];
    if let Some(params @ Value::Object(_)) = &form.params {
        sources.push(params.clone());
    }
    for source in &sources {
        if let Some(map) = source.as_object() {
            for (key, value) in map {
                let ollama_key = match key.as_str() {
                    "max_tokens" | "max_completion_tokens" => Some("num_predict"),
                    "frequency_penalty" | "presence_penalty" | "temperature" | "top_p" | "seed"
                    | "stop" => Some(key.as_str()),
                    _ => None,
                };
                if let Some(okey) = ollama_key {
                    options[okey] = value.clone();
                }
            }
        }
    }

    Ok(json!({
        "model": form.model,
        "messages": messages,
        "stream": true,
        "options": options,
    }))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

/// Line-delimited JSON stream adapter.
pub struct OllamaDeltaStream {
    inner: Pin<Box<dyn Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    buffer: Vec<u8>,
    finished: bool,
    last_finish: Option<String>,
}

impl OllamaDeltaStream {
    fn parse_line(&mut self, line: &[u8]) -> Option<Result<StreamDelta>> {
        let Ok(line) = std::str::from_utf8(line) else {
            return None;
        };
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return None;
        };

        let done = v.get("done").and_then(Value::as_bool).unwrap_or(false);
        if done {
            let prompt = v
                .get("prompt_eval_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let eval = v.get("eval_count").and_then(Value::as_u64).unwrap_or(0);
            return Some(Ok(StreamDelta::Usage {
                prompt_tokens: prompt,
                completion_tokens: eval,
                total_tokens: Some(prompt + eval),
            }));
        }
        let message = v.get("message")?;
        if let Some(thinking) = message.get("thinking").and_then(Value::as_str)
            && !thinking.is_empty()
        {
            return Some(Ok(StreamDelta::Reasoning {
                content: thinking.to_string(),
            }));
        }
        if let Some(content) = message.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            return Some(Ok(StreamDelta::Content {
                content: content.to_string(),
            }));
        }
        // keep-alive / empty chunks are skipped
        None
    }
}

impl Stream for OllamaDeltaStream {
    type Item = Result<StreamDelta>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // drain complete lines from the buffer first
            if let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=pos).collect();
                if let Some(delta) = self.parse_line(&line) {
                    if let Ok(StreamDelta::Usage { .. }) = &delta {
                        self.last_finish = Some("stop".to_string());
                    }
                    return Poll::Ready(Some(delta));
                }
                continue;
            }
            if self.finished {
                return Poll::Ready(None);
            }
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => self.buffer.extend_from_slice(&bytes),
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(Error::Internal(format!(
                        "ollama stream error: {e}"
                    )))));
                }
                Poll::Ready(None) => {
                    self.finished = true;
                    return Poll::Ready(Some(Ok(StreamDelta::Done {
                        finish_reason: self.last_finish.clone().or(Some("stop".to_string())),
                    })));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Non-streaming helper (open-webui `stream=false` callers): collect and
/// shape an OpenAI-style chat.completion response.
pub async fn complete(base_url: &str, form: &ChatCompletionForm) -> Result<Value> {
    let mut deltas = stream_chat(base_url, form).await?;
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

#[allow(unused)]
fn _unused_type_map(_: BTreeMap<String, Value>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use serde_json::json;

    // 覆盖矩阵：
    // ✅ payload 构建：消息字符串化、thinking 字段、options 嵌套
    //    （max_tokens→num_predict、stop/temperature 透传）
    // ✅ ndjson 流：content/thinking 分片、done 行 usage、空行容错、
    //    跨 TCP 分块的行重组
    // ✅ 非流式 complete 聚合
    // ⛔ 刻意不覆盖：ollama 不可达（reqwest 语义，已在 ollama.rs 测）

    #[test]
    fn payload_maps_options_and_messages() {
        let form: ChatCompletionForm = serde_json::from_value(json!({
            "model": "qwen3:8b",
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "ok", "reasoning_content": "hmm"}
            ],
            "max_tokens": 100,
            "temperature": 0.3,
            "vendor_key": 1
        }))
        .unwrap();
        let payload = build_payload(&form).unwrap();
        assert_eq!(payload["model"], json!("qwen3:8b"));
        assert_eq!(payload["messages"][2]["thinking"], json!("hmm"));
        assert_eq!(payload["options"]["num_predict"], json!(100));
        assert_eq!(payload["options"]["temperature"], json!(0.3));
        assert!(
            payload["vendor_key"].is_null(),
            "vendor extras stay out of ollama payload"
        );
    }

    #[tokio::test]
    async fn streams_ndjson_lines_across_chunk_boundaries() {
        // lines will be split mid-JSON by sending the body in two parts
        let ndjson = concat!(
            r#"{"model":"m","message":{"role":"assistant","content":"He"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":"llo"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","thinking":"why"},"done":false}"#,
            "\n",
            r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":4,"eval_count":6}"#,
            "\n"
        );
        let (part1, part2) = ndjson.split_at(ndjson.len() / 2);
        let app = axum::Router::new().route(
            "/api/chat",
            axum::routing::post(|| {
                let p1 = part1.to_string();
                let p2 = part2.to_string();
                async move {
                    let stream = futures::stream::iter(vec![
                        Ok::<_, std::io::Error>(bytes::Bytes::from(p1)),
                        Ok::<_, std::io::Error>(bytes::Bytes::from(p2)),
                    ]);
                    axum::http::Response::builder()
                        .header("content-type", "application/x-ndjson")
                        .body(axum::body::Body::from_stream(stream))
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
        let mut stream = stream_chat(&format!("http://{addr}"), &form).await.unwrap();

        let mut acc = rc_core::chat::OutputAccumulator::default();
        while let Some(delta) = stream.next().await {
            acc.push(&delta.unwrap());
        }
        assert_eq!(acc.content, "Hello");
        assert_eq!(acc.reasoning, "why");
        assert_eq!(acc.usage, Some((4, 6, Some(10))));
    }

    #[tokio::test]
    async fn complete_shapes_openai_response() {
        let ndjson = r#"{"message":{"role":"assistant","content":"done"},"done":false}
{"message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":1,"eval_count":2}
"#;
        let app = axum::Router::new().route(
            "/api/chat",
            axum::routing::post(|| {
                let ndjson = ndjson.to_string();
                async move {
                    axum::http::Response::builder()
                        .header("content-type", "application/x-ndjson")
                        .body(axum::body::Body::from(ndjson))
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
        let response = complete(&format!("http://{addr}"), &form).await.unwrap();
        assert_eq!(response["choices"][0]["message"]["content"], json!("done"));
        assert_eq!(response["usage"]["total_tokens"], json!(3));
    }
}
