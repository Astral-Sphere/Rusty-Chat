# ARCHITECTURE.md — 系统架构

> 配套阅读：决策记录 `docs/DECISIONS.md`；兼容性事实 `docs/COMPATIBILITY.md`；进度 `docs/PROGRESS.md`。

## 1. 总览

```
┌────────────────────────────────────────────────────────────┐
│                    rusty-chat（单二进制）                    │
│                                                            │
│  ┌──────────┐  ┌──────────────────────────────────────┐    │
│  │ Dioxus   │  │              axum                    │    │
│  │ WASM 前端 │──│  REST /api/v1/* · /ollama · /openai  │    │
│  │(rust-embed│  │  WS /ws（房间+RPC） · SSE 兜底         │    │
│  │ 内嵌)    │  └───────┬──────────────────────────────┘    │
│  └──────────┘          │                                    │
│         ┌──────────────┼───────────────┐                    │
│  ┌──────▼─────┐ ┌──────▼──────┐ ┌──────▼─────────┐          │
│  │   rc-db    │ │   rc-llm    │ │  rc-realtime   │          │
│  │ SeaORM     │ │ 流式状态机    │ │ WS hub/rooms   │          │
│  │ SQLite|PG  │ │ Ollama/OpenAI│ │ 事件语义对齐原版 │          │
│  └──────┬─────┘ └──────┬──────┘ └────────────────┘          │
│         │       ┌──────┴──────┐ ┌────────────────┐          │
│  ┌──────▼─────┐ │  rc-rag     │ │   rc-tools     │          │
│  │ rc-auth    │ │ embedding/  │ │ MCP 客户端       │          │
│  │ JWT/bcrypt │ │ hybrid/向量库 │ │ builtin tools  │          │
│  └────────────┘ └──────┬──────┘ │ rusty-tools SDK│          │
│                        │        └────────────────┘          │
│                 ┌──────▼──────┐ ┌────────────────┐          │
│                 │  rc-search  │ │   rc-media     │          │
│                 │ 搜索/SSRF loader│ STT/TTS/图像  │          │
│                 └─────────────┘ └────────────────┘          │
└────────────────────────────────────────────────────────────┘
```

依赖方向：`rusty-chat → {rc-*} → rc-core`；`rc-db/rc-auth` 被 rc-llm/rc-tools 等按需依赖；**rc-core 不依赖任何兄弟 crate**。

## 2. crate 边界与职责

| crate | 职责 | 明确不做 |
|---|---|---|
| rc-core | DTO、WS/SSE 事件协议类型、错误、`Secs/Nanos` 时间戳策略、兼容版本常量 | 不碰 IO |
| rc-db | SeaORM 实体/仓库、DDL bootstrap、alembic 守卫、config 表引擎、双方言特例（ILIKE 等） | 不懂 HTTP |
| rc-auth | JWT 签发校验、密码哈希、API key、Fernet 解密存量数据 | 不做 OAuth（M7 另加） |
| rc-llm | OpenAI/Ollama/Responses 三方言流式解析与归一化（OR-style output items）、payload 转换、多后端模型注册表、任务模型选择 | 不持有 DB 状态（由调用方传） |
| rc-rag | 抽取（PDF/DOCX/PPTX/XLSX/CSV/HTML）、切分、embedding（fastembed/ollama/openai）、hybrid BM25+向量、rerank、向量库 trait + sqlite-vec/qdrant/pgvector、重索引 CLI | 不做 Web 搜索 |
| rc-search | 搜索引擎适配（30+）、SSRF 安全 web loader。**按可独立发布 crate 标准设计** | 不依赖 rc-db |
| rc-media | STT（whisper-rs/openai/deepgram/azure）、TTS（openai/elevenlabs/azure+浏览器合成）、图像（openai/a1111/comfyui/gemini） | 不直接写 DB |
| rc-realtime | WS hub、房间（user:/channel:）、事件→客户端分发、request/response RPC、SSE 兜底、presence | 不实现业务逻辑 |
| rc-tools | MCP 客户端管理器（spawn stdio 子进程）、builtin tools（原版 `tools/builtin.py` 的 Rust 重写）、原生 FC 循环、审批暂停、delegate_task 子代理 | 不执行 Python |
| rusty-tools | `#[derive(Tool)]` SDK：struct → MCP stdio server（rmcp），JSON Schema 自动导出。独立发布 | — |
| rusty-chat | CLI（serve/migrate-check/reindex/create-admin）、axum 组装、路由、中间件（CORS/请求日志）、rust-embed 静态资源、启动编排 | 业务逻辑下沉到 rc-* |

## 3. 聊天管线（复刻 `utils/middleware.py` 语义）

`POST /api/chat/completions`（JSON 请求，返回 task envelope；**增量走 WS**）：

