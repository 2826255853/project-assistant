# 步骤 0.3：cc-switch 接口与凭据调查

> 2026-10-11 当前范围：用户已确认“用户手动管理 cc-switch、应用自管 profile”，见 [ADR-0011](self-managed-profile-adr.md)。步骤 0.3、0.7 与阶段 0 按替代范围通过，允许启动阶段一；原自动集成/共享代理未通过的事实及以下当时记录保留为历史，不作为当前自动需求或验收安排。生产自管凭据/路由/恢复仍待实现实测；旧切回 cc-switch 的约定已撤销。

日期：2026-10-10（Asia/Shanghai）。历史原范围结论：**自动集成阻塞**；已完成可安全执行的安装/存储调查与源码选型，未操作真实 provider、Key 或共享代理。该结论不表示当前阶段门仍阻塞；自动集成已按 ADR-0011 退出范围。

配置修复后追加了只读来源一致性诊断和独立非官方测试，见 [补测记录](cc-switch-retest.md)。首轮“不查询表”的边界仅描述 `inspect:cc-switch` 调查脚本；补测脚本在内存中一次性核对当前 provider 的 endpoint/协议/凭据，数据库以只读模式打开，不复制数据库、不持久化凭据，也不作为产品运行时接口。自动切换与共享代理门槛继续独立保留。

2026-10-11 追加：[社区 CLI 5.11.0 隔离实测](cc-switch-cli-verification.md)补齐添加/列表/current/A→B→A 切换、合成 Key 绑定及备份/导出证据；桌面/CLI schema 均为 20。编辑/导入/恢复在无终端调用中超时，Windows 缺少 `start`/daemon；桌面双向契约、跨实现完整兼容和共享代理影响仍阻塞。下表“社区 CLI 未安装/未测”保留为首轮历史状态，以此追加记录为最新结论。

## 版本及证据

本机桌面 cc-switch.exe 的 ProductVersion/FileVersion 为 4.0.6，进程正在运行，Windows `ccswitch://` 协议已注册。`npm run inspect:cc-switch` 可重复只读调查，输出 `artifacts/cc-switch/report.json`。默认按 LOCALAPPDATA/Programs/CC Switch 发现安装，其他位置可设绝对路径 `CC_SWITCH_BIN`。调查成功退出 0 表示报告生成成功，报告 status=blocked 表示集成门槛未通过。

本轮对照上游 `farion1231/cc-switch` 的 v4.0.6 标签，tag object 为 `dfddb3b5106638931d4e13c14a5d78fd98864c91`，解析出的 commit 为 `c261126630b3416e67b407da018cc614bbbe84cd`。本机文件版本相同不证明二进制逐字匹配上游构建。调查下载文件位于 artifacts/codex-research，不复制用户数据库或凭据。

可复核的固定版本来源：

- `https://github.com/farion1231/cc-switch/blob/c261126630b3416e67b407da018cc614bbbe84cd/README.md`
- `https://github.com/farion1231/cc-switch/blob/c261126630b3416e67b407da018cc614bbbe84cd/src-tauri/src/commands/provider.rs`
- `https://github.com/farion1231/cc-switch/blob/c261126630b3416e67b407da018cc614bbbe84cd/docs/user-manual/zh/5-faq/5.3-deeplink.md`
- `https://github.com/farion1231/cc-switch/blob/c261126630b3416e67b407da018cc614bbbe84cd/src-tauri/src/database/dao/providers.rs`
- `https://github.com/farion1231/cc-switch/blob/c261126630b3416e67b407da018cc614bbbe84cd/src-tauri/src/codex_config.rs`

## 接口区分

| 入口 | 调查依据 | 本轮状态 / 判断 |
| --- | --- | --- |
| 桌面版 UI | 上游 README；本机程序版本和运行状态 | 可由用户在 UI 管理 provider、切换、编辑和认证；本轮未驱动 UI，不记为实际操作通过 |
| 内部 Tauri invoke | commands/provider.rs 的 get_providers、switch_provider、add_provider、update_provider 等，lib.rs 的 invoke_handler | 属于该应用自身 WebView/后端接口，未发现可供本应用调用的受支持外部地址/认证/兼容契约；不将这些函数名作为外部 API |
| 桌面命令行 | 主入口运行 Tauri；README 区分桌面和社区 CLI | 未发现官方桌面版本的双向 provider 列表/切换命令，不假设 cc-switch.exe list/switch 存在 |
| 社区 CC Switch CLI | 上游 README 引用 SaladDay/cc-switch-cli，独立发布且数据库版本可能落后 | 本机未检测到 cc-switch 命令；未安装/实测，不将其接口当成桌面版 4.0.6 的接口 |
| ccswitch:// 导入 | 固定版深链文档和 parser.rs；本机协议注册 | 文档说明有 UI 确认；provider 导入不会自动切换，无法读取列表/当前状态/回滚结果；带 Key 的 URL 有泄露风险。本轮未打开带凭据链接 |
| SQLite 内部表 | 本机数据库存在、固定版 DAO | 禁止作为集成接口；本轮只读 16 字节文件头及 ACL，没有查询表、复制数据库或改写配置 |

因此阶段 0 未建立满足“读取 → 测试 → 切换 → 核验 → 失败恢复”的 cc-switch 桌面外部调用链。没有稳定接口时，应联系上游约定桥接契约，或明确决定降级为用户在 cc-switch UI 手动管理、应用自行维护凭据引用的方案；降级会改变 FR-AI-006 自动集成验收，不能静默当作通过。

## 存储保护

本机 `.cc-switch/cc-switch.db` 的前 16 字节为普通 SQLite header，不是文件级加密证据。ACL 调查没有发现 Everyone、Authenticated Users、BUILTIN Users 的直接允许 ReadData 项；该检查不等同完整有效权限审计，也不代表数据库内容已加密。

固定版 DAO 使用 serde_json 序列化 provider.settings_config 写入 SQLite 字段，再反序列化读取，Codex 配置逻辑含 OPENAI_API_KEY/auth.json 处理。因此不能假定中转 provider 的 Key 使用系统凭据库或已加密。本轮没有读取现有 Key，也未确认每个已有 profile 的具体凭据形态；中转 Key 的存储仍需核验，官方登录/keyring 模式仅作预留，无需实测且结果不纳入通过判定。备份、WebDAV 同步和导出同样需要后续脱敏评审。

本应用普通数据库只能保存 profile 元数据和 Key 引用；自管 Key 需使用系统凭据库。复用 cc-switch 凭据前必须明确可用接口、读取授权与保护措施，不能直接把内部 settings_config 导入普通数据库。

## 切换与外部任务边界

README 描述直接配置切换需要工具重启，路由模式可能影响后续请求，并提示共享 Codex daemon 重启会中断运行任务。这些是上游说明，**不是本机全局路由实测**。

本轮只在独立 CODEX_HOME 和自有 stdio 进程上验证了合成 A/B 站点切换与候选失败恢复；结果见 codex-verification.md。没有改变 `.codex/config.toml`、cc-switch 实时 provider、代理模式或共享 daemon，不能因此推断外部任务安全隔离。

历史原范围的解除条件是：提供受支持的桌面外部接口或正式接受降级路线，然后在隔离实例验证非官方 provider 读取/编辑/切换/认证、受控非官方站点切换、备份回滚、代理影响及空闲等待/归属策略。当前已正式采用 ADR-0011 的降级路线；应用不自动接入 cc-switch，生产自管 profile/Key/路由按阶段一功能门验证。官方源相关功能仅作预留，无需实际测试且测试结果不纳入通过判定。当前原配置保留，后续阶段不得默认终止外部任务。
