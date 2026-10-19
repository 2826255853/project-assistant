# 步骤 1.6：系统凭据库与安全引用

完成日期：2026-10-19；收尾复核：2026-10-20（Asia/Shanghai）。状态：通过（本机 Windows x64、合成凭据及故障注入范围）。凭据持久化功能门已完成；AI 配置页、路由隔离和真实任务仍按后续步骤实施，不开放真实 Key 页面或任务。

## 实现范围

- Windows 使用当前用户的 Credential Manager generic credential，目标名位于 `ProjectAssistant/Credential/<profile UTF-8 字节的小写十六进制>/<revision>` 应用命名空间，避免 Windows 大小写不敏感的目标名把不同 profile 混用。非 Windows 平台固定返回不可用，不回退到文件、SQLite 或继承外部认证。
- 凭据 payload 带格式版本、profile ID、credential revision 和规范化 HTTPS origin；读取时逐字段校验绑定。只允许无 userinfo、路径、查询和 fragment 的 HTTPS origin。
- `credential_put` 与 `credential_remove` 仅授权 main 窗口。Key 仅由 put 请求进入 Rust 凭据服务；IPC 只返回状态与 revision，没有读取 secret 的命令。Rust `resolve` 是仅供后续自有进程服务使用的内部接口，并校验 profile、revision、origin 和状态。
- 普通数据库仅保留逻辑引用、状态和修订；新建 api_key profile 使用 `credential:<id>:0`、revision=0、status=missing，符合现有 schema，不使用 NULL 引用。替换/删除在 SQLite `IMMEDIATE` 事务内校验配置修订、任务占用、系统凭据操作和审计；成功同时递增 config revision、失效 profile 的能力缓存及旧连接验证依据。
- 凭据操作运行于 blocking worker；等待活动任务空闲时不持有数据库锁，不阻塞状态 IPC。5 分钟仍占用返回 `PA_CREDENTIAL_BUSY`；等待期间变更修订在写锁内再次核验。任务占用验证使用合成数据库记录，真实任务网关仍待阶段 3。
- 凭据成功变更发布无载荷状态失效事件；固定错误不包含凭据库原始错误或 secret。

## 补偿与恢复规则

1. 校验 profile、origin、expectedRevision、安全整数范围和任务占用后，读取并校验旧 OS payload。新版本目标已存在时拒绝覆盖，保留中断遗留条目供人工恢复，不能把它当作已提交 Key。
2. put 写入新版本后移除旧版本；remove 移除旧版本。元数据、能力缓存失效与成功审计同一 SQLite 事务提交；审计失败返回 `PA_AUDIT_WRITE_FAILED`，不返回保存成功。
3. 元数据、审计或最终 COMMIT 失败时，先恢复旧版本，再删除本次未提交新版本；整个补偿过程保持 SQLite 写锁，避免另一进程抢占相同修订。COMMIT 使用显式 SQL，避免 consuming commit 失败时提前释放锁。
4. 补偿失败返回固定存储错误，并尽可能持久标记 `external_unavailable`；重启后拒绝解析。数据库也不可写时仍必须在 resolve 中核对实际 OS payload，缺失/绑定错误不会泄露或转用其他认证。缺失外部条目允许用户通过显式 put/remove 修复；畸形 payload 拒绝继续修改。
5. OS 凭据库和 SQLite 不存在跨介质原子提交保证；本次覆盖可返回错误后的补偿，不声称断电/强杀时自动恢复。中断后不能解析缺失凭据或覆盖遗留版本；人工核对后只清理该应用的确切引用，不枚举或清理用户其他凭据。完整进程/恢复流程在后续功能接入时复验。

## 可重复验证与本轮结果

```powershell
npm run verify:credentials
npm run verify:storage
```

统一报告：`artifacts/phase-1-6/report.json`；真实桌面：`artifacts/shell-{dev|built}-credentials/report.json`。使用本次唯一合成 profile 和临时目录，结束时逐条删除并读回确认 OS 记录已不存在；不访问 cc-switch、共享 Codex 配置或用户 Key。独立进程检查通过 Rust 测试可执行文件运行相同生产内部服务，不是 UI 模拟或远端请求。

