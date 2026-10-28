# 步骤 2.5（S2-05）：授权目录与指定文件的系统打开

日期：2026-10-28（Asia/Shanghai）。实际负责人：本次 Codex 会话，负责 Rust、IPC、桌面集成与自动验收。

状态：**有条件通过**。本机 NTFS 的打开服务、受限文件选择器适配、默认程序与结构化编辑器适配已实现；完整 `npm run verify:projects` 通过。打开必须取得 NTFS/TxF 元数据保护及目标属性写权限，不支持时固定失败；不宣称 ReFS 或未来 Windows 兼容通过。原生选择器的鼠标/键盘人工点验在 2.9，页面在 2.7。M2 未通过。

## 命令与服务

本次新增三个正式 IPC v1 命令，仅授权真实本地 main：

| 命令 | 参数 | 成功数据 |
| --- | --- | --- |
| `project_open_directory` | `{projectId}` | `{systemAccepted:true}` |
| `project_open_file` | `{projectId,relativePath}` | `{systemAccepted:true}` |
| `project_pick_file` | `{projectId}` | `{relativePath}` 或 `{outcome:"cancelled"}` |

Rust 注册、build manifest、main capability、TypeScript `ImplementedCommandMap`、返回校验和命令日志白名单同步接入。前端没有新增 Opener、任意绝对路径、URL、Shell 或自由程序参数权限。overlay、未知窗口和远端 main 的七个项目命令均被 capability 拒绝，Rust 同时核验真实 label/URL。`project_select`、Git、活动项目及项目设置仍按后续步骤推进。

入口为 `projects.rs`、`project_paths/windows/delivery.rs`；最终保护由 Rust 持有，不依赖 UI 预检查。使用仓储事务串行化撤权/重新授权，核验修订和已登记目录 File ID，只有同步 OS 交付成功才更新一次 `project_usage`，随后发布无载荷失效事件。schema 仍为 v5，没有改写或追加迁移。列表、搜索和文件选择不增加使用次数；选择器正常取消不写项目、授权或使用数据。

文件参数复用严格相对路径校验：拒绝绝对路径、UNC、设备、ADS、穿越、空组件、DOS 设备名（含上标数字别名）、尾随点/空格及超长输入。逐组件、同卷核验，不把字符串前缀当范围证明；不追随任何重解析点。目录身份失效由既有查询/审计流程记录；未知/已撤权项目固定失败，无路径回退。

## 最终目标保护与平台决定

补充的真实 Win32 攻击用例发现：仅持有 `FILE_GENERIC_READ` 且不共享写入/删除，仍不能阻止此前打开的 `FILE_WRITE_ATTRIBUTES` 句柄执行 `FSCTL_SET_REPARSE_POINT`，空目录可以被原地改成 junction。普通改名测试通过不足以开放系统交付。该发现已转为保留的回归用例；早期未带最终保护的桌面通过记录不作为本步最终证据。

最终适配器采取以下边界：

1. 从盘符根逐级以 `FILE_FLAG_OPEN_REPARSE_POINT` 打开目录/叶目标，使用真实读访问及仅共享读的句柄，核验每级属性、规范路径、父子组件、卷和最终身份；句柄拒绝改名、删除及数据写入。文件路径的每个祖先都保留不可删除的下一组件，因此不能被清空后转成重解析点。
2. 对实际交付的最终目录、文件或编辑器 exe，使用 `CreateTransaction` / `CreateFileTransactedW` 取得属性写权限，在事务内暂存**未改变的** `FILE_BASIC_INFO`，取得元数据写锁并再次核验身份/重解析属性。已有非事务属性写句柄使保护获取失败；保护后新属性写句柄被拒绝。事务不提供有限自动超时，避免调用者停止等待后保护提前失效。
3. 所有文件系统事务**只回滚，永不提交**；RAII 关闭句柄并回滚，源码内容和原有创建/修改/变更时间、属性保持不变。正常数据读句柄继续固定名称；该保护是系统交付的短期锁，不是新的路径授权或持久授权记录。
4. 默认程序在专属 STA 线程用 `ShellExecuteExW`、固定 `open` 动词和 `SEE_MASK_NOASYNC` 同步交付；保持保护直到该调用返回。文本文件先用 `AssocQueryStringW` 且忽略未知关联查询默认程序，防止 Windows 把弹出“选择应用”误记为默认程序成功。默认交付拒绝 `.lnk/.url/.website/.appref-ms/.search-ms/.library-ms` 等会转向其他目标的 Shell 文件。
5. 配置编辑器只采用已保存 `{id:"custom",executable}`。验证本地绝对 `.exe`、逐级身份、无重解析及 DOS/PE 装载条件；对 exe 也取得最终保护，`Command` / `CreateProcess` 使用单一绝对目标参数、系统工作目录和空标准流，不经命令解释器，不接收前端参数数组。配置失效固定失败，不回退默认程序。

Windows 的 TxF 是本次为满足严格路径竞态门选择的受限适配手段，并非通用兼容承诺。微软已建议优先采用替代方案，并说明未来可能移除此 API；`CreateFileTransactedW` 不支持 ReFS。当前没有可证明满足同等交付身份约束的更弱回退，因此 ReFS、TxF 不可用、受保护/只读权限目标、并发元数据冲突等均拒绝交付。特别是无法对已配置 exe 取得属性写权限时也拒绝，该 exe 的语法合法不代表可启动。重新设计其他文件系统/系统版本适配时须重验安全门；登记层的 NTFS/ReFS 范围不等同于打开层范围。

