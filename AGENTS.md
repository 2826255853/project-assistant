# 项目接续说明

本文件于 2026-10-16（Asia/Shanghai）在系统重装后，根据本地源码和既有记录整理。用于后续会话快速恢复项目上下文；进度变化后同步更新。下述历史验收结论不代表重装后的环境已复验。

## 产品与当前进度

本机项目助手（Project Assistant）是 Windows 桌面开发工作台，规划包含项目发现/打开、Codex 对话与审批、差异查看、Git 和搜索，以及全局常驻悬浮对话入口。

- 阶段 0 已按应用自管 profile 的替代范围通过。
- 阶段 1.1 主框架与常驻入口：有条件通过；1.2 IPC/事件：已完成；1.3 数据库与启动恢复：通过；1.4 设置与常驻恢复：有条件通过；1.5 错误呈现、日志、审计与清理：通过（2026-10-18）。
- 1.1/1.4 的实际托盘鼠标操作、物理显示器拔插和实时 DPI 变化仍需人工复核，复审点为 1.10；注入测试不能替代硬件验收。
- 步骤 1.6（S1-05）系统凭据库已通过本机合成凭据验收（2026-10-19），2026-10-20 已在固定基线 Node/npm 24.18.0/11.16.0、Rust 1.99.0 下完成收尾复验：7 个独立进程读写/替换/删除/清理、真实 OS 与 SQLite 提交故障补偿、dev/built 桌面及存储回归。服务/权限失败为适配层注入。1.7、1.8 已随后完成，1.9 AI 配置页已实现；2026-10-23 已补通过 `verify:providers`，包括 104 项 Rust 单测、实际 CLI/双合成 HTTPS、隔离回归和 built/dev provider/credential 桌面场景。页面控件仍待人工点验，后续为 1.10 阶段回归。
- 步骤 1.7（S1-06）已实现正式 Rust 自有 stdio 候选适配器和配置层实际路由隔离，见 `docs/phase-1/step-1.7.md`；每候选独立固定上游转发层仅在 Rust 内存中使用站点 Key，子进程只持临时入口凭据。正常启动只初始化/清理自有资源，不启动 Codex。项目管理、AI 配置和真实任务入口尚未开放；1.8 profile/能力/切换恢复已完成配置层验收；阶段 3 网关仍待实现。不能宣布 M1 或阶段 1 整体完成。
- 阶段 2 步骤 2.1（S2-01）于 2026-10-25 完成契约定义；步骤 2.2（S2-02）于 2026-10-26 实现 Windows 原生目录选择、NTFS/ReFS 本地路径约束、逐级重解析点拒绝、短期身份句柄、卷序列号/文件 ID 身份绑定、大小写去重键和替换复核，合成文件系统测试通过，记录见 `docs/phase-2/step-2.2.md`。2.3 已于 2026-10-27 通过仓储/恢复范围：schema v5、登记/详情/撤权 main-only 命令、授权/审计与使用事务、启动复核；当轮 `verify:projects` 通过 136 Rust、29 IPC、15 离线、46 存储及 built/dev 项目桌面，详见 `docs/phase-2/step-2.3.md`。列表搜索最新状态见下一条；系统打开、Git、活动项目及原生选择器人工点验仍待 2.5–2.9，M2 未通过。

- 步骤 2.4（S2-04）于 2026-10-28 通过列表/搜索服务范围：新增 main-only `projects_list`、全局 project_sort、稳定分页、中文/路径子串和字符顺序匹配、响应代次及 IME 控制器。`verify:projects` 最新报告为 `artifacts/phase-2-4/report.json`，142 Rust、33 IPC、15 离线、52 存储和 built/dev 通过；1,000 条合成记录 WebView IPC 搜索 P95 为 built 21.1 ms / dev 24.3 ms。项目查询仅读元数据及已保存可用性，不写使用记录、不遍历文件系统；页面/实际中文输入框在 2.7。系统打开、Git、活动项目及选择器人工点验仍待 2.5–2.9，M2 未通过。

