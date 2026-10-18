# 步骤 0.1：环境基线

> 2026-10-11 当前范围：用户已确认“用户手动管理 cc-switch、应用自管 profile”，见 [ADR-0011](self-managed-profile-adr.md)。步骤 0.3、0.7 与阶段 0 按替代范围通过，允许启动阶段一；原自动集成/共享代理未通过的事实及以下当时记录保留为历史，不作为当前自动需求或验收安排。生产自管凭据/路由/恢复仍待实现实测；旧切回 cc-switch 的约定已撤销。

记录日期：2026-10-10（Asia/Shanghai）。本轮按计划默认选当前 Windows 桌面作为首个可运行环境。

## 范围与未验证假设

| 项目 | 本轮决定 | 限制 |
| --- | --- | --- |
| 操作系统 | Windows 11 家庭版中文版，10.0.26300 / build 26300 | 只保证该实测基线；产品最低 Windows 版本待阶段门决定 |
| CPU | x86_64 / x64；Rust MSVC target | ARM64、x86、macOS 和 Linux 未验证 |
| 首个运行环境 | 本机交互式 Windows 桌面 | 不保证无桌面会话、容器和远程 Windows Server |
| 权限 | 当前用户启动应用，工程位于可写目录 | 依赖安装可能需系统管理员；应用运行不要求提升权限 |
| 网络 | 首次安装工具链和 npm/Cargo 依赖需网络 | 离线恢复和企业代理环境未验证 |
| 产品最低版本 | 待决策（R-003） | 不能将 Tauri 理论支持范围当作实测兼容范围 |

## 可复现工具链

| 项目 | 实测版本 / 状态 | 核验方式与复现说明 |
| --- | --- | --- |
| Node.js | 24.18.0，x64 | `node --version`；`.node-version` |
| npm | 11.16.0 | `npm --version`；`packageManager`；`npm ci` |
| Rustup | 1.29.1 | `rustup --version`；官方 Rustup 安装器 |
| Rust | rustc 1.99.0 (b940084d7 2026-09-28) | `rustc --version`；`rust-toolchain.toml` |
| Cargo | 1.99.0 (5f94df478 2026-08-27) | `cargo --version`；同一 Rust 工具链 |
| Rust target | x86_64-pc-windows-msvc | `rustup show active-toolchain` |
| Visual Studio | Community 2026，18.10.3，安装版本 18.10.12224.181 | `vswhere -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64`；`tauri info` |
| MSVC | 默认 14.51.36231；同时安装 14.44.35207 | Visual Studio VCToolsVersion.default.txt；Tauri 检测通过 |
| Windows SDK | 10.0.26100.0，安装根目录 `C:\Windows Kits\10` | Installed Roots 注册项与 Include 目录；实际 Rust 编译验证 |
| WebView2 Runtime | 154.0.4258.62 | EdgeUpdate Clients 注册项；`tauri info` |
| Git | 2.55.0.windows.5 | `git --version` |
| Tauri CLI | 2.12.1 | `npm run tauri -- --version`；npm lock |
| Tauri Rust / API | 2.12.2 / 2.12.2 | Cargo.lock / package-lock.json |
| React / React DOM | 19.3.0 / 19.3.0 | `npm list --depth=0`；npm lock |
| TypeScript / Vite | 6.0.3 / 8.3.4 | `npm list --depth=0`；npm lock |
| React Vite 插件 | 6.1.2 | npm lock |
| 初始化工具 | create-tauri-app npm 包 4.7.5，CLI 自报 4.8.0 | 使用明确包版本；模板实际内容已固化到源码 |

本机初始未安装 Rust，已通过官方 Windows x64 Rustup 安装器安装 minimal MSVC 工具链；当前进程通过临时补充 PATH 使用，干净环境应重新打开终端。锁文件纳入源码，复现不再依赖重新运行初始化工具。可选包管理器不作为基线，本轮只使用 npm。

## Codex 客户端可用性

| 项目 | 实测结论 |
| --- | --- |
| Codex Desktop | 已安装且本次会话正在运行；Windows 应用包版本 26.1007.2314.0，X64 |
| 本机 CLI | 已随 Desktop 安装；版本 0.162.0-alpha.17.2；绝对路径调用 `--version` 成功 |
| PATH | 当前 `Get-Command codex` 未发现 CLI；不能假设独立终端可直接运行 `codex` |
| 认证 | 首轮 `login status` 返回成功并报告 API key 认证；v1.2 补核验时同版本 CLI 返回退出码 1、未登录；当前配置存在 bearer 认证参数；不记录凭据、账号或原始认证输出 |
| 认证能力限制 | 首轮只确认客户端认为已认证；本轮复查为未登录。第三方 bearer 配置存在性也不证明远端服务、额度、协议能力或实际任务可用 |
| 后续动作 | 步骤 0.3 明确可配置的 CLI 路径并实测 app-server；本轮没有创建 Codex 任务 |

安装目录含随版本改变的 hash，不写为工程硬编码路径。复查认证时先捕获并丢弃原始输出，只记录成功/失败，避免 CLI 打印脱敏令牌也进入项目日志。

