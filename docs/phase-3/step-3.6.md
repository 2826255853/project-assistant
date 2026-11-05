# 步骤 3.6（S3-06）：主窗口会话、任务结果与差异

日期：2026-11-06（Asia/Shanghai）。负责人：本次 Codex 会话；依据用户按计划完成步骤 3.6 的授权。

状态：**通过（主窗口会话、任务、受限结果/差异和自动验收范围）**。schema 仍为 v7，没有改写迁移 SQL 或校验和。

## 实现

- 主导航新增“任务”：选择已登记项目，显示目标工作目录与授权状态；新建、选择、恢复和结束会话，分页查询会话与任务历史。
- 遵循既有会话状态契约：结束后的 closed 不可恢复，需新建；interrupted 只有显式恢复才回 active，不重放任务。事件失效后重新查询当前会话，按钮依据服务端状态启停。
- 从项目 settings_get 返回的完整 ai_defaults 及来源展示本次配置；支持本次服务源/模型/强度覆盖，模型与强度来自服务端能力。失效默认不自动换源。发送前明确确认项目、会话、配置；准入仍由 Rust 复核项目/会话/项目设置/全局修订、能力、连接证据和凭据。
- 展示 preparing/running/approval_required/cancelling/completed/failed/cancelled/interrupted 权威状态，以及不可变的实际配置快照。单次批准/拒绝调用 3.5 服务，分别显示决定和消费状态；取消回执只说明接受，等待查询终态。
- 主窗口指令支持 Enter 发送、Shift+Enter 换行、IME 组合阻断。同步互斥防重复点击；创建会话/提交任务生成独立 clientRequestId；响应不明可按原 ID 查询，停止等待不取消后端，也不自动重发。
- `useAppStatus` 先监听无载荷事件再查询；任务视图在通知、stream 重建及显式刷新后读取权威 task/events/content/approvals/changes，并通过有界定时读取补偿最后通知丢失。回复/进度替换快照，不拼接重复事件；异步查询清理及目标检查防旧项目/会话/差异回填。

## 结果读取与安全边界

新增开放三个既有契约命令：`task_content_get`、`task_changes_get`、`task_change_get`。已同步 Rust handler/注册/build manifest、main capability、TypeScript 调用表/返回校验和产品日志白名单；overlay 没有这些权限，Rust 仍检查真实窗口 label/本地 URL。未知字段、错误版本、错误类型、伪造 changeId/路径拒绝。

- 回复/进度每页最多 32 KiB，游标绑定 task、channel 和确切缓存内容；UTF-8 不截断字符，内容在分页中变化时拒绝旧游标。客户端从新快照恢复。正文仅渲染为 React 文本，不解释 HTML/脚本或直接展示协议帧。
- CLI 的同一 thread/turn `item/completed(fileChange)` 成功通知归一为任务变更，限制文件数、正文大小、操作类型、控制字符和站点凭据/入口令牌泄露；不接受 move_path、绝对越界、相对逃逸、UNC/设备/ADS/保留名称。未知或无绑定补丁的文件变更审批仍按 3.5 拒绝。
- 结果只保存服务端生成的 changeId、规范化相对路径、类型、受限补丁和内存文件身份。捕获、列表和差异读取均复核任务原项目修订及授权根 File ID；现存文件复用 2.5 名称句柄/TxF 元数据保护，已删除文件复核父目录身份及目标仍不存在。目录/文件替换、重解析点和删除后重建不返回差异正文。文件失效可保留原记录名称并明确不可用，不阻断其他回复展示。
- 差异取自已归一的任务缓存，不根据模型文本扫描文件系统或打开任意路径；“在编辑器中打开”沿用 2.5 的授权路径/身份复核交付服务。Git 通用差异和历史仍属于阶段 4。
- 回复/进度合计每任务最多 2 MiB，差异缓存另最多 2 MiB、100 文件、单补丁 64 KiB；完成任务仅保留最近 16 项进程内缓存。重启/逐出后明确正文不可用；SQLite、备份、审计、诊断和普通事件不持久化 prompt、回复、补丁或原始外部输出。摘要采集保持关闭。

## 本轮修复

首次真实桌面闭环发现 EventData 枚举只重命名了 variant，`content_revision`/`approval_id` 仍按 snake_case 序列化，与前端 camelCase 契约不一致。本步增加 `rename_all_fields="camelCase"` 与明确的序列化断言。

包含真实 Wry 窗口的 Cargo 测试程序需要独立 Common Controls v6 manifest；补齐链接清单后修复 STATUS_ENTRYPOINT_NOT_FOUND。测试二进制的缺省图标资源告警不代表正式桌面构建缺图标。