优先阅读 `README.md`、`计划文档.txt` 第 21/22 节、`docs/phase-2/step-2.4.md`、`docs/phase-2/step-2.3.md`、`docs/phase-2/step-2.1.md`、`docs/phase-1/step-1.7.md` 和 `step-1.4.md`。旧阶段文档描述的是当时状态，例如早期“设置/业务数据库未接入”已被 1.3/1.4 取代；结合最新记录和源码判断。

## 已确认的产品约定

- cc-switch 由用户在应用外手动管理。生产应用不自动读取、导入、同步或切换其 profile/Key，不依赖安装/版本，不修改用户共享 Codex 配置。依据 `docs/phase-0/self-managed-profile-adr.md`（ADR-0011）。旧调查脚本仅是历史实验，不能作为生产适配器复用。
- 用户在应用内单独创建 profile 和提交 Key；secret 由系统凭据库承载，普通 SQLite 仅存安全引用/版本。凭据库不可用不得降级明文或继承外部认证。
- 自有 Codex 子进程须使用应用专属配置和受控环境；仅独立 home 不足以证明实际路由隔离。环境/项目覆盖及共享可变代理无法核验隔离时拒绝启动任务，只管理本应用自有进程。
- OpenAI 官方源相关功能（官方认证/请求及官方源与中转站切换）仅作预留，无需实际测试；未测试、失败或历史结果不影响验收/发布判定，也不得标记为实测通过。非官方源按既定标准验收。
- 真实 Key 和任务功能须等待对应凭据、隔离与恢复功能门通过。摘要采集默认关闭；保存留存期限不会开启采集。

## 技术栈与代码入口

- React 19、TypeScript 6、Vite 8；Tauri 2、Rust edition 2024；SQLite 通过 `rusqlite` bundled/backup 接入。精确依赖以两个 lock 文件为准。
- `src/main.tsx`：按真实窗口身份选择主窗口 Bootstrap 或 OverlayPage。
- `src/app/Bootstrap.tsx`：桌面初始化、失败重试、渲染边界和显式阶段 0 页；`src/App.tsx`：导航、工作台、诊断和占位页面。
- `src/pages/SettingsPage.tsx`：正式全局设置表单；`OverlayPage.tsx`：悬浮球/展开面板，草稿仅在本次进程内保留，发送按钮不可用。
- `src/ipc/contracts.ts`：DTO；`client.ts`：集中调用入口；`response.ts`、`settings.ts`、`state.ts`：运行时返回校验和状态恢复；`transport.ts`：超时/停止等待。`CommandMap` 含未来接口，**实际可调用范围看 `ImplementedCommandMap` 和 Rust 注册表**。
- `src/ipc/projects.ts`、`src-tauri/src/project_contract.rs`：项目 DTO 与校验；2.3 登记/详情/移除、2.4 列表搜索已接入 client、Rust 注册和 main capability，完整项目服务状态仍待后续。`src/ipc/projectSearch.ts` 为分页/响应代次/IME 控制器，`storage/projects/search.rs` 为元数据搜索、稳定排序和指纹游标。`src-tauri/src/project_paths.rs` 与 `project_paths/windows.rs`：2.2 Windows 选择器、路径核验与身份句柄；`storage/projects.rs` 为 2.3 持久仓储/使用事务及启动复核，`projects.rs` 为服务入口，实际系统打开仍须按 2.5 单独验证竞态。
- `src-tauri/src/lib.rs`：命令封装、启动顺序、状态注入、窗口生命周期和命令注册；`desktop.rs`：初始化模式；`app_state.rs`：权威状态与事件游标。
- `src-tauri/src/settings.rs`：设置 DTO、校验、运行状态应用/回滚和统一更新；`overlay.rs`：托盘、快捷键、位置、显示器校正与窗口操作。
- `src-tauri/src/storage/mod.rs`：连接保护、迁移/备份、启动恢复及 StorageState；`repositories.rs`：类型化仓储；`storage/settings.rs`：设置事务、修订与旧 JSON 一次迁移。
- `src-tauri/src/owned_process/`：1.7 正式自有候选进程、配置/来源核验、固定上游转发、当前用户 DACL、Windows Job 与自有 home 清理；Rust 内部 `probe_candidate` 留给 1.8 使用，未新增业务 IPC。`verify:isolation` 为实际 CLI/合成 HTTPS 验收；生产不允许用环境变量开启 loopback/自签 CA。
- `src-tauri/src/ipc.rs`：严格请求、窗口授权、固定错误和白名单命令日志；`safety.rs` 是阶段 0 路径/审批参考，不是完整生产安全服务。
- `src-tauri/build.rs`、`capabilities/*.json`：命令清单与窗口最小授权；新增命令须同步 Rust 注册、DTO/返回校验、权限和契约。
- `scripts/verify-*.mjs`：验收入口；`scripts/ipc/`：前端协议/状态/设置及桌面场景；`scripts/codex/`：阶段 0 Node 客户端和合成站点实验，不等于正式 Codex 服务。

