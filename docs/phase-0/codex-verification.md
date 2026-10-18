# 步骤 0.3：Codex 通信实测

日期：2026-10-10（Asia/Shanghai）；2026-10-11 替代范围复审。步骤 0.3 当前状态：**通过（ADR-0011 替代范围）**。真实/合成/离线证据保持区分；原自动集成未通过，已退出当前需求，生产自管隔离/凭据与恢复尚待实测。

2026-10-11 历史补验：[社区 CLI 替代接口补验](cc-switch-cli-verification.md)完成 9 项能力/限制检查；当时自动编辑/恢复、桌面兼容和共享代理缺口导致阻塞。随后用户确认手动 cc-switch / 应用自管路线，当前阶段门以 [ADR-0011](self-managed-profile-adr.md) 为准，旧实验报告不改为通过。

## 当前执行约定

cc-switch 仅由用户在应用外手动管理；应用 profile/Key 独立录入，不自动继承或同步外部配置。此前要求切回 cc-switch 的约定撤销。官方源相关功能仍仅预留、不实测、不纳入通过判定，不记为实测通过。以下真实实验曾导入当时外部非官方配置到受限临时 home，只是历史连接验证，不构成生产自管实现。

## 范围与复现

本机 Windows 11 build 26300 x64，Node.js 24.18.0；Codex Desktop 26.1007.2314.0，随附 CLI `0.162.0-alpha.17.2`。首轮实测真实 provider 为 `kakou`；配置修复后为 cc-switch 导出的 `custom`，均为非官方卡扣中转站。补验入口与结论见 [配置修复后补测](cc-switch-retest.md)。进程由实验客户端启动，工作目录使用含中文、空格和 `&` 的隔离临时项目。

```powershell
npm ci
npm run test:codex
npm run verify:codex
npm run verify:codex:providers
npm run inspect:cc-switch
npm run build
```

CLI 路径按 `CODEX_BIN`（绝对路径）→ PATH 中的 codex.exe → 当前用户 Codex Desktop 安装包发现，未硬编码安装 hash。实际失败退出非零；`inspect:cc-switch` 成功完成调查时退出 0，但报告的集成状态仍为 blocked，不能作为阶段门通过信号。

真实服务实验会发送四条短消息/审批测试及一条取消测试，可能产生用量。默认使用当前已配置的模型/思考强度；可通过 `CODEX_PROBE_MODEL`、`CODEX_PROBE_EFFORT` 覆盖，只允许模型目录声明的组合。`CODEX_PROBE_TIMEOUT_MS` 为每轮结束等待上限，默认 90000，允许 1000–300000 ms。RPC 默认 15000 ms，关闭等待 EOF 3000 ms，超时仅清理自己启动的进程树。

只复制连接相关根配置、model_providers 表和可用 auth.json 到受限临时目录，不导入 MCP、插件、notify、项目信任、用户提示词或技能。Windows 临时根 DACL 仅授予当前用户，文件继承该保护；完成后删除临时根。没有改写真实 CODEX_HOME、cc-switch 数据库或正在运行的 Desktop 进程。异常强制中断脚本或主机断电后的临时目录清理仍需后续产品实现，不属于普通正常退出保证。

## 真实服务结果

证据：`artifacts/codex-real/report.json`；入口：`scripts/verify-codex.mjs`。报告保留状态、模型名、事件计数、审批字段名和布尔核验结果，不保留完整回复、命令输出、认证内容或 stderr 原文。

