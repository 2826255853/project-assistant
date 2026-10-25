# 步骤 1.7 / S1-06：自有进程配置与实际路由隔离

本步骤对应计划第 21.2 节、ADR-0011、AT-S1-007 及 AT-AI-005/008 的配置层范围。负责人职责：Rust / 集成 / 验收。本轮采用既定工作包，新增进程专属转发边界以核验实际请求；不延伸实现 1.8 的 profile、能力目录、切换状态机或阶段 3 的任务网关。

实施与自动验收执行者：本次 Codex 会话；按用户“根据文档，完成阶段 1.7”的授权实施。实际本机验证跨 Asia/Shanghai 2026-10-20–21，最终机器时间见报告。人工硬件/托盘复核仍由阶段 1.10 的桌面验收负责。

状态：**通过（2026-10-21，Asia/Shanghai，配置层既定范围）**。本轮 `npm run verify:isolation` 报告 `artifacts/phase-1-7/report.json` 为 `validation=full`、`passed=true`，临时资源清理通过；`--routes-only` 不代表完整验收。报告时间为 ISO UTC，另标注 `Asia/Shanghai`。

## 实现与接入边界

| 文件 | 职责 |
| --- | --- |
| `src-tauri/src/owned_process/mod.rs` | 后端选择并固定 exe 文件身份/SHA-256；参数数组启动；环境允许列表；stdio 握手、RPC/通知关联、早到终态、有界流、固定错误；配置指纹与生效核验；候选取消/退出 |
| `src-tauri/src/owned_process/route.rs` | 每候选独立的 loopback 入口、临时入口凭据、固定上游、DNS 地址固定、禁用代理/重定向、模型/强度校验、TLS、文本响应校验和秘密哨兵拒绝 |
| `src-tauri/src/owned_process/home.rs` | 随机专属 home、目录身份/重解析点拒绝、活跃锁、自有目录清理和启动时遗留目录回收 |
| `src-tauri/src/owned_process/windows.rs` | 当前用户专用 DACL；挂起创建、先加入 Job 再恢复；Job 关闭终止自有树，避免早期后代逃出归属 |
| `src-tauri/src/credentials.rs`、`storage/repositories.rs` | 内部 profile 元数据读取和持锁凭据解析；普通库仍仅存安全引用，未改已有迁移 |
| `scripts/verify-isolation.mjs`、Rust 测试 | 实际 Rust 适配器 + 实际 CLI + 两个合成 HTTPS 站点的请求证据和桌面回归 |

正常启动只初始化自有资源根并回收已标记、未被活跃进程持锁的遗留 home，不启动 Codex、不自动发现/读取外部配置、不发网络请求。`probe_candidate` 是供 1.8 调用的 Rust 内部接口，尚未新增可调用的业务 IPC、Tauri capability 或 UI 入口。`ImplementedCommandMap` 和真实任务不可用状态保持符合当前阶段范围。

内部候选只接受应用数据库中的启用 `relay/self_managed/api_key/responses` profile，不接受外部 profile ID。调用者仅选择 profile/revision、模型和强度；endpoint/origin/凭据修订来自应用仓储。凭据操作锁覆盖解析和候选完整生命周期，防止替换/删除侵入正在使用的版本；阶段 1.8 还须实现队列/等待状态、能力组合校验及提交时重验。当前串行执行配置探测，不宣称任务并发与切换恢复已完成。

## 配置和路由拒绝策略

