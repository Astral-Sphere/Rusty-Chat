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
- **2026-09-30 落地修正**：comrak 0.55 的 GFM/数学扩展全部是**运行时 Options**（`Options::extension` 字段），不是 cargo feature；default feature 含 syntect-onig（C 依赖，wasm 编译不过），wasm 侧必须 `default-features=false`。聊天语义开 `render.hardbreaks`（单换行即换行）。另：dx/manganis 不处理 CSS 内 `url()`，KaTeX 字体以 data URI 内联（`web/tools/gen_katex_inline.py`）。
- 日期：2026-09-15；2026-09-30 修正

## D-007 代码高亮用服务端原生 tree-sitter
- **决策**：后端 tree-sitter + highlights.scm（复用 zed `crates/grammars/src/*/highlights.scm` 查询文件 + crates.io tree-sitter crate，**不引入** zed 的 gpui/语言 crate）产出彩色 span 回填前端；流式期间纯文本。浏览器端备用 syntect(default-fancy)。
- **依据**：zed 研究结论——其高亮核心可分离但直接复用是"手术级"工程；查询文件才是高价值资产。
- 日期：2026-09-15

## D-008 数据库层用 SeaORM 2.0.3 + sqlx 0.9 逃生舱
- **决策**：SeaORM 实体/仓库（底层 sqlx ^0.9，无版本耦合）；特例（自定义 like、函数索引探测）走裸 sqlx。**不用** SeaORM 迁移：全新库写内嵌 alembic-head 终态 DDL 并盖戳；启动校验 `alembic_version == d4c1a8e37b62`，低于 head 拒绝启动。
- **理由**：40 表 CRUD 工作量、双方言由 sea-query 处理、泛型 JSON 列行为与 SQLAlchemy 一致。
- 日期：2026-09-15

## D-009 License 未定（但 AGPL 约束已解除）
- 2026-09-16 更新：`openai-interface` 0.11.0-rc1 起**改为 MIT**（用户宣布），链接它不再强制 AGPL。整体 License 仍待定（发布前决定），当前未复制 open-webui 代码（其 LICENSE 历史复杂），locale 仅取 zh/en 数据参考。
- 日期：2026-09-15 记录，2026-09-16 放宽

## D-010 v1 范围
- v1 含：核心聊天、Ollama/OpenAI、RAG/知识库、文件、管理后台、用户组权限、Channels、Automations、Calendar、中英 i18n。
- 后置（M7）：OAuth/LDAP/SCIM、Notes 协作（yrs）、Redis 多实例、分析面板、终端、OTel。
- 日期：2026-09-15

## D-011 OpenAI 等 Web API 以独立 crate 形式沉淀
- **决策**：OpenAI 兼容层基于用户维护的 `openai-interface`（Codeberg/Hammerklavier；0.11.0-rc1 起 MIT，edition 2024，deepseek/qwen feature）；Ollama 客户端、搜索 loader 等同样按可独立发布标准设计（无服务器状态耦合）。缺口薄封装并向 crate 作者（即用户）反馈需求。
- **注意**：M1 接入时核对流式 SSE 解析、Responses API、audio/images/embeddings 覆盖面。
- 日期：2026-09-15（用户主导修订）；2026-09-16 版本与许可更新

## D-012 上下文压缩恢复协议
- 对话上下文会多次压缩；每次压缩后必须先读 `docs/PROGRESS.md` → `DECISIONS.md` → `ARCHITECTURE.md`（涉兼容再读 `COMPATIBILITY.md`）→ `git log/status` 对账 → 相关代码，再继续。写入 AGENTS.md §1。
- 日期：2026-09-15（用户要求）

## D-013 zed highlights.scm 以 vendored 数据资产引入（T3）
- **决策**：zed 的 `highlights.scm` 查询文件是**数据**非源码，复制到 `crates/rc-highlight/assets/highlights/<lang>.scm`（附 NOTICE：来源、zed 许可 Apache-2.0、本地补丁说明），rust-embed/include_str! 编译期内嵌。与 AGENTS.md §4「不复制代码进源码」的调和：不进 `src/` 源码树、保留许可归属、变动可追溯。已在 M1-6 收官时经用户拍板。
- **落地记录**：12 语言可用（bash/c/cpp/css/diff/go/javascript/json/python/rust/tsx/typescript）。两处 fork 漂移补丁（cpp 去掉 C++20 module 模式、javascript 去掉 TS 混合节点）用 node-types.json 驱动的脚本化剔除；markdown 弃用（zed 查询依赖 fork 专属节点 `pipe_table` 等，与 crates.io tree-sitter-markdown 0.7.1 不兼容）；html 用 grammar crate 自带查询。
- **被否**：自写查询（22 语言工作量不可行且质量打折）；跳过 T3。
- 日期：2026-09-30

## D-014 Tailwind v4 standalone CLI（全程无 JS/无 Node）
- **决策**：CSS 构建用 Tailwind v4 standalone CLI（Rust 二进制，无 Node/npm/JS 运行时），`web/input.css`（CSS-first 配置 + `@source` 扫描 rsx 类名）→ `just web-css` 生成 gitignored 的 `web/assets/tailwind.css`。dx 0.7 **不会**自动运行 Tailwind（官方指南即手动并行进程），just 配方是唯一入口；`web-release` 依赖 `web-css`。markdown 排版手写 `.markdown-body` prose 块，不引入 JS 版 typography 插件。
- **依据**：产物是纯静态 CSS，浏览器零 JS 运行时；构建器本身是 Rust（Oxide），契合全 Rust 技术栈。原「推迟到 M2」在确认无 Node 依赖后由用户改为本轮引入。
- **代价**：生成物 gitignored——fresh clone 后必须先跑 `just web-css` 才能 `dx build`/`cargo check`（input.css 头注释已写明）。
- 日期：2026-09-30（用户拍板）

## D-015 前端 1:1 仿 open-webui 界面（仅深色 + 中文文案）
- **决策**：Dioxus 前端按 open-webui 0.11.3 的布局与视觉逐类名仿写（用户提供截图对拍）：245px 侧栏（新对话/搜索/置顶/日历时间分组/相对时间/条目 ⋯ 菜单/用户块）、42px 折叠栏、空态居中占位（模型名 + 圆角 3xl 输入卡 + 6 张默认建议卡）、聊天导航栏标题、用户消息右对齐 rounded-3xl 灰泡（bg-gray-850）、assistant 头像+模型名+动作行（复制/编辑/‹n/m›）、底部输入卡（模型下拉 pill + 圆形发送键）、滚动钉底按钮。**仅深色主题**（亮色后置）；界面文案硬编码中文（对齐 OWU zh-CN 翻译值）。
- **实现约束**：Tailwind v4 `@theme` 把灰阶整体替换为 OWU 的消色差色阶（oklch→hex 近似，扩展 gray-850=#262626）；body=#171717、面板=gray-950(#0d0d0d)；图标是 `web/src/icons.rs` 内联 lucide 风格 SVG（无资产/无 JS）；时间分组与相对时间是 `sidebar.rs` 纯函数（Hinnant civil_from_days，tz 偏移注入，host 测试）。事件处理遵循：**调 `.set()` 的闭包是 FnMut 不可多处分发**——多目标动作写成接收 Signal 拷贝的自由函数。
- **被否**：引入组件库/JS 运行时（违反 D-014）；亮色主题（工作量大、用户截图即深色）；原生 `select` 模型选择器（样式不可控）。
- 日期：2026-10-01（用户要求对照原版界面修改）
