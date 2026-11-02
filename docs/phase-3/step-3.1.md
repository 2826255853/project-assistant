# 步骤 3.1（S3-01）：任务契约与功能门

日期：2026-11-02（Asia/Shanghai）。实际负责人：本次 Codex 会话。

状态：**通过（契约与未开放断言范围）**。本步交付任务/会话/审批 DTO、状态转换、错误/副作用边界、窗口权限草案、恢复/快照不变量及 M3 验收映射，并验证所有规划命令仍不具备调用通路。本步没有实现 Rust 会话/任务服务、存储或网关；没有追加迁移、注册命令或 capability；不表示阶段 3、M3 或真实任务通过。步骤 3.2–3.9 仍按计划实施。

基线按仓库当前可用环境核验：Windows x64；Node.js 24.18.0、npm 11.16.0、Rust 1.99.0、Cargo 1.99.0。实际 CLI、provider、数据库运行时、桌面任务及阶段 1/2 硬件/人工边界未在本步重新验收；上游证据仅沿用各自记录范围。

## 产物与范围

- `src/ipc/taskContracts.ts` 定义未来任务域 DTO 与 `TaskCommandMap`；`src/ipc/contracts.ts` 只把其接入未实现的 `CommandMap`，并以类型断言要求 `ImplementedCommandMap` 与任务命令集合无交集。
- `EventPayload` 仅保留无载荷 `app_state_changed`。任务正文、增量、审批详情不进入应用事件；任务状态/内容须由单独、授权、有界的查询获取。
- `scripts/ipc/tasks-contract.test.mjs` 静态核对 DTO 覆盖以及前端调用器、Rust handler/注册、build manifest 和两种 capability 中无任务命令。
- 本步不改数据库迁移、错误注册表、窗口权限、Rust 注册、UI 或服务。下一步实现具体命令时，必须同步 Rust DTO/运行时验证、固定错误、白名单日志、审计和真实调用窗口。

## 领域模型与权威性

Rust 服务是项目授权、目录身份/cwd、会话归属、配置解析/快照、任务/审批状态、游标和取消结果的唯一权威。前端提供项目/session 引用、一次性客户端请求 ID、用户输入与乐观修订；它不能提供授权标志、工作目录、有效凭据、审批状态或结果。项目活动选择/系统打开成功都不能代替任务启动时的项目登记、授权、稳定目录身份、会话归属与 cwd 复核。

会话属于一个项目且不能跨项目迁移。会话恢复是显式用户动作，只恢复会话上下文；不重启、重放或推断之前的任务/审批。项目撤权、身份变化或失效阻止恢复和新任务。普通关闭/隐藏窗口不等于取消任务；明确退出应用走本应用自有任务中止及进程清理流程。

任务以服务端生成 UUID 标识，持有一个完整、不可变、非 secret 的 `PublicSnapshot` 和单一选择来源 `task | project | global`。优先级为完整本次选择、完整有效项目默认、完整有效全局默认；不得跨来源拼接。`selection: null` 表示由服务端解析默认值。启动的 Immediate 事务须原子写入任务、快照关联、幂等键与审计事实，并重核 profile/credential/config revision、能力与有效性。只有提交成功才可启动对应自有进程；启动失败更新固定失败终态并清理自有资源。项目、目录、会话、配置或审计校验失败时不创建可运行任务、不发外部请求。

## 状态转换与幂等

任务唯一状态机：

```text
preparing ──> running ──> approval_required ──> running
    │             │              │
    ├─────────────┴──────────────┴──> cancelling
    ├────────────────────────────────> failed
    ├────────────────────────────────> cancelled
    └────────────────────────────────> interrupted

cancelling ──> cancelled | completed | failed | interrupted
running ─────> completed | failed | interrupted
```

`preparing` 表示已原子提交、等待唯一自有执行上下文；`running`/`approval_required` 可多次往返；`cancelling` 表示取消已接受而非已完成。`completed`、`failed`、`cancelled`、`interrupted` 是不可逆终态。已终态任务的重复取消只返回当前权威状态与 `accepted:false`。命令不暴露 `idle` 任务状态；空闲仅属于 UI/服务状态。

