# 步骤 0.6：IPC 契约 v1 草案

日期：2026-10-11（Asia/Shanghai）。适用：当前 Windows x64 基线与阶段一服务实现。TypeScript DTO 见 `src/ipc/contracts.ts`，Rust envelope/首个命令见 `src-tauri/src/ipc.rs`。本文是应用内部 IPC；Codex wire 协议由 [0.3 契约](codex-contract.md) 单独管理，SQLite schema 版本由 [迁移 ADR](storage-migration-adr.md) 单独管理。

> 2026-10-13 步骤 1.2 更新：生产已实现的命令与状态/事件契约以 [步骤 1.2 记录](../phase-1/step-1.2.md) 及其正式契约为准。本文下方业务命令表仍是后续步骤草案，未注册、未授权的接口不可调用。`ImplementedCommandMap` 明确限定当前可调用的正式入口；早期 `app_get_status.codex` 安装探测草案由返回 `services` 不可用原因的实现替代。

## 职责与信任边界

| 层 | 职责 | 禁止事项 |
| --- | --- | --- |
| React / `src/ipc/client.ts` | 收集用户选择、呈现状态/审批、运行时校验 envelope，调用类型化服务入口；不持有可用于授权的状态 | 不读写配置/数据库/凭据文件，不启动进程，不根据按钮禁用状态判断权限，不使用 Codex wire 字段，不输出 IPC payload 到 console |
| Rust IPC handler | 检查调用窗口能力、解析严格 DTO/版本/输入上限、分配关联 ID、映射固定错误、调用服务 | 不接受前端传入 correlationId、任意 SQL、shell、环境变量或裸外部审批 request ID；不把库/插件异常直接返回 |
| Rust 服务 | 设置/仓储、路径授权、凭据解析、profile/revision 校验、不可变快照、状态机、超时取消、审批/审计、候选切换与回滚 | 不信任 React 传入的 cwd、Key 归属、模型能力、审批状态或运行中配置；不让 UI 修改快照 |
| 外部适配器 | Git、系统打开、Codex 与应用自管配置；参数数组、受控 cwd/env、协议解码、归属进程管理 | 不决定用户授权、不改历史、不停止外部任务；不读取/同步/写入 cc-switch 或用户共享 config/auth；原始输出不穿透普通 IPC |

当前 build.rs 为两个实验命令生成 Tauri app permissions，capability 仅允许本地 `main` 窗口；没有 remote capability、shell/fs 通用权限。**仅 invoke_handler 注册并且显式授权的命令可用**。生产每个新增命令同时更新权限清单和边界测试。未来审批窗口单独授权 `approval_decide`，不能继承凭据权限。CSP、Tauri capability 与 Rust 业务校验共同构成边界；capability 不代替项目授权。

## 编码、版本与返回

- 命令 snake_case，字段 camelCase，状态/类别 snake_case。时间 UTC Unix 毫秒；UI 按本地时区显示。未知/尚未产生为 null，不用空字符串替代。业务 ID 由服务层生成（正式实体 UUID）；外部 ID 不作授权凭据。
- `invoke(command, { request: { apiVersion: 1, params: ... } })`。Rust 使用 `deny_unknown_fields`，缺字段/错类型/多余字段返回 `PA_IPC_INVALID_REQUEST`；版本不等于 1 返回 `PA_IPC_VERSION_UNSUPPORTED`，任何副作用之前拒绝。缺少整个 request、命令不存在或 Tauri 权限拒绝发生在 transport 层，由客户端映射固定 `PA_IPC_TRANSPORT_FAILED`。
- 应用结果为 **resolved envelope**，业务失败不通过 Tauri 原始 reject 字符串。成功：`{apiVersion, correlationId, status:"ok", data}`；失败：`{apiVersion, correlationId, status:"error", error:{category,code,message,suggestion,retryable}}`。data/error 互斥，不返回 stack、rawMessage、stdout、stderr、完整配置或 Key。客户端 transport 捕获禁止 String(cause)/console(cause)。
- Rust 在入口生成进程实例+单调计数的 correlationId，同一请求日志、回复、关联事件一致；不是授权 token，不接受客户端指定。异步操作追加 operationId；任务事件追加 taskId。重启产生新实例。正式请求结果固定码与提示来源见 [错误规范](errors-and-logging.md)。
- API 版本独立于数据库版本、应用版本和 CLI 版本。v1 新增非必需返回字段允许旧客户端忽略；请求保持严格，旧客户端不发送未知字段。改变必需字段/枚举含义/权限则升主版本，提供显式兼容适配或拒绝；不能静默降级。未知事件类型丢弃并重新取状态，未知审批类型拒绝。DTO 改动必须同时更新 TS/Rust、文档与序列化测试；当前不依赖代码生成器。
- 当前首命令 `framework_probe` 限 4096 UTF-8 字节（trim 前），仅回显用户主动输入的实验消息及平台/架构；没有文件、网络、数据库或进程副作用。回显是 UI 功能，不进入日志。生产任务文本暂定 ≤64 KiB UTF-8、名称 ≤128 字符、endpoint/路径 ≤4096 UTF-8 字节、Key ≤8192 UTF-8 字节、设置批次 ≤16 项；Rust 收到后逐项校验，不能只靠前端限制。

