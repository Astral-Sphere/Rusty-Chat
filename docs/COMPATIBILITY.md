# COMPATIBILITY.md — open-webui 0.11.3 兼容性权威参考

> 本文档是数据库/API 兼容性的**唯一权威来源**。其他文档只引用，不复制。
> 基准：open-webui 0.11.3，alembic head `d4c1a8e37b62`（58 个线性迁移）。
> 参考仓库路径：`open-webui/backend/open_webui/`。

## 0. 与旧版认知的关键差异（0.11.3 实况）

1. ORM 是 **SQLAlchemy 2.x**（非 peewee），迁移是 **Alembic**，版本表 `alembic_version`。
2. 只有一个 `DATABASE_URL`（默认 `sqlite:///{DATA_DIR}/webui.db`），无独立内部库。
3. 时间戳**混用秒与纳秒**（见 §2），`chat.timer_at`（纳秒）与同表 `created_at`（秒）并存——重写第一大坑。
4. 无 `histories` 表。聊天历史在两处：`chat.chat` JSON blob（`history.messages` map + `currentId`）**和**规范化的 `chat_message` 表（迁移 `8452d01d26d7` 从 blob 回填），二者由模型层方法保持同步。
5. 访问控制是新表 `access_grant`；旧 `access_control` JSON（`{read:{user_ids,group_ids},write:…}`）仍留在 model/knowledge/prompt 的 `meta` 里，读取方必须双表示兼容（`utils/access_control` 的 `migrate_access_control`）。
6. auth.py 不存在——鉴权在 `utils/auth.py` + `routers/auths.py` + `utils/oauth.py`。

## 1. 数据库层机制

- 引擎：**双引擎**——sync（启动配置加载/迁移/健康检查）+ async（全部运行时操作）；SQLite 用 aiosqlite，PG 用 psycopg v3。
- SQLite PRAGMA：WAL（`DATABASE_ENABLE_SQLITE_WAL`）、busy_timeout、cache_size 等（env `DATABASE_SQLITE_PRAGMA_*`）；注册自定义 `like()` SQL 函数实现 ASCII 不区分大小写 LIKE。
- `postgres://` 自动重写为 `postgresql://`；可选 `DATABASE_SCHEMA`（PG schema）。
- JSON 存储两种策略（**兼容关键**）：
  - `sqlalchemy.JSON`：PG 上原生 `JSON` 列、SQLite 上 TEXT。大多数表用这个（`chat.chat`、`user.settings`、`message.*`…）。
  - `JSONField`（自定义 TypeDecorator over UnicodeText）：**两后端恒为 TEXT**，读时须 `json_parse`。用于 `tool.specs/meta/valves`、`function.meta/valves`、`model.params/meta`。
- `ENABLE_ORJSON=true` 时 DB 中 JSON 可能为紧凑序列化（NaN→null、大整数→float 等怪癖），serde 解析需宽容。

### 1.1 本项目的 schema 再生成方法（ground truth 流程）

不安装 open-webui 全量依赖即可生成基准：

```bash
uv venv /tmp/owui-fixture --python 3.12
uv pip install --python /tmp/owui-fixture/bin/python \
  sqlalchemy alembic pydantic pydantic-settings redis requests authlib \
  markdown beautifulsoup4 cryptography psycopg2-binary psycopg \
  python-engineio python-socketio aiosqlite bcrypt mimeparse \
  typer uvicorn fastapi websockets aiohttp
uv pip install --python /tmp/owui-fixture/bin/python --no-deps -e ./open-webui

mkdir -p /tmp/rc-fixture
# SQLite:
cd open-webui/backend/open_webui   # alembic.ini 的 script_location 相对 CWD
DATA_DIR=/tmp/rc-fixture DATABASE_URL='sqlite:////tmp/rc-fixture/webui.db' \
  WEBUI_SECRET_KEY=test-secret /tmp/owui-fixture/bin/python -m alembic -c alembic.ini upgrade head
# Postgres（podman 容器，见 justfile pg-up）:
DATA_DIR=/tmp/rc-fixture DATABASE_URL='postgresql://postgres:fixture@127.0.0.1:5433/fixture' \
  WEBUI_SECRET_KEY=test-secret /tmp/owui-fixture/bin/python -m alembic -c alembic.ini upgrade head
```

