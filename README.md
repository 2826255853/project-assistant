# 本机项目助手 · 桌面基础

阶段 2 步骤 2.4（S2-04）已于 2026-10-28 完成项目列表、全局排序、稳定分页和中文/路径模糊搜索；main 新增 `projects_list`，并提供防止过期响应覆盖及 IME 组合输入的客户端控制器。`npm run verify:projects` 已通过：142 项 Rust、33 项 IPC、15 项离线 Codex、52 项存储及 built/dev 真实桌面；1,000 条合成记录的搜索 P95 分别为 21.1/24.3 ms，低于 300 ms。报告 `artifacts/phase-2-4/report.json`，实现与测量边界见 [步骤 2.4](docs/phase-2/step-2.4.md)。2.3 的仓储/身份与 schema v5 保持；系统打开、Git、页面、活动项目及选择器人工点验仍按 2.5–2.9 推进，M2 未通过。详见 [步骤 2.3](docs/phase-2/step-2.3.md)、[项目契约](docs/phase-2/step-2.1.md) 和 [计划第 22 章](计划文档.txt)。

步骤 1.1–1.5 已分别实现桌面主框架、正式 IPC/状态、数据库恢复、全局设置以及错误呈现/日志/审计/清理。步骤 1.6 系统凭据库及安全引用已于 2026-10-19 通过本机合成凭据验收，并于 2026-10-20 在固定工具链基线完成收尾复验。步骤 1.7 已实现 Rust 自有 stdio 候选适配器、当前用户专属 home、环境/项目覆盖拒绝及每候选独立固定上游转发层；实际 CLI 与两个合成 HTTPS 站点覆盖参数/Key 绑定、外部夹具共存、重定向拒绝和清理。真实 Key、profile 和任务入口继续受限；1.8 已完成 profile/能力/FIFO 切换恢复及不可变快照，1.9 已接入 AI 配置页、凭据表单、能力选择、连接测试和默认切换状态展示。步骤 1.1/1.4 的真实托盘、物理显示器和实时 DPI 仍待 1.10 人工复核。详细规划以 [计划文档 v1.5](计划文档.txt)、[步骤 1.8 实施记录](docs/phase-1/step-1.8.md)、[步骤 1.9 实施记录](docs/phase-1/step-1.9.md) 和 [替代方案 ADR-0011](docs/phase-0/self-managed-profile-adr.md) 为准。

步骤 1.2（S1-02）已接通正式 IPC、服务状态查询和版本化事件；主窗口与悬浮入口共享服务状态，事件缺口和窗口重建可重新查询恢复。详情见 [验收记录](docs/phase-1/step-1.2.md) 与 [正式契约及命令授权矩阵](docs/phase-1/ipc-state-contract.md)。复现入口为 `npm run verify:state`，报告位于 `artifacts/phase-1-2/report.json`。

## Codex 配置约定

cc-switch 由用户在应用外手动管理。本应用不自动读取、导入、同步或切换其 profile/Key，不依赖其安装或版本，不修改用户共享 Codex 配置。用户在应用内单独创建 profile 和提交 Key；系统凭据库承载 Key，自有 Codex 进程使用应用专属配置与受控环境。此前要求用户切回 cc-switch 的约定撤销。

生产功能须验证环境/项目配置覆盖、站点绑定、失败恢复及实际路由隔离；共享可变代理或无法核验隔离的配置不得启动任务。1.7 的配置层由正式 Rust 适配器验证，1.8 的切换恢复和阶段 3 的真实任务仍按功能门接入；阶段 0 实验脚本不代表这些生产能力。

今后 Codex 的 OpenAI 官方源相关功能仅作为预留功能，包括官方认证、官方请求及官方源与中转站之间的切换，无需实际测试。判断测试、阶段验收或发布是否通过时，不考虑这些功能的测试结果；其未测试、失败或历史结果均不构成阻塞，也不得记为实测通过。本约定覆盖计划及关联文档中的相关要求，替代此前“暂缓测试、保留为未完成验收项”的约定。非官方源功能仍按既定标准测试和验收。

