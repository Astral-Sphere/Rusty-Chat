# PROGRESS.md — 进度看板（每次交付后必须更新）

> 接手/恢复上下文时**先读本文**，再读 DECISIONS / ARCHITECTURE（见 AGENTS.md §1）。
> 最后更新：2026-09-30（M1 七段完成；openai-interface 0.14.0 / sea-orm 2.0.4 升级核验；M1 收官计划定稿，见「下一步」）

## 里程碑总览

- [x] **M0 脚手架**（2026-09-16 完成）
- [ ] **M1 核心聊天**：rc-db 全实体 + auth/users/roles/config + chats CRUD（树状 history/分支/归档/置顶/分享/标签）+ Ollama/OpenAI 模型注册（接入 openai-interface）+ 统一 chat/completions + WS 流式 + Dioxus 聊天 UI（markdown+KaTeX+高亮）
- [ ] **M2 RAG**：文件上传（本地+object_store）+ 知识库 + chunking + embedding 引擎 + 三向量后端 + hybrid/rerank + 重索引 CLI
- [ ] **M3 工作台与管理**：prompts（版本史）、模型预设、memories、groups/权限、folders/share、管理后台、反馈/评估
- [ ] **M4 Channels + Automations + Calendar**
- [ ] **M5 媒体与搜索**：STT/TTS/图像全引擎、30+ 搜索引擎、web loader、任务端点（title/tags/queries/auto/moa）
- [ ] **M6 工具生态**：rusty-tools derive SDK、MCP 客户端管理、builtin tools 全量移植、子代理、代码解释器
- [ ] **M7 企业与协作**：OAuth/LDAP/SCIM、Notes+yrs、Redis 多实例、分析面板、终端、OTel

## M0 完成清单（2026-09-16）

- [x] cargo workspace（resolver 3 / edition 2024 / rust-version 1.94），11 个 crate + web/ 独立 workspace，`cargo check` 全绿。
- [x] 依赖版本基线核实并锁入 `Cargo.toml`（sea-orm 2.0.3、sqlx 0.9.0、axum 0.8.9 等，见 DECISIONS；web 侧 dioxus 0.7.10）。
- [x] **fixture 基准**：open-webui 0.11.3 alembic 链跑通 SQLite + Postgres（podman postgres:17-alpine）至 head `d4c1a8e37b62`；产物 `docs/fixtures/webui-head.db`（43 表）、双方言 schema dump。
- [x] `rc-core`：`Secs/Nanos` 时间戳策略（类型强制单位）、Error、兼容版本常量；5 个单元测试。
- [x] `rc-db::bootstrap`：双方言内嵌 DDL（`src/ddl/{sqlite,postgres}.sql`）、`inspect_database`（Fresh/Compatible/WrongRevision/LegacyUnstamped）、幂等 bootstrap；7 个集成测试（含真实 PG，`RC_TEST_PG_URL` 门控）。
- [x] 文档体系：README、AGENTS.md（压缩恢复协议/测试纪律）、ARCHITECTURE、COMPATIBILITY（全表 schema+时间戳+鉴权+事件）、TESTING、DECISIONS（D-001..D-012）。
- [x] rustfmt/clippy/justfile；`cargo test`、`cargo clippy -D warnings`、`cargo fmt --check` 全绿。

### M0 踩坑记录（避免重蹈）
- sqlx 0.9：`sqlx::query` 要求 `&'static str`（动态 SQL 需 `raw_sql`/绑定）；Any 驱动要先 `sqlx::any::install_default_drivers()`（封装为 `rc_db::install_drivers()`）；SQLite URL 默认不建文件（`?mode=rwc`）；`PoolConnection` 未 drop 时 `pool.close()` 会挂；Executor 是泛型绑定——`sqlx::query*` 调用须显式 `&mut *conn`（PoolConnection→AnyConnection 的解引用不会被泛型自动做），而我们自己的具体类型函数（参数 `&mut AnyConnection`）直接传 `&mut conn` 即可（clippy::explicit_auto_deref 会强制这一点）。
- alembic 跑法：必须 `cd open-webui/backend/open_webui`（script_location 相对 CWD）；最小依赖清单见 COMPATIBILITY.md §1.1（无需装 torch/chroma）。
- SQLite `.schema`/rootpage 顺序不是安全执行序：bootstrap DDL 必须「先全部 CREATE TABLE 再 CREATE INDEX」。
- open-webui head 库有 43 表（含遗留 document/chatidtag/config_old），rc-db 测试对此计数断言。

