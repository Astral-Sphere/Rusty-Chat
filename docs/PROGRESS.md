# PROGRESS.md — 进度看板（每次交付后必须更新）

> 接手/恢复上下文时**先读本文**，再读 DECISIONS / ARCHITECTURE（见 AGENTS.md §1）。
> 最后更新：2026-09-16（M1 进行中：M1-1/2/3/4 已完成）

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

**M1-4：rc-llm 模型注册**（Ollama/OpenAI 合并 + /api/models + openai-interface 0.11.0-rc1 接入调研）

## M1 分段进度

- [x] **M1-1**（e02b612）：rc-db 9 实体 + 仓库层（users/auths/chats/chat_messages/tags/shared_chats/config 引擎）+ chat.chat blob 算法纯函数（merge/upsert/delete/repair，1:1 移植 chats.py）+ 22 项双方言集成测试。踩坑：argcon2 0.6 新 API（CustomizedPasswordHasher + 裸盐字节）、sea-query 1.0 的 ExprTrait/PgExpr trait 方法、json_each/json_array_elements_text 方言分支、functional unique index（lower(email)）下测试 email 必须唯一。
- [x] **M1-2**（d78ad77）：rc-auth — JWT HS256（claims id/exp/iat/jti、epoch 秒）、bcrypt cost12 + 72 字节语义（verify 截断/signup 拒绝）、argon2 前缀识别、sk- API key、parse_duration、placeholder-hash 防时序；**契约测试：PyJWT 签发的 token 可被解码、Python bcrypt hash 可被验证**（jsonwebtoken 11 需显式 crypto provider，选 rust_crypto）。
- [x] **M1-3**（本次提交）：rusty-chat lib+bin 拆分（契约测试用 oneshot 驱动完整 router）；settings（WEBUI_SECRET_KEY 三级解析+落盘）；DEFAULT_CONFIG registry（/api/config 全部 48 键 + user.permissions 完整树）；GET /api/config（匿名公共子集/onboarding/登录后全量）；/api/v1/auths/{signin,signup,signout,update/password,api_key} + GET /（session）——响应形状与 OWU routers/auths.py 逐字段对齐（首用户 admin+enable_signup 自动关闭、TOCTOU 注释、placeholder 烧录、cookie token httponly samesite=lax）；2 个契约测试覆盖完整流程。
- [x] **M1-4**（本次提交）：rc-llm — `ollama.rs`（多后端 /api/tags 扇出合并、urls 聚合、lowest_version）、`registry.rs`（OpenAI 兼容 /models 拉取 + bearer/prefix_id/urlIdx + 去重 last-wins）、`models.rs` DTO（serde flatten 保留未知字段，urlIdx rename）；rusty-chat `/api/models`（VerifiedUser + config 驱动）与 `/ollama/*` 流式反向代理；7 项单元测试含 mock 后端。**openai-interface 0.11.0-rc1 调研完成**（docs/OPENAI_INTERFACE.md）：MIT、无阻断、流式/工具/Responses/embeddings/audio/images 全覆盖；缺口清单（对称 derive、宽容 chunk 解析、de-gate reasoning_content、发 0.11.0 final）已整理待反馈作者。
- [ ] **M1-5**：/api/chat/completions + WS events
- [ ] **M1-6**：web/ 登录 + 聊天 UI
- [ ] **M1-7**：chats CRUD 端点

## 下一步（M1-5 起点）

1. rc-core：内部 OpenAI-shape 类型 + OR-style output items（message/reasoning/function_call/function_call_output）数据模型。
2. rc-llm `openai_chat`：接 openai-interface（"0.11.0-rc1"），请求映射 + 流式 chunk → output items 流；Ollama ndjson 方言转换（payload OpenAI↔Ollama + 响应转回 OpenAI SSE 语义）。
3. `POST /api/chat/completions` 端点 + WS `/ws`（auth 帧、user:{id} 房间、events 事件帧）+ task envelope。
4. 之后 M1-6 web UI、M1-7 chats 端点。

## 阻塞/待办

- LICENSE 未定（D-009）：M1 前不阻塞；公开发布前必须定（注意 openai-interface AGPL 联动）。
- PG DDL 的 `public` schema 硬编码：待支持 `DATABASE_SCHEMA` 时参数化（低优先）。
