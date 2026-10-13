# 正式 IPC 与状态事件契约 v1

日期：2026-10-13（Asia/Shanghai）。覆盖步骤 1.2 实际注册接口；后续业务接口继续按步骤 0.6 草案随服务实现接入。

## 命令与授权

所有正式命令使用 `{request:{apiVersion:1,params:...}}`，返回 resolved `Response<T>`。空参数必须是 `{}`；额外字段、错误类型、未知 action、前端伪造 correlationId 和错版本在副作用前拒绝。缺少整个 request、未注册命令、Tauri ACL 拒绝属于 transport reject，由调用封装转为固定 `PA_IPC_TRANSPORT_FAILED`，不回显原始原因。

| 命令 | main | overlay | params / 返回 |
| --- | --- | --- | --- |
| desktop_initialize | 允许 | 拒绝 | `{}` / `{mode:normal或phase0}` |
| app_get_status | 允许 | 允许 | `{}` / `AppStatus` |
| overlay_get | 允许 | 允许 | `{}` / `OverlayView`，main 的 draft 固定为空 |
| overlay_control | 窗口入口/开关/快捷键/退出 | 窗口入口/开关/私有草稿 | `OverlayAction` / `null` |
| framework_probe | 仅调试 phase0 | 拒绝 | 4096 UTF-8 字节消息 / 测试回复 |
| phase0_check_open_path | 仅调试 phase0 | 拒绝 | 旧阶段 0 受限实验接口，保留旧返回，不作为生产接口范式 |
| ipc_fixture | 仅调试且显式开启验收 | 拒绝 | 合成测试控制；发布不注册 |

overlay 不能退出应用、编辑快捷键、修复位置、访问 Key/profile 管理或审批。main 不能写入 overlay 私有草稿。两个窗口只允许监听/注销事件，拒绝 `emit`/`emit_to`；系统打开只在显式 phase0 的受限夹具 capability 中授权。所有 capability 都只授权本地窗口；Rust 同时校验真实调用窗口 label 和 URL，不接受前端声称的身份。任意窗口创建、shell、文件系统和通用退出均未授权。

`OverlayAction`：expand、collapse、enable、disable、open_main、exit、repair_position；set_shortcut 额外且仅允许 shortcut；draft 额外且仅允许 draft。草稿 ≤32768 UTF-8 字节，快捷键 ≤80 UTF-8 字节并解析组合格式。无效输入拒绝，不改变旧状态；快捷键占用返回 `PA_SHORTCUT_CONFLICT` 并保留原快捷键。

错误统一返回 category/code/message/suggestion/retryable，成功 data 与失败 error 互斥。关联 ID 由服务端生成，IPC 导致的事件使用同一 ID；托盘、快捷键和启动产生独立 ID。日志固定白名单字段 apiVersion/occurredAt/level/command/event/correlationId/outcome/errorCode，不记录请求、草稿、配置、Key 或原始异常。正式文件日志及轮转属于步骤 1.5。

## 状态快照

`AppStatus` 包含 apiVersion、streamId、sequence、configState、services、ccSwitch、activeContext、taskStatus、overlay。每个服务包含 id/state/reasonCode；ready 的 reasonCode=null，其余状态给出固定原因。生产 Codex/仓储/设置/日志/凭据/profile/项目/任务尚未接入，明确返回 unavailable；阶段 0 存储实验不代表生产 ready。

ccSwitch 固定返回 `{management:user_manual,integration:disabled,readSupported:false,switchSupported:false}`，不检测安装、版本或外部配置。activeContext 仅包括 projectId/sessionId/profileId/modelId/effort；当前生产为 null，调试验收使用合成值。taskStatus 当前仅 idle；真实任务和不变配置快照在对应后续阶段接入。overlay 的 draft 只有 overlay 查询可获取，主窗口、日志和事件不包含草稿。

## 事件与恢复

频道 `project-assistant://event/v1` 仅向 main/overlay 的 WebviewWindow 目标推送。事件 `{apiVersion,eventId,streamId,sequence,occurredAt,correlationId,payload:{type:app_state_changed}}`；序号从 1 单调增长，快照返回当前序号，时间为 UTC Unix 毫秒。当前事件为有界失效通知，不承载可执行操作或秘密；后续任务 delta/审批必须增加各自严格校验及权限。

窗口先注册监听，再取快照。收到新事件查询权威状态；重复或旧序号忽略，缺口、未知/非法事件或 stream 变化重新查询。查询期间有新事件则丢弃旧查询结果再查，不回滚较新快照；已纳入快照的事件及已退休 stream 的迟到事件忽略。组件卸载注销 JS/native listener，异步注册晚到也清理；窗口重建重新监听/查询并恢复同一服务上下文。事件不保证离线重放，不能靠重发任务恢复正文。

## 超时、取消与异步操作

统一调用默认等待 8 秒，允许调用方设置 timeoutMs 和 AbortSignal；已取消的 signal 不发请求。超时返回固定 `PA_IPC_TIMEOUT`，停止等待返回 `PA_IPC_CANCELLED`，底层异常转固定 transport 错误。超时/停止等待不等于服务端操作取消，已发出的变更不自动重发；调用方通过 app_get_status 查询实际状态。同步 overlay 控制返回最终 null；关闭窗口控制不创建虚假的 operationId。

未来长操作必须返回 OperationReceipt，真实取消通过 operation_cancel，receipt 的 accepted 仅代表收到；最终结果由 operation_get 查询及 operation_completed 事件确定。操作可超时/取消，仍需服务端清理或回滚后确定终态；断连不可重发 provider_switch/task_start。相关 DTO 保留在 CommandMap 中，当前没有注册或授权这些未实现服务；本步骤验证调用等待超时/取消以及权威查询恢复，不宣称真实后台长操作已通过。
