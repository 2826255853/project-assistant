# 数据库问题修复与复验

日期：2026-10-19（Asia/Shanghai）。基于当前工作区已有的步骤 1.6 实现检查，不改变阶段完成状态。

## 问题与修复

1. `verify:storage` 将 schema 版本写死为 2，当前数据库已经是 v3。修复前完整验收在独立进程读回后报 `3 !== 2`，数据库读写本身成功。现在先检查迁移文件编号从 1 连续递增，再按该清单同时校验 seed 和 restart 进程返回的版本，防止后续追加迁移时误报或漏注册。
2. SQLite 错误被过度归并：非数据库文件在连接初始化时返回 `PA_STORAGE_SQL_FAILED`；开启 query_only 后启动迁移事务的失败被归为 `PA_STORAGE_BUSY`。现在按 SQLite 错误类型区分 busy/locked 与 corrupt/not-a-database，其余保留打开、SQL 或迁移历史的固定上下文错误。新增回归验证损坏库的启动健康状态、原文件字节保留、query_only 失败和真实双连接写锁冲突。错误内容仍只含固定代码。
3. `CapabilityRepository::count_valid` 原先只检查过期时间，未排除尚未到观测时刻或 profile 配置修订已经变化的记录。现在要求 `observed_at <= 查询时刻 < expires_at`，并关联当前 profile 的 `config_revision`。回归检查时间边界、配置修订失配以及历史任务快照保持原修订。

补充带数据的 v2→v3 升级回归：旧连接保持打开以保留 WAL；检查配置修订、凭据逻辑引用/修订、常驻停用状态、普通设置及原审计关联保留，凭据状态按旧引用初始化。升级前备份仍为 v2，可只读取回原设置及审计；升级后再次打开通过历史校验。预留认证类型仅作合成数据库行迁移检查，没有认证或网络请求。

本次没有修改任何既有迁移 SQL 或校验和，也没有对用户业务数据库执行修复、删除或重建。验证使用自有临时库及合成记录。

## 实际验证

| 入口 | 结果 |
| --- | --- |
| `npm.cmd run verify:storage` | 通过；前端构建、30 项 Rust 存储测试、嵌入资源调试构建、两个独立进程写入/读回 schema v3、WAL 和测试目录清理 |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib` | 54 项通过；包含原有 Windows 合成凭据测试 |
| `cargo check --locked --manifest-path src-tauri/Cargo.toml` | 通过；仍有既有未使用代码警告 |
| `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | 通过 |
| `npm.cmd run test:ipc` | 17 项通过、1 项失败；`scripts/ipc/response.test.mjs` 的旧断言禁止 main capability 中出现任何 credential 权限，与工作区已有的 `allow-credential-put/remove` 不一致；本次未修改该测试或权限 |

存储报告为 `artifacts/storage/report.json`，属于本机生成物。该报告记录 `expectedSchemaVersion=3`、`processRestartVerified=true`、`cleanupPassed=true`。

本次实际 Node.js 为 24.22.0、npm 为 11.19.1、Rust 为 1.99.0；Node/npm 与仓库固定基线 24.18.0/11.16.0 不同，不能宣称固定基线已复验。没有执行真实桌面托盘/显示器验收、真实 Key 或模型请求，也不代表步骤 1.6 剩余跨进程凭据与跨存储故障验收完成。

## 同日步骤 1.6 收尾追加

上述为数据库修复时的历史结果。后续按 `step-1.6.md` 完成凭据独立进程与跨存储故障补偿，修正 main 凭据专用权限的旧断言。最新 `verify:credentials` 为 64 项 Rust、19 项 IPC、15 项 Codex 离线及 dev/built 真实凭据桌面通过；`verify:storage` 再次通过，当前为 **31 项存储测试**，新增双连接核验补偿期间保持写锁（含真正 COMMIT 失败）。schema 仍为 v3，原迁移 SQL/校验和不改，存储探针独立进程与清理通过。不能据此宣称固定 Node/npm 基线、真实路由或阶段 1/M1 整体通过。