产物：
- `docs/fixtures/webui-head.db` — SQLite head 库（43 表，含遗留 `document`/`chatidtag`/`config_old`），rc-db 集成测试直接使用。
- `docs/fixtures/sqlite-head-schema.sql` / `postgres-head-schema.sql` — dump 基准。
- `crates/rc-db/src/ddl/{sqlite,postgres}.sql` — 可执行 bootstrap DDL（SQLite 先建全部表再建索引；PG 为 pg_dump 清洗版；末尾均盖 `alembic_version` 戳）。

## 2. 时间戳策略（最危险）

| 单位 | 表 |
|---|---|
| **秒** | `user`、`api_key`、`chat`（created/updated/last_read_at）、`chat_message`、`chat_file`、`shared_chat`、`tag`、`memory`、`access_grant`、`config`、`tool`、`function`、`model`、`file`、`folder`、`prompt`、`prompt_history`、`feedback`、`knowledge*`、`group`、`group_member`、`oauth_session`、`skill` |
| **纳秒** | `message`、`message_reaction`、`channel`、`channel_member`、`channel_file`、`channel_webhook`、`note`、`pinned_note`、`automation`、`automation_run`、`calendar`、`calendar_event`、`calendar_event_attendee`、以及 `chat.timer_at` |

- Python 侧：`int(time.time())` vs `int(time.time_ns())`。
- Rust 侧：**只允许用 `rc_core::timestamp::{Secs, Nanos}`**，禁止裸 i64 传时间。类型上强制，测试中显式断言单位。
- `chat.timer_at` 为纳秒「到期时间」，同行的 created_at/updated_at 是秒。

## 3. 全表清单（head 时点，43 表）

> `String`=VARCHAR，`Text`=TEXT，`JSON`=SQLAlchemy 泛型 JSON（PG 原生 JSON/SQLite TEXT），`JSONField`=恒 TEXT。ID 除注明外均为 UUIDv4 字符串。FK 大多带 `ondelete=CASCADE`。

### user（models/users.py）
id PK UUID（=auth.id）；email UNIQUE（写入 lowercase + 函数索引 `uq_user_email_lower` on lower(email) WHERE email IS NOT NULL）；username(50)；role `pending|user|admin` 默认 pending；name NOT NULL；profile_image_url、profile_banner_image_url、bio、gender、date_of_birth(Date)、timezone、presence_state、status_emoji、status_message、status_expires_at(BigInt 秒)；info JSON；variables JSON；**settings JSON**（前端持有的大 blob：含 `functions.valves`/`tools.valves`（可能 Fernet 加密）等，合并语义不可覆盖）；oauth JSON（`{provider:{sub,email,…}}`）；scim JSON（externalId）；last_active_at/updated_at/created_at BigInt **秒**。

### auth（models/auths.py）
id PK（=user.id）；email（与 user.email 同步）；password Text（bcrypt cost 12 / argon2 字符串）；active Boolean。

### api_key（models/users.py）
id PK UUID；user_id；key UNIQUE 明文 `sk-`+uuid4 无连字符（32 hex）；data JSON；expires_at/last_used_at（秒，可空）；created_at/updated_at（秒）。

### chat（models/chats.py）——核心表
id PK UUID（legacy 行可能他格式）；user_id idx；title Text；**chat JSON**（blob 结构见 §4）；created_at/updated_at BigInt **秒** idx；share_id UNIQUE（/s/{id}）；archived Bool；pinned Bool 默认 false；meta JSON 默认 `{}`（`internal:true`、`type`、`note_id`、`parent_chat_id`、`tags` map…）；variables JSON；folder_id；tasks JSON；summary Text；current_message_id（→chat_message）；last_read_at BigInt 秒；timer_at BigInt **纳秒**（部分索引 WHERE timer_at IS NOT NULL）。
索引：folder_id、(user_id,pinned)、(user_id,archived)、(updated_at,user_id)、(folder_id,user_id)、(user_id,updated_at,id DESC)、timer_at 部分索引 ×2、(user_id,folder_id,archived,updated_at,last_read_at,id) covering。