## 验证环境

本轮目标：Windows 11 x64，版本 10.0.26300。产品最低 Windows 版本和 ARM64 支持尚未决定，不能从本机结果推断兼容性。

- Node.js 24.18.0、npm 11.16.0。
- Rust 1.99.0，`x86_64-pc-windows-msvc`；版本由 `rust-toolchain.toml` 固定。
- Visual Studio 的「使用 C++ 的桌面开发」工作负载，含 MSVC 和 Windows SDK。
- Microsoft Edge WebView2 Runtime；本轮实测版本 154.0.4258.62。
- Git 2.55.0.windows.5；步骤 0.4 的只读能力通过独立实验验证。Codex 不参与步骤 0.2 的运行。

Tauri 官方系统依赖说明：https://v2.tauri.app/start/prerequisites/

## 从干净仓库运行

1. 安装上述 Windows 依赖、指定 Node.js/npm 和 Rustup；Rustup 安装后重新打开 PowerShell，让 `%USERPROFILE%\.cargo\bin` 进入 PATH。
2. 切换到仓库目录。首次执行 Rust 命令会按 `rust-toolchain.toml` 下载指定工具链。首次 npm/Cargo 安装需要访问 npm registry、crates.io 和 Rust 下载站点。
3. 执行：

```powershell
npm ci
rustc --version
cargo --version
npm run tauri -- info
npm run build
cargo check --locked --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

若安装后当前终端还未刷新 PATH，可以仅在当前 PowerShell 中执行：

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
```

开发模式使用固定端口 1420；端口占用时退出并提示错误。Rust 首次编译时间较长，后续会复用缓存。正常启动先显示初始化状态，再进入工作台；导航可用 Tab 与 Enter/空格操作，切换后焦点移至页面标题。未开放的服务显示明确提示，设置表单只作禁用预览。初始化失败提供“重试初始化”和“退出应用”。常驻模式开启时，主窗口 X 隐藏窗口；停用常驻模式后 X 退出应用。“退出应用”始终结束应用；若 Vite 仍在终端运行，按 Ctrl+C 结束开发命令。

`npm run dev` 仅启动浏览器前端，普通浏览器中没有 Tauri IPC，会显示固定本地服务不可用提示。请使用 `npm run tauri dev` 查看真实主窗口。

## 可重复验证

步骤 1.7 隔离验收入口为 `npm run verify:isolation`，报告位于 `artifacts/phase-1-7/report.json`。它使用正式 Rust 适配器、实际 CLI 和两个一次性合成 HTTPS 站点，不调用真实模型、不读取外部配置、不使用真实 Key；包括 release check 和 dev/built 桌面。测试证书仅在 `cfg(test)` 客户端中信任，不安装到 Windows；需要 Git 所带 OpenSSL，详见 [步骤 1.7](docs/phase-1/step-1.7.md)。`--routes-only` 只作路由调试，报告单独写到 `artifacts/phase-1-7-routes-only/report.json`，不能替代完整入口。

步骤 1.6 凭据收尾入口为 `npm run verify:credentials`，报告位于 `artifacts/phase-1-6/report.json`。该入口只使用临时合成凭据，覆盖独立进程与 dev/built 桌面，并清理确切自有记录；系统服务/权限失败采用适配层注入。2026-10-20 已在固定基线 Node/npm 24.18.0/11.16.0、Rust 1.99.0 下通过该入口和 `verify:storage`，补齐此前的版本差异复验，详情见 [步骤 1.6](docs/phase-1/step-1.6.md)。

步骤 1.4 已提供可用设置页：全局设置与常驻开关/快捷键/位置统一保存到 SQLite；修订冲突须重新读取后提交，非法值/写入或快捷键注册失败保持旧值。原常驻 JSON 仅一次性迁移，不再写入。AI 默认值与真实凭据仍不开放。

