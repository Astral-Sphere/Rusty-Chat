# AGENTS.md — 编码代理工作协议

本项目工程量巨大，对话上下文会被**多次压缩**，压缩后的摘要不完全可靠。因此为所有（人类与 AI）协作者定下以下强制协议。

## 1. 上下文压缩恢复协议（每次压缩后必做）

上下文被压缩/重启/接手他人工作后，**禁止直接动手写代码**。按顺序执行：

1. 读 `docs/PROGRESS.md` —— 当前里程碑、已完成/进行中/下一步。
2. 读 `docs/DECISIONS.md` —— 所有已拍板的技术决策，**不要重新讨论已定决策**。
3. 读 `docs/ARCHITECTURE.md` —— 系统结构与 crate 边界。
4. 涉及数据库/API 兼容时读 `docs/COMPATIBILITY.md`（schema、时间戳、鉴权格式的唯一权威）。
5. 用 `git log --oneline -20` 和 `git status` 确认实际代码状态与 PROGRESS.md 一致；不一致时**以代码为准**并修正文档。
6. 读完目标模块的相关代码后再继续推进。

## 2. 文档纪律（与代码同等级别的交付物）

- 每完成一个可交付单元，**必须同步更新** `docs/PROGRESS.md`（完成项打勾、写明关键文件路径）。
- 任何拍板的技术决策（选型、协议、格式、取舍）写入 `docs/DECISIONS.md`，含理由与被否方案。
- 兼容性事实（表结构、时间戳单位、字段格式、事件名）只记录在 `docs/COMPATIBILITY.md`，其他文档引用它，不要复制。
- 文档与代码冲突 = bug。修代码或修文档，不允许共存。

## 3. 测试纪律（强制，不可协商）

**测试必须全面详细，不允许漏掉任何可以想到的情况。** 具体要求：

1. **新功能先想全测试面再动手**：正常路径、边界值（空/零/极大/极小/UTF-8/emoji/超长）、错误路径（非法输入、权限不足、不存在、冲突）、并发与幂等、兼容性回归（对拍 open-webui 行为）。
2. 每个测试文件顶部注释说明覆盖矩阵（哪些情况覆盖了、哪些刻意不覆盖及原因）。
3. 兼容性代码的测试必须基于真实基准（`docs/fixtures/webui-head.db`、`docs/fixtures/*-head-schema.sql`），不允许凭记忆编造 schema。
4. 涉及数据库的测试同时覆盖 SQLite 与 Postgres（PG 用 `RC_TEST_PG_URL` 环境变量门控，未设置时打印 skip 而非 panic）。
5. `cargo test` 与 `cargo clippy --workspace` 必须全绿才能提交。
6. 修 bug 先写复现测试，再修。
7. 时间戳、权限、JSON blob 等高危兼容点：测试中必须显式断言单位/格式，不接受"看起来对"。

## 4. 仓库约定

- 依赖一律用当前最新稳定版（`cargo add` 解析，不凭记忆写版本）；版本变动在 PR/提交说明里注明。
- edition 2024、resolver "3"、rustfmt 默认配置；clippy 不允许 `#[allow]` 除非注释说明原因。
- `open-webui/` 与 `zed/` 是**只读参考**（已被 .gitignore 忽略）：可以读、可以跑其迁移生成 fixture，但**绝不复制代码进我们的源码**，也不修改它们。
- 提交信息用祈使句英文，标题 ≤72 字符；一个逻辑变更一个提交。

## 5. 快速命令

```
cargo check --workspace                 # 快速编译检查
cargo test --workspace                  # 全部测试（PG 测试需 RC_TEST_PG_URL）
cargo clippy --workspace --all-targets  # lint
just pg-up / just pg-down               # 起停 PG fixture 容器
just test-pg                            # 带 PG 的完整测试
```
