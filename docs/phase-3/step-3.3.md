# 步骤 3.3（S3-03）：会话、任务仓储与启动恢复

日期：2026-11-04（Asia/Shanghai）。实际负责人：本次 Codex 会话。

状态：**有条件通过（仓储、迁移、原子快照与启动恢复范围）**。本步已完成会话/任务/审批元数据仓储、v6→v7 追加迁移、任务创建事务、受限状态事件日志、恢复事务及独立进程重开验证。任务 IPC、真实 Codex 子进程、审批决定消费、取消网关和桌面任务闭环仍保持未注册，分别属于 3.4–3.9；因此不宣称 M3 或真实任务可用。

## 实现范围

- 新增 `src-tauri/migrations/0007_task_runtime.sql`，仅在已有 v6 后追加为 schema v7。迁移增加会话/任务修订与 UUID `client_request_id` 唯一索引、任务选择来源和项目/设置修订引用、审批过期/指纹/窗口/消费字段，以及有界 `task_events` 日志。既有 v1–v6 SQL、校验和、快照不可变触发器和外键未改写。
- `src-tauri/src/storage/tasks.rs` 提供类型化会话/任务/审批仓储：会话创建、幂等查找、稳定分页、恢复/结束；任务原子创建、幂等查找、稳定分页、详情、取消受理、状态转换、审批请求登记和事件读取。请求文本只在事务入口验证，不进入 SQLite、日志或审计；快照由 provider 的 `capture_snapshot_in_transaction` 在同一个 `BEGIN IMMEDIATE` 事务中捕获。
- 任务创建在提交前重新核验项目登记、授权、目录身份、会话归属、项目/global 修订、完整配置来源、能力证据、profile/config/credential 修订和物理凭据。事务、快照、任务事件和白名单 `task_transition` 审计任一失败即回滚；验证回调不会在重复 `clientRequestId` 对账时再次执行。
- 会话与任务读取都重新核验项目授权和目录身份；撤权、失效目录、跨项目/跨会话请求在外部副作用前拒绝。任务绑定字段、事件序号、状态转换、审批绑定/决定/消费阶段和修订由 SQLite 触发器约束；终态不可逆，事件不可更新；被任务引用的快照受外键保护，快照内容不可更新。
- 启动恢复由 `Storage::recover_on_startup` 与 `tasks::recover_in_transaction` 组成一个事务：将遗留非终态任务标为 `interrupted`，待处理审批标为 `interrupted`、claimed 消费标为 `abandoned`，active 会话标为 `interrupted`，追加恢复事件并写固定审计事实。不续跑、不恢复内容、不启动 CLI；重复执行无业务变化。
- 新增 `src-tauri/src/tasks.rs` 作为后续 IPC/网关的服务边界和严格 DTO。它目前没有 `#[tauri::command]`、没有 `ImplementedCommandMap`、Rust manifest 或 capability 注册；3.4 开放真实入口前继续保持关闭。

## 数据与安全边界

- v7 的 `task_events` 只允许 status/content_changed/approval/completed/failed 及固定状态、错误码、approval ID、通道和内容修订；不保存 prompt、reply、原始协议帧、Key、endpoint、路径或自由文本摘要。
- `PublicSnapshot` 只包含非 secret 的 profile、endpoint、model、effort、config/credential 修订、能力生效时间和 owned-process 路由范围。快照更新、删除和任务替换绑定均被拒绝。
- 审批仓储只保存规范化操作指纹、窗口绑定、过期时间和消费阶段；展示摘要默认为空，决定与单次 claim/ack 的执行动作仍留给 3.5。
- 任务状态事件按 task 单调 sequence 保存，按 24 小时/10,000 条清理；游标缺口返回 `resetRequired`，不会重放任务或审批。

## 验证

已运行：

- `cargo fmt --check --manifest-path src-tauri/Cargo.toml`
- `cargo check --locked --manifest-path src-tauri/Cargo.toml`
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib`：**175 passed、7 ignored**。
- `npm run build`：TypeScript 与 Vite production build 通过。
- `npm run test:ipc`：**37/37**，包括任务命令仍未注册、固定错误本地目录和无载荷事件断言。
- 新增 Rust 覆盖：v6→v7 备份/迁移失败回滚、旧行兼容、任务快照/绑定不可变、prompt/secret 不持久化、重复创建不重验/不重复快照、项目撤权/能力失效副作用前拒绝、状态转换/审批请求/事件缺口、审计失败整批回滚，以及独立进程重开后的会话/任务读取。

测试范围使用本地临时 SQLite、合成 profile/能力/凭据元数据和 Windows 项目身份；没有启动真实 Codex、没有访问 cc-switch、没有发送模型请求，也没有进行 built/dev 任务桌面闭环。官方源仍为预留范围。

## 遗留与移交

- 3.4 需要把已提交 `preparing` 任务绑定到唯一自有进程，使用快照启动并归一化 CLI 事件；只有 owned-process 资源清理完成后才能推进任务终态。
- 3.5 需要补齐 approval decide、expiry、fingerprint 核验、单次 claim/ack 和取消竞态；当前审批请求仅持久化安全绑定，不允许继续高风险动作。
- 任务 IPC/capability、overlay 最小动作、真实 CLI/双合成 HTTPS、桌面人工边界仍未验收。当前步骤不关闭 R-014/R-016，也不把本步合成回归写成真实任务通过。