会话状态为 `active | closed | interrupted`：新建为 active；显式结束变 closed，closed 不再恢复，需新建会话；启动恢复把遗留 active 会话变 interrupted，显式恢复才回 active。同一会话最多一个未终态任务，存在任务时结束/再次启动返回 `PA_SESSION_BUSY`。关闭会话不改写任务历史；恢复 interrupted 会话须主窗口显式授权且重新验证项目身份。恢复不必加载 CLI 旧文件或 transcript；没有受控内存上下文时明示新执行上下文、保留元数据历史，不假称回复/模型上下文已恢复。会话状态变化、任务状态变化均递增修订；旧修订写入返回固定冲突。

所有创建/变更请求使用 UUID v4 `clientRequestId` 或 `expectedRevision`。未完成任务最多 8 个（含 `preparing/running/approval_required/cancelling`），超过返回固定 `PA_TASK_QUEUE_FULL`；该边界须与 provider 占用和 profile FIFO 共用有界 admission，不得排无限后台工作。相同创建 ID 再次到达只返回既有会话/任务 receipt，绝不再启动副作用；task_start 中一次调用获服务端受理后，如 IPC 超时，前端先按项目 ID、会话 ID 和 `clientRequestId` 查询权威任务，不自动重发或生成第二个请求。ID 不得携带用户文本。请求 ID 永久绑定安装内创建记录；服务不可判定是否已提交时进入受限恢复状态。取消 receipt 的 `accepted` 仅说明服务端接受取消；实际终态须由后续状态查询确认。审批决定同理：超时后读取状态，不重复批准/拒绝。

## 会话、任务、审批与内容 DTO

具体 TypeScript 类型见 `src/ipc/taskContracts.ts`。实体 ID 都是服务端 UUID v4，创建请求的 clientRequestId 是客户端生成 UUID v4，correlation ID 沿用 IPC 服务端生成格式，游标是服务端不透明引用；修订、时间、sequence 为非负安全整数（实体修订从 1 开始，上下文/无设置修订可从 0 开始）。这是后续 Rust serde/前端运行时验证须满足的严格解析要求，本步没有实现这些未来 DTO 的运行时验证器；不得仅靠 TypeScript 类型开放调用。

| 对象/操作 | 参数与返回要点 | 约束 |
| --- | --- | --- |
| 活动上下文 | main 写入 `projectId + sessionId + AiSelection + expectedContextRevision`；返回最小 `ActiveTaskContext` | 服务端核验同项目、项目授权/身份与完整选择；上下文修订递增。重启/撤权后不自动恢复为有效上下文。 |
| 会话创建/列表/详情/恢复/结束 | `session_create` 接项目 ID、项目修订、request ID，回 `SessionReceipt`；其他命令按项目或会话分页/修订操作 | main only；会话与项目绑定。创建可幂等；恢复不恢复任务；关闭不取消任务。每页最多 100 项。 |
| 任务启动 | `task_start` 接项目/会话 ID、各自修订、项目设置/全局设置修订、可空完整选择、request ID、prompt；回 `TaskReceipt`（状态、duplicate） | main only；立即事务快照后才启动自有进程。`task_start_from_active` 只接 overlay 的 context revision、request ID、prompt；项目/会话/配置/cwd 全由服务端活动上下文解析。 |
| 任务查找/列表/详情 | 幂等对账按项目 ID、会话 ID、request ID 三元组查询；会话任务稳定分页 | main only；服务端校验同项目与会话归属，避免 request ID 单独查询跨项目记录。元数据读取不更新项目使用时间；任务内容/差异还需当前目录身份复核。 |
| 取消 | task ID + expected task revision → `CancelReceipt` | main only。overlay 只允许对当前绑定任务调用 `overlay_task_cancel`，同时带 context/task 修订。只取消/终止本应用拥有且能确认归属的执行。 |
| 审批列表/决定 | 读取 `ApprovalView`；决定携 approval/task ID、修订、服务端操作 fingerprint 和 approve/deny | main only。详情含限长展示摘要；原始协议帧/任意命令正文不直接渲染。主窗口真实窗口授权、任务/会话绑定、expiry、fingerprint、pending 状态和审计须在一个 Immediate 事务复核。 |
| 事件 | `task_events_get` 按不透明游标读取 task 内状态事实、内容变更通告、approval ID 和终态；返回 throughSequence/resetRequired | main only。事件不含 prompt、reply、delta、Key、endpoint、路径、命令原文或文件内容；缺口后读取任务快照及对应内容通道。sequence 单调且同一事件只出现一次。 |
| 内容/变更/差异 | 单独 `task_content_get` 分别读取 reply/progress；`task_changes_get`、`task_change_get` 读取任务相关变更 | 仅 main。内容按 cursor 分段、限长；没有内存/持久内容时明确 `unavailable`，不能伪造空成功。变更路径只能是相对项目路径，读取前重新核验文件身份和授权根。 |
| overlay 最小状态 | `overlay_task_get` 返回最小上下文与任务状态；`overlay_task_find_by_request` 只用于幂等对账且须匹配 context revision | overlay 可用：当前上下文提交、当前任务状态、取消；reply preview 最多 512 UTF-8 字节、固定错误码、approvalPending 布尔值。不可读取会话列表、完整任务/审批、prompt、差异、原路径或完整回复。 |

