# 步骤 0.6：安全边界与威胁测试清单

日期：2026-10-11（Asia/Shanghai）。威胁输入包括不可信 React 请求、项目路径/文件/输出、外部 provider/协议事件及共享配置变更。信任服务端生成的实体/关联 ID、系统目录选择授权和拥有的进程记录。安全目标是不越过项目/站点边界、不绕过确认、不泄露秘密、不在失败中损坏原配置。以下明确区分本轮参考验证与后续产品实现。

## 路径授权与文件使用

1. 项目注册须通过本地目录选择器或用户明确确认后由 Rust canonicalize、存在性/类型检查；只记录规范化绝对路径和平台去重键。读取文件/打开目录用 projectId + 相对路径，服务端根据授权根解析；不能让前端传新的根、任意绝对路径或修改授权布尔。移除记录不删除项目文件。
2. 拒绝相对根、`..` 分量、UNC/设备命名空间、NTFS ADS 和项目外路径。Windows 兼容性/大小写/路径键另按实测制定；真实目标 canonicalize 后按组件比较，不能用字符串 startsWith。百分号路径默认当字面，不额外 URL 解码。符号链接/联接必须规范化并仍在根内。
3. 新建文件先规范化存在的父目录并检查边界，用受控 basename、no-follow/exclusive 创建策略。生产读写要锁定/复验文件身份与父目录，不能“React 预检查通过→另一次插件打开”当安全保证；Windows 重解析点检查与句柄级使用是上线前必须验证项。阶段0 existing_scoped_path 仅已有文件预检查，存在 TOCTOU，不支持安全写文件。无法落实原子身份约束时拒绝写入。
4. 系统打开由 Rust 统一验证后立即执行；插件仅授权已批准路径范围，remote webview 无权限。HTTPS 搜索使用固定引擎/域名、关键词 URL 编码，不能接受任意 URL、file/javascript/data scheme 或用户传 opener 可执行程序。当前临时夹具/127.0.0.1 scope 保留，正式扩展必须复验。

## 外部进程、超时与取消

- 程序使用受支持绝对可执行路径或经安全发现后验证的路径，参数数组、固定 allowlist 子命令，受控 cwd；不得执行 UI 提供的 shell/SQL/env、`cmd /c <输入>`、`powershell -Command <输入>`、`sh -c <输入>`。允许编辑器 executable 不允许 shell 参数模板。Git 不从项目目录寻找 git.exe，不接受任意 git args；使用已验证只读 CLI 命令、`--`/路径分隔防选项注入。
- 后续 Codex **请求执行命令**若确需 shell，视为审批操作，展示完整规范化可执行程序/argv/cwd摘要及风险，不用简单字符串前缀认定安全；批准只对应当前精确操作。阶段0 Get-Location 白名单只用于低风险夹具，不是通用自动批准机制。
- 正式默认时限：本地设置/列表 5 秒，Git 查询 10 秒，系统打开交付 10 秒，app-server 启动/初始化/RPC 30 秒，候选连接测试 90 秒，活动任务无事件 120 秒转错误/断连处理；审批 pending 最多10分钟，用户等待时暂停任务活动超时，不延长已到期审批。SQLite写锁5秒、备份30秒沿用0.5。网关有心跳时明确计入活动，但不将 stdout 任意噪声当成功进度。阶段0脚本既有独立超时是实验参数，不声称以上产品时限已实现。
- stdout/stderr 有界读取（单流累计≤1MiB，单协议帧≤1MiB；超限终止自有进程并固定错误），禁止落原始流。取消先发协议 interrupt；10秒无终态则仅终止本应用进程树并记录未知/失败状态，不虚构取消成功。记录拥有的 PID/启动标识，不能按进程名终止其他实例；共享 daemon/cc-switch 禁止强杀。
- 断连后不自动重发写操作；ephemeral 会话只创建新的会话，已运行任务标 interrupted 并保留快照。重启需要重新校验授权/配置/认证，不继承历史 approve。

## 审批与变更