```powershell
# 全局设置、回滚/冲突、两种资源模式独立进程退出重启
npm run verify:settings
```

记录与人工复核边界见 [步骤 1.4](docs/phase-1/step-1.4.md)，报告位于 `artifacts/phase-1-4/report.json`。托盘回调与显示器缺失注入已纳入自动验收；实际托盘鼠标操作和物理显示器/DPI 变化不记为自动实测通过。

```powershell
# 步骤 1.1：构建、Rust/IPC/Codex 离线测试，两种桌面模式的正常/失败/阶段0入口及系统打开回归
npm run verify:shell

# 步骤 1.2：正式 IPC、状态、事件恢复、窗口 ACL 与双资源模式桌面验收
npm run verify:state

# 启动真实桌面窗口，自动验证往返、错误、重试和窗口关闭退出
npm run verify:desktop

# 构建嵌入前端资源的调试应用，不生成安装包
npm run tauri -- build --debug --no-bundle
npm run verify:desktop -- built
```

桌面验证脚本仅支持 Windows，会为自己启动的子进程临时启用 WebView2 CDP 调试端口，使用正常窗口关闭消息测试退出，并清理自己的开发进程。运行前关闭本工程已有的窗口并释放 1420 端口。结果、进程日志和窗口截图写入 `artifacts/desktop-dev/` 或 `artifacts/desktop-built/`，不进入版本管理。失败返回非零退出码；查看对应 `process.log` 和 `report.json`。

`verify:shell` 串行启动桌面场景，汇总 `artifacts/phase-1-1/report.json`；主框架截图和报告位于 `artifacts/shell-{dev|built}-{shell|shell-error}/`。该入口包含受限系统打开实验，会打开本机夹具文件和浏览器页面。

阶段 0 原脚本入口保持不变，会自行设置 `PA_DESKTOP_MODE=phase0`。手动进入旧验证页时，仅在调试构建启动前设置该变量；验证完移除。正常模式没有 probe/Opener 权限，发布构建忽略验证模式和失败注入变量。`PA_SHELL_FAIL_ONCE=1` 可在调试构建中模拟一次 Rust 初始化失败，重试后恢复；这只验证桌面初始化链路，数据库与外部适配器失败随对应服务另验。

## 步骤 0.4：Git 与系统打开实验

```powershell
npm run verify:git
npm run tauri -- build --debug --no-bundle
npm run verify:system-open
```

Git 脚本只在系统临时目录创建夹具仓库，验证分支、状态、结构化差异与失败分类。系统打开脚本在真实 WebView2 中调用 Tauri Opener，验证受限临时目录的文件/目录、编码后的本机搜索 URL 和权限拒绝；它会启动系统默认程序，浏览器测试页可能需要手动关闭。结果写入 `artifacts/git-system/`。建议命令、错误码、平台限制及阶段一权限前置条件见 [步骤 0.4 验证记录](docs/phase-0/git-system-verification.md)。这里的 Opener 权限仅用于阶段 0 夹具，不支持打开任意用户项目或外部搜索站点。

## 目录与生成产物

| 路径 | 内容 | 是否提交 |
| --- | --- | --- |
| `src/` | React 桌面壳层、状态组件、启动入口与 IPC 类型；阶段0页独立按需加载 | 是 |
| `src-tauri/src/` | Rust 桌面生命周期、验证命令与已有存储/安全参考模块 | 是 |
| `src-tauri/tauri.conf.json`、`capabilities/` | 窗口、IPC 能力与 CSP | 是 |
| `package-lock.json`、`src-tauri/Cargo.lock` | 本轮完整依赖版本锁定 | 是 |
| `rust-toolchain.toml`、`.node-version` | 工具链基线 | 是 |
| `docs/phase-0/` | 环境、实验、决策、风险、验收 | 是 |
| `scripts/` | 真实桌面验收工具 | 是 |
| `node_modules/`、`dist/` | npm 依赖与前端构建输出 | 否 |
| `src-tauri/target/`、`src-tauri/gen/schemas/` | Rust 编译与 Tauri 自动生成文件 | 否 |
| `src-tauri/permissions/autogenerated/` | build.rs 按命令清单生成的 app permissions | 否；构建时重建 |
| `artifacts/`、`*.tsbuildinfo` | 本机验收证据与编译缓存 | 否 |