prompt 必须非空、UTF-8 不超过 64 KiB；允许正常换行/制表，拒绝 NUL/无效控制字符。task_start envelope 上限为 96 KiB，其他任务请求 16 KiB。所有编号与页大小有上限，所有字符串长度按 UTF-8 字节计算。事件一页最多 100 项、单 DTO 编码后不超过 16 KiB；相邻 delta 只合并到内容通道，不进入事件；normalized content_changed 通告每 task 每秒最多 20 次。任务内容单次最多 24 KiB，游标分页，单任务进程内 reply 缓冲最多 1 MiB、progress 最多 64 KiB，超过时设置 `truncated:true` 并停止累计。单次 diff 最多 128 KiB，relativePath 最多 4 KiB，列表页最多 100 项。具体数值在步骤 3.4/3.6 按 CLI 上限及桌面测量复验；若需调整，更新契约与边界测试。

摘要采集保持关闭，保留天数设置不能启用采集。本契约不将 prompt、reply、progress、审批正文或原始外部协议写入 SQLite、JSONL、诊断或无载荷事件。reply/progress 仅在受控服务内存缓冲；正常 UI 可在有权限时读取。进程重启后所有未终态任务改为 interrupted 且内容不可用，不续跑。会话/任务可保留不含内容的状态与配置快照；`summary` 为 null/不可用，直到未来通过独立脱敏功能门。不得把自由文本当成“已脱敏摘要”。

内容缓冲总额不得因终态任务数量增长而无限累积：保留至多 16 个最近终态任务、总计至多 32 MiB 的进程内任务正文/差异，终态 30 分钟后清除；超出按最旧终态先淘汰且明确返回 contentAvailable:false。正在运行的任务也计入总额，单任务上限仍适用；内存不足拒绝新任务并报告固定限额错误。内容分页游标绑定 task/channel/缓冲代次及字节偏移，按 UTF-8 边界分段；追加增量不使已有前缀重复，发生清理/截断则要求重新取权威内容状态。游标最多 512 字节，所有列表缺省每页 50、范围 1–100，服务端稳定排序以 createdAt/UUID 为最终 tie-breaker。

创建 receipt 丢失时，会话用 session_find_by_request(projectId, clientRequestId) 对账，任务用 task_find_by_request(projectId, sessionId, clientRequestId) 对账；相同 request ID 不能改绑另一项目/会话，也不能用更新后的 prompt/配置再次执行。重复 ID 请求仍先核验真实窗口与记录归属；已撤权时拒绝披露内容。幂等不是自动重发授权。

## 审批、恢复与失败副作用矩阵

