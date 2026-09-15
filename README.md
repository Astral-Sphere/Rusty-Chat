# Rusty-Chat

open-webui 0.11.3 的 Rust 全栈重写：axum 后端 + Dioxus WASM 前端，**数据库与 open-webui 双向兼容**（可直接打开既有 `webui.db` / Postgres 库）。

> 状态：M0 脚手架完成。进度见 [docs/PROGRESS.md](docs/PROGRESS.md)。

## 目标

1. **功能对齐** open-webui 0.11.3：聊天、多模型（Ollama/OpenAI 兼容）、RAG/知识库、文件、管理后台、用户组权限、Channels、Automations/Calendar 等。
2. **数据库完全兼容**：schema 停在 open-webui 的 alembic head `d4c1a8e37b62`；新建库写入相同 DDL 并盖同一版本戳，open-webui 自身也能打开。
3. **REST API 兼容**：路径与 JSON 形状对齐原版（约 30 模块 ≈250 端点），既有脚本/客户端可不改迁移。
4. 单二进制交付（前端 WASM 经 rust-embed 内嵌）。
5. 纯 Rust 技术栈，尽量少引入 JS 依赖（渲染栈全 Rust/WASM）。

## 快速开始

```bash
cargo check --workspace      # 编译
cargo test --workspace       # 测试（PG 门控测试见下）
cargo run -p rusty-chat      # M0: 打印版本；M1 起提供 serve
```

Postgres 兼容测试（可选，需要 fixture 容器）：

```bash
just pg-up                   # podman 起 postgres:17-alpine 于 127.0.0.1:5433
RC_TEST_PG_URL='postgres://postgres:fixture@127.0.0.1:5433/postgres' cargo test -p rc-db
```

## 仓库布局

```
crates/
  rc-core       共享类型：DTO、事件协议、错误、时间戳策略（秒/纳秒）
  rc-db         SeaORM 实体 + 仓库 + DDL bootstrap + alembic 版本守卫 + config 表引擎
  rc-auth       JWT(HS256)/bcrypt/argon2/sk- API key/Fernet 解密存量加密数据
  rc-llm        Ollama(ndjson)/OpenAI(SSE)/Responses API 流式状态机、模型注册表
  rc-rag        切分/embedding/hybrid 检索/rerank/向量库(sqlite-vec|qdrant|pgvector)/重索引
  rc-search     30+ 搜索引擎适配 + SSRF 安全 web loader（可独立发布）
  rc-media      STT/TTS/图像生成
  rc-realtime   WS hub（user:/channel: 房间）+ SSE + 浏览器侧 RPC
  rc-tools      MCP 客户端管理、builtin tools、工具调用循环
  rusty-tools   #[derive(Tool)] → MCP stdio server SDK
  rusty-chat    服务器二进制（CLI + axum 组装 + 内嵌前端）
web/            Dioxus 0.7 CSR 前端（独立 workspace，wasm32 目标）
docs/           架构/兼容性/测试/进度/决策文档（先读 PROGRESS.md）
docs/fixtures/  兼容性基准：真实 head schema 库与 dump
open-webui/     参考仓库（只读，git 忽略）
zed/            语法高亮参考（只读，git 忽略）
```

## 文档地图

| 文档 | 内容 |
|---|---|
| [AGENTS.md](AGENTS.md) | **协作者必读**：上下文压缩恢复协议、测试纪律、文档纪律 |
| [docs/PROGRESS.md](docs/PROGRESS.md) | 里程碑进度与下一步（每次交付后更新） |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 系统架构与 crate 边界 |
| [docs/COMPATIBILITY.md](docs/COMPATIBILITY.md) | 数据库/API 兼容性权威：全表 schema、时间戳、鉴权、事件 |
| [docs/TESTING.md](docs/TESTING.md) | 测试策略与覆盖要求 |
| [docs/DECISIONS.md](docs/DECISIONS.md) | 技术决策记录（ADR） |

## License

未定（见 docs/DECISIONS.md D-009）。当前未复制任何 open-webui 代码。
