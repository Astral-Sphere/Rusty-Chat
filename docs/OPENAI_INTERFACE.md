# OPENAI_INTERFACE.md — openai-interface 接入调研与适配记录

> crate：`openai-interface`（用户自维护，Codeberg/Hammerklavier），**0.11.0-rc1 起 MIT**（此前 AGPL）。
> 当前使用：**v0.12.0**（2026-09-16 适配完成，84 测试全绿）。
> 结论：无接入硬阻断，作为 rc-llm 的 OpenAI 兼容层（DECISIONS D-011）。

## 0. v0.12.0 适配记录（2026-09-16）

初版调研提出的 10 项问题在 0.12.0 中**基本全部解决**，逐项核对：

| 提出的问题 | 0.12.0 状态 |
|---|---|
| choices 严格解析（vLLM null-choices usage chunk 失败） | ✅ `null_to_empty_vec` + `#[serde(default)]`（response.rs §streaming 顶部）——我们用 `"choices": null` 的 fixture 锁定该行为 |
| FinishReason/ResponseRole 未知变体失败 | ✅ `wire_string_enum!` 宏生成 `Unknown` 兜底 + `as_str()` |
| FinishReason 双类型（streaming vs chat） | ✅ 合一，streaming 模块 `pub use crate::chat::FinishReason` |
| 响应缺 Serialize | ✅ Chunk/Completion/delta 全部 `Serialize + Deserialize + Clone` |
| 请求缺 Deserialize | ✅ `RequestBody`、`Message` 枚举、消息结构体全部双向 |
| reasoning_content 被 deepseek 门控 | ✅ 改为 `reasoning` feature（**默认开启**），`deepseek` 仅保留为别名 |
| Message::Assistant 字段膨胀无 Default | ✅ 枚举重构为 `Message::System(SystemMessage)` 等结构体变体，全部 `Default`，另有 `Message::system/user/assistant/tool` 便捷构造器 |
| StopKeywords 仅 Serialize | ✅ untagged + Deserialize，typed `body.stop` 恢复使用 |
| usage 计数 usize | ✅ `prompt_tokens` 等 u64 |
| OapiError 深藏 errors:: | ✅ 根导出 `pub use errors::{ApiError, OapiError}` |
| 无 Client 薄封装 | ❌ 未加（可选项，不影响使用） |
| 0.11.0 final 未发布 | ✅ 直接跳到 0.12.0 |

适配改动（rc-llm）：依赖升 0.12.0；feature 透传改名 `deepseek`→`reasoning`；delta 循环去掉 cfg 门；Assistant/Tool 消息改结构体变体 + `..Default::default()`；finish_reason 用 `as_str()`（含 Unknown）；usage 去掉 as-cast；`stop` 回到 typed 字段。契约测试 fixture 换用 `"choices": null` 锁定宽容解析。

0.12.0 遗留小项（低优先）：`StreamOptions.include_usage` 仍是非 Option bool；base_url 必须带 `/v1` 前缀仍需调用方自知。

## 1. 设计形态

无 Client 结构体，**无状态设计**：请求类型（如 `chat::create::request::RequestBody`）通过 trait
`rest::post::{Post, PostNoStream, PostStream, PostBinary}` 获得方法，每个方法显式接收
`(client: &reqwest::Client, base_url: &str, options: &RequestOptions)`。
流式返回 `impl Stream<Item = Result<Chunk, OapiError>>`，`[DONE]` 自动处理；
`ChatCompletionAccumulator` 按 index 组装 tool_calls/content。

```rust
let options = RequestOptions::bearer("sk-...")            // 或 new() + with_header
    .with_header("HTTP-Referer", "https://my.app")?;     // OpenRouter 等自定义头
let mut stream = request
    .get_stream_response(&client, "http://localhost:11434/v1", &options)  // 任意 base_url
    .await?;
```

## 2. 覆盖面（对本项目的用途）

| 能力 | 状态 | 用途（里程碑） |
|---|---|---|
| /chat/completions 流式+非流式 | ✅（实测核心） | M1-5 聊天管线 |
| SSE chunk：content/role/refusal/finish_reason/usage(stream_options) | ✅ | M1-5 |
| delta.reasoning_content | ✅ 但 feature=`deepseek` 门控 | M1-5（开启该 feature） |
| delta.tool_calls + tools/tool_choice/parallel_tool_calls | ✅ | M1-5/M6 |
| /responses（Responses API，含全部流式事件枚举） | ✅ | M1-5/M6 |
| /embeddings | ✅ | M2 RAG |
| /completions（legacy） | ✅ | 旧兼容 |
| /audio/speech（字节流）/transcriptions/translations | ✅ | M5 |
| /images/{generations,edits,variations} | ✅（未测试） | M5 |
| GET /models（`object` 字段容忍缺失） | ✅ | M1-4 已用 |
| files/uploads、moderations、batches、fine-tuning、vector stores | ✅ 大多未测试 | 按需 |
| realtime WebSocket 传输；流式 transcriptions/images | ❌ 未实现 | 不需要 |

## 3. 关键约束

1. **TLS provider**：reqwest 0.13 `rustls-no-provider` —— 进程级 rustls CryptoProvider 必须在任何
   Client 构建前安装（`ferritls` feature + `rest::install_crypto_provider()`，或依赖并集提供）。
   我们自己的 reqwest 已带 TLS 后端，接入时验证 provider 安装顺序。
2. **serde 不对称**：请求类型仅 `Serialize`（可用 `extra_body_map` flatten 发任意厂商参数）；
   响应类型仅 `Deserialize` —— 无法原样再序列化转发，需映射到我们的内部类型（M1-5 本来就要做 OR-style 归一化）。
3. **chunk 校验严格**：`object` 枚举、非 default 的 `choices` —— 怪异后端（vLLM 末尾 usage chunk
   `"choices": null`）会解析失败；用 `rest::skip_deserialization_errors` 兜底，但会连 usage 一起丢。

## 4. 反馈给作者（= 用户）的需求清单

1. **对称 derive**：响应类型加 `Serialize`（ChatCompletion/Chunk/delta），请求加 `Deserialize`（日志/重放/代理）。
2. **宽容 chunk 解析**：`choices` 加 `#[serde(default)]`（null → 空），`object`/`FinishReason`/`ResponseRole`
   允许未知值降级，避免怪异后端整 chunk 失败。
3. **解除 reasoning_content 的 deepseek 门控**：已是跨厂商事实标准（OpenRouter/ollama/vLLM 推理模型都在发）。
4. 发布 0.11.0 final 到 crates.io（tag 已打，`"0.11"` 才能解析）。
5. （可选 DX）`Client { http, base_url }` 薄封装。

## 5. 本项目接入方式（M1-5）

- 依赖：`openai-interface = "0.11.0-rc1"`（pre-release caret，作者发布 final 后自动升级）。
- `rc-llm::openai_chat`：把我们的请求体（内部 OpenAI-shape JSON）映射进 `RequestBody`
  （或直接用 extra_body 透传未识别参数），流式 chunk 映射为内部 **OR-style output items**，
  reasoning_content → reasoning 项、tool_calls → function_call 项（ARCHITECTURE.md §3 第 5 步）。
- 多后端：每个连接一个 `(base_url, options)`；OpenRouter 需自定义头（`with_header`）。