主窗口的普通读取/变更 IPC 默认等待上限为 8 秒；该等待不覆盖任务执行时间。任务网关从受理到首次运行最多等待 5 分钟，单任务运行硬上限 30 分钟（含审批等待）；审批 TTL 为 5 分钟，使用服务端 UTC 和进程内单调时钟复核。原始 stdio 帧上限 1 MiB，单任务 stdout/stderr 累计各最多 8 MiB；每 task 每秒最多处理 200 帧，超过立即固定失败并清理自有进程。单 task 状态事件保留上限 10,000 条/24 小时（先到为准），游标过期返回 resetRequired；不影响独立持久化的权威任务终态。content_changed 通告可以合并，不丢弃审批和终态；存储限额失败必须阻断后续继续执行。具体限制属于 3.4 待实现规则，现有候选 probe 的时限不等于生产任务支持这些参数。

新建/结束/恢复会话和任务不存在、归属不符分别使用候选固定码 `PA_SESSION_NOT_FOUND`、`PA_SESSION_PROJECT_MISMATCH`、`PA_SESSION_CLOSED`、`PA_SESSION_BUSY`、`PA_TASK_NOT_FOUND`、`PA_TASK_CONFLICT_REVISION`；失效活动上下文为 `PA_TASK_CONTEXT_STALE`；非法审批/指纹为 `PA_APPROVAL_OPERATION_MISMATCH`；不可信协议/超限分别为 `PA_TASK_PROTOCOL_INVALID` / `PA_TASK_LIMIT_EXCEEDED`，自有清理失败为 `PA_TASK_RECOVERY_BLOCKED`。不存在/格式错误归 validation；归属/指纹错误归 permission；closed/revision/busy/context/已决定归 conflict；过期归 timeout；协议归 compatibility；资源超限归 external；恢复阻断归 environment。所有消息/建议必须取本地固定目录，不能携带输入、SQL 或外部 stderr。

审批生命周期拆成三个独立事实：决定 `pending → approved | denied | expired | interrupted`；消费 `unconsumed → claimed → acknowledged | abandoned`；任务运行状态。批准事务原子地核对 pending/expiry、task/session/window、操作 fingerprint、task revision 和审计能力；成功只记录服务端接受的决定，不表示 CLI 已执行。转发前先原子 claim；只对绑定的同一 task/operation/request ID 最多继续一次；收到受控 CLI 确认后 acknowledged。超时或进程/数据库不确定时 abandoned/blocked，绝不重发 approve。拒绝、过期、取消、CLI 失败和重启使待决审批失效。消费尚未完成或审计不能提交时不得继续高风险动作。

operationFingerprint 是 Rust 对规范化结构化操作、已核验 cwd 身份、参数数组/文件变更摘要、task/session 及外部 request ID 计算的 SHA-256（64 位小写 hex）；不能把渲染文本或前端 hash 当作权限。审批展示摘要最多 2 KiB、过滤控制字符和秘密哨兵，无法安全归一/不支持的操作类型 unknown 必须拒绝。3.5 之前网关继续默认拒绝全部审批，不把 on-request/read-only 候选配置当成写入/执行能力已经开放。

