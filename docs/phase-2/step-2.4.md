# 步骤 2.4（S2-04）：项目列表、稳定分页与模糊搜索

状态：**通过（列表/搜索服务范围，2026-10-28，Asia/Shanghai）**。实际实施及自动验收：本次 Codex 会话，依据用户“根据文档要求，完成步骤 2.4”的授权；开始及完成日期均为 2026-10-28。`npm run verify:projects` 完整通过，`artifacts/phase-2-4/report.json` 的 `passed=true`。范围对应计划第 22.2/22.4/22.5 节及 AT-S2-004 的服务部分；项目页面和搜索框控件在 2.7 整合，阶段 2/M2 未整体通过。

## 实现与接口

新增 `projects_list`，仅本地 main 可调用。参数保持 `{query?:string,cursor?:string,limit?:number}`，返回既有 `ProjectListPage`。Rust 注册、Tauri manifest/main capability、日志白名单、TypeScript `ImplementedCommandMap`、集中 client 及严格返回校验同步接通。overlay、未知窗口和远端 main 无权限。完整项目服务状态仍未就绪，`app_get_status` 的项目服务继续报告未实现，避免把局部查询能力当作系统打开/活动项目完整可用。

| 入口 | 职责 |
| --- | --- |
| `src-tauri/src/storage/projects/search.rs` | SQLite 元数据流式读取、排序、匹配、游标与有界页面 |
| `src-tauri/src/projects.rs`、`lib.rs` | 严格请求、真实窗口授权、阻塞线程池查询与固定错误 |
| `src/ipc/projectSearch.ts`、`client.ts` | 可复用 `ProjectSearchController` / `createProjectSearch`；查询代次、下一页、组合输入与卸载处理 |
| `scripts/ipc/project-search.test.mjs` | 可控响应乱序、迟到错误、分页竞态、IME 和销毁验证 |
| `storage/projects/search_tests.rs` | 匹配样例、稳定排序/分页、游标变更、输入与读写边界 |
| `scripts/ipc/project-search-desktop.mjs`、`verify-projects.mjs` | built/dev 真实 IPC、权限、故障回归和 1,000 条记录性能测量 |

无新增迁移，schema 仍为 v5；已有迁移 SQL/校验和不变。搜索不读取文件内容，不打开/遍历项目目录，不启动 Git、Codex 或网络请求，不写使用记录、搜索历史或授权审计。列表返回数据库中最近一次可用性观测；启动恢复、详情以及后续实际使用负责重新核验身份。可用性不是使用授权，查询不会自动重新授予权限。

## 排序、匹配与分页规则