## 阶段一最小命令清单

参数和结果的完整字段以 CommandMap 为准。除 framework_probe 外均是**待实现草案，当前未注册**；下表是实现范围与权限约定，不表示外部集成可用。

| 命令 | 参数 / 返回 | 权限、行为与验收要求 |
| --- | --- | --- |
| app_get_status | 空 → configState、Codex AdapterStatus、ccSwitch ManualCcSwitchStatus | ccSwitch 仅返回 management=user_manual、integration=disabled、readSupported=false、switchSupported=false；不检测安装/版本，不影响自管 configState；尚未注册 |
| settings_get / settings_update | global 或 projectId；update 带 expectedRevision 和类型化 changes → revision/values | 全局仅允许已有 key；项目必须已登记且授权；editor 只存规范化可执行路径，不含 shell 字符串；approval_policy 仅 ask；单事务并发修订冲突返回 PA_CONFLICT_REVISION |
| providers_list | 空 → profiles、active、configState | endpoint 先验证 HTTPS、无 userinfo/query/fragment/敏感 path；凭据只返回状态+revision，模型默认值与 availability 可见；不返回 credential_ref、完整 Key/尾部、外部 provider JSON |
| provider_save / set_enabled / delete | 新建/编辑带 expectedRevision，启停/删除带 profileId/revision → profile 或 removed | 仅应用自管 relay/Responses，adapterKind=self_managed；不读取/同步 cc-switch。历史引用阻止删除并建议停用；外部管理记录拒绝激活，返回 PA_PROVIDER_ADAPTER_UNAVAILABLE；修改递增 revision 并失效缓存 |
| provider_models | profileId/revision/refresh → capabilities | 每模型列支持 efforts、是否可用默认、来源/版本/过期；缓存不可保证远端可用；未知组合拒绝；切模型重新校验，不继承不兼容强度 |
| credential_put / remove | profileId/revision，put 唯一附带 secret → CredentialState/configRevision | **secret 只到 Rust 凭据服务**，不经过普通配置仓储/日志/任务提示；main 单独能力授权；系统凭据库未实现前不可对真实 Key 开放；无 get_secret、批量导出接口；删除/替换检查受影响任务并等待空闲 |
| provider_test / provider_switch | AiSelection(profileId, expectedRevision, modelId, effort) → OperationReceipt | 只作用于自管配置和自有进程；receipt 仅表示接收，后台可取消；凭据/生产隔离未验收时不可用；不依赖 cc-switch 自动能力；完整事务见下文 |
| operation_get / cancel | operationId → 最新状态/最终结果，cancel → accepted | 仅本应用拥有的操作；取消不是完成；已提交后 cancel accepted=false；失联先查状态，不自动重发变更 |

设置首次应用值：search_engine=bing、project_sort=recent、approval_policy=ask、log_level=info、summary_retention_days=30；editor 与 ai_defaults 未配置=null/无条目。revision 从服务端初始状态取得。`summary_retention_days` 只接受 1–365 的整数，保存摘要仍需独立显式开启，不因期限设置自动开启。settings_get 返回持久化值，缺项使用上述默认，不悄悄填入其他 provider 的值；expectedRevision 不相符拒绝整个批次。editor 的结构化 DTO 在仓储中编码为经服务校验的 value_text，不增加任意 JSON 设置入口。

## Codex 生命周期接口预留

阶段一建立类型和不可用提示，完整实现属于后续 Codex 工作流阶段；不为 0.6 构建正式对话产品。

| 命令 | 边界 |
| --- | --- |
| session_create(projectId) | 仅登记项目，服务层解析规范化 cwd；返回本地 sessionId；当前持久恢复未验证，不承诺通过外部 ID 恢复 |
| task_start(sessionId, selection, prompt) | 校验项目/源/revision/模型/凭据；任务开始前同事务固定快照；prompt 仅送所选网关，不入日志/审计；不接受 UI cwd 或 snapshot/Key |
| task_get / task_cancel(taskId) | 只访问本应用任务；cancel accepted=true 不等于 cancelled；按网关终态更新，断连不重发 task_start |
| approval_decide(approvalId, taskId, operationFingerprint, decision) | 后端查询 pending，匹配调用窗口、任务、完整操作指纹、未过期、未消费；一次消费，同步记录决定后应答外部协议；前端外部 requestId/approved 布尔不能授权 |

