# 步骤 3.5（S3-05）：审批、取消与单次消费

日期：2026-11-05（Asia/Shanghai）。实际负责人：本次 Codex 会话，依据用户“根据文档要求，完成步骤 3.5”的授权。

状态：**通过（单次命令审批、取消服务和自动验收范围）**。

## 实现与权限

- 新增 main-only `approval_decide`，同步 Rust handler/注册、build manifest、main capability、前端调用器与运行时返回校验。请求携带 approvalId、taskId、expectedRevision（审批修订）、expectedTaskRevision、operationFingerprint 和 approve/deny；窗口身份取真实本地 main 授权，拒绝前端传入窗口、摘要、命令或消费凭证。overlay 没有决定权限。
- `BEGIN IMMEDIATE` 事务核对项目授权/目录身份、task 仍可决策、task 与审批修订、pending/unconsumed、窗口和操作指纹、服务端 UTC TTL；唯一决定、消费 claim、任务状态、状态事件和决定审计同事务提交。SQL、审计或最终 COMMIT 失败均不释放决定；重复点击和并发决定不会第二次发送。
- 操作指纹由 Rust 对结构化 CLI 参数、request ID、当前 thread/turn、已核验 cwd/File ID、应用 task/session 计算 SHA-256；客户端指纹只用于与服务端既有值比较。重复外部 request ID、跨 task/thread/turn、变更操作和不支持的类型拒绝。
- 仅规范化的本机命令可以批准：必须有完整、限长且经过控制字符/格式控制符及秘密标记过滤的命令，cwd 必须等于授权目录，环境为 local，network/writeStdin 等不支持的请求拒绝。普通 execpolicy 建议被指纹绑定，回应始终只有单次 accept/decline，不提交 session grant 或 policy amendment。
- 当前 CLI 的 `fileChange/requestApproval` 仅提供 grantRoot，没有可绑定的完整补丁，因此按 unknown 默认拒绝；不能把一次决定转为整个目录/会话写授权。该限制是“无法安全归一则拒绝”的实现边界，不宣称文件变更批准已通过。
- 摘要最多 2 KiB，仅保存在受限进程内存；每 task 最多 100 条审批，最多保留最近 16 个已完成任务的内容与摘要。数据库继续只保存指纹/绑定元数据，摘要字段为 NULL，摘要采集没有开启；消费 token、外部 request ID 和窗口字段不返回 UI。

## 决定消费与取消

- 批准/拒绝事务原子生成不可复用的 consume_request_id 并进入 claimed，提交后只交给相应自有 worker 的内存通道。worker 转发前再次核验 claim、项目授权、任务状态、UTC/单调时钟期限、CLI/配置/cwd/路由和取消状态；没有内存等待通道的旧记录不能重新发送。
- stdio flush 不等于消费确认。只有绑定的 `serverRequest/resolved` 或同一 thread/turn/item 的 `item/completed` 才推进 acknowledged；该确认不表示命令执行成功。真实命令成功另以 `commandExecution` 的完成/exitCode=0 检查。确认缺失时保持 claimed 并拒绝新的审批动作；任务结束转 abandoned，不通过重发推断结果。
- 决定审计使用 `approval_decision`，确认/放弃使用携带 approval_id 的 `task_transition`；每个审批只有一个决定事实。取消和失败/终态使 pending 变 interrupted、claimed 变 abandoned，和任务修订、事件、审计原子提交。启动沿用 v7 恢复规则：中断任务、失效审批、放弃 claim，不重放。
- 取消响应只返回 accepted/status/revision。持久化 cancelling 与自有进程发送锁串行化，取消已提交后不再发送批准；存储或审计失败不会设置取消信号。排队等待自有适配器锁可取消，等待上限 5 分钟；不会为已取消的排队任务启动子进程。
- 审批等待时持续核验进程、取消、配置/目录和期限，并有界缓冲通知；异常帧、提前终态、重叠审批或子进程退出阻断继续。worker 只在自有资源清理后写终态。完成与取消以权威修订决定顺序，允许已受理取消最终观察到 completed；IPC 超时/停止等待不取消服务端，也不自动重发。
- 审计/存储无法提交时保留最后可信状态并阻断新自有子进程，不能宣布已完成或成功取消；进程内消费结果不明也不会重发。取消/退出仅涉及本应用自有进程与 Job，外部进程不受影响。