## 步骤 0.3：Codex 通信与配置实验

```powershell
# 不访问网络的客户端协议/安全测试
npm run test:codex

# 真实 app-server 和当前已配置的服务源（会产生模型调用用量）
npm run verify:codex

# 真实 CLI + 两个合成 Responses HTTP 站点，不使用真实 Key
npm run verify:codex:providers

# 历史诊断入口（不是自管路线验收）：只读调查安装/存储/ACL
npm run inspect:cc-switch

# 历史补验入口：会读取外部配置/凭据比对，不用于生产自管配置
npm run verify:phase-0:cc-switch

# 历史自动接入研究：固定社区 CLI 5.11.0 便携包；不安装到 PATH
npm run prepare:cc-switch-cli

# 历史 CLI 隔离补验（不属于新 FR-AI-006 验收）
npm run verify:cc-switch-cli
```

Codex 路径支持 `CODEX_BIN` 绝对路径；未设置时自动查找 PATH 或当前用户 Desktop 安装包。可选 `CODEX_PROBE_MODEL`、`CODEX_PROBE_EFFORT` 按当前模型目录验证，`CODEX_PROBE_TIMEOUT_MS` 默认 90000。cc-switch 调查默认使用当前用户安装路径，也可设置 `CC_SWITCH_BIN`。脚本通过参数数组启动自有子进程，使用受限临时 CODEX_HOME 和含中文/特殊字符的临时项目，并清理自己的进程与目录。

真实实验只导入连接必要的配置和可用认证，排除 MCP、插件、通知钩子、技能和用户项目信任，不改写当前全局配置。默认拒绝审批；唯一批准实验为可信 PowerShell 在隔离项目中精确执行 `Get-Location`。合成站点实验不验证官方认证或 cc-switch 自动切换。

报告分别位于 `artifacts/codex-real/report.json`、`artifacts/codex-controlled/report.json`、`artifacts/cc-switch/report.json`。前两者只有完整验证通过才返回退出码 0；cc-switch 调查成功退出 0 仅表示生成报告，集成状态仍以报告的 blocked 为准。原始凭据、服务端错误和 stderr 不写入这些报告。

协议导出可按本机 CLI 重新生成：

```powershell
$cli = if ($env:CODEX_BIN) { $env:CODEX_BIN } else {
  $command = Get-Command codex.exe -ErrorAction SilentlyContinue
  if ($command) { $command.Source } else {
    Join-Path (Get-AppxPackage OpenAI.Codex).InstallLocation 'app\resources\codex.exe'
  }
}
& $cli app-server generate-json-schema --out artifacts/codex-research/schema
& $cli app-server generate-ts --out artifacts/codex-research/types
```

脚本是阶段 0 实验客户端，尚未构成正式 Rust Codex 服务或 React 业务接口。

2026-10-10 配置修复后补验见 [补测记录](docs/phase-0/cc-switch-retest.md)。新增入口只读核对 cc-switch 选项与 Codex 实际 endpoint、协议和凭据是否一致，允许模型默认值有差异并重新验证实时选择；不把内部数据库查询作为产品接口。真实通信脚本在启动前拒绝官方源、缺失来源或需要官方认证的配置；官方源仅作预留，不纳入通过判定。汇总报告位于 `artifacts/phase-0-cc-switch-retest/report.json`；补验通过与步骤 0.3 完整通过分别判断。