### chat_message（models/chat_messages.py）
id PK UUID（常= blob 内 message id）；chat_id FK→chat CASCADE idx；user_id idx；role `user|assistant|system`；parent_id（树）；**content JSON（字符串或内容块数组）**；output JSON（assistant 的 OR-style 输出项数组）；model_id idx；files/sources/embeds JSON；meta JSON；done Bool 默认 true；status_history JSON；error JSON；usage JSON（token/耗时）；context_summary Text；created_at/updated_at BigInt **秒**。
索引：(chat_id,parent_id)、(model_id,created_at)、(user_id,created_at)、(chat_id,role,done)。

### chat_file（models/chats.py）
id PK；user_id；chat_id FK CASCADE；message_id；file_id FK→file CASCADE；created_at/updated_at 秒；UNIQUE(chat_id,file_id)。

### shared_chat（models/shared_chats.py）
id PK=share token UUID；chat_id FK CASCADE；user_id；title；**chat JSON（分享时快照）**；created_at/updated_at 秒。

### tag（models/tags.py）
复合 PK (id,user_id)；id=slug（name 空格→下划线、lower；legacy 行有 md5 样式）；name idx；user_id idx；meta JSON。

### message（models/messages.py，频道消息）
id PK UUID；user_id；channel_id；reply_to_id（线程）；parent_id（legacy）；is_pinned Bool；pinned_at BigInt **纳秒**；pinned_by；content Text；data JSON（含 files）；meta JSON；created_at/updated_at **纳秒**。

### message_reaction（models/messages.py）
id PK UUID；user_id；message_id；name（emoji 名）；created_at **纳秒**。

### channel（models/channels.py）
id PK UUID；user_id；type `group|dm`；name；description；is_private Bool；data/meta JSON；created_at/updated_at **纳秒**；updated_by、archived_at（ns）、archived_by、deleted_at（ns）、deleted_by（软删）。

### channel_member（models/channels.py）
id PK UUID；channel_id；user_id；role；status；is_active 默认 true；is_channel_muted/is_channel_pinned 默认 false；data/meta JSON；invited_at/invited_by/joined_at/left_at/last_read_at（**纳秒**）；created_at/updated_at（ns）；时间字段均可空。

### channel_file / channel_webhook（models/channels.py）
channel_file：id、user_id、channel_id FK CASCADE、message_id FK→message、file_id FK→file、created_at/updated_at（ns）；UNIQUE(channel_id,file_id)。
channel_webhook：id、channel_id、user_id、name、profile_image_url、**token（webhook 密钥）**、last_used_at、created_at/updated_at。

### folder（models/folders.py）
id PK UUID；parent_id（嵌套）；user_id；name；items JSON（legacy）；meta/data JSON；is_expanded Bool；created_at/updated_at 秒。

### note / pinned_note（models/notes.py）
note：id PK UUID；user_id；title；**data JSON = `{"content":{"md":"<markdown>"}}`**（sanitizer 强制 md 为字符串）；meta JSON；created_at/updated_at **纳秒**。
pinned_note：id PK UUID；user_id；note_id FK CASCADE；created_at（ns）。

### knowledge / knowledge_directory / knowledge_file（models/knowledge.py）
knowledge：id PK UUID；user_id；name；description；meta JSON（legacy access_control 可能在内）；created_at/updated_at 秒。
knowledge_directory：id；knowledge_id FK CASCADE；parent_id 自引用 FK；name；user_id；时间秒；UNIQUE(knowledge_id,parent_id,name)。
knowledge_file：id；knowledge_id FK CASCADE；file_id FK→file CASCADE；directory_id FK SET NULL；user_id；时间秒；UNIQUE(knowledge_id,file_id)。（内容在 file 表 + 向量库，本表无内容列。）

