# 步骤 2.8（S2-08）：活动项目与设置读取边界

日期：2026-10-30（Asia/Shanghai）。

状态：**通过（本步服务、页面与 built/dev 合成桌面验收范围）**。实际实施与验收由本次 Codex 会话完成。阶段 2/M2 尚未整体通过。

## 实现

`AppStatus` 新增独立的 `activeProject: ActiveProject | null`，不借用 `ActiveContext`，也不创建会话、模型或任务。活动项目只存于当前进程；启动时为空，原有项目与 `project_usage` 记录照常恢复。主窗口的“设为活动项目”按钮调用新增 main-only `project_select`。Rust 在当前项目仓储中复核已登记授权、路径身份和可用状态；成功后写入选择使用记录，再发布既有 `project-assistant://event/v1` 无载荷状态失效通知。行焦点与键盘浏览仍只改变页面当前行，不会反复提交选择或更新最近使用。

活动状态与项目撤权/身份复核按同一进程锁顺序协调。成功移除当前活动项目、项目详情/选择/设置读取发现路径失效，或 `app_get_status` 重新核验发现当前活动目录失效时，服务清空活动状态并发出状态通知。项目切换、清空和恢复均不把项目 ID、路径或设置写进事件载荷。进程重启令活动项目回到空值，但不会打开目录或启动任务，项目的近期使用记录仍保留。

悬浮入口只从 `app_get_status` 重新读取 `activeProject` 的项目 ID 与名称；不授予其项目 IPC、设置读取或文件系统权限。悬浮入口返回的状态会隐藏 `ActiveContext`，不包含绝对路径、Git 摘要、项目设置或模型上下文。状态运行时校验同步覆盖新增字段。

现有 `settings_get` 接受 project scope 后，服务只对主窗口执行项目读取：在 SQLite 读取事务中重新核验项目登记、授权与目录身份；读取现行全局有效设置和项目允许列表中的七个结构化键，逐项返回 `source: project | global`。`revision` 表示该项目当前设置行修订（无项目覆盖时为 1），`globalRevision` 对应全局设置批次修订。读取不改变项目使用时间；`config_read` 审计和响应在同一事务内完成，审计失败不返回部分快照。`settings_update` 的 project scope 仍被拒绝；阶段 2 不提供项目设置写入，也不应用项目 AI 默认值。

Rust/TypeScript 响应校验、`ImplementedCommandMap`、Tauri command manifest、main capability 和白名单命令日志已同步。未追加数据库迁移，schema 仍为 v5。

## 验收

入口：`npm run verify:projects`。报告：`artifacts/phase-2-8/report.json`。本轮报告 `passed=true`，合成数据与临时目录范围。

- Windows 11 x64（10.0.26300），Node.js 24.18.0、npm 11.16.0、Rust 1.99.0。
- 前端 build、Rust fmt、release check、全量 Rust lib、IPC、离线 Codex 和 storage regression 通过。全量 Rust 为 **159 passed、5 ignored**；IPC 为 **34 passed**；存储回归为 **55 项**并通过独立进程重开。
- built/dev 真实 WebView 桌面均通过：主窗口显式选择后两窗口从权威快照得到一致的活动项目；overlay 只显示最小项目名称；项目设置页面显示每项项目值/全局继承来源；未登记项目拒绝读取，project scope 写入拒绝。
- 状态回归覆盖事件缺口、streamId 重建、overlay WebView 重载、调试夹具模拟进程状态重启、活动项目移除清空，以及访问检查发现目录失效后清空。重启后项目 `lastUsedAt` 仍保留；不发生自动目录打开。
- 存储测试覆盖项目设置来源、只读读取不写使用记录、`config_read` 审计关联项目 ID，以及审计插入故障时拒绝返回结果。项目源文件、Key、模型请求未参与本步验收。

## 遗留边界

当前验证的跨窗口重载、事件序列与 process-only 清空由桌面窗口和隔离调试夹具完成；未以重启后重新加载真实原生目录选择器的物理交互替代 2.9 人工点验。阶段 1 托盘、多显示器、热插拔和 DPI 人工项仍单列。ReFS 未实测，NTFS/TxF 保护不可用时系统打开继续拒绝。阶段 2 尚未包含任务/会话/审批、项目设置编辑与 AI 默认值应用；阶段 3 必须重新验证项目登记、授权、目录身份和选择来源。M2 与步骤 2.9 未通过。
