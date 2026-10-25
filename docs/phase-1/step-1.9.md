# 步骤 1.9：AI 配置页与选择流程

日期：2026-10-23（Asia/Shanghai）

## 实施范围

- 新增 `AiConfigPage`：应用自管非官方 HTTPS profile 的新增、编辑、启停、删除和状态读取。
- 新增凭据提交/移除表单。Key 使用密码输入，提交前不进入持久化状态，提交后立即清空；页面、日志和错误不显示完整 Key 或末尾片段。
- 新增 `ProviderSelection` 选择组件，展示服务源、模型能力、思考强度和凭据版本；切换模型会重新校验强度，能力失效时禁止提交。
- 接入 `provider_models`、`provider_test`、`provider_switch`、`operation_get` 和 `operation_cancel`。测试和默认切换均以服务端 operation 终态为准，展示检查、等待空闲、应用和恢复阶段；取消只请求可取消阶段，不自动重发变更。
- 设置默认只通过 1.8 的切换状态机提交，失败时显示原配置保留、回滚或受限错误；不会静默改用其他服务源。
- 页面启动和状态事件后重新读取持久化 profile/活动选择，重启后的 `degraded`、`error` 和 `ready` 状态按服务端校验结果展示。
- 主导航、首页说明、诊断页和设置页已从“AI 配置未启用”占位文字改为真实配置页入口；任务输入仍留给阶段 3。

## 验证

已执行：

- `npm run build`
- `npm run test:ipc`
- `cargo fmt --check --manifest-path src-tauri/Cargo.toml`
- `cargo check --locked --manifest-path src-tauri/Cargo.toml`
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib`：104 passed，4 ignored
- `npm run verify:providers`：通过；报告 `artifacts/phase-1-8/report.json`

`verify:providers` 已完成并通过：实际 CLI/双合成 HTTPS 路由、重定向拒绝、取消、隔离回归、release check、debug 构建，以及 built/dev 两种资源模式的 provider/credential 桌面场景；所有路由和桌面清理均通过。桌面脚本验证的是实际 IPC、窗口权限、凭据和清理链路，未自动点击 AI 页面控件；页面控件仍需人工点验新增/编辑、Key 清空、测试/切换进度和重启读回。官方源仍为预留，不纳入实测通过。

## 限制与后续

- 项目级默认和真实任务输入/审批/运行中切换属于后续阶段；当前选择组件已独立，可被阶段 3 任务入口复用。
- 当前页面不读取、导入、同步或切换 cc-switch profile，外部手动管理说明保持可见。