### prompt / prompt_history（models/prompts.py）
prompt：id PK **UUID**（legacy PK 曾是 command）；command UNIQUE idx（`/name`）；user_id idx；name；content（模板正文）；data/meta/tags JSON；is_active Bool 默认 true；version_id（→prompt_history）；时间秒。
prompt_history：id PK；prompt_id idx；parent_id；**snapshot JSON（完整 PromptModel 快照）**；user_id；commit_message；created_at 秒。

### model（models/models.py）——工作区模型预设
id PK（可为 `base_model_id:hash`，可覆盖内置 id）；user_id；base_model_id；name；**params JSONField**（temperature 等，extra=allow）；**meta JSONField**（profile_image_url/description/capabilities{web_search,image_generation,code_interpreter,memory,file_upload,file_context,citations,builtin_tools,terminal}/knowledge/tags/**filterIds（函数 id 列表）**/builtinTools…，extra=allow）；is_active Bool 默认 true；时间秒。

### tool（models/tools.py）
id PK slug；user_id idx；name；**content Text = Python 源码（legacy，本项目不执行）**；specs JSONField（OpenAI tool specs）；meta JSONField；**valves JSONField（可能 Fernet 加密字符串）**；时间秒。

### function（models/functions.py）
id PK；user_id idx；name；type `filter|pipe|action`；content Text（Python 源码，legacy）；meta/valves JSONField；is_active 默认 false；is_global Bool（全局 filter 附着所有聊天）idx；时间秒。

### skill（models/skills.py）
id PK；user_id；name UNIQUE；description；content Text（markdown 技能定义）；meta JSON；is_active 默认 true；时间秒。

### file（models/files.py）
id PK UUID；user_id idx；hash（sha256）；filename；path（本地 data/uploads 或 s3://）；data JSON（可含 `content` 全文）；meta JSON（legacy `context` chunks、tags、**collection_name**）；created_at idx/updated_at 秒。**字节在存储后端（本地/S3/GCS/Azure），不在 DB。**

### memory（models/memories.py）
id PK UUID；user_id idx；type 默认 'context' idx；path；content Text；meta JSON；时间秒；covering index (id,user_id)。向量集 `user-memory-{user_id}`。

### group / group_member（models/groups.py）
group：id PK UUID；user_id；name；description；data JSON（legacy user_ids 列表）；meta JSON；**permissions JSON = 嵌套布尔 dict**（现代式，如 `workspace.models`、`features.web_search`；legacy ≤0.5 为大整数位掩码，读取须兼容，`fill_missing_permissions` 补缺省）；时间秒。
group_member：id；group_id FK CASCADE；user_id；created_at/updated_at 可空秒；UNIQUE(group_id,user_id)。

### access_grant（models/access_grants.py）
id PK UUID；resource_type `knowledge|model|prompt|tool|note|channel|file|folder|calendar|skill|automation`；resource_id；principal_type `user|group|anyone`；principal_id（user_id/group_id/`*`）；permission `read|write`；created_at 秒；UNIQUE 五元组 `uq_access_grant_grant`。

### config（models/config.py）
key Text PK；value JSON（任意 JSON 值）；updated_at 秒。约 200 个点分键（`ui.*`、`auth.*`、`user.permissions`、`model.*`、`rag.*`、`audio.*`、`ollama.*`、`openai.*`…）。机制：env 只做**首次 seed**，之后 **DB 值优先**；`ui.default_models` 是 CSV 字符串、`auth.jwt_expiry` 是时长字符串（如 `30d`）等特殊形态要原样保留。

### feedback（models/feedbacks.py）
id PK UUID；user_id；version BigInt 默认 0；type（rating/vote）；data JSON（rating/reason）；meta JSON；snapshot JSON（聊天/消息快照）；时间秒。

### oauth_session（models/oauth_sessions.py）
id PK UUID；user_id idx；provider；**token Text = Fernet 加密的 JSON 字符串**（`access_token/id_token/refresh_token`，密钥 `OAUTH_SESSION_TOKEN_ENCRYPTION_KEY` 默认 WEBUI_SECRET_KEY）；expires_at 秒；时间秒；每用户上限 10。

