# 步骤 3.7（S3-07）：接入悬浮入口任务闭环

- **实际日期**：2026-11-06（Asia/Shanghai）
- **状态**：有条件通过（自动软件验收通过；物理 IME 与常驻硬件边界待人工复核）
- **实现范围**：悬浮入口已接入服务端活动上下文、任务提交、幂等对账、最小反馈、取消和主窗口跳转。主窗口仍是活动上下文的唯一写入者；overlay 不能提供任意项目、会话、profile、模型、cwd、审批正文、完整回复、差异或文件路径。

## 实现

1. 新增 `active_context_set`、`task_start_from_active`、`overlay_task_get`、`overlay_task_find_by_request`、`overlay_task_cancel` 五个正式 IPC 命令。命令已同步 TypeScript DTO/返回校验、Rust handler/build manifest、main/overlay capability 和固定错误目录。
2. 活动上下文保存在进程内权威状态中，带单调 `contextRevision`。上下文设置时重新复核项目授权、会话归属、项目/全局修订、profile/Key/能力和连接证据；配置、授权、路径或运行环境发生变化时 overlay 读取只返回无上下文，旧修订提交固定拒绝。
3. overlay 任务提交复用同一任务 admission、owned worker、审批和取消服务，任务记录保存 `origin_window=overlay`。`clientRequestId` 在 transport timeout 后只能用于原请求查询；服务端不会自动重发。提交接受后草稿清空，未确认时显示原请求查询入口。
4. overlay 反馈只返回任务状态、修订、512 UTF-8 字节以内回复预览、审批待处理布尔值、内容可用性和固定错误码。完整正文、进度、审批摘要、差异和路径仍通过主窗口读取。
5. overlay 的打开主窗口动作写入一次性 focus task 标识，主窗口随后加载对应任务详情；活动上下文切换时清除任务绑定和 focus 标识。Enter 发送受 IME composing 保护，Shift+Enter 换行，Esc 收起，草稿不跨进程持久化。

## 自动验证

- `npm run build`：通过。
- `npm run test:ipc`：40/40 通过，覆盖 overlay 命令目录、最小权限、活动上下文 DTO、固定错误和 invalidation 事件不携带任务内容。
- `cargo fmt --check --manifest-path src-tauri/Cargo.toml`：通过。
- `cargo check --locked --manifest-path src-tauri/Cargo.toml`：通过。
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib`：192 passed、10 ignored。
- `npm run verify:task-overlay`：新增 dev/built 真实 WebView 入口，覆盖无上下文阻断、活动上下文继承、IME guard、overlay 任务提交/真实 CLI 回复、审批提示与主窗口跳转、取消、上下文切换后的旧修订拒绝、overlay ACL、秘密/正文/原始协议字段哨兵检查。报告写入 `artifacts/phase-3-7/report.json`，临时目录退出时清理。
- 当前报告：`artifacts/phase-3-7/report.json`，`passed=true`；Node.js 24.18.0、npm 11.16.0、Rust 1.99.0、Codex CLI 0.167.0-alpha.3.2；真实 CLI 经双端点边界核验的合成 HTTPS 站点发出 12 个请求，Key 与目标站点绑定。dev/built 各完成 6 项 overlay 桌面检查，临时目录和凭据清理通过。

## 证据边界与遗留项

自动场景使用真实自有 CLI 和合成 HTTPS 站点；不使用真实 Key，不把官方源或合成回复记为官方源通过。overlay 只展示有界预览，任务正文仍为进程内缓存，schema 保持 v7，无新增迁移。

自动注入的 IME composing 保护通过；实际中文 IME 物理输入、托盘菜单、显示器拔插、多显示器位置和实时 DPI 仍需人工复核，这些边界不因 WebView 自动化通过而关闭。阶段 3.8 的运行中 profile/凭据切换、并发、崩溃/退出组合恢复仍待后续步骤。