1. 后端选择绝对 `.exe` 路径，检查目录/文件身份，并保持 Windows 只读共享句柄，阻止其在选择、启动与退出之间被改写或替换。启动只允许固定 `app-server --listen stdio://`；不接收任意 shell、参数、环境或可执行路径的 IPC。
2. 每个候选创建当前用户专用 DACL 的独立目录，分别隔离 `CODEX_HOME`、USERPROFILE/HOME、APPDATA/LOCALAPPDATA、PROGRAMDATA、TEMP/TMP。环境清空后只加入这些路径、必要系统目录与临时入口凭据；不继承 API Key/provider/代理、外部 CODEX_HOME、cc-switch、预加载或调试变量，不继承用户 PATH。
3. 配置完全由应用所选参数生成，不导入 MCP、插件、通知钩子、外部认证或信任设置。探测目录有独立项目根标记；拒绝项目根以内的 `.codex` 覆盖。每次发送/接收前重查项目层、专属配置哈希与异常 `auth.json`，配置漂移返回 `PA_PROVIDER_SHARED_ROUTE_UNSAFE`。
4. `config/read(includeLayers=true)` 核对所选 provider/model/effort、私有入口、Responses、专属 env_key、`requires_openai_auth=false`。仅接受自己的 user 文件和空 system/session 层；未知/非空外部层拒绝。`thread/start` 再核对 provider/model/effort/cwd 回显，之后才允许产生连接测试请求。空 system 层是 CLI 的“未配置”来源标记，不是导入外部连接。
5. Codex 子进程只获得进程专属 loopback 入口及其随机临时凭据，真实站点 Key 只在 Rust 转发层内存中使用。转发层固定目标 HTTPS 地址、规范化 origin 和启动时解析的公开地址集合，使用正常证书/主机名验证，禁用系统/继承代理，不转发任意 CLI 头，不跟随任何 3xx。loopback、私网、链路本地及保守特殊地址范围的生产目标拒绝，避免接入共享本地可变代理。
6. loopback 入口只接受精确 authority/path、Bearer、Content-Length 的 Responses POST；拒绝重复长度、分块请求、错误模型/强度和超限。隔离错误会封闭本次入口。配置探测向上游清空 tools、固定 tool_choice=none；SSE 只接收文本/推理事件，不允许工具调用、模型漂移或响应回显 secret。远端错误 body/header 不进入 CLI、IPC、日志或报告。
7. 指纹涵盖适配器版本、exe SHA-256、profile/config/credential 修订、endpoint/origin/model/effort，不包含 Key 或临时入口凭据。请求证据说明“向所选 HTTPS endpoint 发出了指定模型/强度”，不证明服务商内部的模型实现。远端服务商内部路由、受信任 OS/证书库遭篡改不在本机隔离证明范围内。

DNS 前置核验最多等待 5 秒，普通 RPC 上限 30 秒，内部进程完整候选上限 90 秒；单 stdio 帧及 stdout/stderr 各自累计上限均为 1 MiB，待处理消息最多 64，缓存终态最多 8。原始流只在内存中处理，不落产品日志。候选超时/取消在已知 thread/turn 时先尝试 interrupt，再停止转发、关闭 stdio；未退出时在 2 秒后只终止本应用 Job。记录 PID、退出码、是否强制、资源是否已清理，不把强制关闭记为真实任务取消成功。成功候选必须正常退出；清理失败封闭新候选，需启动恢复重新核验。

不创建站点认证文件，也不复制外部 `auth.json`；真实 Key 不出现在 config、临时入口环境、SQLite、备份或报告。专属配置仅记录临时入口地址和 env_key 名称。退出、失败、取消清理本次目录；进程异常结束依靠 Job 的 kill-on-close 和下次启动持锁检查回收，未知或重解析目录不自动接管/递归清理。

## 可重复验收

```powershell
npm run verify:isolation

# 仅调试配置层实际路由，不替代上面的完整入口
npm run verify:isolation -- --routes-only
```

`--routes-only` 的临时报告位于 `artifacts/phase-1-7-routes-only/report.json`，不会覆盖 `validation=full` 的正式验收报告。

依赖固定 Node/npm/Rust 基线、Windows 桌面依赖、实际 Codex CLI 和 Git 所带 OpenSSL。仅验证脚本用 `CODEX_BIN` 选择实际 CLI；正常产品不从该变量导入连接。测试 OpenSSL 默认位于 `%ProgramFiles%\Git\usr\bin\openssl.exe`，可用 `PA_ISOLATION_OPENSSL_BIN` 指定绝对路径。两站点使用一次性自签证书；证书只注入 `cfg(test)` 上游客户端，不安装到用户证书库，不影响生产 TLS 信任。生产构建无法通过环境变量开启 loopback/自签 CA 放行。

验收密钥均为随机 `synthetic-isolation-*`；每个实际路由 worker 先通过正式 SQLite/Windows 凭据服务写入、错误 origin 拒绝、读回与删除，随后使用已解析的合成值。两个 HTTPS 站点分别核对 Authorization、path、model、reasoning.effort；报告仅记录匹配布尔、参数和进程白名单，不保存原始请求、输出、Key 或证书私钥。外部 config/auth/cc-switch 版本均为隔离夹具，不访问用户实际文件或 cc-switch 实例。

