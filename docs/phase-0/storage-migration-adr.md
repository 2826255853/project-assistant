# ADR-0008：Rust SQLite 与事务迁移、升级前一致性备份

> 2026-10-11 范围更新：按用户确认的手动 cc-switch / 应用自管 profile 替代方案，步骤 0.3、0.7 与阶段 0 按替代范围通过，允许启动阶段一，见 [ADR-0011](self-managed-profile-adr.md)。本文实测证据与原当时阶段门描述保留；当前生产自管凭据、路由隔离及恢复仍须独立验收。

日期：2026-10-11。状态：采用步骤 0.5 初稿和当前 Windows x64 实验路线；正式仓储、IPC、凭据库与启动恢复服务留到阶段一。

## 背景与选项

首版项目、任务、审批、profile 与配置快照需要引用完整性、原子更新和重启保留。选择 SQLite 而非散落 JSON；选择 Rust `rusqlite 0.32.1` 的 bundled/backup 功能，而非 React 可直接执行任意 SQL 的插件。只引入连接/迁移和合成实验，不建立 CRUD/通用 SQL IPC。随 Cargo.lock 固定 SQLite 库，避免依赖用户安装 sqlite3.exe。SHA-256 使用 sha2 0.10；可执行版本和实测见 storage-verification.md。

## 决策

正式位置由 Tauri `app.path().app_data_dir()` 决定，数据库名 `project-assistant.sqlite3`，不得把数据库放在 cwd 或项目目录。阶段 0 实验只使用该目录下 `phase-0-storage/probe-<UUID>/`，完成后清理自己创建的子目录；正常应用启动目前不接入业务数据库。测试参数仅存在于 debug 构建，release 忽略该入口。

每条连接开启 foreign_keys、5 秒 busy_timeout、trusted_schema=OFF、synchronous=FULL；成功验证/迁移后使用 WAL。业务层后续采用单写入者/后台执行，SQLite Connection 不直接作为并发 IPC 状态共享。WAL 与 SHM 文件是数据库运行状态的一部分，不能当作独立垃圾随意删掉。

迁移为编译嵌入的连续编号 SQL（0001_initial.sql 起），版本号与 SQL 的 LF 规范化 SHA-256 记录到 schema_migrations；PRAGMA user_version 是快速版本标记，两者必须一致。发布后禁止改写已应用 SQL，修改只能增加下一版本。禁止 down migration 和“失败后重新创建库”。schema v1 的实体/字段见 data-model.md。

## 执行和失败原则

1. 在应用数据目录打开文件，设置连接级保护，取得 BEGIN IMMEDIATE 写锁。锁获取失败报告 STORAGE_BUSY；不跨进程跳过锁，不偷偷另建数据库。
2. 检查 user_version、quick_check、迁移历史/校验和、foreign_key_check。高于应用支持版本报告 STORAGE_NEWER_SCHEMA；无版本但有未知表报告 STORAGE_UNRECOGNIZED_SCHEMA；损坏/历史不一致报告对应固定码；都拒绝使用原库，不重置版本或修复猜测。
3. 有待迁移且原数据库已存在时，在保持写锁期间，通过独立只读连接调用 SQLite backup API，复制已提交的一致性视图（包含 WAL），写到同数据目录 backups/before-vN-<时间戳>-<PID>.sqlite3。create_new 防覆盖，备份 quick_check 成功且 sync_all 后才能开始升级。不使用只复制主 .sqlite3 文件的方法。
4. 备份按 128 页一批执行，循环限制为 30 秒，busy/locked 短暂重试，超时或创建/校验/同步失败报告 STORAGE_BACKUP_FAILED，停止升级，保留原库与失败备份。不清理或覆盖用户数据。新建无数据数据库可不备份；上次失败遗留的既有空库再次迁移也会先备份。该限时不等于能中断操作系统阻塞 I/O，正式大库需后台进度与更完整预算。
5. 所有待执行 SQL、schema_migrations 插入、user_version 更新和最终 foreign_key_check 在同一事务中执行；任一步失败自动 ROLLBACK。报告 STORAGE_MIGRATION_FAILED，并在 Rust 内携带 backup_path 供未来恢复 UI 使用；Display/普通日志仅固定错误码，禁止返回 SQLite 原 SQL、参数和原始错误。
6. 成功一次 COMMIT；不自动删备份。断电/进程退出后的原子恢复由 SQLite journal/WAL 管理，下一次打开重新校验，禁止另写“自动重建库”。本轮未做物理断电实验，不能承诺磁盘故障自动恢复。

备份与迁移在同一写锁保护区，避免多个实例读取旧版本后交错迁移。它不能保证不受磁盘损坏或外部直接改库影响；新环境需复测磁盘权限、空间、同步/杀进程场景。阶段一会将固定码映射为“关闭其他实例/释放磁盘/使用兼容版本/选择已验证备份”等用户动作，完整错误/日志契约由 0.6 定义。

## 手动恢复原则

事务失败且原库可读时，先保留原文件并排查/修正升级实现，重新打开重试；通常不需 restore。只有原库不可用或用户明确选择恢复时才用备份：

1. 退出所有使用该数据库的本应用实例和进程，停止任务；确认没有打开的连接。
2. 只读打开候选备份，运行 quick_check/foreign_key_check，核对 user_version、schema_migrations 校验和和目标应用版本；记录不含内容的结果。
3. 将原数据库及其存在的 -wal/-shm/-journal 一起保留到同磁盘唯一隔离目录，禁止先删原库或覆盖正在使用的文件。
4. 将候选备份复制到数据目录一个新的暂存文件，再校验；在连接全部关闭时把暂存文件替换到正式名称，故障时保留两套文件并报告。禁止遗留旧 sidecar 到新库旁，禁止用旧应用强制降级打开高版本库。
5. 用匹配版本重新打开/校验，确认项目、设置与任务记录；凭据引用可能失效，重新连接测试，不从备份恢复真实 Key或重放任务/审批。

本阶段实现备份与失败保护，未提供自动 restore/备份清理/UI；这些按上述流程在阶段一实现并验证。测试只验证备份可读、升级前数据/版本保留以及失败后原库能重开，不把手动说明写成已实现恢复产品。

## 失效条件与复审

SQLite/tauri/rusqlite 更新、schema 变动、多个写进程、跨平台目录/权限、加密存储和跨设备迁移出现时复审。若未来引入摘要存储或外部 profile 接口，必须先完成输入白名单/脱敏和凭据隔离验证，不能把 auth 文件或第三方数据库拷贝进本库/备份。

步骤 0.5 的数据/迁移通过不替代步骤 0.6 与 0.7 的验收；自动 cc-switch 集成已按 ADR-0011 退出范围，生产自管路由与凭据仍需按功能验收。
