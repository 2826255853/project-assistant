# 步骤 1.2（S1-02）验收记录

日期：2026-10-13（Asia/Shanghai）  
结论：**通过**。正式 IPC、状态查询、事件基础和主窗口/悬浮入口权限边界已实现，并在 Windows WebView2 开发资源模式与嵌入资源模式完成自动验收。

## 实现范围

- 统一 `apiVersion: 1` 请求、成功/失败 envelope、服务端 `correlationId` 和白名单日志；Rust DTO 使用严格字段校验，命令按 UTF-8 字节限制输入。
- 新增 `app_get_status`，返回桌面、IPC、事件、悬浮入口、存储、设置、日志、凭据、provider、Codex、项目和任务服务的明确状态。尚未实现的服务返回固定 `reasonCode`；cc-switch 只报告 `user_manual / disabled`，不检测安装、版本或外部配置。
- 主窗口和 overlay 均通过显式 capability 访问状态命令。服务端继续检查真实窗口 label 与 URL；远端页面、未授权窗口和 overlay 高权限操作均在副作用前拒绝。
- 新增 `project-assistant://event/v1` 状态失效事件。事件包含 `eventId`、`streamId`、单调 `sequence`、时间戳和关联 ID，不携带草稿、Key、任务正文或原始异常。前端先注册监听再查询状态，重复事件忽略，序列缺口、非法事件、快照竞态和 stream 重启会重新查询权威状态；监听销毁会注销 JS/native handler。
- overlay 状态、活动上下文和任务状态由服务层统一快照管理；草稿只在 overlay 查询时返回，主窗口和公共事件不会收到草稿。

## 验证结果

正式契约、授权矩阵和异步操作规则见 [IPC 与状态事件契约](ipc-state-contract.md)。

统一入口：

```powershell
npm run verify:state
```

| 检查 | 结果 |
| --- | --- |
| TypeScript/Vite 构建 | 通过 |
| Rust 发布配置编译检查 | `cargo check --locked --release` 通过；调试验收命令不注册 |
| Rust 锁定依赖测试 | 28 项通过 |
| IPC 与事件单元测试 | 11 项通过；版本、未知字段、边界、重放、缺口、stream 变化、取消、超时和卸载清理 |
| Codex 离线回归 | 通过 |
| 真实 WebView2 开发模式 | 通过；正式状态/错误往返、ACL、事件恢复、overlay 重建和远端页面拒绝 |
| 真实 WebView2 嵌入资源模式 | 通过；同上 |
| 日志检查 | 通过；仅固定字段和错误码，不包含测试 payload、草稿或合成秘密 |

报告：`artifacts/phase-1-2/report.json`。该目录为本机验收产物，不提交版本库。

## 边界

本步骤只接通状态和事件基础，不提前实现数据库、设置、凭据、provider、Codex 任务、审批或真实业务命令。它们由后续步骤按同一 DTO、capability、错误和事件规则接入；当前状态查询会明确返回不可用原因。

合成状态仅在调试构建设置 `PA_IPC_VERIFY=1` 时动态授权 `ipc_fixture` 给 main；发布构建不注册该命令。IPC 场景使用自己的临时常驻设置目录，不读取真实外部配置、不调用模型、不使用真实凭据。`streamId` 变化通过合成进程流重启注入验证，真实应用重启生成新流 ID 的逻辑另由 Rust 测试覆盖。步骤 1.1 的真实托盘菜单点击和多显示器人工验收仍待完成，本步骤不将它们记为通过。