## 当前进行中

（无进行中任务）依赖升级核验完成：openai-interface 0.14.0（0.13/0.14 仅新增 vllm/zai 专属类型，我们零代码改动，提交 70351f5）+ sea-orm 2.0.4，`cargo test --workspace` 全绿。下一步为 M1 收官增强。

## M1 状态总结（2026-09-16）

**全部 7 段完成**（ef7744a→e7b37e2 共 9 个提交）：后端 REST 面（auth/chats/models/config）+ 聊天管线（双方言流式 + WS events）+ Dioxus 前端骨架（登录/聊天/流式渲染）全部可用并有契约测试覆盖；84 native 测试 + web wasm 编译零警告。**遗留增强**（不阻塞 M1 验收，见「下一步」）：markdown/KaTeX/高亮渲染管线、标题生成、消息树分支 UI、content search。

## M1 分段进度

- [x] **M1-1**（e02b612）：rc-db 9 实体 + 仓库层（users/auths/chats/chat_messages/tags/shared_chats/config 引擎）+ chat.chat blob 算法纯函数（merge/upsert/delete/repair，1:1 移植 chats.py）+ 22 项双方言集成测试。踩坑：argcon2 0.6 新 API（CustomizedPasswordHasher + 裸盐字节）、sea-query 1.0 的 ExprTrait/PgExpr trait 方法、json_each/json_array_elements_text 方言分支、functional unique index（lower(email)）下测试 email 必须唯一。
- [x] **M1-2**（d78ad77）：rc-auth — JWT HS256（claims id/exp/iat/jti、epoch 秒）、bcrypt cost12 + 72 字节语义（verify 截断/signup 拒绝）、argon2 前缀识别、sk- API key、parse_duration、placeholder-hash 防时序；**契约测试：PyJWT 签发的 token 可被解码、Python bcrypt hash 可被验证**（jsonwebtoken 11 需显式 crypto provider，选 rust_crypto）。
- [x] **M1-3**（本次提交）：rusty-chat lib+bin 拆分（契约测试用 oneshot 驱动完整 router）；settings（WEBUI_SECRET_KEY 三级解析+落盘）；DEFAULT_CONFIG registry（/api/config 全部 48 键 + user.permissions 完整树）；GET /api/config（匿名公共子集/onboarding/登录后全量）；/api/v1/auths/{signin,signup,signout,update/password,api_key} + GET /（session）——响应形状与 OWU routers/auths.py 逐字段对齐（首用户 admin+enable_signup 自动关闭、TOCTOU 注释、placeholder 烧录、cookie token httponly samesite=lax）；2 个契约测试覆盖完整流程。
- [x] **M1-4**（本次提交）：rc-llm — `ollama.rs`（多后端 /api/tags 扇出合并、urls 聚合、lowest_version）、`registry.rs`（OpenAI 兼容 /models 拉取 + bearer/prefix_id/urlIdx + 去重 last-wins）、`models.rs` DTO（serde flatten 保留未知字段，urlIdx rename）；rusty-chat `/api/models`（VerifiedUser + config 驱动）与 `/ollama/*` 流式反向代理；7 项单元测试含 mock 后端。**openai-interface 0.11.0-rc1 调研完成**（docs/OPENAI_INTERFACE.md）：MIT、无阻断、流式/工具/Responses/embeddings/audio/images 全覆盖；缺口清单（对称 derive、宽容 chunk 解析、de-gate reasoning_content、发 0.11.0 final）已整理待反馈作者。
- [x] **M1-5**（本次提交）：rc-core 新增 chat.rs（ChatCompletionForm/ChatMessage/StreamDelta/OutputItem + OutputAccumulator）与 events.rs（WsFrame + 事件载荷构造器，形状对齐 Chat.svelte chatEventHandler）；rc-llm 新增 openai_chat.rs（openai-interface 0.11.0 适配：请求构建 typed 参数+extra_body 透传、SSE chunk→StreamDelta 含 reasoning_content（deepseek feature 透传）、tool_calls 分片聚合、非流式 complete）与 ollama_chat.rs（/api/chat ndjson：OpenAI→Ollama payload 转换（max_tokens→num_predict 等）、thinking→Reasoning、跨 TCP 分块行重组）；rc-realtime Hub（user:{id} 房间、多会话、离线降级）；rusty-chat /api/chat/completions（模型解析 404、用户消息+助手占位持久化、stream=false 同步 OpenAI JSON、stream=true 任务 envelope + tokio spawn）+ /ws（首帧 token 握手、heartbeat-ack、hub 注册）；2 个端到端契约测试（真实端口 + WS 客户端：事件序列 delta→done→active(false) + blob/chat_message 持久化断言）。
- [x] **M1-6**（本次提交，骨架完成）：Dioxus 0.7.10 CSR 应用 — api.rs（gloo-net HTTP + web-sys WebSocket 通道 + localStorage token + uuid）、登录视图（signin/signup 切换、错误显示）、聊天列表侧栏（/api/v1/chats/ 分页列表、置顶/active 标记、signout）、聊天视图（/api/models 模型选择、消息流式渲染（WS delta 追加→message_done 终态）、Enter 发送、chat:active 生成中状态、完成后按服务端持久化状态重载）；静态 CSS（asset! 加载，M2 换 Tailwind v4 管线）；dx 代理配置（/api + /ws → 8080）。wasm32 编译零警告。**M1-6 剩余**：markdown 渲染管线（comrak+katex-rs+服务端 tree-sitter 高亮）、标题生成展示、多分支消息树、占位符差异对拍。
- [x] **M1-7**（本次提交）：/api/v1/chats 全套路由 — new/list(+list 别名, page 分页 60/页)/search/pinned/archived/archive_all/unarchive_all/shared/share/{share_id}(公开)/{id}(GET|POST|DELETE)/{id}/pin/{id}/archive/{id}/share(GET/DELETE)/{id}/tags(GET/POST)+DELETE /api/v1/chats；ChatResponse=实体直接序列化（serde 加到 chat entity），title 行带 active:false；所有权校验（非属主 401）；1 个大契约测试覆盖 CRUD+pin/archive+share+tags+搜索+越权。

