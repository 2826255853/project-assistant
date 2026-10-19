import type { ErrorCategory } from "./contracts.ts";

type ErrorDefinition = readonly [category: ErrorCategory, message: string, suggestion: string, retryable?: boolean];
// Only this version-controlled catalog supplies public error text. Wire text and
// native/plugin exceptions must never become UI content, including unknown codes.
const catalog = {
  PA_INTERNAL_UNEXPECTED: ["internal", "本地操作发生异常。", "请重启应用；持续失败时提供错误码和关联 ID 排查。"],
  PA_INTERNAL_STARTUP_FAILED: ["internal", "应用启动失败。", "请重启应用，并提供错误码和关联 ID 排查。"],
  PA_DESKTOP_INITIALIZATION_FAILED: ["internal", "桌面服务暂时不可用。", "请重试初始化；持续失败时关闭并重新启动应用。", true],
  PA_DESKTOP_RENDER_FAILED: ["internal", "页面加载失败。", "请重新加载页面；持续失败时关闭并重新启动应用。"],
  PA_IPC_INVALID_REQUEST: ["validation", "请求格式无效。", "请刷新应用后重试；持续失败时更新应用。"],
  PA_IPC_VERSION_UNSUPPORTED: ["compatibility", "前后端接口版本不兼容。", "请使用同一版本的前端与桌面程序。"],
  PA_IPC_RESPONSE_INVALID: ["compatibility", "服务返回格式不兼容。", "请更新应用。"],
  PA_IPC_TRANSPORT_FAILED: ["environment", "本地服务调用失败。", "请重试；持续失败时重启应用。", true],
  PA_IPC_PERMISSION_DENIED: ["permission", "当前窗口无权执行此操作。", "请使用应用主窗口中的对应入口。"],
  PA_IPC_TIMEOUT: ["timeout", "本地服务响应超时，操作结果尚未确认。", "请查询状态后再决定是否重试；变更请求不会自动重发。", true],
  PA_IPC_CANCELLED: ["cancelled", "已停止等待，服务端操作可能仍在执行。", "请查询状态确认操作结果。"],
  PA_IPC_STATE_STALE: ["compatibility", "状态尚未同步。", "请重试查询。", true],
  PA_IPC_STATE_UNAVAILABLE: ["environment", "无法同步本地服务状态。", "请重试同步；持续失败时重启应用。", true],
  PA_VALIDATION_EMPTY_MESSAGE: ["validation", "测试消息不能为空。", "请输入内容后重试。"],
  PA_VALIDATION_INPUT_TOO_LARGE: ["validation", "输入超出长度限制。", "请缩短输入后重试。"],
  PA_SETTINGS_INVALID_VALUE: ["validation", "设置值无效。", "请检查允许值、摘要期限和编辑器绝对路径后重试。"],
  PA_CONFLICT_REVISION: ["conflict", "设置已被其他入口修改。", "请先重新读取设置，再确认并提交；不会自动覆盖。", true],
  PA_SETTINGS_CORRUPT: ["storage", "本机设置数据损坏。", "原值已保留，请退出应用并按存储恢复说明检查数据库或旧设置文件。"],
  PA_SETTINGS_STORAGE_FAILED: ["storage", "设置读取或保存失败。", "请检查本机存储、权限和磁盘空间后重试。"],
  PA_SETTINGS_RECOVERY_FAILED: ["internal", "常驻设置恢复失败。", "请明确退出后重启应用，核对设置与常驻入口状态。"],
  PA_SETTINGS_AI_UNAVAILABLE: ["compatibility", "AI 默认配置服务尚未启用。", "请等待配置服务就绪；当前保持未配置。"],
  PA_SETTINGS_NOT_IMPLEMENTED: ["compatibility", "设置服务尚未启用。", "请等待设置服务就绪。"],
  PA_OVERLAY_CONTROL_FAILED: ["internal", "常驻入口操作失败。", "请从托盘重试；持续失败时重新启动应用。", true],
  PA_OVERLAY_PHASE0_DISABLED: ["compatibility", "验证模式未启用常驻入口。", "请在正常模式下使用常驻入口。"],
  PA_SHORTCUT_INVALID: ["validation", "快捷键格式无效。", "请更换快捷键组合后重试。"],
  PA_SHORTCUT_CONFLICT: ["conflict", "快捷键已被系统或其他应用占用。", "请更换快捷键组合；悬浮球仍可使用。"],
  PA_CREDENTIALS_UNAVAILABLE: ["environment", "系统凭据库当前不可用。", "请检查 Windows 凭据服务和当前用户权限后重试；不会降级保存到文件或数据库。", true],
  PA_CREDENTIAL_INVALID: ["validation", "凭据或服务源绑定无效。", "请检查 profile、HTTPS 服务源和 Key 格式后重试。"],
  PA_CREDENTIAL_PROFILE_NOT_FOUND: ["validation", "服务源 profile 不存在。", "请先读取可用 profile，再提交凭据。"],
  PA_CREDENTIAL_REVISION_CONFLICT: ["conflict", "服务源已被其他操作修改。", "请先重新读取 profile 状态，再提交凭据变更。", true],
  PA_CREDENTIAL_BUSY: ["conflict", "该服务源仍有任务占用。", "等待相关任务结束后重试；凭据未修改。", true],
  PA_CREDENTIALS_STORAGE_FAILED: ["storage", "凭据状态保存失败，原可用状态已保留或已标记不可用。", "请检查应用数据目录权限和 Windows 凭据服务后重新读取状态。"],
  PA_LOG_CLEANUP_FAILED: ["storage", "日志清理失败。", "请刷新日志状态，检查磁盘空间和应用日志目录权限后重试。"],
  PA_AUDIT_WRITE_FAILED: ["storage", "安全审计写入失败，本次变更未执行。", "请检查磁盘空间和应用数据目录权限，恢复后重新读取状态再提交。"],
  PA_LOGGING_NOT_IMPLEMENTED: ["compatibility", "日志服务尚未启用。", "请等待日志服务就绪。"],
  PA_CREDENTIALS_NOT_IMPLEMENTED: ["compatibility", "凭据服务尚未启用。", "请等待凭据服务就绪。"],
  PA_PROVIDER_NOT_IMPLEMENTED: ["compatibility", "服务源管理尚未启用。", "请等待凭据和连接隔离功能就绪。"],
  PA_CODEX_NOT_IMPLEMENTED: ["compatibility", "任务进程服务尚未启用。", "请等待凭据和连接隔离功能就绪。"],
  PA_PROJECTS_NOT_IMPLEMENTED: ["compatibility", "项目管理尚未启用。", "请等待项目管理功能就绪。"],
  PA_TASKS_NOT_IMPLEMENTED: ["compatibility", "任务服务尚未启用，草稿保留在本次运行中。", "请打开主窗口查看服务状态。"],
  PA_STORAGE_NOT_INITIALIZED: ["storage", "本地存储尚未就绪。", "请重启应用；持续失败时检查存储状态。"],
  PA_STORAGE_BUSY: ["storage", "本地数据库正忙。", "请关闭重复写入操作后重试。", true],
  PA_STORAGE_DIRECTORY_FAILED: ["storage", "应用数据目录不可用。", "请检查目录权限和磁盘空间。"],
  PA_STORAGE_OPEN_FAILED: ["storage", "本地数据库无法打开。", "请检查应用数据目录权限和磁盘空间，保留原数据库。"],
  PA_STORAGE_SQL_FAILED: ["storage", "本地数据操作失败。", "请检查应用数据目录权限和磁盘空间，并提供关联 ID 排查。"],
  PA_STORAGE_CLOCK_FAILED: ["storage", "系统时间无法用于本地数据操作。", "请校正系统时间后重启应用。"],
  PA_STORAGE_BACKUP_FAILED: ["storage", "数据库升级备份失败，原数据保留。", "请检查空间和权限，按存储恢复说明处理；请保留原数据库。"],
  PA_STORAGE_MIGRATION_FAILED: ["storage", "数据库升级失败，原数据保留。", "请检查空间和权限，按存储恢复说明处理；请保留原数据库。"],
  PA_STORAGE_NEWER_SCHEMA: ["storage", "本地数据来自更高版本。", "请使用匹配的应用版本，保留原数据库。"],
  PA_STORAGE_CORRUPT: ["storage", "本地数据完整性检查失败。", "请保留原数据库，按存储恢复说明从已验证备份人工恢复。"],
  PA_STORAGE_UNRECOGNIZED_SCHEMA: ["storage", "无法识别本地数据结构。", "请使用匹配版本或按存储恢复说明人工恢复，保留原数据库。"],
  PA_STORAGE_MIGRATION_HISTORY: ["storage", "本地数据迁移记录不符合要求。", "请保留原数据库，按存储恢复说明检查版本及备份。"],
  PA_STORAGE_MIGRATION_SEQUENCE: ["storage", "本地数据迁移顺序不符合要求。", "请保留原数据库，按存储恢复说明检查版本及备份。"],
  PA_STORAGE_FOREIGN_KEY_CHECK: ["storage", "本地数据关联检查失败。", "请保留原数据库，按存储恢复说明从已验证备份人工恢复。"],
} as const satisfies Record<string, ErrorDefinition>;

