# 步骤 0.4：Git 与系统打开能力验证

> 2026-10-11 范围更新：按用户确认的手动 cc-switch / 应用自管 profile 替代方案，步骤 0.3、0.7 与阶段 0 按替代范围通过，允许启动阶段一，见 [ADR-0011](self-managed-profile-adr.md)。本文实测证据与原当时阶段门描述保留；当前生产自管凭据、路由隔离及恢复仍须独立验收。

执行环境：Windows 11 build 26300 x64；Git `2.55.0.windows.5`；Tauri 2.12.2；`tauri-plugin-opener` 2.7.0。结论只适用于本机交互式桌面。步骤 0.4 在此范围通过；阶段 0 按 ADR-0011 的替代范围通过，不扩大本步骤的平台或权限验证范围。

## 复现

```powershell
npm ci
npm run verify:git
cargo check --locked --manifest-path src-tauri/Cargo.toml
npm run tauri -- build --debug --no-bundle
npm run verify:system-open
```

`verify:git` 创建并删除系统临时目录中的 Git 仓库，不改动本仓库；初始化和提交只发生在该夹具中。可用 `GIT_BIN` 指向另一 `git.exe`。`verify:system-open` 必须在交互式 Windows 桌面运行，启动本工程调试程序和真实 WebView2，临时打开目录、文本文件及默认浏览器的本地搜索页；脚本关闭自己启动的程序和可识别的夹具资源管理器窗口，浏览器页可能需要手动关闭。证据生成在被忽略的 `artifacts/git-system/`：`git-report.json`、`system-open-report.json` 和进程日志。脚本失败返回非零退出码。

## Git 实测表

| 用例 | 调用 / 夹具 | 结果 |
| --- | --- | --- |
| 可用性 | `git --version` | 2.55.0.windows.5；可启动 |
| 仓库与当前分支 | 临时仓库内 `rev-parse --is-inside-work-tree`、`symbolic-ref --quiet --short HEAD` | `true`、`phase-0-main` |
| 工作区状态 | `status --porcelain=v2 --branch -z --untracked-files=all` | 精确解析修改、暂存重命名、未跟踪文件及中文、空格、`& ! ^ ( ) [ ]` 路径 |
| 结构化差异 | `diff --raw -z --no-ext-diff --no-textconv --find-renames --`；另加 `--cached` 取暂存区 | 解析旧/新模式、对象 ID、状态、原/目标路径；未暂存修改与暂存重命名均匹配 |
| 非仓库 | 真实非 Git 临时目录中 `rev-parse --is-inside-work-tree` | 退出 128，stderr 为 `not a git repository`；判为 `NOT_REPOSITORY` |
| 未安装 | 将执行文件指定为不存在的 `git-missing.exe` | 进程创建错误 `ENOENT`；判为 `GIT_NOT_FOUND`，未动系统 Git 安装 |
| 超时 | 同一无 Shell 启动器运行会等待的 Node 夹具，150 ms 截止 | 子进程被终止；判为 `COMMAND_TIMEOUT`，未用真实 Git 人为挂起 |
| 非零退出 | 真实仓库运行 `git status --phase-0-invalid-option` | 退出 129；判为 `NONZERO_EXIT`，保留退出码 |
| 参数安全 | 仓库目录名含 `& mkdir INJECTED`，文件名含中文和 Shell 元字符 | `spawn(program, args, { shell: false, cwd })`；路径按字面处理，未产生注入标记 |

阶段一建议由 Rust 服务层使用 `std::process::Command::new(git_exe).args([...]).current_dir(project)`，不经过 `cmd.exe`、PowerShell 或拼接命令字符串。先做 Git 可用性和仓库判定，再执行只读查询。命令统一附 `--no-pager`；状态和差异使用上表的 NUL 分隔格式，不能按换行或空格拆文件名。`--raw -z` 是差异元数据，不包含补丁正文；若审查界面需要正文，另取 `git diff --no-ext-diff --no-textconv` 并设置输出大小与超时上限。未跟踪文件不在 `git diff` 中，必须以状态输出单独显示。裸仓库、detached HEAD、子模块、巨大仓库和权限拒绝尚未实测，阶段一适配器应单独处理。不能仅以退出码 128 断言“非仓库”；本轮只在 `rev-parse` 的对应英文诊断出现时这样分类，其他 128 保留为命令失败。