普通应用自带 Tauri resource.lib 的 manifest，不能再生成同 ID 资源；本轮标准应用构建发现重复 manifest 后，已对正常 bin 禁止额外清单生成，保留其原 Tauri 资源。普通应用构建及 built shell 启动/退出复验通过，测试程序仍有独立清单。会话恢复验收最初误把 closed 当作可恢复；已按 3.1 契约修正控件，并以生产启动恢复事务注入 interrupted 后验证显式恢复。

测试项目最初没有仓库边界，因用户目录的既有 `.codex` 祖先触发正式隔离拒绝；夹具补充专属 `.git` 边界，未放宽生产祖先配置检查。测试驱动也修复连续任务查询误选旧 task 的竞态：点击前记录已有 ID，只等待新出现的任务。

适配器更新为 `owned-stdio-relay-v3`；旧版本能力与连接证据失效，用户需要主动刷新能力、测试和切换。没有读取外部配置或静默重试变更。

## 验收入口与证据

```powershell
npm run verify:task-ui
```

入口执行前端构建、IPC/离线测试、Rust fmt/全量 lib/release check，随后在 cfg(test) 应用进程中运行正式 Tauri handler、SQLite、Windows 凭据库、自有任务 worker、真实 CLI 和合成 HTTPS；dev 使用 Vite，built 使用 custom-protocol 嵌入前端。报告为 `artifacts/phase-3-6/report.json`。测试用 CA/loopback 信任和差异/事件缺口/逐出注入仅编译进测试二进制，普通 debug/release 应用不能通过环境变量启用它们，系统证书库不安装证书。

最终 `verify:task-ui` 报告 passed=true：190 Rust 单测通过、10 个有独立入口/helper 的 ignored 测试，40 项 IPC、15 项离线 Codex、fmt、前端构建、release check 均通过。真实 WebView 的 dev/built 各通过 12 项检查：无项目阻断、会话与配置确认、IME 组合守卫、真实 CLI 提交/回复/单次审批确认/取消/失败、91,000 字节 长回复多页读取及 HTML 作为文本显示、窗口重载与状态恢复；并明确区分差异、事件缺口、缓存逐出、叶文件替换、审计失败和恢复事务的注入范围。结束会话后禁用恢复，新建会话经生产恢复事务变 interrupted，再显式恢复，不重放任务。

两种资源模式共观测 16 次合成 HTTPS 请求，站点 Key 均匹配；overlay 拒绝五个任务/内容/审批命令，未知字段、伪造 changeId/路径、越界变更和文件替换均拒绝正文读取。DOM 不含 Key，产品/进程日志与 SQLite 不含指令、回复、补丁及原始字段哨兵；两种模式合成凭据显式删除，应用退出后自有 home 为空，临时目录和站点清理通过。报告不保存任务正文或路径。

本轮另通过 `verify:task-gateway`（9 项检查、8 个实际 CLI/双合成 HTTPS 场景）、`verify:approvals`（6 个真实 CLI 审批场景）、`verify:isolation -- --routes-only`（14 个实际路由场景），报告分别在 `artifacts/phase-3-4/`、`phase-3-5/`、`phase-1-7-routes-only/`；路由子集不替代完整隔离/阶段回归。`npm run tauri -- build --debug --no-bundle` 与 `npm run verify:desktop -- built shell` 的普通应用启动、导航、权限、常驻和退出回归也通过。

环境：Windows 11 x64 / 10.0.26300，Node 24.18.0、npm 11.16.0、Rust 1.99.0，CLI 0.166.0-alpha.6，实机 WebView2 156.0.4314.58。未新增依赖或改动 lock；历史失败与修复已在上文记录，不以最终报告覆盖先前失败原因。以上均使用合成站点/Key，不调用收费模型，不把浏览器 mock 或历史报告视为当前桌面证据。

## 后续边界

悬浮入口提交/内容/取消权限仍关闭，等待 3.7；运行中配置切换与组合生命周期仍在 3.8，统一阶段回归在 3.9。文件变更批准、自由文本持久摘要、真实中文 IME、托盘/显示器/DPI、ReFS/其他平台等不由本轮自动验收代替。官方源仍仅预留，不纳入通过判定。阶段 3/M3 未整体通过。

变更列表来自 CLI 明确上报的成功 fileChange；不会根据回复文本推断变更，也不承诺发现命令产生但 CLI 未上报的全部文件修改。生产写补丁批准仍未开放；本轮差异正文/文件身份验收使用显式合成通知注入，不能记作真实文件补丁执行通过。
