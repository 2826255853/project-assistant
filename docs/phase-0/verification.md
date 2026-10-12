# 步骤 0.1、0.2 验收记录

> 2026-10-11 当前范围：用户已确认“用户手动管理 cc-switch、应用自管 profile”，见 [ADR-0011](self-managed-profile-adr.md)。步骤 0.3、0.7 与阶段 0 按替代范围通过，允许启动阶段一；原自动集成/共享代理未通过的事实及以下当时记录保留为历史，不作为当前自动需求或验收安排。生产自管凭据/路由/恢复仍待实现实测；旧切回 cc-switch 的约定已撤销。

日期：2026-10-10（Asia/Shanghai）。环境见 environment.md；操作顺序见 README；实验与失败动作见 experiments.md。

## 步骤 0.1

结论：通过本轮当前 Windows x64 验证基线。

- OS、架构、Node/npm、Git、Rust、MSVC、SDK、WebView2 和 Codex 版本均已核验。
- 本机初始缺 Rust，已补装并固定工具链，复现过程已记录。
- Codex Desktop 正在运行；CLI 绝对路径可启动；首轮认证成功，本轮补查已变为未登录（详见下文）；记录中不保留原始认证输出或凭据。
- 产品最低 Windows 版本、其他 CPU、远端认证有效性与 Codex 协议能力均明确为未验证；已登记后续复审点。
- 全阶段实验清单、风险初稿和决策记录已建立。
- v1.2 新增的 cc-switch 基线已补核验：4.0.6、安装/可配置路径、共享用户配置范围及官方/第三方认证类型已记录；当前 CLI 未登录和第三方 bearer 参数存在分别记录，真实认证仍未验证。

## 步骤 0.2

结论：通过当前 Windows 11 build 26300 x64 环境的最小骨架验收。开发模式和嵌入资源模式均完成真实 IPC、错误恢复和退出验证。

| 检查 | 命令 / 动作 | 本轮结果 |
| --- | --- | --- |
| 安装前端依赖 | `npm install` | 成功；27 个包，audit 报告 0 vulnerabilities |
| 锁文件重装 | `npm ci` | 成功；按锁文件重新安装，audit 报告 0 vulnerabilities |
| Windows 前置依赖 | `npm run tauri -- info` | WebView2、MSVC、Rust、Cargo、工具链检测全部通过 |
| 前端类型与构建 | `npm run build` | TypeScript 成功；Vite 8.3.4，18 modules；生成 dist |
| Rust 编译检查 | `cargo check --locked --manifest-path src-tauri/Cargo.toml` | 成功；首次 check 约 2 分钟；锁定版本复查成功 |
| 开发模式桌面 | `npm run verify:desktop` | 退出码 0；6 项验证全部通过，正常关闭应用退出码 0 |
| 无开发服务器的资源构建 | `npm run tauri -- build --debug --no-bundle` | 成功；生成 src-tauri/target/debug/project-assistant.exe，不生成安装包 |
| 嵌入资源桌面 | `npm run verify:desktop -- built` | 无 Vite 服务器时通过全部 6 项验证；退出码 0 |

两种模式的实际结果一致：真实 WebView2 加载 React 页面；中文、空格和特殊字符通过 Rust 往返回传；空白消息返回 EMPTY_MESSAGE 并清除旧成功状态；失败后重试成功并清除旧错误状态；正常关闭窗口后应用退出码 0；进程日志包含固定错误码且未输出测试消息。

| 本机证据 | 内容 |
| --- | --- |
| `artifacts/desktop-dev/report.json` | 开发模式 passed: true，6 项通过 |
| `artifacts/desktop-dev/process.log` | Vite/Tauri/Rust 编译启动与固定错误码 |
| `artifacts/desktop-dev/window.png` | 真实开发窗口的成功回传 |
| `artifacts/desktop-built/report.json` | 嵌入资源模式 passed: true，6 项通过 |
| `artifacts/desktop-built/process.log` | 独立应用 Rust 错误输出 |
| `artifacts/desktop-built/window.png` | 真实独立窗口的成功回传，已查看布局 |

自动脚本运行完毕后，已检查测试应用没有残留进程，1420 无监听。证据目录被忽略，干净检出按 README 重建；以上表格保留本轮验收结论。验证在本机安装依赖后的环境执行，不等同于在第二台全新 Windows 上验证。

自动验收检查实际表单提交后的中文消息往返、平台架构、Rust 错误/脱敏进程输出、重试恢复和正常窗口关闭后的退出码，结果写入 artifacts。截图仅作界面证据，成功结论还必须依赖真实 IPC 回复。

