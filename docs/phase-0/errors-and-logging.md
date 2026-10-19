# 步骤 0.6：错误、日志与最小审计规范

日期：2026-10-11（Asia/Shanghai），1.5 实施补充：规范先于实现；阶段 1.5 已接入设置审计、持久 JSONL、轮转和清理。凭据、profile、任务及审批业务的专用审计仍须在对应阶段接入，不能把本补充算作阶段 1 或 M1 整体完成。

## 错误格式与重试语义

AppError 固定 category/code/message/suggestion/retryable；correlationId 在 envelope 中，前端展示错误码、用户提示、可执行建议、关联 ID。message/suggestion 从受版本控制的本地映射表取得，各≤512 字符；不得拼接 raw error、用户输入、完整 endpoint/path、外部输出、HTTP body 或 SQL。诊断查询只用关联 ID，必要的恢复路径通过专用本地恢复视图按授权提供，不写入通用错误/日志。

前缀统一 `PA_<模块>_<原因>`，模块包括 IPC、VALIDATION、PATH、PROCESS、CODEX、PROVIDER、CREDENTIAL、APPROVAL、STORAGE、CONFLICT、INTERNAL。category 是错误性质，不根据模块名称机械推断。retryable=true 表示可以由用户在消除条件后重试，**不授权自动重发**；task_start、switch、credential_put、approval_decide 等有副作用命令均先取状态，避免重复收费、执行或审批。取消与失败分开；拒绝审批与任务终态分开。

## 必需码与提示口径

同一行含多个原因时，须按原因选择具体码；未知内部错误一律固定 PA_INTERNAL_UNEXPECTED。除已实现四码外，下列为阶段一注册表要求。

| code / category | 固定用户提示与可执行建议 | retryable |
| --- | --- | --- |
| PA_IPC_INVALID_REQUEST / validation | 请求格式无效；刷新应用，持续失败更新应用 | false |
| PA_IPC_VERSION_UNSUPPORTED / compatibility | 前后端接口版本不兼容；使用同一版本的前端与桌面程序 | false |
| PA_IPC_TRANSPORT_FAILED / environment | 本地服务调用失败；重试，持续失败重启应用 | true；客户端固定码，通常无服务端关联 ID |
| PA_IPC_RESPONSE_INVALID / compatibility | 服务返回格式不兼容；更新应用 | false；客户端固定码 |
| PA_VALIDATION_EMPTY_MESSAGE、INPUT_TOO_LARGE / validation | 空输入或超限；填写内容/缩短至所示固定上限 | false |
| PA_PATH_DENIED / permission | 路径不在已授权项目范围；通过目录选择器授权项目后重试 | false |
| PA_PATH_MISSING、ACCESS_DENIED、CHECK_FAILED / environment 或 permission | 文件不存在/无权访问/校验失败；确认目标存在、权限及项目授权 | false |
| PA_PROCESS_NOT_INSTALLED / environment | 必需程序未安装或不可用；安装并配置受支持可执行路径 | false |
| PA_PROCESS_TIMEOUT、PA_CODEX_REQUEST_TIMEOUT、PA_CODEX_EVENT_TIMEOUT / timeout | 程序或请求超时；查询任务状态，确认结束后重试 | true |
| PA_PROCESS_CANCELLED / cancelled | 操作已取消；需要时重新发起 | false |
| PA_PROCESS_EXIT_FAILED、PA_CODEX_DISCONNECTED / external | 外部进程失败或连接中断；检查客户端版本与服务状态，查询未决任务后重连 | true |
| PA_CODEX_NOT_INSTALLED、PA_CODEX_VERSION_UNSUPPORTED / environment 或 compatibility | 客户端缺失或协议不兼容；安装/选择约定版本并重新探测 | false |
| PA_PROVIDER_ADAPTER_UNAVAILABLE / compatibility | 此记录的管理方式不在应用自管范围；请手动建立自管 profile 并单独配置凭据 | false |
| PA_PROVIDER_UNAVAILABLE、MODEL_UNAVAILABLE、EFFORT_UNSUPPORTED / validation | 服务源或所选模型组合不可用；选择启用的源与支持组合后重新测试 | false |
| PA_PROVIDER_ENDPOINT_INVALID / validation | 服务地址不满足协议/安全要求；填写有效 HTTPS Responses 地址，移除认证信息/查询/片段 | false |
| PA_CREDENTIAL_MISSING、INVALID / authentication | 凭据未配置或认证失败；在所选站点设置页添加/替换 Key 并测试 | false |
| PA_CREDENTIAL_SITE_MISMATCH / permission | 凭据与目标站点不匹配；重新绑定所选 profile 的凭据，禁止直接发送 | false |
| PA_PROVIDER_TEST_FAILED / external | 候选连接测试失败，原配置保留；检查站点、认证和模型组合后重测 | true |
| PA_PROVIDER_SHARED_ROUTE_UNSAFE / conflict | 共享路由影响尚无法安全隔离；等待受影响任务结束，使用经验证的适配路线 | false |
| PA_CONFLICT_REVISION / conflict | 配置已被其他操作修改；重新读取配置并确认后提交 | true |
| PA_PROVIDER_QUEUE_FULL / conflict | 切换队列已满；等待当前操作结束再提交 | true |
| PA_PROVIDER_SWITCH_FAILED / external | 配置切换失败；查看 unchanged/rolled_back 结果并检查候选 | true；回滚核验后 |
| PA_PROVIDER_ROLLBACK_FAILED / internal | 配置恢复失败，新任务已暂停；通过本地恢复入口检查原配置，重新验证后启用 | false |
| PA_APPROVAL_DENIED、INVALID、EXPIRED / permission | 操作未获有效确认；查看本次操作，必要时创建新的审批，不复用历史批准 | false |
| PA_STORAGE_BUSY / storage | 本地数据库正忙；关闭重复写入操作后重试 | true |
| PA_STORAGE_DIRECTORY_FAILED、OPEN_FAILED、SQL_FAILED、CLOCK_FAILED / storage | 本地数据访问失败；检查应用数据目录权限、磁盘空间与系统时间；SQL 失败不得展示原查询 | false |
| PA_STORAGE_BACKUP_FAILED、MIGRATION_FAILED / storage | 升级失败，原数据保留；检查空间/权限并按迁移 ADR 人工恢复，禁止删除库“修复” | false |
| PA_STORAGE_NEWER_SCHEMA、CORRUPT、UNRECOGNIZED_SCHEMA、MIGRATION_HISTORY、MIGRATION_SEQUENCE、FOREIGN_KEY_CHECK / storage | 数据版本/完整性不符合要求；使用匹配版本或从已验证备份人工恢复；禁止覆盖原库 | false |
| PA_INTERNAL_STARTUP_FAILED、UNEXPECTED / internal | 应用启动或内部操作失败；重启并提供关联 ID/固定码排查 | false |

