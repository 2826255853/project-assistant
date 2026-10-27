# 步骤 2.3（S2-03）：项目仓储、使用记录与启动恢复

状态：**通过（本步仓储与恢复范围，2026-10-27，Asia/Shanghai）**。固定基线 Node.js 24.18.0、npm 11.16.0、Rust 1.99.0；`npm run verify:projects` 完整通过，报告 `artifacts/phase-2-3/report.json` 的 `passed=true`。阶段 2/M2 尚未整体通过。

实际实施与自动验收：本次 Codex 会话，依据用户“根据文档要求，完成步骤 2.3”的授权；原生选择器及硬件人工复核仍由后续验收负责。

## 实现与接入

| 入口 | 职责 |
| --- | --- |
| `src-tauri/migrations/0005_projects.sql` | schema v5：授权身份、修订、可用性、撤权移除时间、可见身份唯一索引，以及项目授权审计类型；v1–v4 SQL 与校验和不变 |
| `src-tauri/src/storage/projects.rs` | 正式类型化项目/使用仓储，登记去重、详情复核、撤权保留历史、事务、修订与启动恢复 |
| `src-tauri/src/storage/mod.rs` | `Storage::projects()`；应用启动初始化存储时复核已登记目标，失败令存储不可用，不绕过授权 |
| `src-tauri/src/projects.rs`、`lib.rs` | 本地 main 的登记、详情、移除，阻塞工作在线程池执行；登记/移除/失效变更后发布既有无载荷事件 |
| `src/ipc/contracts.ts`、`client.ts`、`projects.ts` | 仅三个本步命令进入实际可调用表和响应校验；未来项目命令仍由类型断言与静态测试限制 |
| `scripts/verify-projects.mjs`、`scripts/ipc/projects-desktop.mjs` | 可重复完整验收与 built/dev 真实桌面 IPC、权限、故障及源文件检查 |

正式新增命令：`projects_add_from_picker`、`project_get`、`project_remove`。同步 Rust 注册、Tauri command manifest、main capability、日志枚举、TypeScript 实际命令表和返回校验；overlay 无项目权限。Rust 先校验真实窗口 label/URL，再执行严格 IPC v1、8 KiB 大小与参数校验。前端不能提交根路径、授权、文件系统身份或任意字段。普通数据库错误沿用当前固定 `PA_SETTINGS_STORAGE_FAILED`，审计失败返回 `PA_AUDIT_WRITE_FAILED`，不传递 SQL/OS 原文。

项目页面、列表搜索、系统打开、Git、活动项目和项目设置读仍由 2.4–2.8 实现；`app_get_status` 的完整项目服务状态仍为未实现。三个 main 命令通过集中 client 可调用，尚未提供项目管理页面。原生目录选择流程的实际鼠标/键盘点验留 2.9。

## 数据及事务语义