| 条件 | 固定结果/状态 | 外部副作用 |
| --- | --- | --- |
| 窗口非本地/不是命令允许的窗口；未知字段/版本/超限输入 | `PA_IPC_PERMISSION_DENIED` / `PA_IPC_INVALID_REQUEST` / `PA_VALIDATION_INPUT_TOO_LARGE` | 无数据库业务写入、文件读取或进程启动。 |
| 项目未登记/撤权/授权目录丢失或身份变更/cwd 不一致；session 与项目不符 | 复用 `PA_PROJECT_NOT_FOUND`、`PA_PROJECT_PATH_UNAVAILABLE`、`PA_PROJECT_PATH_CHANGED`、`PA_PROJECT_CONFLICT_REVISION`；新 session/task 固定归属错误 | 不发起 Codex 请求；不自动重新授权。 |
| 本次选择无效、项目默认无效、能力过期、profile/credential 修订变化或凭据不可用 | 复用 provider/settings/credential 固定错误（如 `PA_SETTINGS_AI_UNAVAILABLE`、`PA_PROVIDER_CAPABILITY_INVALID`、`PA_CREDENTIALS_UNAVAILABLE`） | 不回退到另一配置来源，不创建可运行任务，不启动子进程。 |
| 创建/快照/审计 Immediate 事务失败，或备份/迁移/恢复状态不可信 | 复用 `PA_AUDIT_WRITE_FAILED`、固定存储错误；服务不可判定时 `PA_TASK_RECOVERY_BLOCKED` | 事务回滚，无子进程/请求；保留旧库及一致性备份。审计不可写时拒绝高风险决定。 |
| 客户端超时/中止等待 | IPC transport/timeout 固定错误；原请求视为结果未知，先按 request ID/权威查询对账 | 不隐含服务端取消；不自动重放创建、决定或取消请求。 |
| 自有子进程启动失败/路由隔离核验失败 | 固定 process/provider 错误，任务 `failed`，自有临时资源清理 | 无模型请求；只清理由本次任务创建且可归属的资源。清理失败置 blocked，拒绝新任务。 |
| 单次取消已受理/任务已终态 | 返回 accepted 与服务端权威状态/修订；完成竞争由唯一终态裁定 | 仅中断本应用受控任务；不杀外部进程，不承诺立即停止远端已经接受的请求。 |
| 审批过期/重复/错任务/错操作/窗口失权/审计失败 | `PA_APPROVAL_EXPIRED`、`PA_APPROVAL_ALREADY_DECIDED`、`PA_IPC_PERMISSION_DENIED`、`PA_AUDIT_WRITE_FAILED` 等固定错误 | 不发送批准帧、不继续目标操作；唯一已提交的决定与审计保持一致。 |
| 事件缺口/游标过期/进程重启/内容缓冲耗尽 | 返回 `resetRequired`、interrupted 或 `PA_TASK_OUTPUT_UNAVAILABLE`；读取权威 task snapshot | 不重放任务或审批；无内容可恢复时明确显示不可用。 |

上表 `PA_TASK_RECOVERY_BLOCKED`、`PA_TASK_OUTPUT_UNAVAILABLE`、`PA_TASK_QUEUE_FULL` 及 `PA_APPROVAL_*` 中未列入既有 `src/ipc/errors.ts`/Rust `ErrorCode` 的名称均为本步建议的候选固定码，不是已注册运行时码。3.3/3.5 开放命令时须为每码补齐 Rust 分类、前端本地 message/suggestion、响应校验和测试；也可复用等价既有码，但不得返回外部原文。

终态更新、task_transition 审计和事件序号必须采用同一事务边界；如果审计/持久化失败，不能向 UI 宣布成功。任何日志仅记录静态 command 名、时间、correlation ID、状态和固定错误码；不记录参数、prompt/reply、审批内容、路径、endpoint、Key、原始 stderr/stdout。

CLI 是否存在可确定“决定已消费”的确认信号必须在 3.5 实测；stdout 写入成功本身不等于操作执行成功。若无法核实 acknowledgment，保持 claimed/不确定状态并阻断新继续动作，直到任务结束或用户显式修复；进程重启后将其 abandoned，不通过重发来推断结果。

## 窗口与命令权限

权限规则按真实 Tauri window label 与本地 URL 在 Rust 服务重复验证，capability 与前端调用器也按最小权分开。`CommandMap` 是设计；下表不是运行时 allowlist。

| 真实窗口 | 规划命令 |
| --- | --- |
| main only | `active_context_set`；`session_create/session_find_by_request/sessions_list/session_get/session_resume/session_end`；`task_start/task_find_by_request/tasks_list/task_get/task_cancel`；`task_content_get/task_events_get`；`approvals_get/approval_decide`；`task_changes_get/task_change_get` |
| overlay 最小集 | `overlay_task_get`、`task_start_from_active`、`overlay_task_find_by_request`、`overlay_task_cancel`；只依当前服务端上下文，无任意 project/session/profile/model/cwd 参数 |
| 禁止 | 未知/远端窗口；overlay 不得列出会话、读取完整任务正文/事件/审批/差异，设置活动上下文、选择 profile、提供项目 ID 或读取文件系统；两窗口均不得越过自有进程或项目授权边界 |