| 项目 | 本轮结论 | 实测断言 |
| --- | --- | --- |
| 初始化 | 通过 | initialize → initialized；服务确认独立 CODEX_HOME |
| 模型能力发现 | 通过，目录来源有限制 | model/list 分页；返回模型及 supportedReasoningEfforts；本机目录可能来自 model_catalog_json，不能将目录视为每个远端模型均可用 |
| 无修改消息 | 通过 | thread/start → turn/start → agentMessage delta → turn/completed；最终标记匹配；未发生工具调用、项目目录仍为空 |
| 默认模型及强度 | 通过所选组合 | `gpt-6.1-sol / high`；thread/start 的 model、reasoningEffort 回显匹配，真实服务完成回复 |
| 取消 | 通过 | turn/interrupt 后收到终态 interrupted；应用应映射 cancelled，而非仅将 interrupt 的 RPC 成功视作完成 |
| 模型/强度变更 | 通过所选第二组合 | 首轮新会话 `gpt-6-astra / xhigh`；配置修复后补验按当前目录默认值选择 `gpt-6-astra / low`；两轮各自回显匹配并完成真实回复；非法强度在请求前被拒绝，原选择保持冻结 |
| 审批拒绝 | 通过命令审批 | 客户端返回 decline；命令未成功执行，模型停止重试并正常完成回复；拒绝操作不等同整个任务失败 |
| 审批批准、实际 cwd | 通过命令审批 | 只允许可信已安装 PowerShell 执行精确 Get-Location，且审批 cwd 等于隔离项目；accept 后退出码 0，输出包含实际临时项目路径，项目仍为空 |
| 正常关闭 | 通过 | stdin EOF 后自行退出码 0；临时目录清理完成 |
| 认证 | 当前中转源调用通过；官方未验证 | account/read 返回 account=null、requiresOpenaiAuth=false；自定义 provider 可用 bearer 凭据，不能把 null 直接判为未登录，也不能据此认定官方认证可用 |

首次批准实验因命令白名单未匹配而安全拒绝。核查发现本机审批展示命令对 Windows 可执行文件路径使用双反斜线；用 Windows 路径规范化比较可信可执行文件，并在测试消息中明确传入绝对 workdir 后，批准路径通过。没有放宽至任意命令、session 级批准或 execpolicy 修改。

模型和强度回显是配置证据，不是远端模型内部执行的独立证明；HTTP 请求字段核验见下节。文件变更审批、其他审批类型和所有模型的远端可用性未验证。

## 真实 CLI + 合成站点结果

证据：`artifacts/codex-controlled/report.json`；入口：`scripts/verify-codex-providers.mjs`。使用两个仅监听 127.0.0.1 的 Responses SSE 站点、两枚每轮随机生成的合成 Key 和不含真实凭据的临时 home。服务返回固定合成文本；这不是官方源、真实中转站认证或 cc-switch 操作。

| 实验 | 本轮结果 |
| --- | --- |
| Key 元数据绑定 | profile ID 和 endpoint origin 不一致时，KEY_SITE_MISMATCH；发送前拒绝，HTTP 请求数为 0 |
| 配置实际生效 | 真实 CLI 发往 A 的 Authorization 匹配 A 的合成 Key；HTTP body.model 与 body.reasoning.effort 匹配选择，本轮为 gpt-6.1-sol / medium |
| 活跃任务中改配置 | 保持 A 的 SSE 未结束时只改隔离 home 为 B；当前 turn 仍在 A 完成，B 未收到请求 |
| 加载时机 | 同一旧进程的 model/list 仍返回目录；config/read 已看到 B，随后未指定 provider 的新 thread/start 也回显 B；本机不能假设所有配置只在启动时读取 |
| A → B → A | 各次等待归属进程空闲、关闭并重启、创建新会话；真实 CLI 正确发送各站点 Key，完整结束 |
| 候选失败恢复 | 用另一临时 home/进程测试返回 401 的 A 候选；turn 失败；当前 B 配置不变且仍能完成下一轮 |
| 超时与取消 | 站点挂起时客户端返回 CODEX_EVENT_TIMEOUT；显式 interrupt 后收到 interrupted，正常退出 |
| 跨 origin 重定向 | A 返回 307 到另一端口的 B；B 收到一次请求但没有 Authorization；最终失败。只证明此版本/此合成场景，尚未验证所有 redirect/认证头类型 |
| 未认证 | 空 home 下 account=null、requiresOpenaiAuth=true，可向用户返回 CODEX_AUTH_REQUIRED；没有发送真实官方请求 |
| 重启恢复限制 | ephemeral 会话关闭原进程后 thread/resume 被拒绝；需新建会话。持久会话恢复、崩溃中断重放另行验证 |

工程保留“等待本应用进程空闲 → 测试候选 → 固定快照 → 重启本应用进程 → 新建会话”的保守策略。不能因新会话可读取新配置，就在运行中切换全局路由或假设旧任务一定隔离。外部任务、cc-switch 路由代理热切换及 daemon 归属的真实影响未测试；当前脚本不重启共享 daemon。