- 默认 ask，未知审批 deny；配置文件中的 trust、外部消息中的 approved、React 按钮状态、任务提示词均不能替代确认。批量、永久允许、never-ask 不纳入 v1；远程能力不在首版。
- 请求实体由 Rust 建立 pending、绑定 task/project/canonical cwd、完整规范化操作哈希、外部协议 request ID、到期时间与当前本地批准窗口。前端只提交本地 approvalId/taskId/fingerprint/approve或deny；重新查 pending 验证，不相信 UI 给出的操作内容。
- 指纹涵盖 kind/executable/全部 argv/cwd/项目身份与文件变更目标+内容哈希，带固定版本的确定性编码后 SHA-256；临时凭据不进入操作。PendingApproval 参考只验证提供的指纹绑定，指纹生成/状态仓储/协议应答留后续网关实现。
- 决定与单次消费必须在服务锁/事务中原子完成，拒绝/过期不能后续升为批准；审批超时/窗口关闭/任务取消/断连/重启一律拒绝或失效。多个并发审批只影响匹配的任务。操作或路径身份变化必须新建审批。拒绝操作后等协议 turn/completed，再决定整个任务终态。
- 凭据增删、删除历史、自管配置变更、恢复等风险在本地专用流程确认；不得依靠通用任意命令入口。连接测试与切换点击是对所展示候选配置的授权，不授权中断用户其他任务。只读列表不需要确认。

## 凭据、站点与共享路由

- secret 仅 credential_put 入站。系统凭据库保存 payload，普通库只保存 `credential:<profile_id>:<revision>`，UI 仅状态/revision。React 输入不用 localStorage/持久状态，不进入表单分析/console；提交后清空，IPC 请求不得被通用 logger 拦截序列化。凭据库不可用时固定失败，禁止回退明文 JSON/SQLite。
- 凭据解析时绑定 profileId/revision/规范化 origin；发送前再校验 origin/HTTPS/允许协议与模型组合，跨站重定向默认拒绝且不转发 Authorization。认证失败不把 Key、远端 body 或原始请求放错误/日志/历史/诊断/备份。完整 endpoint path 也须排除内联秘密，无法安全验证的站点不可保存。
- 换 Key/profile 产生新修订，使候选与排队项失效；任务固定不可变快照。只切换应用默认值/自有进程配置，等待受影响自有任务空闲；不改变旧任务的 Key/路由。环境/项目覆盖或实际路由隔离无法核验时拒绝启动，异常失效 ready，不静默改站点。
- 按 ADR-0011，cc-switch 仅用户外部手动管理，不读取/导入/同步其配置或凭据，不依赖安装版本。自有进程专属 home、环境允许列表、受限临时认证文件及清理；不支持共享可变代理/接管路由。生产隔离和系统凭据库需实测；官方源仅预留、不测试、不纳入通过判定。

## 威胁用例与验收证据

“本轮通过”仅表示指定夹具/参考代码，不等于完整产品已抵御同类攻击。未实现项的失败动作默认拒绝或功能不开放，阶段一/对应功能上线前必须补测。

