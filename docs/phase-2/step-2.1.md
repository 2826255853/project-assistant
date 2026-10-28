# 步骤 2.1（S2-01）：项目契约与授权模型

状态：通过（契约定义范围，2026-10-25，Asia/Shanghai）。本记录完成接口、权限、错误、审计和路径安全契约；未实现项目服务、未开放项目 IPC，也不表示阶段 2 或 M2 通过。

2026-10-27 接续：以下未实现状态描述 2.1 当时范围。2.3 已按既有 DTO 开放 `projects_add_from_picker`、`project_get`、`project_remove` 三个 main-only 命令，详情见 [步骤 2.3](step-2.3.md)。其余项目命令、活动状态、设置范围及页面仍按后续步骤推进。撤权审计自身产生项目历史引用，因此实际移除统一保留 tombstone、返回 `retainedForHistory:true`，不丢失审计中的项目 ID；授权状态/可用性改变增加修订且使用时间不变。

## 目标与边界

2026-10-28 接续（S2-04）：新增 main-only `projects_list`，沿用既有请求/页面 DTO 和大小边界；显式 null 不等同缺省，严格拒绝。排序和匹配等级、分页指纹、查询变化以及客户端竞态的具体规则见 [步骤 2.4](step-2.4.md)。列表仅读数据库中最近一次可用性观测，不作文件系统遍历或自动授权；游标格式错误返回 `PA_IPC_INVALID_REQUEST`，查询/排序/可见项目快照改变后的续页返回 `PA_PROJECT_CONFLICT_REVISION`。其他待实现接口边界不变。

本步骤对齐 FR-PM-001–007、M2 和阶段 1→2 移交要求，为后续路径授权、仓储、搜索、系统打开、Git 摘要和页面工作包确定共同语义。契约依照计划文档第 22 章及当前 IPC v1、SQLite schema v4、窗口授权、设置和日志实现制定。

范围仅包括项目服务的目标 DTO、拟议 IPC、main/overlay 权限、固定错误、审计要求和路径安全用例。目录选择、路径身份核验、仓储、业务命令、界面、活动项目运行状态均由后续步骤实现和验收。schema v4 中虽已有 `projects` 和 `project_usage` 表，但尚无正式项目仓储或项目授权记录；不得把现有表或阶段 0 的路径 guard 当成生产授权。

阶段 2 不启动 Codex、不创建会话或任务、不处理审批、不执行 Git 写操作、不读取项目文件内容建立索引，也不开放项目级设置写入。项目级 AI 默认值编辑和应用仍属阶段 3。用户的 cc-switch 配置仍由用户在应用外管理。

## 领域与授权语义

| 术语 | 契约语义 |
| --- | --- |
| 登记（register） | 用户在应用主窗口发起本地目录选择器并明确选定目录。Rust 服务校验目标后创建或返回项目记录，并建立对该目录身份的应用内授权。选择器取消不写入项目、使用记录或审计成功事件。前端不提交任意根路径，也不提交授权标记。 |
| 项目授权（grant） | 只对登记时用户选定并经 Rust 核验的目录身份有效；项目 ID 是引用，不是授权凭证。授权必须同时绑定规范化路径和稳定文件系统身份。每次实际使用及重启恢复都重新核验。授权失效时不自动改绑同路径的新目标、不扫描磁盘寻找替代目录。 |
| 选择（select） | 将一个已登记、授权有效且当前可用的项目设为本进程活动项目。选择不代表打开系统文件管理器、不创建 Codex 会话、不启动任务，也不扩大该目录授权。每次明确选择成功才更新最近使用记录。 |
| 打开目录/文件（open） | 只对已登记项目执行。目录由项目 ID 唯一确定；文件由项目 ID 加项目根目录内的相对路径确定，或由 Rust 发起的受限文件选择器返回相对路径。Rust 在系统交付前复核授权、文件身份和边界。成功仅表示操作系统接受了交付，不表示外部程序已完成加载。 |
| 移除（remove） | 从用户的项目列表移除并撤销应用授权，不删除或改写用户项目文件。活动项目立即清空。没有历史外键引用时可删除项目及使用记录；存在会话、任务、审计等历史引用时保留必要的历史行并标记为已移除/授权撤销，列表默认隐藏，不级联删除历史数据。 |
| 活动项目（active project） | 独立于 `ActiveContext`，可为空，只包含最小显示信息。它不是会话或 AI 上下文。该状态仅在应用进程内有效；重启后不恢复为活动项目，只展示近期记录，不自动打开目录或启动任务。 |