## Schema、适配器与固定错误

本步不追加迁移：schema 仍为 v7，复用既有不可变绑定、决定/消费状态、唯一消费 ID 和修订 guard；0001–0007 SQL/校验和均未修改。普通 SQLite/备份仍不保存 prompt、回复、命令摘要、Key 或原始协议帧。

适配器版本升为 `owned-stdio-relay-v2`，旧版本能力/连接证据需要重新核验，不会静默复用或换源。候选 catalog/probe 继续强制 tools=[]/tool_choice=none 并拒绝工具输出；只有已准入的任务允许固定工具类型经过 relay，仍逐次绑定站点/Key/model/effort。新增 `PA_APPROVAL_NOT_PENDING`、`PA_APPROVAL_EXPIRED`、`PA_APPROVAL_BINDING_MISMATCH`、`PA_APPROVAL_UNCERTAIN` 的 Rust/前端固定呈现；任务/会话/审批命令已加入产品日志白名单，日志没有请求载荷。

## 可重复验证

```powershell
npm run verify:approvals
npm run verify:task-gateway
npm run verify:isolation -- --routes-only
npm run tauri -- build --debug --no-bundle
```

`verify:approvals` 汇总格式检查、前端构建、Rust check/全量单测、IPC、离线 Codex、当前 CLI 版本及重新生成的 schema，再启动双合成 HTTPS 和真实 CLI。主站依次请求无副作用的 Get-Location，覆盖批准、拒绝、过期、审批等待时取消、真实自有子进程退出、真实 SQLite 审计触发器拒绝六种场景。批准必须成功执行一次，其余不得成功执行；每次检查站点/Key、无跨站、项目未改、自有 Job/临时目录清理、外部哨兵进程仍存活。

Rust 回归覆盖八个独立连接并发批准/拒绝只有一胜者，错 task/window/fingerprint、重复/已消费决定、错误消费 token、UTC 过期、未知操作、claim 期间拒绝新请求、取消后的失效、重启放弃、审计失败和延迟外键导致的最终 COMMIT 失败；取消涵盖 preparing、running、approval_required、终态及完成/取消的先后顺序。排队取消直接验证自有适配器锁，IPC 的停止等待/超时测试确认调用仅发送一次。

最终结果：`npm run verify:approvals` 通过，报告为 `artifacts/phase-3-5/report.json`（passed=true、14 项检查、6 个真实 CLI 场景）；186 项 Rust passed / 9 个显式入口或独立进程 helper ignored，38 项 IPC、15 项离线 Codex、build、fmt/check 均通过。批准场景成功执行一次，拒绝/过期/取消/子进程退出/审计失败场景均未成功执行；正常消费确认与任务终态一致，自有资源和临时目录清理通过，外部哨兵不受影响。审计故障场景保留 pending/unconsumed，未宣称成功终态。

另通过 `verify:task-gateway` 的 9 项检查/8 个场景和 `verify:isolation -- --routes-only` 的 14 个真实 CLI 路由场景，报告分别为 `artifacts/phase-3-4/report.json`、`artifacts/phase-1-7-routes-only/report.json`；后者仅是路由子集，不能替代完整 isolation 的 dev/built 桌面回归。`npm run tauri -- build --debug --no-bundle` 通过。本轮工具链为 Node 24.18.0、npm 11.16.0、Rust 1.99.0、Windows x64；未新增依赖或修改 lock。报告/生成 schema/日志为忽略的本地证据，不提交源码。

## 范围与后续

真实测试使用 Codex CLI `0.166.0-alpha.6`、临时合成站点/Key/CA 和临时项目，不调用真实模型或读取 cc-switch；测试 CA 只在 cfg(test) 客户端信任，不安装系统证书。过期的真实 CLI 场景缩短了私有测试对象的单调期限，生产期限固定 5 分钟；并发、错误绑定和恢复为生产仓储/状态机测试。审计故障下保持 pending/未消费是预期结果，不能把存储失败包装成成功终态。

本步关闭审批与取消服务范围；主窗口审批控件/回复/差异、overlay、真实运行中配置切换和阶段汇总继续在 3.6–3.9 验收。文件变更批准、完整自由文本持久化脱敏、其他 CLI/Windows 版本和人工 IME/托盘/多显示器/DPI 等不在本轮通过范围；阶段 3/M3 未整体通过。