## 系统打开实测表

| 用例 | 真实调用 | 结果 |
| --- | --- | --- |
| 桌面链路 | React `openPath` / `openUrl` → Tauri 插件 → Windows 默认处理器 | 嵌入资源的真实 WebView2 页面成功调用 |
| 打开目录、文件 | 系统临时夹具，路径含中文、空格、`& ^ ! ( )` | 两次调用均被系统接受；目录对应资源管理器窗口可观察到。文件仅证明插件/系统调用被接受，不证明默认应用已完成加载 |
| 缺失路径 | 受限夹具内不存在的 `.txt` | Rust 预检查返回 `OPEN_PATH_MISSING`，页面提供中文提示。直接依赖 Opener 时，本机曾返回 Windows `os error 1223`（用户取消），不能把该码等同于“不存在” |
| 路径越界 | 本仓库现有 `README.md` | 页面预检查返回 `OPEN_PATH_DENIED`；直接调用插件命令也被权限范围拒绝 |
| 编码后的搜索 URL | `http://127.0.0.1:<临时端口>/search?q=...`，查询含中文、空格、`& / ? # ü` | 默认浏览器实际请求本地 HTTP 端点；服务端按 UTF-8 解码后与原搜索词完全相同 |
| URL 越界 | `https://example.com/search` | 插件拒绝；页面返回 `OPEN_URL_DENIED`，未打开外部站点 |

插件只开放两个实验权限：`opener:allow-open-path` 的 `$TEMP/phase-0-system-open/**` 和 `opener:allow-open-url` 的 `http://127.0.0.1:*/*`。没有 `opener:default` 或任意路径/URL 权限。UI 使用 `URL.searchParams.set("q", query)` 编码，不拼接 URL；调用插件不构造 Shell 命令。临时路径预检查先限制绝对路径、拒绝 `..`、规范化并核对范围，再区分不存在、访问拒绝和其他错误。预检查与插件打开之间仍存在文件系统变化窗口；这不是阶段一任意项目路径的最终访问控制。

## 错误和平台约定

| 分类 | 识别依据 | 用户提示建议 |
| --- | --- | --- |
| `GIT_NOT_FOUND` | 启动 Git 返回 `ENOENT` | 安装 Git，或在设置中指定可执行文件 |
| `NOT_REPOSITORY` | 仓库探测失败且诊断明确为非仓库 | 选择 Git 项目，或按普通文件夹查看 |
| `COMMAND_TIMEOUT` | 到期后终止本次子进程 | 缩小查询范围或检查仓库/磁盘，再重试 |
| `NONZERO_EXIT` | Git 已启动，退出码非零 | 显示安全裁剪后的诊断、退出码与重试操作 |
| `SPAWN_FAILED` | 除 `ENOENT` 外的进程创建错误 | 检查权限和执行文件路径；本轮未注入权限错误 |
| `OPEN_PATH_MISSING` / `OPEN_PATH_DENIED` | 限定路径预检查或插件权限 | 检查路径；越界时先选择被授权的项目路径 |
| `OPEN_URL_DENIED` / `OPEN_INVALID_URL` | 插件 URL scope 或解析校验 | 只使用已配置的 HTTP(S) 搜索提供方 |
| `OPEN_FAILED` | 默认程序、OS 调用或其他错误 | 保留脱敏诊断；检查默认应用或手动复制路径/地址 |

本轮只验证 Windows 11 x64、已安装 Git、可用默认浏览器和交互桌面。其他 Windows 版本、ARM64、macOS/Linux、无默认文件关联、浏览器策略拦截和真实 HTTPS 搜索提供方未验证。阶段一要开放用户选择的任意项目目录时，需重新设计按项目授权的插件 scope 与 Rust 路径检查；要打开真实搜索结果时，需明确 HTTPS 域名白名单或受控后端调用，并复验。浏览器/资源管理器的进程和窗口归属不能从插件返回成功推断；阶段一不得因返回成功就声称用户已看到内容。