同一目录身份重复登记返回已有项目，不产生第二个 ID。路径字符串别名按 Windows 规范化及目录身份去重；若同一字符串现指向不同身份，则拒绝静默复用，要求后续登记流程明确重新授权。项目 ID 由 Rust 生成 UUID；调用方伪造或猜测 ID 不得构成路径授权。

建议的可用状态为 `available | missing | inaccessible | identity_changed`。缺失、拒绝访问或身份变化都保留项目记录供用户查看/移除，并阻止选择、Git 查询和系统打开。`identity_changed` 必须要求用户通过新一次本地目录选择显式确认；不得自动绑定。撤销授权通过移除完成，不向调用方暴露内部文件身份值。

## DTO 草案

下列为后续实现目标的 JSON 形状（响应包络仍沿用 IPC v1 `{apiVersion:1, correlationId, status, data|error}`）。对应类型已放入 `src/ipc/contracts.ts` 的未来 `CommandMap`，Rust serde DTO 位于 `src-tauri/src/project_contract.rs`，纯响应验证器位于 `src/ipc/projects.ts`；这些定义未加入 TypeScript `ImplementedCommandMap`、IPC `client.ts` 调用映射、Tauri Rust 命令注册或 capability。`ProjectAwareAppStatus` 与活动项目 DTO 是步骤 2.8 的目标形状，当前实际 `AppStatus` 尚未改变。

```ts
type ProjectAvailability = "available" | "missing" | "inaccessible" | "identity_changed";
type ProjectAuthorization = "granted" | "requires_reauthorization" | "revoked";

interface ProjectView {
  id: string;                 // Rust 生成的 UUID
  name: string;               // 文件系统目录名
  displayName: string | null; // 后续若支持别名，由应用保存；不是授权依据
  absolutePath: string;       // 仅主窗口项目 API 返回
  availability: ProjectAvailability;
  authorization: ProjectAuthorization;
  revision: number;           // 项目元数据/授权修订，冲突保护
  createdAt: number;          // UTC Unix 毫秒
  lastUsedAt: number | null;
}

interface ProjectListPage {
  items: ProjectView[];
  nextCursor: string | null;
}

interface ActiveProject {
  projectId: string;
  name: string;
}

type GitSummary =
  | { state: "available"; isRepository: true; branch: string | null; detached: boolean; workingTreeModified: boolean; refreshedAt: number }
  | { state: "not_repository" | "git_unavailable" | "timed_out" | "failed"; isRepository: false | null; branch: null; detached: false; workingTreeModified: null; refreshedAt: number | null };

interface ProjectSettingsView {
  projectId: string;
  revision: number;
  globalRevision: number;
  values: Array<{ key: string; value: unknown; source: "project" | "global" }>;
}
```

`absolutePath` 仅为主窗口中满足 FR-PM-003 的显示数据，不作为后续命令参数或权限依据。文件系统身份、卷标识、句柄、授权令牌、原始 Git 输出、stderr、项目文件内容、Key 和认证信息永不进入这些 DTO。Git 失败是可展示的降级状态，不使列表、搜索、选择或打开失败。

`ProjectSettingsView` 的 `values` 只含允许读取的结构化设置值，并明确逐项来自项目还是全局继承。阶段 2 的 project scope 只读；`settings_update` 对 project scope 继续拒绝。非空 AI 默认值的项目级编辑和应用留阶段 3。

## 拟议 IPC 与权限矩阵

下表中命令均为未来能力名，不是当前可调用接口。所有请求继续使用严格 v1 envelope、拒绝未知字段，并由 Rust 检查真实窗口 label 和 URL。项目管理、路径选择、设置读取及系统打开能力仅授予本地 `main`。`overlay` 不获项目列表、登记、选择、移除、设置读取、Git、目录/文件打开或文件系统插件权限；后续仅可从 `app_get_status` 读取最小 `activeProject`。远端、未知窗口和伪造来源均在任何副作用前拒绝。