PublicSnapshot 只含 profile/endpoint/model/effort/revision/生效时间，routingScope 仅 owned_process；普通库 credential_ref 不返回。预留 shared_global/cc_switch 历史记录不可激活，由服务层拒绝并提示手动重建。前端不能用旧快照恢复凭据/任务；进入 running 必须有快照，未知审批默认 deny。

## 事件契约与恢复

统一本地频道 `project-assistant://event/v1`，仅 emit_to 授权 main（未来审批窗口专用）；不广播 secret。AppEvent 包含 apiVersion/eventId/streamId/sequence/occurredAt/correlationId/payload。单个服务进程 streamId 唯一，全局 sequence 从 1 单调增长；eventId 服务端生成。事件类型见 EventPayload：ai_config_changed、operation_progress/completed、task_status/delta、approval_required。operation_get 的 OperationView 规定：处理中 result/error 都 null；completed 必有结果且 error=null（切换失败/恢复结果在 SwitchResult 内）；failed/cancelled 必有固定错误且 result=null。operation_completed 同样仅允许 result/error 恰有一个非null，不同时返回二者或二者皆空。

先注册监听，再 invoke；任务/操作接受事件可能早于 receipt，按 ID 暂存并与 operation_get/task_get 合并。相同 streamId 中重复或旧 sequence 丢弃，sequence 缺口或 streamId 变化后重新查询相关状态；delta 不保证跨断连补齐，UI 提示内容可能不完整，不重新执行任务来补回复。每帧最多 16 KiB UTF-8，任务 delta 可分片；单任务 UI 内存缓冲限 1 MiB，超出显示截断提示，不能丢终态/审批。审批无安全展示摘要时默认拒绝。原始命令/文件 patch 若后续需要额外审阅，必须通过受授权只读详情服务并独立防秘密泄露，不复用日志字段。

## 连接测试、切换等待与回滚

1. 列表读取不等于 ready。候选先校验 profile/revision、endpoint/origin、凭据绑定、模型/强度；只从凭据服务解析选中站点 Key。官方源仅预留，不执行官方认证/请求/切换实测，不纳入验收。
2. provider_test 在自有隔离进程/home 中发最小连接请求；可以产生用量，UI 应明确显示。返回 testedRevision/时间、认证和选中 model/effort 接受结果；不宣称全模型远端能力。原配置不变；原始远端错误不回显。
3. provider_switch 为自管排队操作：checking → waiting_for_idle → applying → completed；默认值影响未来任务，既有任务继续用原配置。需重建进程或替换占用凭据时等待受影响自有任务空闲，不能停止用户其他任务。共享可变代理/项目覆盖或隔离无法核验时拒绝 PA_PROVIDER_SHARED_ROUTE_UNSAFE。
4. 队列采用 FIFO，最多 8 项；并发候选不覆盖正在测试的结果。锁内重新验证 expectedRevision、凭据 revision 和能力有效期；候选任何变化使测试失效，返回冲突。ready 连接测试证据有效期暂定 5 分钟且绑定完整 selection/revision，不沿用其他组合结果。
5. 仅保留应用旧配置的受保护恢复信息 → 等自有任务空闲 → 重建自有进程/会话 → 核验有效配置和实际路由 → 原子提交默认值/revision → 发布 applied。自有进程使用专属 home、环境允许列表；不继承外部认证/provider/代理覆盖，拒绝未核验项目配置覆盖；不改用户全局配置。
6. 测试失败且未写配置为 unchanged；部分应用失败必须恢复并核验旧配置为 rolled_back；恢复失败为 rollback_failed、active=null、configState=error，阻止新任务并提示手动恢复。不能在未经核验时仍显示 ready。SwitchResult 显式区分三类结果，error 仍遵循固定错误规范。
7. 排队或测试中取消清理自己的候选，结果 unchanged；进入 applying 后先完成原子提交或恢复。应用 profile/credential/revision 变化立即失效候选与缓存；external_change 仅表示自管配置遭外部改动/路由异常，不表示监控或同步 cc-switch；不静默换站。

这些仍是阶段一实现义务。ADR-0011 已改写 FR-AI-006，cc-switch 自动接口不再是阶段门；R-014 的自管生产隔离/路由拒绝、R-016 凭据库和完整恢复仍需实测。TypeScript 草案已收紧，Rust 业务命令尚未注册，不能把 DTO 当成生产边界。

## 首个服务命令完成定义

新增命令必须同时具备：严格 TS/Rust DTO/版本验证、app command manifest 与最小窗口 capability、服务端项目/修订/审批校验、固定错误建议、关联 ID 和白名单日志、输入限制/超时/取消（异步时）、失败不破坏原状态，以及成功/错误/权限拒绝/秘密不回显测试。framework_probe 已提供 envelope/日志样例；phase0_check_open_path/Opener 是 0.4 旧实验接口（字符串错误），生产第一服务不得复制其旧返回形式或两段式前端预检查。
