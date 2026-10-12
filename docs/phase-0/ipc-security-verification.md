# 步骤 0.6：IPC、错误、日志与安全约定验收

> 2026-10-11 当前范围：用户已确认“用户手动管理 cc-switch、应用自管 profile”，见 [ADR-0011](self-managed-profile-adr.md)。步骤 0.3、0.7 与阶段 0 按替代范围通过，允许启动阶段一；原自动集成/共享代理未通过的事实及以下当时记录保留为历史，不作为当前自动需求或验收安排。生产自管凭据/路由/恢复仍待实现实测；旧切回 cc-switch 的约定已撤销。

日期：2026-10-11（Asia/Shanghai）。基线：Windows 11 build 26300 x64、已有固定 Node/Rust/Tauri/WebView2 工具链；不扩大到其他 OS/架构。结论：步骤 0.6 契约及参考验证通过。阶段 0 按 ADR-0011 的替代范围通过；本步骤参考验证不证明生产自管凭据、路由或审批服务已通过。

## 产物与门槛对应

| 0.6 要求 | 产物 / 可审阅结果 |
| --- | --- |
| React/Rust/适配器职责、最小命令与事件、版本策略 | [IPC v1 草案](ipc-contract.md)、`src/ipc/contracts.ts`；阶段一最小服务、后续生命周期预留、事件顺序/恢复、输入限额和权限都有定义 |
| provider/模型/脱敏Key/测试/等待/回滚 | CommandMap 与 EventPayload；CredentialState 仅状态/revision；AiSelection 固定修订；候选测试、FIFO等待、unchanged/rolled_back/rollback_failed 与外部变更规则明确 |
| 错误分类/固定前缀/提示/建议/关联ID | [错误与日志](errors-and-logging.md)；framework_probe 已实现4个业务错误、v1互斥返回和唯一关联ID；客户端校验并屏蔽 transport 原始错误 |
| 日志脱敏/保留/轮转/清理/审计 | 白名单、7天/25MiB日志、90天最小审计、诊断限制和失败动作成文；当前stderr只记录固定元数据，无payload；轮转/清理留阶段一 |
| 路径/进程/超时取消/审批默认 | [安全清单](security-checklist.md)；已有文件参考guard、联接/ADS检查、单次审批参考、owned进程/超时/默认拒绝明确；生产TOCTOU和网关原子审批留后续 |
| 至少四类威胁与对应测试 | SEC-001–014；路径穿越、命令注入、审批绕过、秘密泄露均有本轮可执行断言，其余明确继承证据或后续前置条件 |

第一个服务命令可以沿 framework_probe 的严格请求、最小权限、服务结果 envelope、固定错误、服务端关联ID和日志白名单实现；权限/错误/日志不需要自行猜测。当前没有通用shell/文件写入/自动批准接口，高风险业务操作未开放，参考审批未知类型和拒绝/过期/重放默认不能授权。

## 本轮验证

入口 `npm run verify:ipc`：按序执行全部 Rust lib 测试、IPC Node 测试、Codex 离线夹具、前端类型/构建、Tauri 嵌入资源调试构建、真实桌面验证。不访问真实模型服务、不读取Codex/cc-switch配置或Key、不切换用户全局配置。只有全部成功 report.passed=true、退出码0；独立失败不会由历史报告掩盖。

| 检查 | 本轮结果 | 证据 |
| --- | --- | --- |
| Rust lib测试 | 22通过（新增IPC3、安全3，存储16回归）；0失败/忽略；联接夹具实际建立并清理 | `artifacts/ipc/rust-tests.log` |
| IPC/前端codec、OS参数和静态权限 | 4通过；错误/版本fallback、字面argv、main本地app命令显式授权 | `artifacts/ipc/ipc-tests.log` |
| Codex离线安全/协议回归 | 15通过；Key站点/endpoint、审批白名单、超时/断连、错误不含raw payload等 | `artifacts/ipc/codex-offline-tests.log` |
| TypeScript/Vite与调试程序 | 成功；22 modules，构建独立exe | `artifacts/ipc/desktop-build.log` |
| 嵌入资源真实WebView2 | 7项通过；React中文特殊字符往返、固定错误与重试、v1成功/错误/版本拒绝、唯一关联ID、正常退出0、日志关联和秘密/输入排除 | `artifacts/desktop-built/report.json`、process.log、window.png |
| 开发模式真实WebView2 | 同样7项通过；开发服务和自有测试程序退出/清理 | `artifacts/desktop-dev/report.json`、process.log、window.png |
| 系统打开回归 | 8项通过；app权限与加强路径检查没有破坏夹具目录/文件、编码URL和范围拒绝 | `artifacts/git-system/system-open-report.json` |

汇总：`artifacts/ipc/report.json`。文件均为被忽略的可重建本机证据，文档保留实测结论。`framework_probe` 从旧字符串错误迁移到v1，React原验证页面仍可使用；0.4 preflight暂留旧字符串契约，只抽取/收紧了路径guard，不冒充正式项目文件服务。Tauri build.rs 生成两个app command的权限，default capability显式授权，不为草案里的其他命令增加权限。

开发窗口与额外系统打开回归在汇总入口之后分别执行。开发模式与嵌入资源构建会覆盖同一target/debug程序，首次在开发验收后直接运行system-open因缺Vite且目标URL不是tauri.localhost而等待超时（0项）；重新执行`npm run tauri -- build --debug --no-bundle`后再运行`npm run verify:system-open`，8项全部通过。复现须遵循README的先build再built/system-open顺序，不将启动超时算作安全断言成功。

## 明确未实现和后续验收

- CommandMap 的业务命令、正式event通道、系统凭据库、profile原子切换、真实cc-switch适配器仍是草案，不返回“模拟ready/成功”。官方源仅作预留，无需实测，其结果不纳入通过判定。
- PendingApproval 是绑定/过期/单次消费参考，未连接Rust Codex网关/数据库；生产指纹生成、决定+审计原子事务、多请求/断连和真实文件审批需要实现后复验。0.5 schema尚无指纹/expiry持久字段，阶段一若持久化新增字段，按ADR-0008追加迁移；发布过的0001不得改写。
- existing_scoped_path 验证已有文件的规范化边界；不能消除检查后重解析点替换。任意项目路径/新建写入与句柄身份、remote/未授权窗口动态拒绝按SEC-003/008上线前验收。
- 日志轮转/清理/诊断导出、90天审计和自由文本摘要脱敏未部署，限额/失败动作已有明确规范。凭据库不可用不得降级明文；自由文本历史未经验证不能开启。
- 旧 cc-switch 自动集成和共享代理实验保持其历史结果，本步骤不改变其“未通过”事实；当前需求按 ADR-0011 改为用户手动管理 cc-switch/应用自管 profile，阶段 0 按替代范围通过。生产自管路由及安全风险仍按功能上线门复验，见risks.md、phase-gate-review.md与ADR-0011。