| 拟议命令/入口 | 输入与返回（`data`） | 授权与副作用 | 主要失败 |
| --- | --- | --- | --- |
| `projects_add_from_picker` | `{}` → `{ outcome: "registered", project: ProjectView } \| { outcome: "cancelled" }` | main only。选择器由 Rust 发起，不收前端路径；取消零写入。登记成功可写项目、授权与审计。 | `PA_PROJECT_PICKER_FAILED`、`PA_PROJECT_PATH_UNSAFE`、`PA_PROJECT_PATH_UNAVAILABLE`、`PA_PROJECT_PATH_CHANGED`、`PA_PROJECT_CONFLICT_REVISION`、存储/审计错误。 |
| `projects_list` | `{ query?: string, cursor?: string, limit?: number }` → `ProjectListPage` | main only。仅读已登记记录；空/缺省查询返回列表。排序采用全局 `project_sort`，默认 recent；搜索不遍历磁盘。 | `PA_VALIDATION_INPUT_TOO_LARGE`、`PA_IPC_INVALID_REQUEST`、存储错误；S2-04 追加 `PA_PROJECT_CONFLICT_REVISION` 表示续页与查询/排序/数据快照冲突，须从首屏重新读取。 |
| `project_get` | `{ projectId: string }` → `ProjectView` | main only。读取已登记记录；无效授权仍返回固定可用状态，不因列表查询自动修复授权。 | `PA_PROJECT_NOT_FOUND`、存储错误。 |
| `project_select` | `{ projectId: string }` → `{ activeProject: ActiveProject }` | main only。必须在 Rust 复核登记、授权和路径身份后，更新活动项目并记录一次显式使用；不打开目录、不启动任务。成功后发无载荷状态失效事件。 | `PA_PROJECT_NOT_FOUND`、`PA_PROJECT_PATH_UNAVAILABLE`、`PA_PROJECT_PATH_CHANGED`、存储错误。 |
| `project_remove` | `{ projectId: string, expectedRevision: number }` → `{ removed: true, retainedForHistory: boolean }` | main only。修订匹配后，在事务内撤销授权、清理活动项目、记审计并删除或保留历史引用行；永不删除源文件。 | `PA_PROJECT_NOT_FOUND`、`PA_PROJECT_CONFLICT_REVISION`、`PA_AUDIT_WRITE_FAILED`、存储错误。任何失败均不产生部分撤权。 |
| `project_open_directory` | `{ projectId: string }` → `{ systemAccepted: true }` | main only。Rust 从已授权记录解析目录并在交付前重新核验；成功后更新使用记录。前端不能提供路径、URL、Shell 字符串或程序参数。 | `PA_PROJECT_NOT_FOUND`、`PA_PROJECT_PATH_UNAVAILABLE`、`PA_PROJECT_PATH_CHANGED`、`PA_PROJECT_ACCESS_DENIED`、`PA_PROJECT_OPEN_REJECTED`、`PA_PROJECT_OPEN_TIMEOUT`。不自动重试。 |
| `project_pick_file`（可选） | `{ projectId: string }` → `{ relativePath: string } \| { outcome: "cancelled" }` | main only。选择器限定在已授权目录；返回值必须是经服务端验证的相对路径。取消无副作用。 | 路径授权/身份错误、`PA_PROJECT_PICKER_FAILED`；取消不是错误。 |
| `project_open_file` | `{ projectId: string, relativePath: string }` → `{ systemAccepted: true }` | main only。Rust 拒绝绝对路径并按组件边界解析相对路径；复核后交付默认程序或已验证编辑器。编辑器使用结构化参数数组，不经 Shell。成功后更新使用记录。 | `PA_PROJECT_NOT_FOUND`、`PA_PROJECT_PATH_UNAVAILABLE`、`PA_PROJECT_PATH_CHANGED`、`PA_PROJECT_FILE_OUTSIDE_PROJECT`、`PA_PROJECT_FILE_MISSING`、`PA_PROJECT_ACCESS_DENIED`、`PA_PROJECT_OPEN_REJECTED`、`PA_PROJECT_OPEN_TIMEOUT`。不自动重试。 |
| `project_git_summary` | `{ projectId: string, refresh?: boolean }` → `GitSummary` | main only。授权工作目录内只读查询；有界时间/输出，使用固定参数与受控环境，不执行项目脚本、别名或外部 helper。 | 非仓库、Git 缺失、超时和异常作为 `GitSummary.state` 降级返回；仅 IPC、路径、存储错误使用 error envelope。 |
| `settings_get`（扩展 project scope） | `{ scope: "project", projectId: string }` → `ProjectSettingsView` | main only。要求项目已登记、授权有效且路径身份复核通过；返回项目值和全局继承来源。仅读取。`settings_update` 的 project scope 仍拒绝。 | `PA_PROJECT_NOT_FOUND`、`PA_PROJECT_PATH_UNAVAILABLE`、`PA_PROJECT_PATH_CHANGED`、`PA_IPC_PERMISSION_DENIED`、设置/存储错误。 |
| `app_get_status`（步骤 2.8 扩展） | `{}` → 现有 `AppStatus` 加 `activeProject: ActiveProject \| null` | main 可读完整状态；overlay 只获最小项目 ID/目录名用于上下文提示，不含绝对路径、搜索结果、Git 摘要或草稿。 | 沿用现有固定 IPC/状态错误；路径失效或项目移除时服务端清空活动项目并发布失效事件。 |

