# 步骤 2.6（S2-06）：基础 Git 摘要

日期：2026-10-28（Asia/Shanghai）。

状态：**通过（当前 Windows x64 / 标准 Git for Windows 安装范围）**。正式 Rust 只读适配器、授权项目查询、内存缓存、降级状态、超时和自有进程清理已接通。自动验收不依赖 Node 阶段 0 Git 实验。

## 接口与行为

新增 main-only `project_git_summary` IPC v1：`{projectId, refresh?}` 返回既有 `GitSummary`。`refresh` 只接受可省略的布尔值；缺省允许使用缓存，`true` 绕过缓存，显式 `null`、未知字段和错误类型拒绝。工作线程先复核已登记授权及目录身份，再在授权目录执行 Git；失败的非仓库、Git 不可用、超时和异常输出以固定摘要状态返回，不阻塞列表、搜索或系统打开。

成功摘要含仓库识别、当前分支（detached HEAD 时为 `null`）、detached 标记、工作区修改标记和 UTC 毫秒刷新时间。摘要查询不增加 `project_usage`，没有差异正文、提交历史或路径字段。最多缓存 128 个项目/修订结果，存活 5 秒；缓存仅在内存中。

## 进程与解析边界

Windows Git 仅从 HKLM 的 Program Files 注册位置发现，不依据调用方 `PATH`、项目目录、环境变量或 Git 配置去选择可执行文件。候选程序及完整路径逐级拒绝重解析点，并要求最终路径仍在注册的安装目录内。非标准/用户级安装不被自动执行，会显示 `git_unavailable`。本地 `git.exe` 采用参数数组与已验证项目目录，不经 Shell；环境清空后只设置 Windows 系统运行路径和固定 Git 开关。禁用 system/global 配置、fsmonitor、untracked cache、终端提示与分页，关闭可选锁。

每条子进程共享 4 秒总预算，stdout 上限 512 KiB、stderr 上限 32 KiB；超限或超时会终止 Job 中的自有进程树并等待回收。porcelain v2 NUL 记录按字段严格解析，不向前端、日志或错误透传 stdout/stderr。异常字段或未知记录固定降级为 `failed`。Git status 忽略子模块内部未提交文件变化，仍识别子模块 HEAD 与索引不一致；摘要不递归扫描子模块工作树。

## 验收

入口：`npm run verify:projects`。完整脱敏报告：`artifacts/phase-2-6/report.json`；built/dev 桌面子报告：`artifacts/shell-built-projects/report.json` 和 `artifacts/shell-dev-projects/report.json`。

本轮固定基线 Node.js 24.18.0、npm 11.16.0、Rust 1.99.0；Windows 11 x64 10.0.26300。完整入口通过：前端构建、Rust fmt、Rust 156 passed / 5 ignored、IPC 33、Codex 离线 15、release check、存储 52，以及 built/dev 实际 Tauri WebView 项目场景。built/dev 都通过正式 Git IPC 验证授权合成仓库分支、干净和已修改状态、5 秒缓存刷新、严格请求校验、main-only 权限以及查询不改变 `lastUsedAt`。搜索 IPC P95 分别为 built 29.8 ms、dev 27.3 ms。

专项 Rust `project_git::tests` 5 项通过：空仓库和普通分支、detached HEAD、跟踪/未跟踪变更、文件重命名/合并状态解析、非仓库、缺失 Git、超时、stdout 超限、异常 Git 输出、fsmonitor 外部命令未执行及子模块 HEAD 状态。专门的无 Git、超时、异常输出通过本机子进程适配层夹具验证，没有卸载系统 Git 或挂起系统 Git 服务。整个阶段只使用合成项目与提交，不连接模型、不使用真实 Key。

## 限制与后续

当前通过范围是本机 Windows x64 和注册表发现的标准 Git for Windows 安装。其他 Windows 版本、ARM64、Git 缺失的干净机器、用户级/自定义安装未实测；缺少标准安装会安全降级，不读取 PATH 猜测。子模块内部的未提交文件状态不包含在摘要中，后续如需展示需另行设计范围和权限边界。Git 差异正文、历史与写操作留后续阶段；2.7 已完成项目页面和实际 WebView 控件闭环，原生选择器物理点验、阶段回归与手工点验留 2.9。M2 仍未通过。