- 仅包括 `removed_at IS NULL` 且未撤权的已登记行；缺失、不可访问、需重新授权记录仍可检索、展示。名称、显示名、绝对路径、最近使用时间、可用性沿用 `ProjectView`，不返回文件系统身份。
- 在同一 SQLite 读事务内读取全局 `project_sort` 和项目数据；缺省为 `recent`，非法存储值固定失败。`recent` 按使用时间降序，未使用的 null 最后；同值按归一化显示名（无显示名用目录名）升序，再按 UUID 升序。`name` 只按同一名称键及 UUID。采用 Unicode 小写后的确定性字符串次序，不使用拼音或操作系统区域排序。
- 仅对名称、显示名称、完整显示路径匹配；三个字段各自匹配，不跨字段拼接。双方 Unicode 小写、`/` 统一为 `\`，依次优先完整字段相等、连续子串、Unicode 字符有序子序列。先按匹配等级，再按 `project_sort` 的稳定次序。不做拼音、正则、SQL 通配或分词；空字符串/未提供 query 返回普通列表，空格为字面字符。无匹配返回 `items:[] / nextCursor:null`。
- 每页缺省 50，允许 1–100；Rust 流式扫描已登记元数据，只保留最多 `limit+1` 个候选，返回最多 limit 行。扫描耗时随记录数增长，内存中的候选数量独立于总记录数；无文件索引或缓存。
- 服务端生成的游标绑定归一化查询、排序和可见 `ProjectView` 数据指纹及最后一行 UUID。游标不包含路径或查询文本；客户端只能原样回传。重复读取同一数据得到同一顺序；查询、排序、名称、使用时间、可用性或可见行变化后，续页返回 `PA_PROJECT_CONFLICT_REVISION`，调用方应从第一页重新读取。格式损坏为 `PA_IPC_INVALID_REQUEST`。游标不是授权令牌，不承诺跨变更保留旧快照。
- IPC v1 完整请求最多 8 KiB；query 最多 256 UTF-8 字节、cursor 最多 512；拒绝未知字段、显式 null、错误类型、控制字符、空 cursor、非整数/越界 limit。长度错误用 `PA_VALIDATION_INPUT_TOO_LARGE`，其余固定输入错误；SQLite 错误用既有 `PA_SETTINGS_STORAGE_FAILED`，不传原始异常或 SQL。

`ProjectSearchController` 每次输入立即增加查询代次并停止等待旧请求；即使 native 工作仍完成，旧成功/错误/下一页均不能更新新查询。新输入清空旧页面，明确发布 loading/空结果/错误状态；同一查询下一页串行。调用 `search(text,true)` 处理 IME 组合输入，只作本地失效，组合结束后 `search(text)` 才发送；`dispose()` 阻止迟到响应。项目页面已在 2.7 复用该控制器，页面切换排序或收到失效通知时重新 `search(currentQuery)`；游标冲突保留固定错误供页面显式处理，不自动重放变更。2.7 built/dev 桌面已验证搜索框、组合态提示和控件响应；真实设备中文输入仍留 2.9。

匹配样例（合成元数据）：

| 查询 | 结果 / 次序 |
| --- | --- |
| `项目助手` | `项目助手`（精确）→ `工具项目助手扩展`（子串）→ `项目桌面助手`（字符顺序） |
| `中文工作台` | 显示名为该值的项目 |
| `sYnThEtIc-0099` / `synth0099` | `Synthetic-0099`，分别为忽略大小写的子串/精确字段与子序列匹配 |
| `C:/SYNTHETIC-PRIVATE-PATH/Synthetic-0099` | 与反斜杠路径匹配 |
| `助手项目` / `不存在` / `%_*` | 本夹具无匹配；不倒序、不展开通配符 |

## 可重复验收与性能

```powershell
npm run verify:projects
```

完整报告更新为 `artifacts/phase-2-4/report.json`，关联日志和 built/dev 报告均在忽略的 `artifacts/`；历史 2.3 报告路径只代表当时记录。该入口构建前端、Rust fmt/全部 lib/发布模式 check、IPC/Codex 离线、存储跨进程，再运行两种资源模式的真实桌面查询和权限验证；不调用模型或真实 Key。

性能夹具为独立测试库的 1,000 条固定合成元数据，状态为需重新授权，不能被用于系统打开；无 1,000 个真实目录或文件扫描。两个模式各预热 5 次、采样 100 次，七类查询轮换，页大小 50，采用 nearest-rank P95（升序第 95 个）。计时从真实 WebView 的 `performance.now()` 开始，覆盖 invoke、Rust/SQLite、响应序列化/传输、JavaScript 解码及基础结果检查；不计 CDP 往返或页面渲染。报告记录环境、浏览器 UA 和各分位值，P95 超过 300 ms 则验收失败。

2026-10-28 最终完整验收结果：

| 检查 | 结果 |
| --- | --- |
| 前端 build、Rust fmt、release check | 通过 |
| 全部 Rust lib | 142 passed、5 ignored；新增搜索 6 项通过 |
| IPC / 控制器 | 33 passed；新增 4 项竞态/组合输入/分页/销毁用例 |
| Codex 离线 | 15 passed，无模型请求 |
| 存储回归 | 52 passed、1 ignored worker；schema v5 的独立进程读写/重启与清理通过 |
| built/dev 桌面 | 两种模式的匹配/分页/排序设置/游标冲突、严格输入、四项目命令窗口权限和日志哨兵通过 |
| 项目仓储回归 | 2.3 跨进程、撤权、身份复核、真实 SQLite/审计/提交故障、源文件保护均继续通过 |

性能环境：Windows 11 Home China x64，10.0.26300；Intel Core Ultra 9 275HX、24 逻辑处理器、Node 报告可用物理内存总量四舍五入 15 GiB；Node.js 24.18.0、npm 11.16.0、Rust 1.99.0 / MSVC。WebView2 报告 UA `Edg/155.0.0.0`（UA 不代表完整 Runtime 构建号）。built 为嵌入前端资源的调试程序，dev 为 Vite 开发资源；未将该数据当作优化发布二进制性能。

| 真实桌面资源模式 | P50 | P95 | 最大值 | 目标 |
| --- | --- | --- | --- | --- |
| built | 19.1 ms | **21.1 ms** | 32.5 ms | P95 ≤ 300 ms，通过 |
| dev | 20.6 ms | **24.3 ms** | 31.7 ms | P95 ≤ 300 ms，通过 |

表中为最终完整入口的测量值；早期调试/首轮通过的数值不替代本轮结果。完整检查保留既有两条 Rust warning：`logging::Failure::Unsupported`、`settings::change_resident` 未使用。5 个 ignored 中的凭据、Job、项目 worker 由现有父测试启动；两个 CLI/双 HTTPS 专项未在本步调用。

## 失败与遗留范围

查询/游标输入错误、变更后的续页、真实 SQLite 读取失败、审计故障回滚及撤权隐藏均通过适用自动用例；日志检查覆盖进程输出及独立产品 JSONL 中的合成路径/搜索词哨兵。读取不增加使用计数或审计，授权状态不变，合成项目源文件保持不变。桌面夹具仅写专属临时库/日志，测试结束清理。

2.5 系统交付、2.6 Git、2.7 页面/首页/键盘与实际中文输入法、2.8 活动项目/设置边界、2.9 完整阶段回归及原生选择器人工点验继续按计划推进。阶段 1 托盘/物理多屏/DPI 人工项仍单列。未重新运行实际 CLI/双 HTTPS 专项，不扩大既有隔离结论；官方源仍仅预留。