所有拟议命令的完整 v1 JSON envelope 上限为 8 KiB UTF-8。`projectId` 必须是 Rust 生成的 UUID v4；修订号为 `1..=Number.MAX_SAFE_INTEGER`。列表 `limit` 缺省为 50，有效范围 `1..=100`；`query` 可为空且最多 256 UTF-8 字节；游标最多 512 字节，由服务端生成并保持不透明。项目文件 `relativePath` 最多 4 KiB UTF-8，不能为空、绝对路径、UNC、ADS、`.`/`..`/空路径段；拒绝 Windows DOS 设备名、非法字符及尾随点/空格。项目名称/显示名最多 1 KiB UTF-8，显示路径最多 4 KiB；活动项目名不超过 1 KiB。Git 分支最多 512 字节；设置最多 16 个不重复键，修订号为安全整数。长度越界返回 `PA_VALIDATION_INPUT_TOO_LARGE`；格式/范围错误返回 `PA_IPC_INVALID_REQUEST`，文件路径越界返回 `PA_PROJECT_FILE_OUTSIDE_PROJECT`。任何错误都不回显输入值。

项目列表与搜索请求不更新 `lastUsedAt`；Git 刷新不更新使用时间；只有 `project_select` 成功以及操作系统接受目录/文件打开后更新使用记录。选择或打开的变更请求如果前端超时/停止等待，客户端不得自动重发；应查询权威状态或由用户主动再次操作。

## 数据、审计、日志与事件

- 项目路径和文件系统授权身份是本地敏感数据，只存应用数据库；身份不暴露给 UI。目录/文件路径不得写入产品 JSONL、命令日志、审计摘要、关联 ID、错误文本、截图或诊断报告。
- 项目命令沿用现有白名单日志形状：静态命令名、时间、结果、固定错误码及服务端 correlation ID；不记录请求参数、项目 ID、搜索词、文件名、路径、Git 输出或原始异常。
- 授权授予/重新授权/撤销属于需审计的权限变化。审计至少记录事件类型、UTC 时间、服务端 correlation ID、项目 ID 和 outcome，不记录路径或用户输入。授权变更及其审计同一 SQLite 事务；审计写入失败时拒绝变更并保留旧授权状态。若 schema 当前不能表达项目审计事件，后续只能追加迁移（当前 v4 之后），不可改写已发布迁移或校验和。
- 普通列表/搜索、Git 刷新和系统打开按 `project_usage` 需要维护本地最近使用数据；不额外采集搜索历史或文件工作痕迹。项目级设置读可按现有 `config_read` 审计类别记录 project ID；失败和保留期沿用现有审计约定。
- 项目选择、撤销、失效清除与权威活动状态变化沿用 `project-assistant://event/v1`，事件只携带状态失效通知，不放项目 DTO、路径、查询词或文件名。前端先监听再查询；streamId 改变、sequence 缺口或窗口重建后重查快照。
- 统一错误响应仅使用固定 `category/code/message/suggestion/retryable`；不得传出 SQL、OS 详细路径、Shell 错误原文、Git stderr 或系统异常。