| 检查 | 2026-10-19 结果及范围 |
| --- | --- |
| `verify:credentials` | 通过；前端构建、Rust 格式、64 项 Rust 测试、19 项 IPC 测试、15 项 Codex 离线测试、release check、嵌入资源调试构建和 dev/built 两个真实桌面 |
| `verify:storage` | 通过；31 项 Rust 存储测试、schema v3、真实 app_data_dir 下两个独立进程写入/读回、WAL 与专属目录清理；既有迁移 SQL/校验和未改动 |
| Rust 独立进程 | 7 个独立进程：首次写入、读回、替换、读回、删除、读回缺失和清理；普通数据库/WAL/一致性备份无完整合成 Key；1 个标记 ignored 的 worker 由父测试显式执行，不是未验功能 |
| 绑定与拒绝 | HTTPS origin、profile ID、credential/config revision、NUL/空值/长度、安全整数、畸形 OS payload、大小写不同 profile、遗留新版本拒绝覆盖 |
| 跨存储故障 | 首次写入/替换/删除分别覆盖元数据 trigger、审计 trigger、延迟外键导致的真实 COMMIT 失败；实际 Windows 凭据恢复，旧引用/缓存/成功审计均不误提交 |
| 补偿失败 | 注入恢复旧版本失败或删除新版本失败，持久标记不可用并在独立重开后拒绝 resolve；双 SQLite 连接确认补偿期间写锁未释放 |
| OS 服务/权限错误 | 私有 Rust OS 适配边界注入 read/write/delete 不可用，验证原凭据保留及固定错误；不停止 Windows 服务、不更改用户 ACL，不记为真实系统权限故障实测 |
| 真实桌面 IPC | 严格字段/版本/长度/修订、成功状态、审计失败补偿、任务等待期间状态查询继续响应、空闲后原请求完成、删除/重复删除、无 secret 读取命令 |
| 权限与秘密检查 | overlay、未授权本地窗口、远端 main 被 capability 拒绝；Rust 真实 label/URL 校验保留；返回/状态/普通库/WAL/审计/持久日志/进程输出无完整合成 Key |

本轮实测环境：Node.js **24.22.0**、npm **11.19.1**、Rust **1.99.0 / x86_64-pc-windows-msvc**。Node/npm 与仓库固定基线 24.18.0 / 11.16.0 不同；未修改版本声明，不宣称在固定 Node/npm 版本下重新验收。现有 logging `Unsupported` 与 settings `change_resident` 两项 dead-code warning 不影响检查结果。

首轮全量并发测试曾发生一次 OS 适配返回不可用；专项串行及后续全量复验通过。凭据测试以专用测试锁串行访问 Windows 存储，未修改生产失败行为、未用自动重试掩盖错误；首次失败的确切合成目标和临时目录已通过同一 cleanup worker 清理。此验证不构成 OS 凭据库压力/可用性保证。

R-016 的系统凭据持久化/本地泄露子项及 AT-S1-006 当前范围通过。AT-AI-007 的实际网络/跨站路由、子进程认证文件和未来诊断导出在对应功能启用时复验；R-016 不整体关闭。无真实 Key、无模型用量、无官方源实测。以上结论不构成阶段 1 或 M1 完成。

## 2026-10-20 收尾检查

复核执行：Codex。基于当前工作区核对计划 21.2 的 S1-05 交付范围、生产凭据服务、仓储补偿、IPC 注册/返回校验和 main/overlay capability，未发现当前 1.6 范围的阻断项。本次更新验收与交接记录，无需修改生产代码。

本次实际环境为 **Node.js 24.18.0、npm 11.16.0、Rust 1.99.0 / x86_64-pc-windows-msvc**，与仓库固定基线一致。上文 2026-10-19 的版本差异保留为历史记录，本次已补齐固定基线验证。

| 检查 | 2026-10-20 实际结果 |
| --- | --- |
| `npm run verify:credentials` | 通过；构建、Rust 格式、64 项 Rust（另 1 个 worker 由父测试显式启动）、19 项 IPC、15 项离线 Codex、release check、嵌入资源调试构建及 dev/built 真实桌面 |
| 凭据独立进程及故障补偿 | 7 个独立进程链路通过；元数据/审计/真正 COMMIT 失败补偿、补偿失败重开拒绝、写锁与绑定/遗留版本拒绝检查通过 |
| 桌面边界与清理 | 两种模式的 main 写入/替换/删除、任务等待期间状态查询、overlay/未授权本地窗口/远端拒绝及秘密检查通过；两份报告均为 `credentialCleanupPassed=true` |
| `npm run verify:storage` | 通过；31 项存储测试，schema v3，app_data_dir 下独立进程写入/重开，WAL 与专属目录清理通过 |
| 源码与工作区检查 | 原有 `0001_initial.sql` / `0002_settings.sql` 无改动，迁移仍只追加 v3；`git diff --check` 通过 |

本次报告仍位于 `artifacts/phase-1-6/report.json`、`artifacts/shell-{dev|built}-credentials/report.json` 和 `artifacts/storage/report.json`，已由本轮运行重新生成，属于忽略的本机产物。日志仍仅有上文两项既有 dead-code warning。

1.6 可按既定范围收尾，下一步为 **1.7 / S1-06 进程与实际路由隔离**。OS 服务/权限失败仍仅为适配层注入；断电/强杀自动恢复、真实网络路由、子进程认证文件及未来导出仍按对应功能门复验。1.1/1.4 的硬件与托盘人工检查继续留 1.10，不由本次合成验收代替。
