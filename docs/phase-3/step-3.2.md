# 步骤 3.2（S3-02）：项目级 AI 默认值编辑与应用

状态：**通过（项目 AI 默认值编辑、持久化和设置来源解析范围，2026-11-03，Asia/Shanghai）**。本步完成项目默认的读取、编辑、保存、清除继承、审计、修订冲突和失效提示。它没有实现会话/任务创建或真实任务入口；M3、任务启动和网关仍未验收。

实际负责人：本次 Codex 会话，依据用户“根据文档要求，完成步骤 3.2”的授权。实现与验证日期：2026-11-03。测试使用本地临时数据库、登记项目目录及合成 profile/能力/凭据状态；没有读取或保存真实 Key，没有模型请求，也没有探测/读取 cc-switch。

## 实现

- `src-tauri/migrations/0006_project_ai_defaults.sql` 从 v5 追加 schema v6。`project_settings_revisions` 为每个项目维护单调修订，即使移除 AI 覆盖后也不复用旧修订；升级时按既有项目设置修订回填。审计表新增 `project_ai_defaults_update`，旧审计事实被原样保留。迁移前仍执行存储服务的一致性备份，v1–v5 SQL 与校验和未改写。
- `project_settings_update` 是新的 main-only IPC 命令。它只接受一个完整 `AiSelection` 或 `null`（清除项目覆盖并继承全局值），未知字段、缺字段、secret 字段和其他配置键均拒绝。项目读取继续使用 `settings_get` 的 project scope。
- Rust 使用 `BEGIN IMMEDIATE`，在同一事务内核对项目登记、授权与目录身份；比较项目设置修订和全局设置修订；用现有 provider 校验重新核验自管非官方 profile、profile 修订、启用/凭据状态和当前模型能力；然后更新项目 AI 默认、修订和最小审计事实。SQL、审计或提交失败会回滚本次设置变更。审计仅记录固定事件、项目 ID、服务端 correlation ID 和时间，不记录 Key、模型选择值或项目路径。
- 项目详情页现在可选择完整项目 AI 默认，也可清除项目覆盖以继承全局默认。仅展示应用自管 relay profile；不支持的官方/外部 profile 不会被激活。页面不会读取或回显 secret。修订冲突后页面重读权威设置并要求用户重新编辑提交。
- 任务配置解析沿用完整来源优先级“本次选择 > 项目默认 > 全局默认”，不跨来源拼接。保存过的项目默认如果其 profile 修订、凭据状态或能力失效，会保留项目来源并明确报错/提示，不静默回退到全局。任务启动时的最终身份、凭据和配置复核仍属于后续 3.3/3.4。

## 验证

已运行 `npm run build`、`npm run test:ipc`（36/36）、`cargo fmt --check --manifest-path src-tauri/Cargo.toml`、`cargo test --locked --manifest-path src-tauri/Cargo.toml --lib`（164 passed、6 ignored）及 `cargo check --locked --release --manifest-path src-tauri/Cargo.toml`。新增 Rust 覆盖包括 schema v5→v6 修订回填与备份、项目 AI 默认保存/清除后的单调修订、独立进程重新打开数据库、项目/global 修订冲突、profile 修订/能力/凭据拒绝，以及 SQL 和审计注入失败回滚。已运行的独立进程测试只读取合成设置并确认持久化。

真实 Windows WebView2 的 `built` 和 `dev` 项目桌面场景均通过，报告为 `artifacts/shell-built-projects/report.json` 和 `artifacts/shell-dev-projects/report.json`。桌面用例覆盖页面选择/保存/清除继承、两个修订冲突、未知 profile、能力缺失、凭据状态无效、profile 修订失效后的不回退行为、未知 secret 字段拒绝、审计/SQL 失败整批回滚，以及 overlay/未知本地窗口/远端窗口拒绝新命令。`npm run verify:projects` 完整通过，`artifacts/phase-2/report.json` 的 `passed=true`；包括 Rust 164 passed、IPC 36/36、独立存储回归、built/dev 项目桌面和设置/日志/凭据/隔离/provider 阶段 1 回归。该报告仍按 S2-09 标记，只用作本步的综合回归证据。

本步自动化没有启动真实任务或 Codex 子进程，也没有证明任务实际请求与配置快照一致。任务启动前对项目身份、凭据物理可用性、能力证据期限和单一配置来源的复核必须在 3.3/3.4 实现并测试；不得把本步的配置编辑或合成桌面场景视为真实任务通过。硬件、原生选择器、IME 等既有人工验收仍沿用原阶段边界。