## 固定错误定义

窗口/来源授权拒绝复用 `PA_IPC_PERMISSION_DENIED`；IPC envelope、版本、通用大小限制和存储错误复用当前目录中已有错误码。以下项目错误码已加入 Rust `ErrorCode` 和前端固定错误目录，并经相同本地安全展示逻辑处理；由于项目业务命令尚未注册，当前不会由项目运行时操作产生。

| 错误码 | 类别 | 固定含义 |
| --- | --- | --- |
| `PA_PROJECT_NOT_FOUND` | validation | 项目记录不存在或不可用于此操作。不得披露查询 ID 对应的其他数据。 |
| `PA_PROJECT_PICKER_FAILED` | environment | 本地目录/文件选择器不可用或失败；不回显路径。用户取消则不返回该错误。 |
| `PA_PROJECT_PATH_UNSAFE` | permission | 目标不是被允许的本机目录，或路径包含被拒绝的穿越、设备、UNC、ADS/重解析逃逸形式。 |
| `PA_PROJECT_PATH_UNAVAILABLE` | environment | 已授权目录缺失或当前不可访问；记录保留，不自动重绑定。 |
| `PA_PROJECT_PATH_CHANGED` | conflict | 路径仍存在但文件系统身份与已授权身份不一致；需用户重新选择并明确授权。 |
| `PA_PROJECT_FILE_OUTSIDE_PROJECT` | permission | 相对文件目标解析后越过授权根目录，或经过不安全重解析点。 |
| `PA_PROJECT_FILE_MISSING` | validation | 项目内目标文件不存在。 |
| `PA_PROJECT_ACCESS_DENIED` | permission | 操作系统拒绝访问授权目标；不回显 OS 错误文本。 |
| `PA_PROJECT_OPEN_REJECTED` | external | 系统未接受目录/文件打开请求；不得报告虚假成功。 |
| `PA_PROJECT_OPEN_TIMEOUT` | timeout | 等待系统接受打开请求超时；调用方不得自动重发。 |
| `PA_PROJECT_CONFLICT_REVISION` | conflict | 项目在用户查看后已发生并发变更；重读后由用户重新提交。 |

Git 查询的常见不可用状态由 `GitSummary` 表达，不新增错误封套；无法安全确定工作目录时应拒绝 Git 子进程并返回项目路径固定错误。

## 路径与权限安全用例

以下是后续 S2-02/S2-05/S2-08 的契约测试期望，本步骤只完成人工静态审阅映射，不声称已执行运行时或硬件测试。

| 场景 | 期望 |
| --- | --- |
| 前端伪造 `absolutePath`、授权标志或未定义字段 | 严格反序列化/命令参数拒绝；发生在数据库、文件系统或系统打开副作用前。 |
| 用户取消本地目录/文件选择器 | 返回取消结果，项目记录、授权、最近使用时间和成功审计均无变化。 |
| 同一目录通过大小写/规范化/别名重复登记 | 按 Windows 规范化路径及稳定目录身份返回同一项目 ID，不重复登记。 |
| `C:\work\app` 与同前缀 `C:\work\app-old` | 按路径组件比较，不把字符串前缀当父子关系；不得越权。 |
| 相对路径含 `..`、绝对路径、UNC、设备命名空间、DOS 设备名、ADS、Windows 非法字符或尾随点/空格 | 在路径解析/系统交付前拒绝，返回固定路径安全错误。 |
| 授权根或项目内目标包含指向授权范围外的 junction/symlink/reparse point | 拒绝使用；不能仅靠一次字符串规范化或前端检查放行。 |
| 登记后目录被删除、替换、改名或身份变化 | 列表标记 `missing`/`identity_changed`；select/open/Git/settings scope 失败；不自动授权新目标、不扫描替代目录。 |
| 路径在核验与系统交付之间被替换 | 适配器必须将最终交付绑定至已核验身份；无法保证时拒绝对应打开能力，不把提前检查当作安全证明。 |
| 调用方提交任意或伪造项目 ID | 仅能访问自身已登记授权记录；未知 ID 固定失败，不接受路径参数作为回退。 |
| overlay、远端 URL、动态未知 WebView 调用项目命令 | 真实 Rust origin 校验与 capability 双重拒绝，且无写入/文件系统/OS 副作用。 |
| SQLite/审计写入失败时登记、重新授权或撤销 | 同事务回滚，已有授权和数据保持一致；不显示成功。 |
| 搜索词、路径、Git stderr 或系统错误包含敏感哨兵 | 返回、持久日志、审计和诊断均不包含原始哨兵。 |