| 用例 | 断言 |
| --- | --- |
| A、B、恢复 A | 实际请求仅到目标站点，合成 Key/模型/强度匹配；专属 home 清理，正常退出 |
| 外部缺失/升级、运行中手动改 config/auth | 继承变量被清除；夹具改动前后的指纹由脚本对比；应用不改写夹具，当前请求不漂移 |
| 项目配置、共享本地代理、专属配置漂移 | 固定 `PA_PROVIDER_SHARED_ROUTE_UNSAFE`，无上游请求；项目/共享反例在启动子进程前拒绝 |
| A 重定向到 B | A 收到自己的 Key；B 零请求，不转发 A Key；候选失败 |
| 不信任 TLS 证书 | TLS 校验失败，站点没有收到 HTTP Authorization |
| 401、错误/成功正文回显秘密 | 固定失败；秘密不进入子进程/证据/日志，原配置保持不动 |
| hang、取消 | 固定超时/取消，interrupt/进程退出、转发和自有 home 清理，不影响无关存活进程 |
| 伪造入口认证/authority/path/model/effort/长度 | Rust 入口反例在上游网络前拒绝 |
| 非法帧、累计流超限、早期后代、exe 替换、junction、遗留 home | 有界失败、Job 归属、文件身份锁、自有清理和外部目标保留 |

综合入口还执行前端构建、Rust fmt/全量 lib 测试、IPC/Codex 离线测试、release check、嵌入资源构建及 dev/built 真实桌面主框架和凭据回归。报告均为忽略的本地生成物，不提交。

本轮修正了一个既有桌面验收脚本断言：1.5 已将错误展示迁移到 `.error-notice`，旧脚本仍查询 `.overlay-error`，导致发送不可用提示处超时。现改为统一组件及固定 `PA_TASKS_NOT_IMPLEMENTED` 错误码；保留展开聚焦、组字 Enter 和草稿权限的原断言。此前失败结果不记为成功，最终入口重跑生成报告。

当前验证基线：Node 24.18.0、npm 11.16.0、Rust 1.99.0 / Windows MSVC，实际 CLI `0.163.0-alpha.10.2`。本版配置不再接受旧的 `untrusted` 字面值；应用使用 `on-request`，未知/已知审批均不批准，不改变产品默认 ask 约定。早期 Node 实验仍为历史脚本，其旧配置不能替代本次 Rust 验收。

最终综合结果：

| 检查 | 实际结果 |
| --- | --- |
| 前端构建、Rust fmt、release check、嵌入资源构建 | 通过 |
| Rust lib | 75 passed、0 failed、3 ignored；ignored 为凭据独立进程 worker、Job 后代 worker、实际 CLI 路由 worker，均由对应父测试/综合脚本显式启动 |
| IPC / Codex 离线 | 19 / 15 passed |
| 实际 CLI / 双合成 HTTPS | 14 场景通过；包括秘密错误/成功回显反例，TLS 失败零 HTTP 请求、跨站重定向目标零请求 |
| 自有候选退出与清理 | 所有已启动候选退出码 0、无强制关闭、home 已删除；项目/共享反例未启动子进程；自有 home 在进程退出后、删除前扫描无站点 Key |
| dev / built 桌面 | 两种模式各执行 shell、credentials 共 4 场景，均通过；凭据清理布尔为 true |
| 其他 | 既有迁移无改动，schema 仍 v3；本轮文档本地链接检查通过；保留原有两项 dead-code warning，无新增 warning |

当前结果由本轮重新生成的报告证明；调试中曾出现旧审批字面值不兼容、空 system 层识别、旧 UI 选择器及活跃 SQLite 锁导致的验收失败，均修正后重跑，未将这些失败的中间报告当成通过。未使用真实 Key 或收费模型。

## 后续功能门

本次只关闭 R-014 / AT-AI-005/008 的配置层子门，以及 R-016 的当前转发/不落认证文件/清理子项。1.8 继续实现 profile/能力/默认值、候选操作和完整恢复；1.9 才开放相应页面。真实任务启动、并发、多 profile、既有任务不可漂移、重启恢复及持久审批仍须阶段 3 网关实测。未调用真实收费模型/Key，官方源和官方切换仅预留、不纳入判定；1.1/1.4 的人工硬件/托盘验收仍留 1.10，M1/阶段 1 未整体完成。
