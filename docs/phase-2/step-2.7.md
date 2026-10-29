# 步骤 2.7（S2-07）：项目页面和首页近期入口

状态：**有条件通过（2026-10-29，Asia/Shanghai）**。实施与自动验收由本次 Codex 会话完成。

本步依据计划第 22.3 节，接入现有项目登记、列表搜索、排序、目录/文件打开、Git 摘要和移除命令，未实现 2.8 的活动项目选择。项目页面使用 `ProjectSearchController`，搜索输入立即使旧请求失效；输入法组合阶段不发送查询，组合结束才提交。列表行支持鼠标、Enter、上下方向键、Home/End。

首页通过同一 `projects_list` 服务以显式 `sort: "recent"` 显示最近有使用记录的项目；项目页面的全局排序仍通过设置修订提交并在冲突时保留固定错误。页面展示加载、空列表、无搜索结果、路径失效、权限失败和 Git 降级，错误提示只使用本地错误目录并显示安全关联 ID。移除需要二次确认，只撤销应用记录，不删除源文件；每个操作期间按钮禁用，失败后重新读取列表。

## 变更与验证

- `src/pages/ProjectsPage.tsx` / `.css`：项目页面、近期入口、搜索/IME、键盘导航、Git 刷新、系统打开、文件选择、移除确认和状态反馈。
- `src/App.tsx`：项目导航和首页近期入口；项目服务状态随本地存储健康状态报告，不在存储不可用时误报就绪。
- `src/ipc/contracts.ts`、`src/ipc/projects.ts`：列表请求增加受限可选排序覆盖；默认排序规则保持兼容。
- `src-tauri/src/project_contract.rs`、`storage/projects/search.rs`：验证并应用 `recent/name` 只读排序覆盖；无迁移。
- `scripts/ipc/projects-desktop.mjs`、`verify-desktop.mjs`、协议/搜索测试：加入页面控件、空结果、排序提交、近期入口、键盘选择、IME 合成、添加→打开→近期→移除闭环及服务回归。

已通过：`npm run verify:projects`（报告 `artifacts/phase-2-7/report.json`）：Node 24.18.0/npm 11.16.0/Rust 1.99.0；前端构建、格式、157 项 Rust 单测（5 ignored）、33 项 IPC、离线 Codex、release check、存储重启回归，以及 built/dev 真实 WebView2 项目桌面。页面闭环覆盖隔离调试选择器的添加、目录/指定文件打开、首页近期入口、确认移除且源目录保留；该夹具仅由项目桌面验收显式开启，不改变正式原生选择器。最终搜索 1,000 条元数据 WebView IPC P95 built 23.7 ms / dev 22.9 ms（目标 300 ms）。

有条件通过边界：原生目录/文件选择器首次物理鼠标与键盘点验、真实中文 IME 设备输入和外部程序加载完成仍不由自动桌面夹具宣称，留待 2.9 人工复核；活动项目与跨窗口状态留待 2.8。项目页面不选择活动项目，不启动任务，不读取文件内容。
