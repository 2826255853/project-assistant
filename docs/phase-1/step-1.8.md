# 步骤 1.8（S1-07）：profile、能力、默认选择与快照

日期：2026-10-22（Asia/Shanghai）。范围为《计划文档》第 21.2 节步骤 1.8；不包含 1.9 AI 配置页、阶段 2 项目授权或阶段 3 对话任务。配置层验收通过，不代表阶段 1 / M1 整体完成。

## 正式实现

- Rust providers/ 服务接入 providers_list、provider_save、provider_set_enabled、provider_delete、provider_models、provider_test、provider_switch、operation_get、operation_cancel 九命令。TS DTO、运行时返回校验、固定错误、Rust 注册、manifest、main capability 和日志白名单同步；Rust 再查真实 label/URL。overlay、未授权窗口和远端 main 均拒绝。
- profile ID 为应用生成 UUID。严格校验名称、HTTPS endpoint/origin、Responses、修订和必需可空字段。拒绝认证信息、查询、片段、IP/localhost、官方域名和任意携密路径；支持根路径、/v1、/api/v1。只激活 relay/self_managed/api_key/owned_process。历史 cc_switch/official 行可展示、停用，激活时提示手动重建。
- 元数据/启停递增 configRevision、清空能力/active 证据，审计同事务。存在任务占用时不改连接元数据；历史/默认/未完成操作引用阻止删除，建议停用。删除前须移除凭据。已有 Key 的跨 origin 编辑拒绝，必须先显式移除 Key。
- provider_models 在专属 home、固定路由中读取实际 CLI model/list，有界分页并拒绝游标循环，不启动 turn、不用站点 Key、不继承外部目录。来源 cli_catalog，sourceVersion 含适配器版本和可执行文件 SHA-256；最长缓存 24 小时，profile/凭据修订、适配器版本变化失效，probe 再核对 CLI 身份。只广告当前适配器支持的强度，新 CLI 枚举不自动开放。目录声明不等于认证/远端可用。
- test/switch 共用最多八项未完成操作的 FIFO；操作阶段和结果持久化，receipt 仅代表受理。证据绑定完整 selection、profile/credential revision、解析后的默认强度、能力和适配器版本，五分钟有效。列表不产生 ready。test 不改全局默认；switch 在 applying 再用隔离候选核验实际路由；连接请求可能产生用量。
- checking → waiting_for_idle → applying → completed，必要时 rolling_back。等受影响自有任务空闲，最多五分钟，不停止其他任务/客户端。排队、检查、等待可取消；applying 后不打断提交/恢复。FIFO 后项执行时捕获全局修订，前项合法提交不造成假冲突；候选期间其他 profile/凭据/设置变更仍在 Immediate 事务内重验。
- 未改配置失败为 unchanged；部分应用失败重新探测并核验旧完整选择，成功 rolled_back；恢复失败为 rollback_failed、active=null、blocked。恢复审计本身失败保留 blocked/durable rolling_back，不伪装 ready。只在核验成功后原子提交默认、全局批次 revision、active、审计和完成结果，再发布既有无载荷状态失效事件。
- 恢复入口为主动重新测试/切换，或 settings_update 显式清空 ai_defaults。启动不启动 Codex、不重放请求，清除旧 active/admission 证据；持久默认显示 degraded，重新核验才 ready；applying/rolling_back 中断保持 error/blocked。providers 恢复初始化失败拒绝继续服务。

## 存储与不可变快照

只追加 0004_providers.sql，schema v4；0001–0003/校验和与两个 lock 文件不变。新增 ai_default_revisions、ai_runtime、provider_operations，不改旧 settings 列布局。v3→v4 保留一致性备份，旧默认回填修订但不生成 ready 证据。

非空全局默认只经 provider_switch 写入；settings_get 返回完整 AiSelection，普通设置修改不清空它；settings_update 拒绝非空 AI 默认直写。项目写入/授权入口仍留阶段 2。

providers::snapshots 是 Rust 内部生成/只读仓储，无 task/snapshot IPC。解析按“本次完整选择 > 已授权项目完整默认 > 全局完整默认”，无跨源拼接，无效项目默认不静默回落。capture_in_transaction 要求已有事务，检查 blocked、修订、能力、完整五分钟证据，固定 profile 名称、endpoint/origin、credential_ref/revision、model、实际解析后的 effort、capability/adapter version、owned_process 和时间。PublicSnapshot 不返回凭据引用或 Key；既有触发器禁止改快照/替换任务快照。

阶段 3 必须在已授权项目/会话的 task_start 同一个 Immediate 事务调用该入口并关联实际执行；本步骤只以合成任务验不可漂移，任务和悬浮发送/取消仍不可用。

## 可重复验证

主入口：npm run verify:providers。报告 artifacts/phase-1-8/report.json，产物不提交。覆盖前端构建、Rust fmt/全量 lib、IPC、Codex 离线、release check、嵌入资源构建、实际 CLI 配置服务 worker、1.7 十四个真实路由反例回归，以及 dev/built 各 providers/credentials 两场景。

实际 CLI 配置服务六场景：目录八模型且零远端请求，主动 test，A/B 分别应用并固定快照，重定向拒绝且目标零请求，候选取消与清理。两个临时合成 HTTPS 站点、临时 CA 仅 cfg(test) 信任，不安装系统证书。合成 Key 经 Windows 凭据真实绑定解析并删除确切目标，站点 Key 仅 Rust 转发内存持有；生产无 loopback/自签 CA 开关。worker 调用生产状态机/仓储，仅候选网络边界使用 cfg(test) 受控目标；不等于收费站点实测。

三类恢复、FIFO 八项、等待空闲/取消、profile/全局并发修订、审计失败、能力/默认强度失效、启动恢复和 Key/默认值变化后的不可变快照，使用生产状态机加有界候选结果注入及合成任务验证，不冒充真实运行任务并发。另执行 verify:storage（schema v4 独立进程重开/清理）和 verify:settings（dev/built 六个设置/重启场景）。最终计数以本轮报告为准。

基线：Node 24.18.0、npm 11.16.0、Rust 1.99.0 / Windows MSVC；CLI 0.163.0-alpha.11，不是 1.7 历史报告 alpha.10.2。新目录强度按适配器白名单过滤，不读外部模型目录。保留既有两项 dead-code warning。

## 后续边界

- AT-AI-001/003/004/006/007 完成上述配置层/合成任务子项；未宣称收费中转站全部模型、真实任务并发/运行中切换通过。
- 1.9 才交付 AI 配置页、测试用量告知和选择/恢复交互。1.10 托盘/物理显示器/DPI 人工验收仍待；不能宣布 M1/阶段 1。
- R-014 配置候选/切换、R-016 凭据/快照/转发不含 Key 子项有证据；未来导出和阶段 3 网关仍未整体关闭。官方认证/切换仅预留、不测试、不纳入判定，cc-switch 始终外部手动管理。
