# 步骤 0.3 最小事件与能力契约（实验稿）

日期：2026-10-10；支持基线为 CLI 0.162.0-alpha.17.2 的本机 v2 导出/实测。实现见 scripts/codex/client.mjs、profile.mjs。正式 React ↔ Rust IPC 和持久实体留给 0.5、0.6；React 不直接连接此 Node 实验客户端或依赖 wire 字段。

## 传输和关联

启动自有进程，参数数组为 `app-server --listen stdio://`；cwd 使用已规范化绝对临时目录。每条 JSON 占一行。RPC 请求带客户端独立递增数字 id；服务端审批请求有自己的 id，原样应答，不能把它当作客户端请求的回复。先 initialize，再 initialized，再业务请求。

RPC 响应与通知可交错，turn/completed 甚至可先于 turn/start 的响应。客户端用有界内存事件序列暂存，按 threadId + turnId + sequence 匹配，不能依赖消息严格先后。保留未知通知兼容性，但未知服务端请求返回 -32601，不默认批准。请求/事件分别超时；进程结束立即使所有待处理请求失败。

以下样例是脱敏结构示意，`t1/r1/i1` 为替换 ID，cwd 仅作占位；实际事件证据由 report.json 的计数和断言提供，不保存原始消息。

```json
{"id":1,"method":"initialize","params":{"clientInfo":{"name":"project_assistant_phase0","version":"0.1.0"},"capabilities":{"experimentalApi":false}}}
{"method":"initialized","params":{}}
{"id":2,"method":"thread/start","params":{"cwd":"C:\\isolated-project","ephemeral":true,"model":"selected-model","sandbox":"read-only","approvalPolicy":"untrusted","config":{"model_reasoning_effort":"high"}}}
{"id":3,"method":"turn/start","params":{"threadId":"t1","model":"selected-model","effort":"high","input":[{"type":"text","text":"non-sensitive probe","text_elements":[]}]}}
{"method":"item/agentMessage/delta","params":{"threadId":"t1","turnId":"r1","itemId":"i1","delta":"example"}}
{"id":"approval1","method":"item/commandExecution/requestApproval","params":{"threadId":"t1","turnId":"r1","itemId":"i2","command":"Get-Location","cwd":"C:\\isolated-project"}}
{"id":"approval1","result":{"decision":"decline"}}
{"id":4,"method":"turn/interrupt","params":{"threadId":"t1","turnId":"r1"}}
{"method":"turn/completed","params":{"threadId":"t1","turn":{"id":"r1","status":"interrupted"}}}
```

## 最小应用事件

| 应用事件 | 本机方法 / 内容 | 行为 |
| --- | --- | --- |
| running | turn/started，turn.id | 进入运行中 |
| message_delta | item/agentMessage/delta | 业务服务内存处理文本，UI 增量显示；实验持久证据只记录字节数/计数 |
| approval_required | item/commandExecution/requestApproval；item/fileChange/requestApproval | 关联请求 id、thread、turn、item，展示待审批操作；文件审批尚未真实验证 |
| error | error，willRetry | 区分重试中的错误和真正终态；错误正文需服务层脱敏 |
| completed | turn/completed，status=completed | 结束成功 |
| failed | turn/completed，status=failed | 结束失败，不把普通 error 通知单独当终态 |
| cancelled | turn/completed，status=interrupted | 取消完成 |

取消请求后先进入 cancelling；收到 interrupted 才进入 cancelled。decline 仅拒绝该操作，本轮任务仍可 completed。批准必须限定一次请求；阶段 0 只白名单精确 Get-Location 和可信 PowerShell 执行路径，默认拒绝其余请求，不接受 acceptForSession、策略变更或 grantRoot。产品阶段应由用户审阅操作摘要后决定。

## 模型、强度和配置快照

1. 按所选 provider 获取 model/list，跟随 nextCursor；发现 cursor 循环视作协议错误。最小能力字段为 model、supportedReasoningEfforts[].reasoningEffort、defaultReasoningEffort。目录可能是缓存/配置声明，远端可用性必须另测。
2. 用精确 model 标识选择；每次变更模型或服务源重新校验 effort。未声明的组合拒绝，不静默回退；省略 effort 时使用模型目录默认值。
3. profile 至少包含 id、endpoint、协议 responses、凭据引用；凭据元数据包含 profileId、origin、reference。发送前 profile ID 和 endpoint origin 双重匹配。生产 endpoint 必须 HTTPS，禁止 URL 内凭据、查询令牌和片段；实验仅显式允许 127.0.0.1 HTTP。
4. 固定不可变快照：profileId、endpoint、credentialReference、model、effort。完整 Key 不进入快照、任务记录、错误、审计或普通数据库。凭据服务仅在启动归属进程时解析所选站点引用。
5. thread/start 明确 model、modelProvider、cwd、approvalPolicy、sandbox 和 config.model_reasoning_effort；turn/start 明确 model 和 effort。校验 thread/start 的 model、modelProvider、reasoningEffort 与 cwd 回显。它们证明应用配置状态，不能独立证明远端内部模型行为。
6. account/read 的 account=null 仅在 requiresOpenaiAuth=true 时形成官方认证缺失。当前中转 provider 使用自有 bearer，requiresOpenaiAuth=false 的 null 不能误判。
7. 测试候选使用独立自有进程/home；失败不提交。按 ADR-0011 使用应用专属配置和环境允许列表，拒绝未核验的环境/项目覆盖和共享可变代理。等待受影响自有任务空闲后才重建进程/会话；既有任务保留原快照，不终止外部任务。历史实验脚本导入外部连接不作为生产路径。

官方认证仍预留，不实测、不纳入通过判定。步骤 0.3 按 ADR-0011 的自管替代范围通过；本契约仍是实验稿，生产凭据库、环境/项目覆盖隔离与实际路由拒绝尚待实现验证，不承诺完整 AI 配置产品。
