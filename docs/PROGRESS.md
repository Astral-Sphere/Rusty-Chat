# PROGRESS.md — 进度看板（每次交付后必须更新）

> 接手/恢复上下文时**先读本文**，再读 DECISIONS / ARCHITECTURE（见 AGENTS.md §1）。
> 最后更新：2026-09-16（M0 完成时）

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

（无——M0 收尾，等待开始 M1）

## 下一步（M1 起点）

1. `rc-db`：生成全部 43 表 SeaORM 实体（先 user/auth/api_key/config/chat/chat_message/shared_chat/tag/folder 七个核心），时间戳列全部用 `Secs/Nanos` 包装；仓库层方法名对齐 open-webui models 层语义（如 `Chats::get_chat_by_id_and_user_id`）。
2. `rc-auth`：JWT HS256 签发/校验（claims id/exp/iat/jti）、bcrypt/argon2、`sk-` API key、cookie `token`。
3. `rusty-chat`：`serve` + `create-admin` + `migrate-check` 子命令，axum AppState（config engine + db pool），`/api/config` 与 `/api/v1/auths/*` 首批端点 + 契约测试。
4. `web/`：dioxus-cli 脚手架、登录页、聊天骨架。
5. `openai-interface` 接入调研：核对 SSE 流式解析/Responses API/工具调用覆盖面，缺口列清单反馈用户（联动 D-011）。

## 阻塞/待办

- LICENSE 未定（D-009）：M1 前不阻塞；公开发布前必须定（注意 openai-interface AGPL 联动）。
- PG DDL 的 `public` schema 硬编码：待支持 `DATABASE_SCHEMA` 时参数化（低优先）。