2026-10-11 [社区 CLI 替代接口补验](docs/phase-0/cc-switch-cli-verification.md)完成 9 项检查：添加/列表/切换、合成 Key 绑定、无效候选保留配置、备份/导出通过；编辑/导入/恢复在无终端调用中超时，Windows 缺少 `start`/daemon。报告 `artifacts/cc-switch-cli/report.json` 的 `passed=true` 表示本组检查通过，`integrationStatus=blocked` 表示完整集成仍阻塞。CLI 从未接触用户桌面库；一次性只读 schema 版本一致不代表完整兼容。官方源不参与测试。

## 步骤 0.5：数据与迁移验证

2026-10-19 数据库复验修复了 schema v3 的验收版本误报、SQLite 错误分类和能力缓存修订/时间校验；步骤 1.6 收尾已补齐凭据验证并修正 IPC 旧权限断言。最新 31 项存储测试、19 项 IPC 及独立进程读回通过，具体结果和工具链差异见 [数据库修复记录](docs/phase-1/storage-fixes-2026-10-19.md) 与 [步骤 1.6](docs/phase-1/step-1.6.md)。以下阶段 0 说明保留历史范围，当前实现以最新记录为准。

```powershell
npm run verify:storage
```

该入口构建前端、运行 Rust SQLite 测试、构建嵌入资源调试程序，再通过两个独立进程在 Tauri `app_data_dir` 下的专属测试子目录写入/读取合成数据，最后清理自己的目录。报告在 `artifacts/storage/report.json`；测试和报告不读取真实 Codex/cc-switch 配置或凭据。SQL schema v1、字段和留存策略见数据模型，迁移/备份/恢复原则见 ADR-0008。正常启动尚未启用业务数据库，正式仓储、凭据库及 IPC 留到阶段一。

可单独运行 `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib storage::`。迁移失败、备份失败、损坏或高版本数据库都拒绝静默重建；迁移 SQL 发布后只能追加版本。当前仅验收 Windows x64；真实凭据库、自动清理/恢复界面和断电场景未实测。

## 步骤 0.6：IPC、安全和日志约定

```powershell
# Rust 22项、IPC 4项、Codex离线15项、类型/构建与嵌入资源真实桌面验证
npm run verify:ipc

# 单独执行前端返回校验、OS参数与最小权限检查
npm run test:ipc
```

结果写入 `artifacts/ipc/report.json`，失败返回非零退出码；真实窗口报告和截图在 `artifacts/desktop-built/`。本入口不调用真实模型、不读取认证配置、不修改 cc-switch。新增 v1 请求/返回与服务端关联 ID 已接入 framework_probe，app command 权限仅授权本地 main；旧系统打开 preflight 仍限阶段0夹具。provider/模型/Key状态/切换等待和回滚的完整业务 DTO 是阶段一实现草案，当前没有注册这些服务命令。日志白名单已示范，文件轮转/清理、系统凭据库、原子审批和完整网关留后续验收。

## 阶段记录

