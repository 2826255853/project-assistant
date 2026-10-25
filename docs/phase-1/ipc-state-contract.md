# 正式 IPC 与状态事件契约 v1

日期：2026-10-13（Asia/Shanghai）。覆盖步骤 1.2 实际注册接口；后续业务接口继续按步骤 0.6 草案随服务实现接入。

## 命令与授权

所有正式命令使用 `{request:{apiVersion:1,params:...}}`，返回 resolved `Response<T>`。空参数必须是 `{}`；额外字段、错误类型、未知 action、前端伪造 correlationId 和错版本在副作用前拒绝。缺少整个 request、未注册命令、Tauri ACL 拒绝属于 transport reject，由调用封装转为固定 `PA_IPC_TRANSPORT_FAILED`，不回显原始原因。

| 命令 | main | overlay | params / 返回 |
| --- | --- | --- | --- |
| desktop_initialize | 允许 | 拒绝 | `{}` / `{mode:normal或phase0}` |
| app_get_status | 允许 | 允许 | `{}` / `AppStatus` |
| credential_put | 允许 | 拒绝 | `{profileId,expectedRevision,secret}` / `{credential:{status,revision},configRevision}`；无读取 secret 命令 |
| credential_remove | 允许 | 拒绝 | `{profileId,expectedRevision}` / 同上 |
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
## 步骤 1.4 设置契约补充（2026-10-13）

`settings_get({scope:"global"})` 与 `settings_update({scope:"global", expectedRevision, changes})` 已开放给本地 main，沿用 IPC v1 信封、关联 ID 和固定错误。overlay 无完整设置权限，启停仅通过既有受限动作进入同一后端事务。项目范围暂因缺少阶段 2 授权入口拒绝。

返回 `SettingsView={revision,values}`，values 完整包含 search_engine、editor、project_sort、approval_policy、log_level、summary_retention_days、ai_defaults、overlay_enabled、overlay_shortcut、overlay_position。editor 为 null 或 `{id:"custom",executable}`；ai_defaults 目前仅 null；位置为 null 或 `{x,y,monitor,scaleFactor}`。摘要期限不会开启采集。

expectedRevision 作用于整个全局设置批次；未知字段/键、重复键、非法值、冲突或 SQL/快捷键注册失败均拒绝整个事务。成功递增修订并发送无载荷 app_state_changed；位置实际变化也递增，故编辑表单必须保留旧修订并处理冲突。PA_CONFLICT_REVISION 要求用户先重读再提交；不自动覆盖。

错误：PA_SETTINGS_INVALID_VALUE、PA_SETTINGS_CORRUPT、PA_SETTINGS_STORAGE_FAILED、PA_SETTINGS_RECOVERY_FAILED、PA_SETTINGS_AI_UNAVAILABLE、PA_CONFLICT_REVISION，以及已有 IPC/快捷键错误。状态服务 settings 已可 ready；损坏/启动失败返回 unavailable，不回显错误输入、SQL 或原始异常。

## 步骤 1.6 凭据契约补充（2026-10-19）

凭据命令仅本地 main 授权，Rust 再校验真实 label/URL。`expectedRevision` 是 profile 的 **config revision**；credential revision 随成功 put/remove 递增，初始为 0。Key 为非空、无 NUL、最多 1800 UTF-8 字节；超过 24 KiB 的 put 输入在解析前拒绝。返回只含凭据状态、credential revision 和 config revision，严格拒绝额外字段；不存在读取 Key 的 IPC。

修订冲突先重新读取 profile（读取服务随 1.8 接入），由用户再次确认提交；凭据变更不自动重发。占用自有任务时由 blocking worker 等待空闲，状态查询仍可响应；5 分钟未空闲返回 `PA_CREDENTIAL_BUSY`。客户端超时/停止等待不取消服务端操作。成功变更及服务不可用发布既有无载荷状态失效事件，完整 Key 不进入事件或诊断。

固定错误：PA_CREDENTIALS_UNAVAILABLE、PA_CREDENTIAL_INVALID、PA_CREDENTIAL_PROFILE_NOT_FOUND、PA_CREDENTIAL_REVISION_CONFLICT、PA_CREDENTIAL_BUSY、PA_CREDENTIALS_STORAGE_FAILED；审计提交失败为 PA_AUDIT_WRITE_FAILED。失败补偿及验收范围见 `step-1.6.md`；本步骤通过不开放 AI 配置页或真实任务。

## 步骤 1.8 配置服务补充（2026-10-22）

九个正式 profile/operation 命令仅本地 main capability，Rust 再核验真实窗口；严格 IPC v1，不开放 task_start/快照写入。请求的 effort、profileId/expectedRevision、defaultModel/defaultEffort 可以 null，但仍必需。

provider_models 是隔离 CLI 目录，最长 24 小时，修订/适配器变化失效，不产生 ready。test 主动请求，仅证明所选完整组合；switch 为最多八项 FIFO，receipt 不是成功。五分钟证据绑定完整选择、profile/凭据 revision、实际强度及能力/适配器版本。operation_get 为权威状态，失联不重发；unchanged/rolled_back/rollback_failed 在结果内区分。

事件继续无载荷 app_state_changed，不广播配置/Key，查询 providers_list/operation_get 恢复。configState 接受 ConfigState，providers ready 不表示任务 ready。settings_get.ai_defaults 可返回完整选择，非空只由 provider_switch 提交；settings_update 拒绝非空直写、允许清空。旧“默认仅 null”“provider 未注册”均为历史步骤状态。

启动不连接/重放，默认 degraded 并清旧证据；应用/恢复中断保持 rollback_failed、active=null、blocked。用户主动重新测试/切换或清默认，初始化恢复失败不开放 providers。见 [1.8 记录](step-1.8.md)。