技术依据：[CreateFileTransactedW 的限制与兼容表](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createfiletransactedw)、[CreateTransaction 的无限超时语义](https://learn.microsoft.com/en-us/windows/win32/api/ktmw32/nf-ktmw32-createtransaction)、[设置重解析点及非空目录限制](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/fsctl-set-reparse-point)。普通共享锁和 advisory oplock 的实验不能代替最终事务保护，本次没有把实验工具加入生产依赖。

## 选择器、失败和结果语义

`GetOpenFileNameW` 以项目目录为初始位置，使用文件存在、不改变进程工作目录、不写最近文档和不解引用快捷方式的选项。**原生界面允许导航；限制位于 Rust 的结果接受边界**：只接受项目根内经组件/身份/重解析点复核的现有普通文件，只返回相对路径；外部位置固定拒绝。弹窗期间不持数据库锁，选完后重新核验项目修订、授权、目录身份及文件，撤权不能被旧弹窗绕过。打开文件时仍独立重新验证，不把选择结果当长期授权。

缺失文件、失效根目录、身份变化、访问拒绝、无关联/编辑器失效/系统拒绝和超时，分别使用现有固定项目错误，不传原始 OS 文本、SQL、路径或 stderr。路径/元数据冲突取不到保护时不会调用系统。

原生交付等待最多 10 秒；超时后返回 `PA_PROJECT_OPEN_TIMEOUT`，不终止或重新调用正在运行的 native 操作。后台线程继续持有全部保护和单次准入许可，直到实际原生调用结束；未结束时后续交付固定拒绝，避免积累工作线程。超时操作可能稍后被系统接受，但不补报成功、不补写本次使用记录、不自动重试。前端打开默认等待 12 秒，选择器等待 120 秒；停止等待仍不等于服务端取消。

`systemAccepted:true` 仅表示 `ShellExecuteExW` / `CreateProcess` 接受交付，**不保证外部程序加载、显示或编辑完成**，也不把保护延伸到外部程序未来的路径重开。如果系统接受后 SQLite 使用事务失败，已发生的系统副作用不能回滚；返回固定存储错误，不能自动再发一次打开。

## 验收证据

入口：`npm run verify:projects`。最终汇总为 `artifacts/phase-2-5/report.json`，built/dev 子报告在 `artifacts/shell-{built|dev}-projects/report.json`，均属忽略生成物。Node.js 24.18.0 / npm 11.16.0 / Rust 1.99.0、Windows 11 x64 10.0.26300；本轮仅使用自建临时目录/数据库、合成文件和合成编辑器，不读取真实 Key，不调用模型。

| 验证 | 实际结果 |
| --- | --- |
| 前端 build / Rust fmt / release check | 通过 |
| 全部 Rust lib | 151 passed、5 ignored；其中交付安全 9 项通过 |
| IPC / Codex 离线 | 33 / 15 passed |
| 存储与 schema v5 独立进程重启、清理 | 52 passed，1 ignored worker 由父测试驱动；通过 |
| 默认目录、中文/空格/特殊字符文本文件 | built/dev 真实 WebView → 正式 Rust 服务 → Windows 默认程序接受 |
| 配置编辑器 | 两种模式编译一次性真实 PE，实际启动后仅收到一个字面目标参数，写固定回执并退出；不把 Node mock 记为编辑器实测 |
| 失败与权限 | 越界/伪造参数/缺失/失效根/无关联/Shell 转向文件/失效 exe、七命令 overlay/未知窗口/远端拒绝，built/dev 通过 |
| 文件系统攻击 | 已有/新属性写句柄、真实 junction/重解析修改、祖先/根/文件改名删除写入、身份替换、超时后保护继续持有及原始元数据回滚通过 |
| 隐私与数据回归 | 命令白名单存在；进程/产品 JSONL 不含路径、查询、文件名或回执哨兵；源文件保持不变，授权/审计/提交失败回滚和搜索回归继续通过 |

属性句柄与真实 `FSCTL_SET_REPARSE_POINT` 是实际 Windows 操作；权限错误码分支另有适配层注入，超时采用有界原生回调注入，不声称已真实挂死系统默认程序。TxF 特殊元数据用例串行运行，产品打开服务也串行准入；其他进程造成的共享/事务冲突属于固定失败，不自动重试。保留既有两条 Rust 未使用 warning；5 个 ignored 的专项执行范围沿用既有入口，未重跑 CLI/双 HTTPS 专项。

原生选择器首次选择/取消/键盘、用户实际页面入口与外部程序加载的人工点验不计为自动通过；2.7/2.9 接续。ReFS/TxF 不可用的实卷与更多 Windows 版本未实测，不记为通过。测试可能留下默认程序窗口，不会批量结束用户的资源管理器/编辑器。阶段 1 的托盘/显示器/DPI 人工项、阶段 2 Git/活动项目、阶段 3 真实任务和阶段 4 HTTPS 搜索保持各自验收门。