1. 鉴权 + 权限 → 2. 参数合并（全局默认 + 模型 params + 请求 params）→ 3. DB 消息重载（chat_message 为准，blob 回填）→ 4. 上下文压缩（`compact_token_threshold`）→ 5. output items 规范化（message/reasoning/function_call/function_call_output；推理按 provider 还原：ollama `thinking`、llama.cpp `reasoning_content`、`<think>` 标签）→ 6. 系统提示变量解析（`{{CURRENT_DATETIME}}`、`{{USER_NAME}}`、聊天变量 schema）→ 7. RAG 注入（rag_template、`<source>` 包裹、user 或 system 位置）→ 8. 工具解析（MCP servers + builtin + skills manifest；原生 FC 或 legacy 提示式）→ 9. 路由（arena 随机子模型 / pipe→函数 / ollama→payload 转换 / openai）→ 10. 流式状态机（SSE/ndjson/Responses 事件；推理标签拆分；delta 合并）→ 11. 工具调用循环（≤N 轮；ask_user/审批暂停→持久化排队）→ 12. 持久化（chat_message + blob 惰性同步）+ 事件分发 → 13. 后台任务（标题/标签/追问/记忆复习）→ 14. outlet 钩子。

## 4. 实时通道（原版 socket.io 语义 → 原生 WS）

- 端点 `GET /ws`（axum WebSocket upgrade），兼容子协议协商；连接后第一帧必须是 `auth {token}`（或 query token）。
- 房间：`user:{id}`（个人事件）、`channel:{id}`（频道）、后续 `note:{id}`（M7）。
- 服务端→客户端帧复用原版事件名与负载形状（见 COMPATIBILITY.md §6），前端语义与测试可对拍。
- RPC：请求帧带 `reply_to` id，浏览器执行（pyodide/直连模型/浏览器侧工具）后回帧；超时由服务端管理。
- SSE 兜底：`GET /api/chat/completions/stream/{task_id}` 只读重放增量（多实例部署用 Redis 时 M7 扩展）。

## 5. 前端（web/，Dioxus 0.7 CSR）

- 独立 workspace（wasm32-unknown-unknown），构建产物 `web/dist`（`just web-build`：dx build --release + 拷贝）。rusty-chat 以 feature `embed-frontend`（rust-embed）把 web/dist 内嵌为单二进制（`just web-release`，SPA index 回退、api/ollama/ws 前缀不回退）；dev/测试默认走 `FRONTEND_DIST_DIR`（ServeDir）。注意 fresh clone 先跑 `just web-css` 生成 gitignored 的 tailwind.css。
- 路由对齐原版页面：`/auth`、`/`（聊天）、`/c/[id]`、`/s/[id]`（分享）、`/workspace/{models,knowledge,prompts,tools,skills}`、`/admin/{users,settings,evaluations,functions}`、`/channels/[id]`、`/automations`、`/calendar`、`/playground/*`。
- 状态：Dioxus Signals（config/user/models/settings/chats/…），WS 单例。
- 渲染栈（全 Rust/WASM，已落地，见 DECISIONS D-004/D-005/D-006/D-007/D-013/D-014）：
  - 样式：Tailwind v4 standalone CLI（无 Node/JS）从 `web/input.css` 生成 `assets/tailwind.css`（`just web-css`；markdown 排版为手写 `.markdown-body` 块）；
  - markdown：comrak（GFM+hardbreaks+math_dollars 运行时选项）→ `$..$`/`$$..$$` 经 katex-rs（KaTeX 0.18.5 兼容输出，css+字体 data URI 内嵌）→ ammonia 单点消毒 → `dangerous_inner_html`；流式期间纯文本，done 后 keyed memo 渲染（`web/src/render.rs`）；
  - 代码高亮：**服务端** `rc-highlight`（tree-sitter + vendored zed highlights.scm，12 语言）经 `POST /api/v1/utils/highlight` 出字节偏移 span，前端 `web/src/highlight.rs` 回填 `hl-*` span（`web/src/mermaid.rs` 先行处理 mermaid）；
  - Mermaid：sebastian（纯 Rust，直接 wasm 目标可用）把 ```mermaid fence 渲染成 SVG，失败保留源码 + 错误提示；
  - 消息树：`web/src/branches.rs` 纯函数（active_path/siblings/leaf_descendant），编辑重发生成 sibling 分支，‹ › 切换经部分 currentId 更新；
  - Artifacts：iframe 沙箱（sandbox 属性 + CSP）。（M3+）
- JS interop 仅限：pyodide（M6 代码解释器）、浏览器语音 API（MediaRecorder/Web Speech）。

## 6. 配置系统

- `config` 表逐键（`key TEXT PK, value JSON`）；启动时 seed DEFAULT_CONFIG 缺失键；运行时 **DB 值优先于 env**（env 仅首次 seed）。
- rc-db 提供 `ConfigEngine`（get/get_many/upsert + 类型化访问器），rusty-chat 的 `AppState` 持有。

## 7. 交付与部署

- Docker 多阶段（前端 wasm 构建 → 单二进制）；`DATA_DIR` 布局与原版一致（`webui.db`、`uploads/`、`cache/`）。
- 启动序：load env → install_drivers → connect → `inspect_database`（Fresh→bootstrap / Compatible→继续 / 其他→拒绝并给出指引）→ seed config → 加载模型注册表 → 起调度器（M4）→ 挂路由 + 内嵌前端。

## 8. 测试基建

见 `docs/TESTING.md`。fixture：`docs/fixtures/webui-head.db`（真实 head 库）、`*-head-schema.sql`；PG 容器 `just pg-up`；`RC_TEST_PG_URL` 门控 PG 测试。