export type PublicErrorCode = keyof typeof catalog;
export function isCorrelationId(value: unknown): value is string {
  return typeof value === "string" && /^ipc-[a-z0-9-]{1,96}$/.test(value);
}
export class PublicError extends Error {
  readonly code: PublicErrorCode;
  readonly category: ErrorCategory;
  readonly suggestion: string;
  readonly retryable: boolean;
  readonly correlationId: string | null;
  readonly userMessage: string;
  constructor(code: PublicErrorCode, correlationId?: unknown) {
    const [category, message, suggestion, retryable = false]: ErrorDefinition = catalog[code];
    const id = isCorrelationId(correlationId) ? correlationId : null;
    super(`${code}：${message} ${suggestion}${id ? `（关联 ID：${id}）` : ""}`);
    this.name = "PublicError";
    this.code = code;
    this.category = category;
    this.userMessage = message;
    this.suggestion = suggestion;
    this.retryable = retryable;
    this.correlationId = id;
  }
}
export function errorForCode(code: unknown, correlationId?: unknown): PublicError {
  const known = typeof code === "string" && Object.prototype.hasOwnProperty.call(catalog, code);
  return new PublicError(known ? code as PublicErrorCode : "PA_INTERNAL_UNEXPECTED", correlationId);
}
export function toPublicError(cause: unknown): PublicError {
  // Do not parse Error.message or stringify unknown objects, even if they appear
  // to start with a known code. Only our own typed errors preserve their code/ID.
  return cause instanceof PublicError ? errorForCode(cause.code, cause.correlationId) : errorForCode("PA_INTERNAL_UNEXPECTED");
}