路径根身份的具体 Windows 实现（卷/文件 ID 获取方式、允许的本地文件系统、每级重解析点处理、最终句柄或系统交付约束）由 2.2/2.5 决定并以测试证明；若不能满足“核验后仍是同一授权目标”，不得开放系统打开命令。

## 当前实现对照与验收结论

当前实际可调用命令仍以 `ImplementedCommandMap`、Rust `invoke_handler!` 注册表、`build.rs` command manifest 和 `src-tauri/capabilities/default.json` 为准。它们目前没有任何项目业务命令；`app_get_status` 仍仅含现有 `activeContext`，项目服务仍报告 `PA_PROJECTS_NOT_IMPLEMENTED`，`settings_get` 的 project scope 仍由 Rust 拒绝。以上状态是本契约要求的未实现边界，不应为完成 2.1 而放宽。

契约逐项覆盖 S2-01：

1. FR-PM-001–007/M2 与登记、列表、搜索、展示、打开、移除语义：见领域语义与拟议 IPC 矩阵。
2. Project、availability、authorization、GitSummary、ActiveProject、ProjectSettings DTO：见 DTO 草案。
3. 用户目录选择、身份绑定/撤销、重启复核、失效状态：见领域授权语义与路径安全用例。
4. 新命令、main/overlay/远端权限、输入限制、固定错误、审计/日志：见权限矩阵、错误表与审计段。
5. 对 S2-02–2.8 和 AT-S2-001–012 的接口/拒绝行为覆盖：见各命令失败定义与路径/权限用例表。

**本步骤验证**：环境实测 Node.js 24.18.0、npm 11.16.0、Rust/Cargo 1.99.0。`npm run build` 通过；`npm run test:ipc` 29 项通过；`cargo fmt --check --manifest-path src-tauri/Cargo.toml` 通过；`cargo check --locked --manifest-path src-tauri/Cargo.toml` 通过；`cargo test --locked --manifest-path src-tauri/Cargo.toml --lib` 111 项通过、4 项按项目现有配置忽略。Rust 检查输出两条 warning：`logging::Failure::Unsupported` 未构造、`settings::change_resident` 未使用；不影响命令退出码。新增 TypeScript 项目响应/请求校验器、`ImplementedCommandMap` 类型断言及 Rust DTO serde/边界测试，验证 DTO 字段命名、选择器/移除/打开结果、项目设置来源、Git 降级字段、输入范围和未知字段拒绝。另以静态 IPC 契约测试断言项目接口未进入 Rust 注册表、Tauri command manifest 或 main/overlay capability。对照《计划文档》FR-PM-001–007、第 22.2/22.4/22.5 节、IPC v1、当前 Rust 注册清单、main capability 与 `settings::Scope` 逐项审阅，并将 R-019 更新为“契约明确、生产路径/打开仍待验收”。未执行生产目录授权、文件系统操作、项目仓储或真实桌面验收。

**后续**：步骤 2.2 落实 Windows 目录选择、身份与授权；2.3 落实追加迁移/事务/重启；2.4–2.8 分别实现查询、打开、Git、页面、活动项目和设置读取。每个接口实施时如需调整本契约，先同步更新本记录及计划追踪，再一并更新 Rust/TypeScript DTO、运行时校验、命令清单、capability、固定错误和测试。
