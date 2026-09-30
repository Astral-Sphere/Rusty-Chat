# PROGRESS.md — 进度看板（每次交付后必须更新）

> 接手/恢复上下文时**先读本文**，再读 DECISIONS / ARCHITECTURE（见 AGENTS.md §1）。
> 最后更新：2026-10-01（M1 完成；前端界面已按 open-webui 0.11.3 深色版 1:1 仿写（D-015）；下一步 M2 RAG）

## 里程碑总览

- [x] **M0 脚手架**（2026-09-16 完成）
- [x] **M1 核心聊天**（2026-09-30 完成）：rc-db 全实体 + auth/users/roles/config + chats CRUD（树状 history/分支/归档/置顶/分享/标签）+ Ollama/OpenAI 模型注册（openai-interface 0.14.0）+ 统一 chat/completions + WS 流式 + Dioxus 聊天 UI（markdown+KaTeX+高亮+mermaid+分支）+ 标题生成任务 + Tailwind v4 + 单二进制内嵌
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

（无进行中任务）**M1 已于 2026-09-30 全部完成并验收**（提交 adf196d→b1c6104）。下一步进入 **M2 RAG**。

## M1 收官增强（2026-09-30 完成）

- [x] **T1a Tailwind v4**（adf196d）：standalone CLI（Rust 二进制，无 Node/npm，v4.3.3）；`web/input.css` → `just web-css` 生成 gitignored 的 `assets/tailwind.css`；dx 不会自动跑 Tailwind（官方指南即手动进程），just 配方为唯一入口。markdown 排版样式手写 `.markdown-body` prose 块（不依赖 JS 版 typography 插件）。
- [x] **T1 渲染管线**（3f664e0）：`web/src/render.rs` 纯函数 — comrak 0.55（default-features=false 才能上 wasm；GFM/数学全是运行时 Options 非 cargo feature，修正 D-006）+ hardbreaks → `$..$`/`$$..$$` 经 katex-rs 0.3（wasm feature，跟踪上游 0.18.5）→ ammonia 4.2 单一消毒点（含注入的 KaTeX HTML/MathML/SVG 白名单）；流式纯文本、done 后 keyed 组件 memo 渲染；KaTeX css+fonts vendored 且字体 data URI 内联（dx/manganis 不处理 CSS 内 url()，`web/tools/gen_katex_inline.py` 再生成）。19 host 测试含 XSS 电池。
- [x] **T2 标题生成**（0b0b845）：`rc-core::tasks` 1:1 移植 utils/task.py + middleware.py（prompt/messages 模板族、码点级截断、extract_title 回退链，27 测试）；`POST /api/v1/tasks/title/completions`（200 门/400/404/task model 解析/OpenAI JSON 透传）；首轮完成后台生成 + `chat:title` 广播 + 侧栏刷新；defaults 增 task.model.*/task.title.*（env 派生）。契约测试 3 个。
- [x] **T3 代码高亮**（c6d0948）：新 crate `rc-highlight` — tree-sitter 0.27 + 14 grammar crates（跨版本经 `tree-sitter-language::LanguageFn` 统一）+ zed highlights.scm vendored 数据资产（12 语言，NOTICE 记录补丁；html 用 grammar crate 自带查询；markdown 因 zed fork 节点不兼容弃用）；predicate 由 runtime TextProvider 评估；@none 抹除；字节偏移 slice-back 验证。`POST /api/v1/utils/highlight` + `/languages`；前端 span→HTML 回填（overlap-skip、转义、data-hl 幂等）。19+5+1 测试。
- [x] **T4 消息树分支**（e112c76）：`web/src/branches.rs` 纯函数（active_path 环防护、siblings_of、sibling_position、messages_for_regeneration，host 测试）；编辑用户消息 → 同 parentId 新分支重发；‹ n/m › 切换 = 部分更新 currentId；契约测试锁后端树语义。
- [x] **T1b Mermaid**（9a83e77）：sebastian 0.8.0 直接可上 wasm（无需 -wasm 包装 crate）；```mermaid fence 渲染 SVG（每渲染唯一 id）；失败保留源码 + 错误提示；浅色卡片样式。
- [x] **T5 验收**（b1c6104）：`just web-release` — dx build --release → web/dist → rust-embed（feature `embed-frontend`）单二进制，SPA fallback，api/ollama/ws 前缀不回退。**真实浏览器全链路冒烟通过**：注册→聊天→流式 markdown+KaTeX+高亮 rust fence+mermaid SVG→侧栏标题 Greetings→编辑重生成分支→‹ › 切换标题存活。冒烟揪出并修复 5 个 bug（见 M1-T5 踩坑）。

### M1-T5 踩坑（冒烟发现的真 bug，均有回归测试）
1. serve 全新 SQLite 目录崩（默认 URL 缺 `?mode=rwc`）——M0 已知坑在 settings.rs 复发。
2. 标题触发判定用「chat_id 缺失」——前端总是发客户端生成 chat_id，永不触发；改为「本次请求是否新建了聊天」。
3. `update_chat_title_by_id` 只写列不写 blob（M1-1 移植遗漏 OWU 的 `chat={**chat,'title'}`）——任何部分 blob 更新会从旧 blob title 重新派生列，把生成标题打回 "New Chat"。同步写列+blob 修复。
4. 侧栏选中从未传导到 chat_view（M1-6 留在隐藏 div 里）——共享 Signal 接线，send() 同步维护两信号。
5. 分支切换 currentId 落在兄弟消息本身导致回复不可见——补 `leaf_descendant`（open-webui Messages.svelte 的 youngest-child 链语义）。
- 环境注意：podman 在本会话沙箱内无法启动（newuidmap setuid 缺失）——PG 契约测试需用户在终端跑 `just test-pg`；另 `just` 本机未装，直接用底层命令。

### 分支编辑位置修复（2026-09-30，aa66757，用户报告）
- **现象**：编辑历史消息后新分支被追加到消息列表底部，而 open-webui 是原位替换。
- **根因**：显示列表是扁平 Vec，`send`/编辑保存直接 push；open-webui 的显示永远从 `history.currentId` 祖先链重建（`createMessagesList`）。
- **修复原则（必须遵守）**：**消息顺序的唯一来源是 `build_view(history)` 投影（active_path + 兄弟位置）；任何变更 = 本地 patch history 树（`branches::attach_*`，镜像服务端 upsert）→ 重新投影，绝不直接 push 列表**。
- `send` 补齐原版 submitPrompt 语义：follow-up `parentId = history.currentId`（此前 null → 每条都是新根）、发给 LLM 的 messages 是完整活跃链（此前只有新消息单条）。
- **新踩坑**：`std::time::SystemTime::now()` 在 wasm32-unknown-unknown 上 **unimplemented、直接 panic（release 下静默 unreachable trap，杀掉当前 spawn 的 future 但不崩实例）**——前端取时间必须走 `js_sys::Date::now()`（web/src/main.rs `chrono_secs`）。调试手段：window error 事件能捕到 wasm trap（`Uncaught RuntimeError: unreachable`）。

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

## 下一步（M2 RAG）

- **M2-1** 文件上传与存储：/api/v1/files/（POST 上传/GET 元数据/GET file/content），本地 `DATA_DIR/uploads` + object_store trait 抽象。
- **M2-2** 抽取与切分：rc-rag — PDF/DOCX/PPTX/XLSX/CSV/HTML/text 抽取；RAGTextSplitter 等切分策略对齐原版。
- **M2-3** embedding 引擎 + 三向量后端：fastembed（默认本地）/ollama/openai；sqlite-vec（默认）/pgvector/qdrant，向量库 trait + 重索引 CLI。
- **M2-4** 知识库 API（/api/v1/knowledge/）+ 聊天 RAG 注入（rag_template、`<source>` 包裹）+ hybrid BM25+rerank。
- 顺带（M2 设计系统）：web/ 类名已全面 Tailwind 化，M2 引入主题变量（CSS-first `@theme`）即可，无迁移负担。

## M1-5 踩坑

- openai-interface：Message 枚举变体字段不齐（Assistant 无 function_call、有 prefix/reasoning_content，不可 ..Default::default()）；StreamOptions.include_usage 是 bool 非 Option；StopKeywords 仅 Serialize（stop 走 extra_body）；OapiError 在 errors:: 模块；流式 finish_reason 用 streaming::FinishReason（与 chat::FinishReason 不同类型，usage 计数为 usize）。
- 依赖 feature 透传：deepseek 需在本 crate 声明同名 feature 并映射（#[cfg(feature=…)] 看本 crate）。
- axum WS：Message::Text 需要 Utf8Bytes（.into()）；Parts/Bytes 顺序在 body extractor 之前。
- 测试稳定性：updated_at 秒级精度下相邻创建的排序断言必须显式 sleep ≥1.1s（contract_chats 曾因此抖动）。
- ollama 模型解析依赖 /api/tags——mock 后端别忘了它（只剩 /api/chat 会 404 Model not found）。

## UI 对齐 open-webui（2026-10-01 完成，用户要求对照原版截图仿写）

- [x] **主题**（input.css）：Tailwind `@theme` 整体替换灰阶为 OWU 消色差色阶（gray-950=#0d0d0d…扩展 gray-850=#262626）；body #171717、侧栏 #0d0d0d；细圆角滚动条；`.hover-reveal`（group 悬停显隐）；建议卡 waterfall 入场动画；markdown 排版调至 prose-sm 尺度。
- [x] **结构**：main.rs 瘦身为壳 + 登录页（深色卡片/胶囊切换）；新 `sidebar.rs`（245px 侧栏 + 42px 折叠栏：新对话/搜索（`/api/v1/chats/search?text=`）/置顶段/日历时间分组（今天/昨天/过去 7 天/过去 30 天/月份，Hinnant civil_from_days + tz 偏移注入，9 测试）/条目 ⋯ 删除菜单/用户菜单）；新 `chat.rs`（navbar 标题、空态占位（模型名点击轮换 + OWU 6 张默认建议卡 + 输入卡）、用户右对齐圆角泡、assistant 头像+模型名+动作行、底部输入卡 + 模型下拉 pill + 圆形发送键、滚动钉底按钮）；新 `icons.rs`（内联 lucide 风格 SVG，无资产无 JS）；`api.rs` 增 epoch_secs/tz_offset_secs/copy_text（Clipboard）。
- [x] **语义修正**：assistant 占位节点带 `model` 字段（branches.rs，与后端持久化一致）→ 消息头立即显示模型名；置底判定阈值 120→40px（短对话 88px 可滚动距离会被永久钉底）；打开会话后 300ms 二次钉底（mermaid/高亮异步回填后内容再增长）；mermaid 卡 fit-content 居中。
- [x] **验收**：mock ollama 全链路浏览器冒烟——注册→空态→发送（流式/markdown/KaTeX/tree-sitter 高亮/mermaid）→标题生成（侧栏"今天"分组 + navbar）→编辑原位替换 ‹2/2›→切回 1/2 完整旧链→搜索→删除→折叠/展开→模型下拉→用户菜单。门禁：native 124 + web 56 测试全绿，双 workspace clippy/fmt 干净。
- **环境注意（自动化）**：ZCode IAB 面板失焦时 `requestAnimationFrame` 完全停摆（visibilityState 仍 "visible"），Dioxus-web 依赖 rAF flush 渲染 → 页面"点击无响应"。上一会话的"CUA 事件零到达"即此。冒烟时先注入 `window.requestAnimationFrame = cb => setTimeout(() => cb(performance.now()), 16)` 替身（仅测试环境，真实浏览器无此问题）。另：Playwright 元素 actionability 检查同样依赖原生 rAF 会超时，用 evaluate `el.click()` 替代。

## 阻塞/待办

- LICENSE 未定（D-009）：M1 前不阻塞；公开发布前必须定（注意 openai-interface AGPL 联动）。
- PG DDL 的 `public` schema 硬编码：待支持 `DATABASE_SCHEMA` 时参数化（低优先）。