## 编辑时保持的约束

- 正式 IPC v1：`{request:{apiVersion:1,params}}`，返回带服务端 `correlationId` 的成功/错误信封。拒绝未知字段/版本/类型；前端不得暴露原始异常、SQL、外部输出或 secret。
- 同时使用 Tauri capability 和 Rust 真实窗口 label/URL 授权。完整设置仅开放本地 main；overlay 只能执行受限动作。main 查询不返回 overlay 私有草稿。
- 事件频道 `project-assistant://event/v1`，以 streamId/sequence 跟踪无载荷状态失效通知；先监听后查询，缺口/重建后查询权威快照。超时/停止等待不代表服务端取消，变更请求不得自动重发。
- 设置 `expectedRevision` 是整个全局批次的修订号。非法值、冲突、SQL 或快捷键失败拒绝整批并保持/恢复旧状态；冲突必须重读后由用户重新提交。当前项目范围不授权，非空 AI 默认仅由 provider_switch 验证提交，settings_update 只允许显式清空，审批策略只接受 ask。
- 常驻入口首次默认启用，快捷键 Ctrl+Alt+Space；主窗口 X 在常驻启用时隐藏，停用时退出；明确“退出应用”始终退出。停用状态重启保持停用，启用后重启显示收起悬浮球。
- 数据库位于 Tauri `app_data_dir` 下的 `project-assistant.sqlite3`，当前 schema v5（1.6 追加凭据状态/审计，1.8 追加默认修订/操作/受限状态，2.3 追加项目授权身份/修订/失效/移除与审计类型）。**已有迁移 SQL 与校验和不可改写，只追加新版本**。升级先做 SQLite 一致性备份；损坏/高版本/迁移失败保留原库，不删除重建、不自动降级、不重放任务/审批。
- 产品持久日志与设置审计已在 1.5 接入。其规范见 `docs/phase-0/errors-and-logging.md`：JSONL 白名单、单行 4 KiB、单文件 5 MiB、最多 5 文件/7 天/25 MiB；审计默认 90 天，配置变更与审计同事务，高风险审计失败拒绝执行。清理核验应用目录身份并拒绝联接/符号链接越界；审批决定/消费的完整原子性仍待阶段 3。
- 开发与验收只声明实际验证范围。历史报告、合成测试、当前实现与人工待验应分别说明；不要把阶段 0 实验通过写成生产能力通过。

## 环境与验证入口

仓库固定基线：Node.js 24.18.0（`.node-version`）、npm 11.16.0、Rust 1.99.0 / `x86_64-pc-windows-msvc`（`rust-toolchain.toml`）。Windows 还需 MSVC/C++ 工作负载、Windows SDK 与 WebView2。这是项目既定版本，不代表重装后已安装或已重新核验。

常用命令：

```powershell
npm ci
npm run tauri dev
npm run build
npm run test:ipc
npm run test:codex
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
cargo check --locked --manifest-path src-tauri/Cargo.toml
npm run verify:settings
npm run verify:state
npm run verify:storage
npm run verify:projects
npm run verify:credentials
npm run verify:isolation
npm run verify:providers
```

`npm run dev` 只运行浏览器前端，没有真实 Tauri IPC；开发端口固定 1420。完整桌面验证支持 dev/built 两种资源模式，会启动真实窗口；执行前按 README 释放相关端口/窗口。`test:codex` 为离线测试；`verify:codex` 会调用真实服务并产生模型用量，不是普通离线回归。

生成物 `node_modules/`、`dist/`、`src-tauri/target/`、`artifacts/`、Tauri 自动 schema/permissions 不作为业务源码提交。历史验收报告不随源码保留，重装后应重新生成适用证据。