更新 capability 时遵循同一最小权限表；不授予 `shell`、文件系统插件、远程 capability 或通用 command permission。全局事件只发 `app_state_changed`。overlay 若需短回复，通过其仅有的最小状态 DTO 读取；不把短回复或审批摘要塞进事件载荷。

## 追加迁移草案（只供 3.3 实施）

当前 schema v5 已含 `sessions/tasks/approvals/ai_config_snapshots/audit_events`，不能因本步修改其 SQL、校验和或伪称这些表已有任务服务。3.3 预计仅追加 schema v6，仍以实际设计为准：

- 为 session/task 保存修订与一次性 `clientRequestId`（唯一约束/查找索引），不存 prompt digest、原始 prompt/reply 或凭据；将任务选择来源/能力证据补成可审计快照引用或受约束枚举。
- 增加有界 task status event journal：每 task 单调 sequence、严格 event kind、服务端 UTC 时间、有限安全字段；仅状态、固定错误码及 approval ID，不存任意文本。保留期限与上限须由服务/清理一并定义。
- 为 approval 增加 expires_at、规范化 operation fingerprint、安全 task/window 绑定事实、消费阶段/唯一消费请求 ID；决定+审计在一个事务，消费 claim/ack 不能使旧任务外的动作生效。
- 保留并验证既有快照/任务关联触发器与外键；启动升级先一致性备份、完整性/迁移校验，失败保留原库并阻断任务。不回填为可恢复/ready，不在迁移时启动 CLI。

本步未执行迁移，无 schema 版本变化；v6 草案不是 SQL 实现。若后续证明必须持久保存正文/差异，将作为单独敏感数据设计审阅，不能借此次契约默认启用摘要采集或日志留存。

## FR-CX 与 M3 验收追踪

| 需求 | 未来服务/窗口 | 固定拒绝/失败方式 | 验收用例 |
| --- | --- | --- | --- |
| FR-CX-001 项目/cwd 绑定 | `task_start`，main/overlay 服务端上下文；原子 snapshot | 授权/身份/cwd/session/project 任一不符均副作用前拒绝 | AT-S3-001/002/004 |
| FR-CX-002 会话创建/恢复/结束 | main `session_*` | 撤权、身份变化、错误项目/修订拒绝；恢复不重跑 | AT-S3-001/005/011 |
| FR-CX-003 任务发送 | main `task_start`；overlay active-context variant | 启动 receipt 与终态分离；request ID 对账、不自动重发 | AT-S3-002/003/007/010 |
| FR-CX-004 事件接收 | main 有界 task event/content pull；全局无载荷 invalidation | 缺口时 reset + 权威快照；不可信帧/超限固定失败 | AT-S3-004/005/012 |
| FR-CX-005 任务取消 | main/overlay 最小取消服务 | accepted 非终态，按 task revision 查询，竞态用唯一终态 | AT-S3-007/010/011 |
| FR-CX-006 审批 | main 审批读取、决定及服务端原子消费 | 单次决定/claim/ack，过期/审计失败/错误绑定不继续 | AT-S3-006/007 |
| FR-CX-007 回复与文件差异 | main task content/change/diff pull | 内容缺失明确呈现；相对路径与文件身份逐项复核 | AT-S3-004/009/012 |
| FR-CX-008 本地历史 | main sessions/tasks 列表；无自由文本摘要 | 默认 summary null；采集/留存不暗开；摘要未脱敏拒绝持久化 | AT-S3-005/011/012 |
| FR-CX-009 常驻入口 | 沿用阶段 1 overlay lifecycle | 退出/异常时按自有进程清理；硬件项目仍单列 | AT-S3-010/011 |
| FR-CX-010 悬浮输入 | overlay `task_start_from_active` + 内存草稿 | IME composing 时不发送；缺上下文固定阻止 | AT-S3-010 |
| FR-CX-011 活动上下文继承 | main 设置服务端 active context；overlay 只读 | 上下文 revision/授权/默认值变化必须重新验证 | AT-S3-001/002/010 |
| FR-CX-012 常驻任务反馈 | overlay 最小状态 DTO、短回复/审批布尔标记 | 细节跳主窗；不将审批正文/完整回复/差异暴露到 overlay | AT-S3-006/007/010/012 |
| FR-CX-013 常驻生命周期 | 沿用阶段 1 持久设置；阶段 3 退出流程结合 task owner | 隐藏/收起/停用不隐含取消；明确退出关停自有任务 | AT-S3-007/011 |