- 旧项目行升级后为 `requires_reauthorization / identity_changed`，身份为空；升级不凭历史路径授予权限。新增 `directory_identity` 仅存应用 SQLite，不进入 DTO/日志/审计。
- 登记只能消费 Rust 原生选择器产生的 `VerifiedDirectory`。取消零数据库/审计/使用写入。`BEGIN IMMEDIATE` 内重新打开路径并比较 File ID，候选和新核验句柄保持到提交；返回 UUID v4 由 Rust 生成。Windows 句柄是短期身份快照，不能把它视为后续系统交付的竞态保证。
- 同身份/大小写别名重复登记返回原 ID，不重复授权审计，不更新使用时间。路径相同但仍授予旧身份时拒绝静默改绑，先由详情/启动复核标为需重新授权，再通过新一次原生选择明确授予新目标；移除后重新选择可恢复原历史行并增加修订。同身份被用户显式选择到新位置时可更新原记录，后台不查找新位置。
- 授权授予、重新授予、撤销及可用状态改变，与固定最小审计在同一事务。审计只含事件种类、UTC 时间、服务端 correlation ID、项目 ID 和 outcome；不含路径、名称、查询或文件身份。沿用 90 天保留期。SQL、审计和最终 COMMIT 失败均回滚。
- 详情及启动只重开数据库已知路径。缺失为 `missing`，拒绝访问为 `inaccessible`，记录与原身份保留；路径恢复且身份相同才可再次使用。替换/重解析/身份无法确认转为 `identity_changed / requires_reauthorization`，不会自动回授旧身份或新目标。状态变化增加项目修订，但不计入最近使用。
- 使用仓储只允许 `Selection` 和 `SystemOpen` 两种内部来源。核验修订/身份与使用写入共用 `IMMEDIATE` 事务，串行化跨连接移除/授权。接受回调成功后才更新 `last_opened_at / open_count / last_source`；拒绝、超时、冲突、写入/提交失败均不产生使用记录。选择映射既有 `project_list`，打开映射 `system_open`；不扩展搜索/文件轨迹采集。
- 登记、详情、启动和未来列表/搜索/Git 不写使用记录。活动项目只有本进程状态，不在仓储中恢复；本步不会自动打开目录、启动 Codex、重放任务或恢复会话。
- 移除要求 `expectedRevision`，撤销授权、清除使用记录、增加修订并写撤权审计同事务。所有正式登记都有授权审计引用，移除自身也产生撤权审计，因此统一保留 tombstone 并返回 `{removed:true,retainedForHistory:true}`。移除行从详情和未来默认列表隐藏，未知/已移除 ID 固定失败；会话、任务、审批、项目设置和既有审计关联保留，不删除用户文件。

系统打开适配器与活动状态提交分别留 2.5/2.8。当前仓储回调验证不代表真实 OS 交付已经实现；2.5 必须核验最终交付目标并处理“OS 已接受但后续数据库提交失败”的不可回滚副作用，不得自动重试。2.8 仍须在仓储提交后原子协调活动项目、移除清空与事件。

## 验收证据

运行 `npm run verify:projects`，输出仅位于忽略的 `artifacts/`。本次实际通过：

| 检查 | 结果 |
| --- | --- |
| 前端 build、Rust fmt、release check | 通过 |
| 全部 Rust lib | 136 passed、5 ignored；新增项目 14 passed、1 ignored worker，worker 由父测试实际调用 |
| IPC | 29 passed；仅三个本步命令允许 main，未来能力未开放 |
| Codex 离线 | 15 passed；没有模型请求或真实 Key |
| 存储回归 `verify:storage` | 46 passed、1 ignored worker；schema v5 实际 app_data_dir 两个独立进程读写/重开，清理通过 |
| 项目独立进程 | 3 个子测试进程：登记/使用 → 读回/复核/移除 → 重启确认撤权，源文件哨兵不变 |
| built/dev 桌面 | 两种真实 WebView2：严格参数、长度、版本、未知 UUID、修订冲突、审计失败回滚、缺失复核、撤权隐藏、源文件不变及 overlay/未知本地窗口/远端 main 的 capability 拒绝 |

项目测试还覆盖并发登记去重、使用事务写锁阻止并发移除、旧修订拒绝、选择/系统接受失败不计数、真实 SQLite 语句和延迟外键最终提交故障、恢复审计失败拒绝与回滚、选择后路径替换、missing → identity_changed → 显式重新授权、v4 升级数据/历史引用及旧库一致性备份。通用存储回归覆盖损坏、高版本、迁移失败回滚、原库/备份保留。敏感哨兵检查仅针对合成数据；桌面未保存包含项目路径的截图/响应，产品日志只记录白名单命令结果。

完整 Rust 检查仍有既有两条 warning：`logging::Failure::Unsupported`、`settings::change_resident` 未使用。5 个 ignored 中，本轮父入口实际运行凭据、Job、项目 worker；实际 CLI/双合成 HTTPS 的两个专项 worker 未在本步调用，不扩大历史验证结论。

本步所有项目源文件均为应用创建的临时合成目录。权限拒绝错误的 OS 映射继承 2.2 适配层注入；本轮未修改真实用户目录 ACL、未操作真实项目或付费模型。阶段 1 的托盘/多屏/DPI 人工项及 2.9 原生选择器点验继续单列。