## v1.2 补核验：cc-switch 与认证配置范围

2026-10-10 按新增的步骤 0.1 要求补执行。原计划快照 `artifacts/plan-before-status-update.txt` 的 0.1 尚无本项，0.2 工作内容与现行计划一致。

| 项目 | 本机结果 | 核验依据与边界 |
| --- | --- | --- |
| 安装与实际版本 | CC Switch 桌面版 4.0.6，正在运行 | 卸载注册项与 exe 的 FileVersion/ProductVersion 均为 4.0.6；不推断桌面版提供 CLI |
| 本机安装路径 | `%LOCALAPPDATA%\Programs\CC Switch\cc-switch.exe` | 注册项与文件存在性核验；仅记录基线，后续路径应可配置，不能硬编码到工程 |
| 数据与可配置路径 | `%USERPROFILE%\.cc-switch\settings.json`、`cc-switch.db`；当前 `codexConfigDir` 覆盖未设置 | 4.0.6 源码支持 `codex_config_dir` 覆盖；当前回退到 `%USERPROFILE%\.codex`；工程中的路径设置功能尚未实现 |
| Codex 管理范围 | 用户共享的 `.codex\config.toml`、`auth.json` 和 cc-switch 模型目录 | 版本固定源码的 `get_codex_config_dir` 及文件存在性；当前进程 `CODEX_HOME` 未设置，项目无 `.codex\config.toml`；目录相同不能证明切换对运行中任务的生效时机 |
| 当前活动服务源 | 第三方 HTTPS，`wire_api = responses`，`requires_openai_auth = false`，存在 `experimental_bearer_token`，无 `env_key` 引用 | 解析活动 provider，只输出来源类别、协议、布尔状态；不输出 provider 名称、endpoint、模型或完整 Key |
| 官方与中转站认证类型 | CC Switch 内有 2 项官方类型配置、4 项第三方配置；2 项含 OAuth 认证字段、4 项含 Key/bearer 字段 | SQLite `mode=ro` 一次性分类，输出计数与存在性；没有把内部表结构作为本项目接口，也没有保存或打印凭据 |
| 当前认证限制 | `.codex\auth.json` 不存在；正确的 resources CLI `login status` 退出码 1；本地 bearer 参数存在不代表远端有效 | CLI 未登录和第三方 provider 认证分别记录；中转站真实请求由 0.3 验证；官方源仅作预留，无需实测且结果不纳入通过判定 |
| 共享切换与保护 | Codex 代理的 `proxy_enabled`、`enabled`、`live_takeover_active` 均为 0；`preserveCodexOfficialAuthOnSwitch = false` | 切换官方认证、共享配置、代理路由和外部任务影响未实测；凭据保护、备份排除与回滚登记 R-014/R-016 |

本轮仅核验本地状态，没有切换服务源、修改外部配置、运行远端任务或收集真实 Key。安装与版本信息已确认；读取、编辑、切换、加载时机和失败恢复仍须 0.3 实测。认证变化说明先前成功状态不能当作当前可用凭据的保证。

源码依据固定为 farion1231/cc-switch 的 `v4.0.6`：`src-tauri/src/settings.rs`（目录覆盖与认证保留设置）、`src-tauri/src/codex_config.rs`（默认目录、auth/config/catalog 路径）。原始配置和数据库不复制到 artifacts，观察到的内部表不得据此成为运行时依赖。

## 核验命令

配置修复后补验：cc-switch 当前选项的站点、协议和 Key 与 Codex 实时配置一致，实际 provider 标识为 `custom`。实时模型与 cc-switch 保存的默认模型有差异，测试使用实时值并通过模型目录及真实推理重新校验，不将默认值差异误判为来源失配。详见 [补测记录](cc-switch-retest.md)；历史登录状态保留，官方源仅作预留，无需实测且结果不纳入通过判定。

步骤 0.3 追加核验（2026-10-10）：同版本 CLI 的独立 stdio app-server 已完成真实中转源通信，两个模型组合、取消与命令审批均通过。当前自定义源的 account/read 为 account=null、requiresOpenaiAuth=false，bearer 实际请求可用；此前 CLI 未登录仍不能据此变为“官方已认证”。cc-switch 桌面 4.0.6 的固定源码、协议注册、普通 SQLite 文件头与 ACL 已调查，但外部双向自动接口及共享路由影响仍未验证/阻塞；官方认证仅作预留，不纳入通过判定。见 codex-verification.md、cc-switch-verification.md。

```powershell
Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSArchitecture
node --version
npm --version
rustc --version
cargo --version
rustup --version
git --version
npm run tauri -- info
npm list --depth=0
```

依赖安装、开发运行、输出目录和干净环境操作顺序见仓库 README。工具链安装源：Rust `https://www.rust-lang.org/tools/install`；Tauri 前置依赖 `https://v2.tauri.app/start/prerequisites/`；初始化 `https://v2.tauri.app/start/create-project/`。网络文档只用于安装依据，版本结论来自本机命令。