## 协议来源与失败分类

首选接口为 `codex app-server --listen stdio://`；换行 JSON 双向通信。协议为本机导出的 v2 方法/类型，不声明独立的协议 semver。initialize 当前未提供完整方法协商，因此通过 CLI 版本、导出 schema 和实际响应一起限定支持范围，不打开 experimentalApi。

本机导出：`codex app-server generate-json-schema --out artifacts/codex-research/schema`、`generate-ts --out artifacts/codex-research/types`。产物忽略提交，可按 README 的 CLI 发现方式重新生成。参考官方文档 `https://developers.openai.com/codex/app-server/`，本轮抓取返回 403；补充使用官方仓库说明 `https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md`，其 main 为调查材料，实际字段和结论以本机导出及实测为准。当前服务审批还含 availableDecisions，而本次非实验 TS 导出未列出此字段；客户端须兼容新增字段，不将调查源码 HEAD 当成本机保证。

| 固定错误 | 用户建议 / 行为 |
| --- | --- |
| CODEX_NOT_INSTALLED / CODEX_PATH_NOT_ABSOLUTE | 安装客户端或设置可执行文件绝对路径；不拼接 Shell 命令 |
| CODEX_AUTH_REQUIRED | 仅在服务需要官方认证且没有 account 时引导登录；自定义源另行检查 profile 凭据 |
| MODEL_UNAVAILABLE / EFFORT_UNSUPPORTED | 重新读取所选源的能力并选择受支持组合，不静默替换模型或站点 |
| KEY_SITE_MISMATCH / PROVIDER_ENDPOINT_INVALID | 修正站点与凭据元数据或 endpoint；发送前拒绝 |
| CODEX_RPC_ERROR | 仅记录数字 RPC code 和关联 ID；不将远端原始错误写入日志或直接展示 |
| CODEX_REQUEST_TIMEOUT / CODEX_EVENT_TIMEOUT | 显示等待超时，按已知 thread/turn 执行取消；超时自身不等同远端任务已停止 |
| CODEX_DISCONNECTED / CODEX_INVALID_FRAME | 停止等待并提示连接断开/不兼容；不要自动重发任务；由会话持久性决定恢复方式 |

离线 `npm run test:codex` 的 15 项测试覆盖乱序回复、UTF-8 拆帧、结束通知先于 RPC 响应、服务端 request ID、超时/断连清理、敏感远端错误丢弃、模型分页、强度校验、连接配置过滤、审批命令注入拒绝、Key/endpoint 绑定及真实实验的官方源拒测边界。这些是客户端防回归证据，不替代真实服务实验。

## 未覆盖与阶段门

| 缺口 | 状态 / 下一动作 | 阶段一影响 |
| --- | --- | --- |
| 官方源认证、官方 ↔ 真实中转站切换 | 仅作预留；无需实际测试，不考虑其测试结果 | 不纳入通过判定，不构成阻塞 |
| cc-switch 桌面版外部读取/切换/编辑/认证接口 | 历史完整自动契约未通过；不绕过 UI 或读写内部表 | 已退出当前范围，不再阻塞阶段一；ADR-0011 |
| 外部环境/项目覆盖与实际路由 | 生产自管隔离、共享可变代理拒绝尚待实测；不采用共享全局切换 | R-014 保留为自管任务功能上线门，AT-AI-005/008 |
| 自管 Key 系统凭据库 | 未实现；目前只有受保护临时实验文件和合成 env Key | 阶段一凭据服务必须独立实现 |
| 文件审批、持久恢复、崩溃/断连重放 | 未验证；命令审批与 ephemeral 不能推及这些能力 | 在对应业务接口固化前补证据 |
| 其他版本/平台、任意服务端协议与认证头重定向 | 未验证 | 不作兼容承诺 |

通信主链路采用 app-server，见 ADR-0005。AI 配置边界现由 ADR-0011 定义，步骤 0.3 按替代范围通过；生产自管能力仍须按凭据/隔离/恢复功能门验证。范围调整不能记成旧自动接口或完整业务闭环通过。
