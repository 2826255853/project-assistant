# 步骤 3.4（S3-04）：自有进程任务网关与事件归一

日期：2026-11-04（Asia/Shanghai）。实际负责人：本次 Codex 会话。

状态：**通过（网关实现及真实 CLI/双合成 HTTPS 自动验收，2026-11-05）**。

本步把 3.3 的已提交任务接入应用自有进程边界。任务 IPC、审批最终决定和主窗口/overlay 仍未注册，分别属于 3.5–3.7；因此本记录不宣称 M3 已通过。

## 实现范围

- `src-tauri/src/owned_process/mod.rs` 新增内部任务网关：使用任务不可变快照的 profile、配置修订、凭据修订、endpoint、model、effort 和已复核 cwd 启动唯一自有 Codex 子进程；不读取 UI 参数，不继承共享配置、代理或 cc-switch 环境。
- 任务复用正式自有 stdio/HTTPS relay、应用专属 `CODEX_HOME`、Windows Job 和临时资源清理。启动前验证可执行文件哈希、项目配置层、cwd 身份、路由 origin 和配置文件哈希；运行中持续检查配置、项目身份、取消信号、超时和 relay 安全状态。
- 新增结构化 `TaskEvent`/`TaskRun` 归一边界。只输出启动、运行、回复增量、进度、审批请求、完成、取消和失败等受限事件；原始 JSON-RPC 帧、外部错误文本、Key、prompt 和 endpoint 细节不进入前端事件或日志。
- 任务输出限制为单帧 1 MiB、回复/进度累计 2 MiB、最多 10,000 个事件及每秒 120 个事件；超限、异常帧、事件序列不匹配、未知终态或路由漂移均失败并清理自有资源。
- `src-tauri/src/storage/tasks.rs` 增加任务启动复核、外部 session/turn 绑定、内容代次事件和默认拒绝审批登记；`src-tauri/src/tasks.rs` 增加内部 `TaskRuntimeState`、任务 worker、状态转换、事件发布、取消信号及退出时统一清理。
- `task_start` 仅在 3.3 原子事务成功后启动 worker；worker 启动失败会把已登记任务转为固定失败状态。`task_cancel` 先持久化 cancelling，再只取消应用自有 worker。应用退出时取消所有 worker，最多等待 5 秒，不触碰外部客户端。
- 审批请求已规范化并以默认拒绝方式安全结束，最终批准/拒绝、过期、单次 claim/ack 由 3.5 接管；当前不允许高风险操作因缺少审批服务而继续执行。

## 验收入口

- 离线边界：`cargo test --locked --manifest-path src-tauri/Cargo.toml --lib`，含事件归一和累计输出上限测试。
- 真实网关：`npm run verify:task-gateway`。脚本启动两个临时 TLS 合成站点，调用真实 Codex CLI 和 Rust 任务网关测试，覆盖 A/B 路由、重定向、项目配置漂移、超时、取消和不受信任 TLS，并检查请求模型/强度、prompt 到达、无跨站请求、输出无 Key、worker 清理和 owned 目录为空。
- 报告位置：`artifacts/phase-3-4/report.json`。报告只记录脱敏事件类别、计数、清理结果和失败代码，不写 prompt、回复、Key、路径或外部原始输出。

## 当前验证结论

已完成并复验：

- `cargo fmt --check --manifest-path src-tauri/Cargo.toml`；
- `cargo check --locked --manifest-path src-tauri/Cargo.toml`；
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib`（176 passed、8 ignored；忽略项为其他显式验收入口或独立进程 worker）；
- `npm run build`；
- `npm run test:ipc`（37/37）；
- `npm run verify:task-gateway`（真实 CLI、两个临时合成 HTTPS 站点，9 项检查全部通过）；
- 事件归一/累计输出边界 Rust 单测。

后续步骤继续验收：

- 审批最终决定/单次消费、取消竞态、配置切换等待和恢复失败，由 3.5/3.8 验收；
- built/dev WebView 任务闭环、overlay 反馈和人工 IME/托盘/多显示器边界，这些由 3.6–3.9 验收。

本次网关报告 `artifacts/phase-3-4/report.json` 的 `passed` 为 `true`：A/B 站点请求均匹配各自合成 Key、模型、effort 和任务 prompt；跨站重定向、项目身份失效、配置漂移、不可信 TLS、超时及取消按预期拒绝/终止，各场景自有进程清理成功。报告不含 Key 或 prompt 正文。此结论不代表桌面任务闭环、审批决定消费或 M3 整体通过。

本步未读取、导入、修改或切换 cc-switch；官方源仍为预留范围，不作为当前验收证据。

## 3.5 接续（2026-11-05）

上述默认拒绝全部审批和未开放决定接口为本步当轮边界；[步骤 3.5](step-3.5.md) 已接入 main-only 决定、单次消费确认和取消状态机。任务工具门与候选 probe 分开，适配器升为 v2；本文件历史验收不自动覆盖新增审批能力，当前证据分别见 `verify:approvals` 和网关回归。主窗口/overlay 与 M3 仍待 3.6–3.9。
