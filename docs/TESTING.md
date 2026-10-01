# TESTING.md — 测试策略与纪律

> 铁律（摘自 AGENTS.md，此处展开）：**测试必须全面详细，不允许漏掉任何可以想到的情况。**
> 测试面在写实现**之前**先列全；写完实现再补测试 = 违规。

## 1. 覆盖要求（每个功能单元）

每个功能/模块交付时，测试必须覆盖以下维度，缺一要在测试文件顶部注明原因：

1. **正常路径**：规范输入 → 期望输出。
2. **边界值**：空、零、单元素、极大/极小、超长字符串、深嵌套、Unicode（CJK/emoji/组合字符）、非法 UTF-8 替换、日期边界（闰年/月末/时区/纳秒精度）。
3. **错误路径**：非法输入、缺字段、类型错误、权限不足、资源不存在、唯一约束冲突、后端不可达/超时。
4. **幂等与并发**：重复提交、并发写同一行、重放、取消中断后的状态一致性。
5. **兼容性回归**（凡触及 DB/协议/API 形状）：与 open-webui 真实行为对拍——fixture 库（`docs/fixtures/webui-head.db`）、schema dump、或原版产生的样本 JSON。
6. **双方言**：SQLite 必测；Postgres 用 `RC_TEST_PG_URL` 门控（未设置 → `eprintln!` skip，不算失败）。两侧断言相同的业务语义。
7. **安全**：注入尝试（SQL/HTML/markdown 链接）、路径穿越（文件名/`file.path`）、SSRF（web loader 的私网/元数据地址拒绝表）、权限矩阵（pending/user/admin × 资源属主/组/公共）。

## 2. 测试分层

| 层 | 位置 | 基建 |
|---|---|---|
| 单元 | 各 crate `#[cfg(test)]` | 纯函数/状态机，无 IO |
| 集成 | `crates/*/tests/` | 真库（SQLite tmpfile / PG 门控）、fixture 文件 |
| 契约 | `crates/rc-db/tests/contract/` 等 | 对拍 open-webui：给定与原版相同的 DB 状态/输入，断言我们产出的 JSON 形状/写库效果一致 |
| 端到端 | M1 起新增 `tests/e2e/` | 起 rusty-chat（临时 DATA_DIR），HTTP/WS 客户端走完整流程；LLM 后端用本地 mock SSE/ndjson 服务器 |

## 3. 固定基准（不允许凭记忆编造）

- `docs/fixtures/webui-head.db`：open-webui 0.11.3 alembic 链产出的真实 head 库（43 表）。任何 schema 相关断言以此为准。
- `docs/fixtures/sqlite-head-schema.sql`、`postgres-head-schema.sql`：diff 基准（升级原版版本后重新生成并 review diff）。
- 时间戳单位断言一律用 `rc_core::timestamp::{Secs,Nanos}` + 显式量级区间（秒≈1.7e9，纳秒≈1.7e18）。

## 4. 测试文件模板要求

每个集成/契约测试文件顶部写覆盖矩阵注释：

```rust
//! 覆盖矩阵：
//! ✅ 正常：xxx
//! ✅ 边界：空标题、超长内容(1MB)、emoji 标题
//! ✅ 错误：不存在 id → NotFound；越权 → Forbidden
//! ✅ 幂等：重复 pin 两次
//! ✅ 双方言：SQLite + RC_TEST_PG_URL 门控 PG
//! ⛔ 刻意不覆盖：并发重命名竞争（由 DB 唯一约束兜底，见 issue #…）
```

## 5. 提交门槛

- `cargo test --workspace` 全绿；
- `cargo clippy --workspace --all-targets -- -D warnings` 零告警；
- `cargo fmt --check` 干净；
- PG 门控测试在本地至少跑过一次（`just test-pg`）；
- 新功能 PR 必须附覆盖矩阵，审查者按矩阵逐行核对。

## 6. 兼容性对拍方法（契约测试怎么写）

1. 用 open-webui（参考仓库 + /tmp venv）对 fixture 库执行目标操作（如创建聊天、打标签），导出结果 JSON/行。
2. 把该结果固化为期望值（含字段名、缺省字段、时间戳单位/量级、blob 结构）。
3. 我们的实现对同一初始状态执行同一操作，断言产物与期望值结构等价（时间戳允许量级断言，字段集合不允许多余/缺失——原版多余字段容忍、缺失即 fail）。

## 7. 当前测试资产（随里程碑更新）

- `rc-core`：timestamp 单位/区间/序列化/conversion（5 用例）。
- `rc-db::bootstrap`：fresh bootstrap、幂等、43 表计数、错误版本拒绝、legacy 无戳拒绝、真实 fixture 打开+读写、PG 门控全流程（7 用例）。

## 7. M1 测试加固后的实际布局（2026-10-01）

- **native**（`cargo test --workspace`）：190 项 —— rc-core 40 / rc-auth 28+3 / rc-db 18+9+13 / rc-highlight 21 / rc-llm 26 / rc-realtime 4 / rusty-chat 契约 21+doctest。
- **web**（`cd web && cargo test`，host 跑）：86 项 —— render 26 / branches 21 / sidebar 18 / chat 12 / highlight 5 / mermaid 4。
- **PG 双跑范围**：rc-db repo 全部 flow + bootstrap；rusty-chat 的 `contract_auth` 与 `contract_chats`（RC_TEST_PG_URL 门控，scratch 库自动建删）。其余契约文件 SQLite-only，扩展时按 contract_auth 的 `everywhere!` 模式补 PG 腿。
- **矩阵纪律**：本轮修正了 5 处矩阵虚报（jwt 错误算法、repo api_key 级联、render `\$`、ollama tags 500、auth 重复 email）——矩阵声明必须与测试一一对应，审查时逐行核对。