2026-10-18 环境与仓库检查：PATH 已可找到 node/npm/cargo/rustc/git，实测 Git 为 2.55.0.windows.5；`node_modules/`、`dist/` 和 `artifacts/` 已存在，其存在不代表验收通过。本次按用户要求，以当前本地文件重新初始化 Git 仓库，默认分支为 `main`；这是新仓库，不含原有提交历史，未配置远程仓库。现有忽略规则覆盖依赖、构建产物、验收报告及 Tauri 自动生成文件。Git 提交姓名和邮箱已在本仓库内配置为 `2826255853` / `2826255853@qq.com`，以当前本地源码建立首次基线提交；不修改全局 Git 身份。本次仅处理仓库建立，未执行应用构建或测试，也未重新核验 Node/npm/Rust 版本和桌面依赖。

## 深入阅读索引
- 2026-10-22 步骤 1.8 配置层完成：优先读 docs/phase-1/step-1.8.md，入口 verify:providers。九个 main-only profile/operation 命令、隔离 CLI model/list、24 小时能力/五分钟证据、八项 FIFO、取消/等待/三类恢复及不可变快照已实现。schema v4 仅追加；非空默认由 provider_switch 提交。启动清旧 ready、不重放；恢复失败 blocked。当前 CLI 0.163.0-alpha.11。恢复为生产状态机注入、任务为合成。2026-10-23 步骤 1.9 页面已接入 profile/凭据/能力选择/测试/切换/恢复交互，`verify:providers` 已通过 built/dev provider/credential 桌面场景，记录见 docs/phase-1/step-1.9.md；页面控件仍待人工点验。阶段 3 网关/真实并发和 1.10 硬件仍待。新入口 src-tauri/src/providers/、src/ipc/providers.ts、scripts/verify-providers.mjs、src/pages/AiConfigPage.tsx。

- 2026-10-21 进程与配置层实际路由隔离通过：`docs/phase-1/step-1.7.md`。`verify:isolation` 为完整入口，覆盖 75 项 Rust、19 项 IPC、15 项离线、14 个实际 CLI/双合成 HTTPS 场景及 dev/built 主框架/凭据共 4 桌面场景，清理通过；3 个 ignored worker 均有父入口。真实任务并发/运行中切换、1.8 完整恢复仍待对应功能门；R-014/R-016 未整体关闭。CLI 实测 0.163.0-alpha.10.2 不接受旧 untrusted 配置，当前使用 on-request 且审批一律不批准。

- 2026-10-20 凭据收尾复核：`docs/phase-1/step-1.6.md`。固定 Node/npm/Rust 基线下 `verify:credentials` 通过：64 项 Rust、19 项 IPC、15 项离线 Codex、release check 与 dev/built 桌面；1 个 ignored worker 由父测试显式启动。`verify:storage` 的 31 项存储测试、schema v3 独立进程重开与清理通过；已补齐 2026-10-19 的 Node/npm 版本差异复验。持久化子门通过，R-016 的路由/认证文件/未来导出仍待后续验收。
- 2026-10-19 数据库修复与复验：`docs/phase-1/storage-fixes-2026-10-19.md`。存储验收按连续迁移清单核对版本；SQLite 损坏/锁冲突按实际原因映射，能力缓存仅认可当前 profile revision 及有效观测区间。早期 30 项存储/54 项 Rust 和 IPC 单项失败为当时结果；最新结论见凭据收尾及存储追加复验。
- 当前实施/验收：`docs/phase-1/step-1.1.md` 至 `step-1.4.md`；IPC：`docs/phase-1/ipc-state-contract.md`（含 1.4 补充）。
- 存储：`docs/phase-0/data-model.md`、`storage-migration-adr.md`；以追加迁移和最新实施记录解释早期状态说明。
- 安全与后续日志工作：`docs/phase-0/security-checklist.md`、`errors-and-logging.md`、`risks.md`。
- 配置范围及功能门：`docs/phase-0/self-managed-profile-adr.md`、`phase-gate-review.md`。
- 产品规划与阶段依赖：`计划文档.txt`，尤其第 21.2、21.4、21.5 节。