### automation / automation_run（models/automations.py）
automation：id PK UUID；user_id；folder_id；name；**data JSON = {prompt, model_id, rrule}**；meta JSON；is_active 默认 true；last_run_at/next_run_at **纳秒** 可空；created_at/updated_at **纳秒**；idx next_run_at、(user_id,folder_id)。
automation_run：id；automation_id；chat_id；status `success|error`；error；created_at（ns）。

### calendar / calendar_event / calendar_event_attendee（models/calendar.py）
calendar：id PK UUID；user_id；name；color；is_default；data/meta JSON；时间**纳秒**；idx(user_id)。
calendar_event：id；calendar_id；user_id；title；description；start_at/end_at（**纳秒**）；all_day；**rrule Text（RFC5545）**；color；location；data/meta JSON；is_cancelled；时间 ns；idx(calendar_id,start_at)、(user_id,start_at)。
calendar_event_attendee：id；event_id；user_id；status `pending|accepted|declined|tentative`；meta JSON；UNIQUE(event_id,user_id)。

### 遗留表（仍存在于 head schema，不使用）
`document`（旧 RAG）、`chatidtag`（已迁移至 tag）、`config_old`（迁移残留）。**bootstrap DDL 必须包含它们**（表数 = 43 的断言测试在 rc-db）。

## 4. chat.chat JSON blob 结构（前端契约）

```json
{
  "title": "…",
  "models": ["model-id", …],
  "history": {
    "messages": {
      "<uuid>": {
        "id": "<uuid>", "parentId": "<uuid|null>", "childrenIds": ["…"],
        "role": "user|assistant|system",
        "content": "…", "contentType": "…",
        "reasoning": "…", "sources": […], "files": […],
        "statusHistory": […]
      }
    },
    "currentId": "<uuid>"
  },
  "messages": [ /* 扁平化消息（旧字段，仍会写出） */ ]
}
```
- 树结构：编辑用户消息=新增子分支；`showMessage` 沿 childrenIds 走到叶。
- 临时聊天 id 前缀 `local:`/`temporary:`（不持久化）；频道聊天 id `channel:{id}`。
- 规则：**chat_message 表为准**，blob 惰性同步（`upsert_message_to_chat_by_id_and_message_id` 语义）。

## 5. 鉴权格式

- **JWT**：HS256；secret=`WEBUI_SECRET_KEY`（env 或 `.webui_secret_key` 文件）；claims `{id, exp, iat, jti}`（exp/iat 为 epoch 秒整数）；过期=配置 `auth.jwt_expiry`（时长字符串）。
- **Cookie**：名 `token`，httponly，samesite=lax（可配），secure 默认 false，max_age 与 JWT 对齐。登出清除 `token`/`owui-session`/`oui-session`/`oauth_id_token`/`oauth_session_id`。
- **提取顺序**：`Authorization: Bearer <jwt>` → cookie `token` → `x-api-key`（或 `CUSTOM_API_KEY_HEADER`）。`sk-` 开头按 API key 处理。
- **密码**：默认 bcrypt（cost 12，输入 72 字节截断）；`PASSWORD_HASH_ALGORITHM=argon2` 可选；verify 按 `$argon2` 前缀自动识别。
- **API key**：`sk-` + 32 hex，明文存储（唯一索引）。
- **signup**：首个用户自动 admin 并置 `ui.enable_signup=false`；后续用户角色=`ui.default_user_role`；`pending` 用户无法通过 `get_verified_user`。
- **WEBUI_AUTH=false**：固定 `admin@localhost`/`admin`。
- **OAuth**：google/microsoft/github/oidc（+feishu）；client secret Fernet 加密存 config；路由 `/oauth/{provider}/login|callback`；用户匹配 `user.oauth[provider].sub` → email 合并（`OAUTH_MERGE_ACCOUNTS_BY_EMAIL`）→ signup（`ENABLE_OAUTH_SIGNUP`）。token 持久化在 oauth_session（加密）。（本项目 M7）
- **SCIM**：`/api/v1/scim/v2`，Bearer `SCIM_TOKEN`。（M7）