AT-S3-001–012 按计划逐项落到本步后续服务、权限与门：001 项目授权/cwd；002 来源与不可变快照；003 原子任务创建/故障；004 路由/有界事件/秘密；005 会话历史与恢复；006 审批原子消费；007 取消竞态/超时；008 切换与旧快照；009 主窗口结果/差异；010 overlay 上下文；011 重启/退出；012 窗口权限、日志/审计与隐私。这里只完成映射与命令未开放断言，不声称上述运行用例已通过。

## 风险、前置门与复核

M3 各项逐一对应：项目 cwd → FR-CX-001 / AT-S3-001；发送和事件 → FR-CX-003/004 / AT-S3-003/004/005；审批、取消和失败 → FR-CX-005/006 / AT-S3-006/007/011；模型/强度/站点/Key 切换 → provider 与 3.2/3.4/3.8 / AT-S3-002/008；实际参数与历史快照一致 → 3.3/3.4 / AT-S3-002/004；自管测试/切换等待/恢复及外部 cc-switch 不干扰 → 3.4/3.8 / AT-S3-004/008；悬浮上下文/发起/反馈 → FR-CX-009–013 / AT-S3-010/011；回复/文件差异 → FR-CX-007 / AT-S3-009/012。每项继续受上述窗口权限和失败矩阵限制；官方源不纳入判定。当前仅对应契约，真实场景待 3.2–3.9。

- **阻断真实发送**：阶段 1.7/1.8 的配置层证据不等于阶段 3 网关；实际 Codex 版本、两合成 HTTPS 路由、环境/项目覆盖和自有 Job 清理须在 3.4/3.9 复验。R-014、R-016 未整体关闭。
- **阻断任务启动**：项目登记、授权、目录 File ID 与 cwd 重核；项目 scope 设置审计事务；凭据和能力有效；snapshot 与执行绑定；启动恢复不重放。对应阶段 2/3 服务需分别验收。
- **阻断批准的操作**：R-020 审计、expiry、指纹绑定、并发消费、CLI 确认与恢复状态全部通过 3.5 后才可开放审批。
- **人工待验保留**：阶段 1 托盘/多显示器/DPI，阶段 2 原生选择器/中文 IME 与 NTFS/ReFS 平台边界不被本步覆盖。
- **官方源**：认证、请求及切换始终仅作预留，不实测、不阻断、不标记通过；非官方源验收使用合成凭据/站点且明示合成范围。

## 可重复验证与结论

本步回归命令：

```powershell
npm run build
npm run test:ipc
```

`tasks-contract.test.mjs` 会核对规划命令逐项不在 `ImplementedCommandMap`、client、Rust handler、Rust build command manifest、main/overlay capability；并检查 app event 继续只携带单字段 invalidation。此测试只证明当前源码没有开放任务调用面，不是 IPC 运行时权限或业务验收。

2026-11-02 本步执行：`npm run build` 通过（TypeScript 检查与 Vite production build）；`npm run test:ipc` 通过（36/36，含 S3-01 未开放命令和无载荷事件断言）。本步无迁移、无新增运行时命令、无 capability 变化、无桌面/CLI/外部网络副作用。验证证明契约与调用面边界，不证明任务/审批运行时、SQLite 恢复、真实路由或桌面闭环。