| 编号 | 输入/触发 | 必需断言与失败动作 | 本轮状态 / 对应测试 |
| --- | --- | --- | --- |
| SEC-001 路径穿越 | 相对路径、`../`、合法根的相邻同前缀目录、缺失文件、中文特殊字符 | 越界不读取/打开，缺失返回固定码，合法文件通过 | 通过；Rust safety::traversal_prefix_and_missing_paths_are_denied |
| SEC-002 重解析与 ADS | 根内 junction 指向根外文件；file:secret | canonicalize 后拒绝，ADS 拒绝；夹具清理不跟随联接 | 当前Windows通过；Rust safety::junction_to_outside_scope_is_denied；ADS在SEC-001测试 |
| SEC-003 TOCTOU/新建 | 检查后替换 junction、文件不存在却父目录越界 | 句柄/父目录身份验证失败拒绝，不在根外创建；不能靠预检查 | 未实现；阶段一文件服务上线前阻断，R-019 |
| SEC-004 命令注入 | 参数含 `& ; $() | <>`、引号、空格、中文 | 一个 argv 字面量，不执行附带 shell；固定命令拒绝额外选项 | OS参数数组实测通过；scripts/ipc/response.test.mjs；通用Git/Rust进程服务留阶段一 |
| SEC-005 审批绕过/重放 | 不匹配task/fingerprint/window、过期、重复approve、先deny后approve、unknown | 不执行不匹配/历史操作，未知默认拒绝，正确操作最多消费一次 | 参考guard通过；Rust safety::approvals_cannot_be_forged_replayed_or_used_for_changed_operations；真实协议/持久原子性留网关 |
| SEC-006 审批命令伪装 | `Get-Location;...`、pipe、EncodedCommand、不可信同名shell | 夹具白名单拒绝，不能借低风险前缀自动批准 | 复验通过；scripts/codex/client.test.mjs safePrintCommand；非生产自动许可 |
| SEC-007 秘密泄露/日志注入 | 合成Key+换行、错误payload含secret、类型错误、多余secret字段 | 固定错误、日志无payload、JSON不伪造新行、成功回显不落日志、相同关联ID | 通过；Rust ipc三个测试、TS response两测试、真实桌面日志断言 |
| SEC-008 版本/IPC权限 | v2请求、未知字段、额外correlationId、错误响应结构、remote/main以外窗口 | 副作用前拒绝，客户端固定fallback，权限显式最小 | 版本/字段/格式与静态ACL检查通过，真实main允许；其他窗口/remote动态拒绝待阶段一安全验收 |
| SEC-009 跨站Key/endpoint | profileId/origin不一致，inline认证/query/fragment/http | 网络前拒绝且不泄露；不使用另站Key | 离线复验通过；scripts/codex/profile.test.mjs；真实凭据库留阶段一 |
| SEC-010 网络重定向 | A跳到B或错误Key401 | B不接收A Key；候选失败原配置保留 | 沿用0.3合成A/B证据，见codex-verification.md；本轮未重跑网络实验 |
| SEC-011 超时/断连 | hang、非法帧、外部退出、乱序/早完成事件 | 清 pending/listener、固定码、无自动重发，终态区分 | 离线复验通过；scripts/codex/client.test.mjs；生产输出上限/取消进程树待网关 |
| SEC-012 自管切换并发/回滚 | 活动任务+自管默认值/进程切换、revision 变化、写入失败、恢复失败 | 等待自有任务空闲；unchanged/rolled_back/rollback_failed 区分；恢复失败阻止新任务 | 0.3 独立 home 历史部分证据；生产恢复失败注入和路由隔离待 S1-06/07，R-014 |
| SEC-013 持久化/备份 | SQL错误含secret、迁移/备份失败、库高版本/损坏 | 不回显SQL/值，不静默覆盖原库，不将Key写普通库/备份 | 0.5全部16测试本轮复验；storage::failed_sql_errors_never_expose_query_values等；真实自由文本/凭据库待实现 |
| SEC-014 保留/诊断 | log超5MiB、超过7天/90天、磁盘满、清理目录被替换、导出含secret | 有界轮转，清理限本应用路径，固定失败，诊断白名单不含secret | 规范明确、实现未完成；阶段一日志/清理/诊断上线前验收 |
| SEC-015 外部配置隔离 | 外部 config/auth 改动、继承认证/provider/代理变量、项目级来源覆盖、无 cc-switch/升级、共享可变代理 | 不读取/同步/写入外部配置；请求仅使用应用选中站点与 Key；未知覆盖/无法核验隔离时拒绝；运行中任务不漂移 | 1.7 正式 Rust 候选配置层已实现并以实际 CLI/双合成 HTTPS 验证；完整状态见 ../phase-1/step-1.7.md；真实任务/切换恢复留 S1-07/阶段 3，R-014 未整体关闭 |

## 功能上线前检查

- [x] 分层、最小命令、版本与错误、关联 ID、默认审批、路径/进程/站点规则已有明文契约。
- [x] 路径穿越、命令注入、审批绕过、秘密泄露四类都有可执行夹具断言与失败动作。
- [x] 最小 v1 handler/client、显式 app 命令权限与真实 WebView2 测试可重复运行。
- [ ] 任意项目路径/安全新建、TOCTOU 与动态权限拒绝完成正式服务验证。
- [ ] 系统凭据库、轮转/清理/诊断、审计原子性和完整网关按对应功能实现与测试。
- [x] 用户已确认 cc-switch 手动管理 / 应用自管替代范围，FR-AI-006 和 ADR-0011 已调整；旧自动接口不记为通过。
- [x] 配置层自管 home/env/项目配置、实际路由与共享可变代理拒绝按 1.7 使用正式 Rust 适配器和合成 HTTPS 实测；见 ../phase-1/step-1.7.md。
- [ ] 1.8 切换恢复及阶段 3 真实任务并发/既有任务快照/路由不漂移另行实测，不由配置层验收代替。