## 下一步（M1 收官计划，2026-09-30 定稿）

执行顺序 **T1 → T2 → T3 → T4 → T1b → T5**；每项动手前先按 AGENTS.md §3 列全测试面。完成后 M1 验收打勾，进入 M2。

- **T1 markdown 渲染管线**（前端 wasm）：web/ 新增 `render.rs` 纯函数模块 — comrak（default-features=false，GFM+math-dollars）→ `$..$`/`$$..$$` 经 katex-rs（wasm feature）渲染 → ammonia 消毒 → `dangerously_set_inner_html`；KaTeX css/woff2 静态资源由 rusty-chat 提供；流式期间纯文本、message_done 后整段渲染。测试面（host 端可测，web 是纯函数）：GFM 全要素、数学（inline/display/不完整公式/`$` 转义/代码块内不渲染）、XSS 白名单（script/iframe/javascript:/onerror）、空串/emoji/超长/CRLF。
- **T2 标题生成任务端点**（后端+前端）：`POST /api/v1/tasks/title/completions`（形状对齐 routers/tasks.py）+ 聊天首轮后自动触发 + hub 广播 `chat:title`（rc-core events 已有构造器）→ 侧栏实时更新；task model 解析（config `task.model`，fallback 当前模型）；打样 /api/v1/tasks/ 路由组（M5 的 tags/queries/follow_up/moa 同构复用）。契约测试：端点形状、task model 缺失、标题落库（blob+列）、WS 载荷形状、prompt 模板变量。
- **T3 服务端代码高亮**：新 crate `rc-highlight` — tree-sitter + 常用语言 grammar crates（rust/python/js/ts/go/c/cpp/json/yaml/bash/html/css/sql/markdown 起步），zed 的 `highlights.scm` 以**数据资产**形式 vendored 到 `assets/highlights/`（保留上游许可头，进不了 src 源码树，D-007 与 AGENTS.md §4 的调和方式，**待用户确认**）；`POST /api/v1/utils/highlight`（code+language → span 数组 `{start,end,class}`，未知语言返回 `unsupported:true` 原样透出）；前端完成态回填。测试面：每语言 smoke、未知/空/超长/无换行/深嵌套、非 ASCII 注释、CRLF/tab。
- **T4 消息树分支 UI**：编辑历史消息生成新分支 + `‹ ›` 分支切换（后端 M1-1 的 blob 分支算法已就绪，纯前端为主 + 分支索引纯函数测试 + 契约测试补充编辑分支断言）。
- **T1b Mermaid**：sebastian-wasm 渲染 ```mermaid 代码块为 SVG（D-005；独立任务因 wasm-bindgen 集成有自己的坑）。
- **T5 M1 验收对拍**：`dx build --release` → rust-embed 内嵌 → 单二进制 serve 冒烟（登录→聊天→流式→标题→高亮全链路）；cargo test + clippy + fmt + PG 契约全绿；docs 同步。

### 待用户拍板（不阻塞 T1/T2）
1. T3 的 highlights.scm vendored 方式（推荐 `assets/highlights/` + 许可头 + DECISIONS 记录）。
2. Tailwind v4 构建管线推迟到 M2（推荐，避免阻塞 M1 收官）。
3. Mermaid 放 T1b（推荐）还是与 T1 合并。

## 再下一步（M2 RAG 骨架概要）

- **M2-1** 文件上传与存储：/api/v1/files/（POST 上传/GET 元数据/GET file/content），本地 `DATA_DIR/uploads` + object_store trait 抽象。
- **M2-2** 抽取与切分：rc-rag — PDF/DOCX/PPTX/XLSX/CSV/HTML/text 抽取；RAGTextSplitter 等切分策略对齐原版。
- **M2-3** embedding 引擎 + 三向量后端：fastembed（默认本地）/ollama/openai；sqlite-vec（默认）/pgvector/qdrant，向量库 trait + 重索引 CLI。
- **M2-4** 知识库 API（/api/v1/knowledge/）+ 聊天 RAG 注入（rag_template、`<source>` 包裹）+ hybrid BM25+rerank。

## M1-5 踩坑

- openai-interface：Message 枚举变体字段不齐（Assistant 无 function_call、有 prefix/reasoning_content，不可 ..Default::default()）；StreamOptions.include_usage 是 bool 非 Option；StopKeywords 仅 Serialize（stop 走 extra_body）；OapiError 在 errors:: 模块；流式 finish_reason 用 streaming::FinishReason（与 chat::FinishReason 不同类型，usage 计数为 usize）。
- 依赖 feature 透传：deepseek 需在本 crate 声明同名 feature 并映射（#[cfg(feature=…)] 看本 crate）。
- axum WS：Message::Text 需要 Utf8Bytes（.into()）；Parts/Bytes 顺序在 body extractor 之前。
- 测试稳定性：updated_at 秒级精度下相邻创建的排序断言必须显式 sleep ≥1.1s（contract_chats 曾因此抖动）。
- ollama 模型解析依赖 /api/tags——mock 后端别忘了它（只剩 /api/chat 会 404 Model not found）。

## 阻塞/待办

- LICENSE 未定（D-009）：M1 前不阻塞；公开发布前必须定（注意 openai-interface AGPL 联动）。
- PG DDL 的 `public` schema 硬编码：待支持 `DATABASE_SCHEMA` 时参数化（低优先）。
