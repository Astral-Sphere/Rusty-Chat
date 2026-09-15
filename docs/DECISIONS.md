# DECISIONS.md — 技术决策记录（ADR）

> 新决策追加编号；**已定决策不重开讨论**，除非出现根本性障碍并经用户同意。

## D-001 放弃 Python 插件执行，改用 Rust derive→MCP 工具体系
- **决策**：不执行 DB 中存量 `tool.content`/`function.content`（Python 源码）。存量行保留可读、标记 legacy。工具生态用 `rusty-tools` SDK（`#[derive(Tool)]` 生成 MCP stdio server，rmcp）重建；原版 builtin tools（`tools/builtin.py` ~4500 行）逐个用 Rust 重写。
- **理由**：Rust 中沙箱执行任意 Python 成本极高；MCP 是开放标准，原版 0.11.3 本身支持 MCP tool servers，兼容面不受影响。
- **被否**：Python sidecar（部署重）、WASM/Lua 沙箱跑 Python（不兼容存量生态）。
- 日期：2026-09-15

## D-002 本地 ML 用 Rust 原生推理
- **决策**：embedding+rerank 用 fastembed（ONNX，本地 GPU 可选）；STT 用 whisper-rs（whisper.cpp）；调研 candle/Burn 作为备选/特例。
- **被否**：Python 推理 sidecar（数值一致但部署重）；只支持远程引擎（无法对齐原版默认本地行为）。
- 日期：2026-09-15

## D-003 实时通道用原生 WebSocket + SSE
- **决策**：不实现 socket.io 服务端（Rust 生态无成熟实现，rust_socketio 仅客户端）。WS 承载房间与增量、SSE 只读兜底；**事件名与负载语义保持原版**（`events`、`events:channel` 等），RPC 用带 `reply_to` 的帧。
- **代价**：原版 Svelte 前端无法直连本后端（不作为目标；用自家 Dioxus 前端 + 契约测试对拍语义）。
- 日期：2026-09-15

## D-004 数学渲染用 katex-rs（纯 Rust，WASM）
- **决策**：katex-rs 0.3.0（纯 Rust KaTeX 移植，跟踪上游 0.18.5，输出 KaTeX 兼容 HTML+MathML，带 `wasm` feature）。页面照常加载 katex.css/woff2。兜底：JS interop 调 katex.js。
- **被否**：`katex` crate（2023 年停更，绑 V8/JS 引擎）。
- 日期：2026-09-15（用户提议采纳）

## D-005 Mermaid 渲染用 sebastian（纯 Rust/WASM）
- **决策**：sebastian 0.8.0——mermaid.js 11.15.0 像素级 Rust 移植，官方 `sebastian-wasm`（wasm-bindgen），18 种图型对 mmdc 字节级一致。备选 merman 0.7.0（merman-wasm 0.8.0-alpha）。
- **被否**：mermaid-rs-renderer 0.3.1（**不支持 wasm**：fontdb 系统字体 + Instant panic，issue #120 未决）；mermaid.js JS interop（用户要求减少 JS 技术栈）。
- **风险**：sebastian 单人维护、采用量小（回归测试体系完善，风险可接受）。
- 日期：2026-09-15（用户主导修订）

## D-006 Markdown 用 comrak（default-features=false，wasm 官方支持）
- 上游有 wasm32 cfg 段；GFM 表格/任务列表/脚注/删除线 + `$…$` 数学扩展。citation/mention/colon-fence 用预处理 pass 实现。消毒 ammonia。
- 日期：2026-09-15

## D-007 代码高亮用服务端原生 tree-sitter
- **决策**：后端 tree-sitter + highlights.scm（复用 zed `crates/grammars/src/*/highlights.scm` 查询文件 + crates.io tree-sitter crate，**不引入** zed 的 gpui/语言 crate）产出彩色 span 回填前端；流式期间纯文本。浏览器端备用 syntect(default-fancy)。
- **依据**：zed 研究结论——其高亮核心可分离但直接复用是"手术级"工程；查询文件才是高价值资产。
- 日期：2026-09-15

## D-008 数据库层用 SeaORM 2.0.3 + sqlx 0.9 逃生舱
- **决策**：SeaORM 实体/仓库（底层 sqlx ^0.9，无版本耦合）；特例（自定义 like、函数索引探测）走裸 sqlx。**不用** SeaORM 迁移：全新库写内嵌 alembic-head 终态 DDL 并盖戳；启动校验 `alembic_version == d4c1a8e37b62`，低于 head 拒绝启动。
- **理由**：40 表 CRUD 工作量、双方言由 sea-query 处理、泛型 JSON 列行为与 SQLAlchemy 一致。
- 日期：2026-09-15

## D-009 License 未定
- 发布前决定。约束：链接 `openai-interface`（AGPL-3.0）分发需遵循 AGPL；当前未复制 open-webui 代码（其 LICENSE 已变更历史复杂），locale 仅取 zh/en 数据参考。
- 日期：2026-09-15 记录

## D-010 v1 范围
- v1 含：核心聊天、Ollama/OpenAI、RAG/知识库、文件、管理后台、用户组权限、Channels、Automations、Calendar、中英 i18n。
- 后置（M7）：OAuth/LDAP/SCIM、Notes 协作（yrs）、Redis 多实例、分析面板、终端、OTel。
- 日期：2026-09-15

## D-011 OpenAI 等 Web API 以独立 crate 形式沉淀
- **决策**：OpenAI 兼容层基于用户维护的 `openai-interface`（Codeberg/Hammerklavier，当前 0.10.0，edition 2024，deepseek/qwen feature）；Ollama 客户端、搜索 loader 等同样按可独立发布标准设计（无服务器状态耦合）。缺口薄封装并向 crate 作者（即用户）反馈需求。
- **注意**：AGPL 许可影响整体 License 决策（联动 D-009）；M1 接入时核对流式 SSE 解析、Responses API、audio/images/embeddings 覆盖面。
- 日期：2026-09-15（用户主导修订）

## D-012 上下文压缩恢复协议
- 对话上下文会多次压缩；每次压缩后必须先读 `docs/PROGRESS.md` → `DECISIONS.md` → `ARCHITECTURE.md`（涉兼容再读 `COMPATIBILITY.md`）→ `git log/status` 对账 → 相关代码，再继续。写入 AGENTS.md §1。
- 日期：2026-09-15（用户要求）
