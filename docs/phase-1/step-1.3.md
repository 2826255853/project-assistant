# 步骤 1.3（S1-03）验收记录

日期：2026-10-13（Asia/Shanghai）  
结论：**通过**。正常桌面启动已接入 Tauri app_data_dir 下的 SQLite 业务库，完成 schema v1 迁移、失败保护、类型化仓储和启动恢复；旧的阶段 0 storage probe 仍保持独立调试入口。

## 实现范围

- 正常模式启动时通过 app.path().app_data_dir() 打开 project-assistant.sqlite3，初始化连接保护、迁移校验和 WAL；数据库初始化失败不会静默重建或覆盖原库，应用保持受限状态并在状态服务中返回固定 PA_STORAGE_* 原因码。
- StorageState 持有进程内唯一业务连接和健康状态；app_get_status 的 storage 服务在正常模式初始化成功后返回 ready，失败返回 unavailable 和固定原因码，不暴露 SQLite 原始错误、SQL、参数或路径。
- 增加 SettingsRepository、ProfileRepository、CapabilityRepository、SnapshotRepository、AuditRepository。设置白名单、事务写入、profile/能力/快照引用和最小审计写入均通过类型化 Rust 仓储访问，未增加任意 SQL IPC。
- 启动恢复在同一事务内把 idle/preparing/running/approval_required/cancelling 任务标为 interrupted，把 pending 审批标为 interrupted，写入一条最小 storage_recovery 审计事实；不恢复执行、不重放审批。再次启动没有遗留未终态记录时不重复写恢复事实。
- storage 失败码映射为固定公共码，包括 busy、目录/打开/SQL、备份/迁移、损坏、高版本、未知 schema、迁移历史/序列和外键校验失败。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| cargo fmt --check --manifest-path src-tauri/Cargo.toml | 通过 |
| cargo test --locked --manifest-path src-tauri/Cargo.toml --lib storage:: | 18 项通过 |
| cargo check --locked --manifest-path src-tauri/Cargo.toml | 通过 |
| npm run build | 通过 |
| npm run verify:state | 通过；开发资源和嵌入资源正常启动均确认 storage ready |
| 失败保护回归 | 迁移失败、备份失败、损坏库、高版本库、未知 schema、校验和不一致和外键破坏均保留原库并返回固定错误 |
| 启动恢复回归 | 任务/审批中断、最小审计和重复启动幂等通过 |
| 仓储回归 | 设置读取/事务写入、白名单拒绝、profile/能力/快照查询和审计写入通过 |

桌面综合报告：artifacts/phase-1-2/report.json。阶段 0 独立 SQLite 实验报告仍为 artifacts/storage/report.json；两者均为本机生成物，不提交版本库。

## 边界与遗留

- 本步骤没有实现 settings_get/settings_update IPC 或设置页；全局设置默认值、冲突处理、常驻设置统一数据库化由步骤 1.4 接入。现有步骤 1.1 的 overlay-settings.json 仍是尚未迁移的常驻入口持久化边界。
- 日志文件轮转/清理和完整生产审计策略由步骤 1.5 接入；本步骤只提供数据库中的最小审计仓储和启动恢复事实。
- 系统凭据库、profile 真实创建/切换、外部进程隔离、网络连接测试和官方源预留仍未实现。cc-switch 继续由用户手动管理，本应用不读取、导入、同步或切换其配置和 Key。
- 手动备份恢复 UI、备份清理、断电/磁盘损坏/磁盘满/权限拒绝等 OS 场景尚未宣称通过；当前遵循 ADR-0008 的保留原库和人工恢复说明。
- 阶段 1 尚未整体完成；后续验收仍以计划文档跟踪表、AT-S1-002 和 AT-S1-004 为准。