0.3 Node CODEX_*、MODEL_*、KEY_SITE_MISMATCH 等内部实验码，在正式 Rust 网关边界逐项映射；不把 rawMessage 一并传入。0.5 STORAGE_* 对应加 PA_ 前缀和上述固定提示，backup_path 只供受授权恢复服务。0.4 OPEN_* 暂留旧实验，正式系统打开归 PATH/PROCESS；旧报告是历史事实，不重新标为 v1 验收。

## 普通运行日志

- 结构化 JSON Lines，UTF-8，时间 UTC Unix 毫秒；level 为 error/warn/info/debug，生产默认 info，debug 仍遵守同样白名单。每行≤4 KiB；无法安全生成时输出固定失败码，不能退回 Debug(error) 或整个对象序列化。
- 白名单字段：apiVersion、occurredAt、level、command（静态注册命令）、event（静态事件枚举）、correlationId（服务端）、outcome、errorCode；阶段一可加 durationMs、数值退出码、由服务端生成的 projectId/taskId/approvalId/profileId/operationId。模型/强度仅使用已验证能力目录标识，并检查长度/控制字符；profile 名、Key 标识和任意自由文本不得混入 ID。
- 禁止：Key/token/password/auth headers、凭据引用内容、原始请求/返回/事件、完整提示/回复/命令/patch/文件、URL query/userinfo、用户搜索词、绝对路径、外部配置/SQLite dump、原始 stdout/stderr/HTTP body、stack 中输入。保留字段白名单优先于正则脱敏；未知值删除，不靠 `sk-` 等前缀识别全部秘密。不可避免的内部解析只保留内存中需要的字段。
- 阶段一落盘于 app_log_dir，按当前用户访问权限创建；启动检查保护失败时禁用落盘并固定提示。每文件最多 5 MiB、最多当前+4 个轮转文件，保留最多 7 天；写入前检查大小、按单写者锁轮转，最长7天与总25MiB上限取先到者。磁盘满/无权限不循环重试刷屏，不阻塞任务取消/审批；输出固定 PA_INTERNAL_UNEXPECTED 并禁用本次落盘。
- 设置页清理仅清理本应用确认路径下的日志（不跟随联接/符号链接，不删除项目/数据库/凭据/备份），关闭文件句柄后执行；失败显示固定码。用户手动清理可立即删除自己的日志。诊断导出为白名单汇总，不直接复制原日志/配置/数据库；导出前再验秘密哨兵/字段白名单，先给用户预览并确认。
- 阶段 1.5 已在 Tauri `app_log_dir` 建立持久 JSONL 目录，使用单写者锁、白名单字段、轮转和自动清理；`artifacts` 测试输出仍不是产品日志，仅保存合成夹具/固定结果，不用于保存真实配置和 Key。启动检查、磁盘/权限失败会禁用本次落盘并返回固定状态码。

## 审计与安全失败

审计使用 0.5 audit_events：id、occurred_at、correlation_id、event_kind、outcome（schema 枚举）、error_code；可空 project/task/approval/profile ID、已验证 model/effort。不得增加 prompt、shell、Key、原始错误或自由文本 payload。生产变更与审批关联同一事务记录最小事实；异步过程分别记录 requested/confirmed/failed/rolled_back，只将白名单语义映射到 schema 已有事件种类，新增种类需迁移。

审计默认 90 天，自动清理事务仅清理过期审计/已授权历史；手动清理需明确本地确认，不修改外部任务。高风险操作的 pending/决定/消费记录不能安全持久化时拒绝执行，不能跳过审计继续批准；普通日志故障可降级不阻塞。历史审批重启后 interrupted/expired，不重放 approve。

数据库备份遵循 0.5：最近 3 份且至少 7 天是后续成功备份清理初稿，失败备份保留；不删除原库，备份/恢复从不含凭据 payload。数据库及备份仍含本地路径等敏感元数据，不宣称加密。摘要默认不保存；开启后脱敏且≤512字符、默认30天，不能复制完整提示。未经验证的脱敏不能启用自由文本历史。