## 初始化与工具链问题记录

| 问题 | 原因 / 处理 | 遗留 |
| --- | --- | --- |
| 初始缺少 Rust | 安装官方 Rustup minimal MSVC；固定 1.99.0 | 新终端必须有 Cargo PATH；README 已说明 |
| 安装后旧进程 PATH 不刷新 | 本轮命令临时追加用户 .cargo/bin；干净环境重开终端 | 无 |
| 脚手架非交互执行失败 | 首次命令缺 `--yes`；明确使用 4.7.5 npm 包和 `--yes` | 不再要求复现时跑脚手架 |
| 脚手架强制初始化删除原计划文档 | 本次误用 `--force`；通过此前完整分段读取记录和原补丁恢复原文；恢复后长度 57,295 字节、各章节行号与初始化前一致 | 今后只能在空目录初始化后移入工程，保护未跟踪文件；本轮仅更新跟踪表状态 |
| Codex CLI 不在 PATH | 本轮使用 Desktop 提供的绝对路径核验，路径没有硬编码到应用 | 步骤 0.3 验证可配置客户端路径 |
| SDK 不在常见 Program Files 路径 | 实际安装根目录为 `C:\Windows Kits\10`；按注册项发现 | 已记录，不能凭固定路径判断缺依赖 |
| npm 包版本与初始化 CLI 自报版本不一致 | npm create-tauri-app 4.7.5，自报 4.8.0；保留两者记录 | 工程以已固化的源码和锁文件为准 |
| 首次关闭验收拿到空 ExitCode | Windows Get-Process 需在退出前持有进程 Handle；脚本在发送正常关闭消息前获取句柄，仍严格校验退出码 | 修复后开发/嵌入资源两模式复验全部通过 |

## 2026-10-10：按计划 v1.2 补执行

对照原计划快照与当前第 20.2 节，0.1 新增的 cc-switch 基线尚未执行；0.2 的四项工作内容没有变化，已有两种真实桌面验收证据。已补执行 0.1 安装、版本、路径、配置范围与认证类型核验（E-013），结果见 environment.md。全阶段预设实验补入 E-014–E-017，相应风险补入 R-013–R-017。

本次复查还发现当前认证状态已变化：resources CLI 版本仍为 `0.162.0-alpha.17.2`，`login status` 退出码 1；活动第三方 provider 中存在 bearer 参数，但没有据此宣称远端认证成功。未输出或保存原始认证信息、Key 或 token。基线范围清楚的 0.1 通过不等于 0.3 真实认证通过。

前端 `npm run build`、`cargo check --locked --manifest-path src-tauri/Cargo.toml`、开发模式桌面验收和嵌入资源桌面验收本轮复验均通过；两种模式各完成 6 项检查。

本机补核验摘要见 `artifacts/phase-0-supplement/report.json`，只保存版本、来源/认证类别、计数、退出码和验证结果，不保存原始配置、数据库或凭据；该目录沿用 artifacts 忽略规则。桌面报告与截图已按两种模式更新。结束后测试应用残留为 0，1420 监听为 0。

## 验收范围说明

今后 Codex 官方源相关功能仅作预留，无需实际测试；判断测试、阶段验收或发布是否通过时，不考虑其测试结果，未测试或失败不构成阻塞，也不记为实测通过。下述尚未完成结论仅由其余必需验收项决定。

2026-10-10 cc-switch 配置修复后补验见 [补测记录](cc-switch-retest.md)。步骤 0.2 不依赖 cc-switch，原来没有因它而跳过的桌面测试；本次仍复验前端构建、锁定 Rust check、开发窗口与重新构建后的嵌入资源窗口，两个窗口模式各 6 项通过。

本文件记录的步骤0.1、0.2未测试其他Windows、ARM64、离线/代理环境，其原始验收不包含SQLite和正式IPC安全契约。步骤0.3通信及替代范围结论见codex-verification.md、codex-contract.md与ADR-0011；真实通信、取消和命令审批通过，旧自动接入未通过且退出范围。步骤0.4的Git/系统打开实测见git-system-verification.md；步骤0.5于2026-10-11完成，见storage-verification.md；步骤0.6于同日完成v1契约/错误日志/安全清单及参考验证，见ipc-security-verification.md。framework_probe已迁移v1，本轮桌面脚本增加envelope/版本/关联ID和日志不含payload检查，历史6项记录保留。步骤0.7与阶段0按已确认替代范围通过，详见phase-gate-review.md与ADR-0011；生产自管功能仍按对应上线门验收。