- [步骤 0.7 阶段门评审与阶段一候选清单](docs/phase-0/phase-gate-review.md)
- [步骤 1.1：主框架、模块职责与验收记录](docs/phase-1/step-1.1.md)
- [步骤 1.2：正式 IPC、状态与事件通路验收记录](docs/phase-1/step-1.2.md)
- [步骤 1.3：正式数据库、仓储与启动恢复验收记录](docs/phase-1/step-1.3.md)
- [步骤 1.4：全局设置、常驻恢复与验收记录](docs/phase-1/step-1.4.md)
- [环境基线](docs/phase-0/environment.md)
- [实验清单](docs/phase-0/experiments.md)
- [风险登记册](docs/phase-0/risks.md)
- [决策记录](docs/phase-0/decisions.md)
- [验证结果与工具链问题](docs/phase-0/verification.md)
- [Codex 通信与配置实测](docs/phase-0/codex-verification.md)
- [最小事件与能力契约](docs/phase-0/codex-contract.md)
- [自管 profile 替代方案 ADR-0011](docs/phase-0/self-managed-profile-adr.md)
- [cc-switch 接口与凭据调查](docs/phase-0/cc-switch-verification.md)
- [cc-switch 配置修复后补测](docs/phase-0/cc-switch-retest.md)
- [社区 cc-switch CLI 补验](docs/phase-0/cc-switch-cli-verification.md)
- [Git 与系统打开实测](docs/phase-0/git-system-verification.md)
- [数据模型与留存初稿](docs/phase-0/data-model.md)
- [迁移与备份 ADR-0008](docs/phase-0/storage-migration-adr.md)
- [SQLite 与迁移实测](docs/phase-0/storage-verification.md)
- [IPC v1 契约草案](docs/phase-0/ipc-contract.md)
- [错误、日志与最小审计规范](docs/phase-0/errors-and-logging.md)
- [安全边界与威胁测试清单](docs/phase-0/security-checklist.md)
- [步骤 0.6 验收](docs/phase-0/ipc-security-verification.md)

步骤 1.3（S1-03）已接入正常启动数据库、类型化仓储和启动恢复；数据库失败保持原库并返回固定 storage 原因码。步骤 1.4 追加 schema v2，步骤 1.5 已接入日志与审计；步骤 1.6 追加 schema v3 并通过 Windows Credential Manager、专用 IPC 和跨存储补偿验收。各步骤范围及未验收项分别见对应 `docs/phase-1/step-1.*.md`。

步骤 0.3、0.7 及阶段 0 按替代范围通过；系统凭据持久化已按步骤 1.6 当前范围通过，自有候选进程/配置层路由隔离已在 1.7 接入，1.8 配置层切换恢复已通过，真实任务仍待阶段 3 验收。阶段 0 历史决策见 [阶段门评审](docs/phase-0/phase-gate-review.md)，最新实现以阶段 1 记录为准。

## 步骤 1.8：profile、能力、切换恢复与快照

九个 main-only 命令、隔离 CLI 模型目录、五分钟组合证据、八项 FIFO、取消/等待、三类恢复及不可变快照已接入。schema v4 仅追加；旧迁移和 lock 不变。启动不发请求、不重放，持久默认需重新核验。非空默认仅经 provider_switch 提交，settings_get 返回完整选择，其他设置修改保留它。

完整入口：npm run verify:providers；报告 artifacts/phase-1-8/report.json。实际 CLI/双合成 HTTPS/合成 Key；恢复与任务占用用生产状态机注入/合成任务。另复验 verify:storage、verify:settings。记录见 [步骤 1.8](docs/phase-1/step-1.8.md)。1.9 页面记录见 [步骤 1.9](docs/phase-1/step-1.9.md)；阶段 3 真实任务、1.10 人工硬件仍待，不代表 M1/阶段 1 完成。

## 阶段 1 汇总回归与移交

```powershell
npm run verify:phase-1
```

该 Windows 入口按顺序运行 1.1 shell、1.2 IPC/state、1.3 storage、1.4 settings、1.5 logging、1.6 credentials、1.7 isolation 和 1.8/1.9 providers 的既有完整验收套件，并核对每份子报告的 `passed=true`。它会启动真实 dev/built Tauri 窗口、构建资源、执行受控系统打开场景和合成 HTTPS/凭据测试；运行前请关闭本项目已有桌面窗口并释放 1420 端口。汇总报告及各套件控制台输出位于 `artifacts/phase-1/`，详细证据保留在各步骤的 `artifacts/` 子目录。汇总入口不将注入结果视为物理硬件实测；Windows 托盘鼠标、多显示器移动/拔插及实时 DPI 仍按 [步骤 1.10](docs/phase-1/step-1.10.md) 进行人工记录。AI 配置页控件也需在真实 WebView2 中点验。