## 6. Realtime 协议（原版 socket.io → 本项目 WS+SSE 的语义对照）

原版：python-socketio，挂 `/ws`，path `/ws/socket.io`，命名空间 `/`；连接认证 `auth:{token}`；房间 `user:{id}`、`channel:{id}`、`doc_note:{id}`（Yjs）。

服务端→客户端事件（本项目需保留语义）：
- `events`（聊天管线主通道）：`data.type` = `status|message|replace|embeds|files|source|citation|notification|chat:completion|chat:message:delta|chat:message|chat:message:files|chat:message:follow_ups|chat:message:error|chat:title|chat:tags|chat:reload|chat:active|chat:outlet|chat:tasks:cancel|response:completion|context_compaction`；负载 `{chat_id, message_id, data, user?}`。
- `events:channel`：频道消息/typing/已读；`ydoc:*`（笔记协作，M7）；`error`。

客户端→服务端：`user-join`、`heartbeat`、`usage`、`join-channels`、`join-note`、`events:channel`、`events:chat`（last_read_at → 更新 chat.last_read_at 并广播 `chat:list`）、`ydoc:*`。
RPC（`sio.call`）：`execute:python`（pyodide）、`execute:tool`（浏览器侧工具）、`request:chat:completion`（直连模型）——本项目用 WS request/response 对实现。

**本项目映射**：单一 WS 连接（`/ws`）+ 房间语义不变；聊天增量走 WS `events`；SSE 仅作为只读兜底；RPC 用带 `reply_to` 的请求帧。事件名保持与原版一致以便前端语义与测试对拍。

## 7. REST API 面（≈250 端点，兼容目标）

挂载（原版 main.py）：`/api/chat/completions`（管线入口，JSON task envelope + WS 增量）、`/api/chat/completed`、`/api/chat/actions/{id}`、`/api/config`、`/api/models[?base=]`、`/api/v1/{auths,users,chats,folders,channels,notes,knowledge,models,prompts,tools,functions,skills,memories,groups,files,tasks,configs,audio,images,retrieval,evaluations,analytics,utils,terminals,automations,calendars,notifications,pipelines}`、`/ollama/*`（代理）、`/openai/*`（代理）、`/health`。

完整逐端点清单由前端调用方逆向（`open-webui/src/lib/apis/*`），实现每个模块时先写「端点契约测试」（JSON 形状对拍），再写实现。各模块细节在对应里程碑的模块文档中落地，不在此预填。

## 8. 向量库与文件存储（SQL 之外）

- 原版向量默认嵌入式 Chroma（`DATA_DIR/vector_db`，私有 sqlite+HNSW 格式）。**本项目不读该格式**：提供重索引工具（从 SQL 内 `file.data.content` / 原文件重新抽取+切分+嵌入）。
- 集合命名：`file-{file_id}`、knowledge id、`user-memory-{user_id}`、`web-search-{user_id}-{hash≤63}`。
- 本项目后端：sqlite-vec（默认内置）/ Qdrant / pgvector。
- 文件字节：本地 `data/uploads/{filename}` 或 S3/GCS/Azure（`file.path` 指向）。

## 9. 已知陷阱清单（实现时逐条对照）

1. 时间戳单位混用（§2）——类型强制。
2. `JSONField` 恒 TEXT vs 泛型 JSON（§1）——实体映射必须逐列区分。
3. 权限双表示：access_grant 表 + meta.access_control JSON + legacy 位掩码（§0.5、group.permissions）。
4. valves/oauth token 的 Fernet 加密形态（dict 与加密字符串并存读取）。
5. `user.settings` 合并语义（深合并不可覆盖整对象）。
6. config DB 值优先于 env（env 仅 seed）。
7. orjson 序列化怪癖容忍。
8. blob 与 chat_message 双写一致性。
9. email 唯一性 = lower(email) 函数索引；SQLite 侧原版注册自定义 like() 函数（我们兜底 `lower() LIKE`）。
10. 首用户 admin 的原子性（insert 后 `get_num_users()==1` 检查 + 置 enable_signup=false）。
