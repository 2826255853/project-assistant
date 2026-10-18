# 步骤 0.5：SQLite、数据模型与迁移验收

> 2026-10-11 范围更新：按用户确认的手动 cc-switch / 应用自管 profile 替代方案，步骤 0.3、0.7 与阶段 0 按替代范围通过，允许启动阶段一，见 [ADR-0011](self-managed-profile-adr.md)。本文实测证据与原当时阶段门描述保留；当前生产自管凭据、路由隔离及恢复仍须独立验收。

日期：2026-10-11（Asia/Shanghai）。结论：当前 Windows 11 x64 基线通过步骤 0.5。范围限数据/迁移初稿与真实本地持久化实验；阶段门结论统一见 `phase-gate-review.md` 与 ADR-0011。

## 产物与环境

| 产物 | 位置 |
| --- | --- |
| 十个核心实体、ER、字段/索引/删除关系、默认值与留存说明 | docs/phase-0/data-model.md |
| 可执行 schema v1 | src-tauri/migrations/0001_initial.sql |
| 迁移/连接实现 | src-tauri/src/storage/mod.rs |
| 事务/约束/失败注入测试与合成夹具 | src-tauri/src/storage/tests.rs、probe.rs |
| 版本、备份、错误、手动恢复原则 ADR-0008 | docs/phase-0/storage-migration-adr.md、decisions.md |
| 可复现入口 | npm run verify:storage；scripts/verify-storage.mjs |
| 本机可重建证据（忽略进 Git） | artifacts/storage/report.json 与同目录构建/测试/进程日志 |

Rust 1.99.0，Tauri 2.12.2，rusqlite 0.32.1（bundled/backup），libsqlite3-sys 0.30.1，实际 SQLite 3.46.0；sha2 0.10.9。Cargo.lock 已更新，前端依赖未新增。数据库使用 STRICT、foreign_keys、synchronous=FULL、WAL 和 schema_migrations/user_version；迁移 SQL 的 SHA-256 规范化 CRLF 为 LF，避免不同 checkout 换行产生误判。

## 实验设计与结果

运行前按 E-010 成功标准：真实应用数据目录读写/事务/重启持久化，provider/凭据引用/能力缓存/快照关系明确；失败不静默覆盖删除。失败动作：修正 schema/迁移，保留原文件，不进入业务仓储实现。

`npm run verify:storage` 全链路返回退出码 0、report.passed=true：前端构建 → 16 项 Rust 测试 → 嵌入资源 debug 构建 → 独立 seed 进程 → 独立 verify 进程 → 清理自己创建的应用数据测试目录。

两个进程均通过 Tauri 的 `app.path().app_data_dir()` 取得真实位置，在该目录的 `phase-0-storage/probe-<UUID>/project-assistant.sqlite3` 操作。seed 退出后 verify 再次打开同文件；不是用内存 SQLite 或同连接重开代替进程重启。每个进程各完成 9 项检查：十个实体持久行、中文读写、全局/项目默认值、快照不可变、任务绑定不可修改、外键有效、拒绝 credential_ref 凭据载荷、事务回滚、foreign_key_check 干净。schemaVersion=1、journalMode=wal、sqliteVersion=3.46.0；合成 endpoint 为 fixture.invalid，完整真实 Key 和外部配置均未读取/保存。普通日志只含固定错误码/合成测试或编译结果。

| Rust 用例（16 项） | 结果与保证 |
| --- | --- |
| 新库约束/重开 | schema v1，夹具约束和数据重开通过，无重复迁移/无不必要备份 |
| 成功升级 + 提交 WAL 备份 | 保持原写连接打开，v1 → 测试专用 v2；备份含已提交 WAL 行，版本仍 v1、不含 v2 列 |
| 失败升级及重试 | 注入 DDL/DELETE 后 SQL 失败；DDL、行和版本账本回滚；备份可读，原库重开正常，修正后可升级 |
| 备份失败阻止迁移 | backups 被文件占用，返回 STORAGE_BACKUP_FAILED，原行未删 |
| 高版本拒绝 | user_version=99，拒绝降级且保持 99，无新备份 |
| 校验和不一致拒绝 | 改动账本 checksum 后拒绝使用，数据保留 |
| 损坏库保留 | 非 SQLite 文件拒绝打开，原文件字节不变 |
| 无版本未知库保留 | 自定义未知表，拒绝假定空库；原表/行仍可读 |
| 默认值唯一/allowlist | 全局重复、任意 api_key 设置、带认证 userinfo/query endpoint 拒绝 |
| profile 变动与能力缓存删除 | 历史快照名称/内容不变，引用中的 profile 不可删除 |
| 历史删除依赖与审计 | 项目不能越依赖删除；按事务顺序清理后审计上下文 SET NULL，最小事件保留 |
| FK 破坏拒绝 | 外部关闭 foreign_keys 后造孤儿行，重新打开报告 STORAGE_FOREIGN_KEY_CHECK |
| 审批/任务生命周期 | pending/决定时间与终态结束时间约束；运行前取消可无快照，不虚构实际配置 |
| 失败日志脱敏 | 失败 SQL 中的合成敏感字段名不出现在 StorageError Debug/Display；必须真正进入迁移失败路径 |
| 迁移编号连续 | 起始版本为 2 的迁移表拒绝 |
| CRLF/LF 校验和一致 | 同一 SQL 在不同 checkout 换行得到相同 hash |

测试 v2 仅内嵌在测试用例，应用正式最高版本仍是 v1。额外 `cargo check --locked --manifest-path src-tauri/Cargo.toml`、`cargo fmt --check --manifest-path src-tauri/Cargo.toml` 通过；桌面嵌入资源常规 IPC 回归另见本机 desktop-built 报告。

## 本轮发现与处理

初轮 13/15 通过，两个升级用例得到 STORAGE_BACKUP_FAILED。备份内容创建成功，但 Windows 对只读文件句柄调用 sync_all 无法完成落盘刷新；改成 read/write 文件句柄后 15/15 通过。随后补充换行校验用例和“准备期取消”约束，最终 16/16 与真实应用目录两进程复验通过。

备份 API 默认便捷循环没有业务截止时间，本实现改用逐页 step，30 秒循环预算，busy/locked 短暂重试；失败留原库和部分备份。首次工具链只有 minimal 未带 rustfmt，本轮安装匹配 1.99.0 rustfmt 并完成格式检查。

## 边界与遗留

- 阶段 0 正常启动不创建业务数据库；只提供 debug 合成实验入口，没有 SQL IPC/完整仓储/UI。
- 普通 schema 无原始凭据字段、settings key 白名单、credential_ref 固定格式，实验不用真实 Key；这不替代所有文本输入的秘密识别/脱敏，也不等于库已加密。系统凭据库、endpoint 完整解析/站点绑定和实际 cc-switch 凭据保护留阶段一/0.6。
- 备份失败和 SQL 失败注入已验证；未实测物理断电、磁盘损坏、磁盘满、权限拒绝或其他 OS/CPU。恢复操作/自动清理只有明确流程初稿，尚无自动恢复/清理产品功能。
- 本地重启读回任务不代表恢复 ephemeral Codex 会话；未决任务/审批的启动恢复策略已定义，实际服务留阶段一。
- 本实测记录不覆盖生产自管 profile/Key/路由服务；阶段 0 按 ADR-0011 的替代范围通过。汇总见 `phase-gate-review.md`。
