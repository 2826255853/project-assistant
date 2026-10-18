# 步骤 0.3：社区 CLI 替代接口补验

> 2026-10-11 当前范围：用户已确认“用户手动管理 cc-switch、应用自管 profile”，见 [ADR-0011](self-managed-profile-adr.md)。步骤 0.3、0.7 与阶段 0 按替代范围通过，允许启动阶段一；原自动集成/共享代理未通过的事实及以下当时记录保留为历史，不作为当前自动需求或验收安排。生产自管凭据/路由/恢复仍待实现实测；旧切回 cc-switch 的约定已撤销。

日期：2026-10-11（Asia/Shanghai）。历史原范围结论：**补齐社区 CLI 的隔离实测，步骤 0.3 仍阻塞。** 九项检查通过，其中交互命令和 Windows 功能检查验证的是限制，不代表这些能力可用于自动集成。该结论对应旧 FR-AI-006；当前按 ADR-0011 已退出自动接入范围。

## 选型与版本

当前已安装桌面版仍为 4.0.6，`npm run inspect:cc-switch` 再次生成 blocked 报告。上游最新桌面发布为 [v4.0.8](https://github.com/farion1231/cc-switch/releases/tag/v4.0.8)，2026-10-10 发布；其发布说明没有提供解决外部双向集成的契约证据。本轮未升级用户安装。

桌面版固定 README 推荐的 [SaladDay/cc-switch-cli](https://github.com/SaladDay/cc-switch-cli) 是独立社区项目。本轮选择已发布的 [v5.11.0](https://github.com/SaladDay/cc-switch-cli/releases/tag/v5.11.0)，标签 commit 为 `b781987413279ccca6102ef0ad2a283fe364b566`。仅下载 Windows x64 便携包到被 Git 忽略的 artifacts，不修改 PATH，不全局安装。归档 SHA-256 为 `2493764062063f2b0e5a3a1eda009f4e62451a0c113dc3f294beb10267deb6ff`，与 GitHub release asset digest 核对；可执行文件摘要另记在报告中。

源码依据均固定到该 commit：

- [命令与 Windows 编译边界](https://github.com/SaladDay/cc-switch-cli/blob/b781987413279ccca6102ef0ad2a283fe364b566/src-tauri/src/cli/mod.rs)：`start` / `daemon` 受 `cfg(unix)` 限制。
- [provider 命令](https://github.com/SaladDay/cc-switch-cli/blob/b781987413279ccca6102ef0ad2a283fe364b566/src-tauri/src/cli/commands/provider.rs)：`add --config-file` 可非交互添加；`edit` 进入交互式表单。
- [导入、备份和恢复](https://github.com/SaladDay/cc-switch-cli/blob/b781987413279ccca6102ef0ad2a283fe364b566/src-tauri/src/cli/commands/config.rs)：导入/恢复要求终端确认，不能假设存在 `--yes`；导出为 SQL 数据库备份，不是 provider JSON 编辑接口。
- [隔离路径](https://github.com/SaladDay/cc-switch-cli/blob/b781987413279ccca6102ef0ad2a283fe364b566/src-tauri/src/config.rs)：支持 `CC_SWITCH_CONFIG_DIR` 和 `CC_SWITCH_TEST_HOME`。
- [数据库版本](https://github.com/SaladDay/cc-switch-cli/blob/b781987413279ccca6102ef0ad2a283fe364b566/src-tauri/src/database/mod.rs)：schema 20。

## 可复现结果

```powershell
npm run prepare:cc-switch-cli
npm run verify:cc-switch-cli
```

准备命令通过 GitHub release API 下载固定资产并校验；下载中断可重跑续传。其他二进制位置可通过绝对路径 `CC_SWITCH_CLI_BIN` 指定，验证时仍要求版本 5.11.0。报告在 `artifacts/cc-switch-cli/report.json`。退出码 0 / `passed=true` 只表示本组接口及限制检查符合预期；旧报告的 `integrationStatus=blocked` 仅表示旧自动接入范围未通过，不能解释为当前自管替代方案失败。

| 检查 | 实测结果 | 范围 |
| --- | --- | --- |
| 便携版本与隔离路径 | 5.11.0；config path 指向独立临时目录 | 真实社区 CLI |
| schema 版本 | 当前桌面库与全新 CLI 库均为 20 | 桌面库仅一次性只读 `PRAGMA user_version`；版本一致不证明完整兼容 |
| 非交互添加 A | `provider add --config-file` 成功，列表含 A | 合成 provider、合成 Key |
| 非交互添加 B | 添加成功，列表含 B | 同上 |
| A→B→A 切换及 Key 绑定 | current 与实际 config 的站点/凭据逐次一致 | 两个保留 `.invalid` 域名；没有远端认证实测 |
| 无效候选失败保留 | 不存在的 provider 返回非零，配置指纹和 current 保持 A | 不等同 HTTP 401 候选测试；此前 401 实测仍见 codex-verification.md |
| 备份与导出 | SQL 备份和导出文件均生成 | 仅在受限临时目录；未恢复 |
| 编辑、导入和恢复自动调用 | 三个命令在无终端调用中均超时，3 秒后只终止对应自有子进程；config/current 保持 A | 结合固定源码确认交互要求；自动闭环未通过 |
| Windows 隔离启动/代理 supervisor | `start` 返回不识别的子命令，help 无 daemon | 前台 `proxy serve` 未测；不等同桌面共享代理 |

临时根目录使用当前用户专属 DACL。CLI 的配置根、测试 home、Codex home 和其他工具默认目录指向该根；不导入真实 provider 或凭据，不把用户数据库交给 CLI。脚本只在内存核对合成 Key，报告保存布尔值、固定错误码和版本。SQL 导出/备份可含合成凭据，因此也只留在临时根并在结束时删除。报告确认 `sharedConfigurationUnchanged=true`、`tempRemoved=true`；共享配置核对覆盖 Codex config/auth 与 cc-switch settings/live-state，不宣称监控了用户库的全部后台写入。

## 阻塞判断及后续路线

社区 CLI 已有可调用的添加/列表/current/切换命令，原先“未安装/未测试”的信息缺口得到补齐。但它尚未提供本轮可验证的非交互编辑和恢复闭环；schema 20 相同也不能证明桌面与 CLI 的字段、live-state、代理及并发写入兼容。本轮没有将其接入用户运行中的桌面库。

桌面固定源码支持测试 home，但同时注册 single-instance 插件；不能仅设置隔离目录就假定第二个桌面实例独立。为避免第二次启动转交用户实例，本轮没有启动第二个桌面、注入 WebView 或调用内部 Tauri 命令。因此 E-017 的真实桌面共享路由/外部任务影响仍缺证据。

保持 ADR-0006 的历史判断。若未来重新要求自动集成，仍需受支持的桥接接口，至少提供结构化读取、非交互编辑/凭据绑定、候选验证、切换校验、备份恢复和外部变更检测，并允许独立桌面实例验证。可将社区 CLI 固定版本适配作为部分候选，但不能直接共享用户库后宣称通过；PTY 交互自动化需另行验证兼容性和维护责任，目前未验证。当前已按 ADR-0011 选择 cc-switch 手动管理、应用自管 profile 的路线。

官方源功能继续仅作预留，未执行官方认证或请求，其结果不参与阻塞判断。
