// IPC v1. CommandMap includes future DTOs; ImplementedCommandMap is the callable surface.
export const API_VERSION = 1 as const;
export type ErrorCategory = "validation" | "permission" | "environment" | "authentication" | "compatibility" | "conflict" | "timeout" | "cancelled" | "external" | "storage" | "internal";
export interface AppError { category: ErrorCategory; code: string; message: string; suggestion: string; retryable: boolean }
export interface Request<P> { apiVersion: 1; params: P }
export type Response<T> =
  | { apiVersion: 1; correlationId: string; status: "ok"; data: T }
  | { apiVersion: 1; correlationId: string; status: "error"; error: AppError };
export interface ProbeReply { message: string; platform: string; architecture: string }
// Desktop startup mode. app_get_status returns the separate authoritative service snapshot.
export interface DesktopReady { mode: "normal" | "phase0" }
export interface OverlayView { enabled: boolean; expanded: boolean; shortcut: string; shortcutError: string | null; windowError: string | null; draft: string; trayReady: boolean }
export type SourceKind = "relay" | "official";
export type ConfigState = "unconfigured" | "checking" | "ready" | "switching" | "degraded" | "error";
export interface CredentialState { status: "missing" | "configured_unverified" | "verified" | "invalid" | "external_unavailable"; revision: number }
export interface ProviderProfile {
  id: string; name: string; sourceKind: SourceKind; adapterKind: "self_managed" | "cc_switch";
  endpoint: string; wireApi: "responses"; authKind: "api_key" | "external_login";
  defaultModel: string | null; defaultEffort: string | null; enabled: boolean; configRevision: number;
  credential: CredentialState; available: boolean; unavailableCode: string | null;
}
export interface ModelCapability {
  modelId: string; efforts: string[]; supportsDefaultEffort: boolean; contextWindow: number | null;
  source: "cli_catalog" | "profile_declaration" | "connection_probe"; sourceVersion: string;
  profileRevision: number; observedAt: number; expiresAt: number;
}
export interface AiSelection { profileId: string; expectedRevision: number; modelId: string; effort: string | null }
export interface PublicSnapshot {
  id: string; profileId: string; profileName: string; sourceKind: SourceKind; endpoint: string;
  modelId: string; effort: string | null; configRevision: number; credentialRevision: number;
  effectiveAt: number; routingScope: "owned_process";
}
export interface ConnectionTest {
  testedRevision: number; testedAt: number; authenticated: boolean; modelAccepted: boolean;
  effortAccepted: boolean; evidence: "connection_probe"; // not proof of all model capabilities
}
export interface OperationReceipt { operationId: string; state: "checking" | "waiting_for_idle" | "applying" }
export type SwitchResult =
  | { outcome: "applied"; active: AiSelection; configState: "ready" }
  | { outcome: "unchanged" | "rolled_back"; active: AiSelection | null; configState: ConfigState; error: AppError }
  | { outcome: "rollback_failed"; active: null; configState: "error"; error: AppError };
export type OperationView = { operationId: string } & (
  | { phase: "checking" | "waiting_for_idle" | "applying" | "rolling_back"; result: null; error: null }
  | { phase: "completed"; result: ConnectionTest | SwitchResult; error: null }
  | { phase: "failed" | "cancelled"; result: null; error: AppError }
);
export type TaskStatus = "idle" | "preparing" | "running" | "approval_required" | "cancelling" | "completed" | "failed" | "cancelled" | "interrupted";
export interface TaskView { id: string; sessionId: string; projectId: string; status: TaskStatus; snapshot: PublicSnapshot | null; error: AppError | null }
export type SettingChange =
  | { key: "search_engine"; value: "bing" | "google" }
  | { key: "editor"; value: { id: "custom"; executable: string } | null }
  | { key: "project_sort"; value: "recent" | "name" }
  | { key: "approval_policy"; value: "ask" } // no permissive/never-ask option in v1
  | { key: "log_level"; value: "error" | "warn" | "info" | "debug" }
  | { key: "summary_retention_days"; value: number }
  | { key: "ai_defaults"; value: AiSelection | null }
  | { key: "overlay_enabled"; value: boolean }
  | { key: "overlay_shortcut"; value: string }
  | { key: "overlay_position"; value: { x: number; y: number; monitor: string | null; scaleFactor: number } | null };
export type SettingsScope = { scope: "global" } | { scope: "project"; projectId: string };
export interface SettingsView { revision: number; values: SettingChange[] }
export interface LogStatus {
  enabled: boolean;
  level: "error" | "warn" | "info" | "debug";
  errorCode: null | "PA_INTERNAL_UNEXPECTED";
  fileCount: number;
  totalBytes: number;
  auditRetentionDays: 90;
  summaryCollectionEnabled: false;
}
export interface AdapterStatus {
  installed: boolean; version: string | null; compatible: boolean;
  readSupported: boolean; switchSupported: boolean; reasonCode: string | null;
}
// Informational policy only: no installation probe or external configuration access.
export interface ManualCcSwitchStatus {
  management: "user_manual"; integration: "disabled";
  readSupported: false; switchSupported: false;
}

export interface ActiveContext { projectId: string; sessionId: string; profileId: string; modelId: string; effort: string | null }
// Phase 2 project DTOs are compile-time contracts only until each project
// service is implemented and added to ImplementedCommandMap.
export type ProjectAvailability = "available" | "missing" | "inaccessible" | "identity_changed";
export type ProjectAuthorization = "granted" | "requires_reauthorization" | "revoked";
export interface ProjectView {
  id: string; name: string; displayName: string | null; absolutePath: string;
  availability: ProjectAvailability; authorization: ProjectAuthorization; revision: number;
  createdAt: number; lastUsedAt: number | null;
}
export interface ProjectListPage { items: ProjectView[]; nextCursor: string | null }
export interface ActiveProject { projectId: string; name: string }
export type GitSummary =
  | { state: "available"; isRepository: true; branch: string | null; detached: boolean; workingTreeModified: boolean; refreshedAt: number }
  | { state: "not_repository" | "git_unavailable" | "timed_out" | "failed"; isRepository: false | null; branch: null; detached: false; workingTreeModified: null; refreshedAt: number | null };
type ProjectSettingChange = Exclude<SettingChange, Extract<SettingChange, { key: "overlay_enabled" | "overlay_shortcut" | "overlay_position" }>>;
type WithProjectSettingSource<Change> = Change extends { key: infer Key; value: infer Value }
  ? { key: Key; value: Value; source: "project" | "global" }
  : never;
export type ProjectSetting = WithProjectSettingSource<ProjectSettingChange>;
export interface ProjectSettingsView { projectId: string; revision: number; globalRevision: number; values: ProjectSetting[] }
export type ProjectAwareAppStatus = AppStatus & { activeProject: ActiveProject | null };
export interface ServiceStatus { id: "desktop" | "ipc" | "events" | "overlay" | "storage" | "settings" | "logging" | "credentials" | "providers" | "codex" | "projects" | "tasks"; state: "ready" | "unavailable" | "degraded"; reasonCode: string | null }
export interface AppStatus {
  apiVersion: 1; streamId: string; sequence: number; configState: ConfigState;
  services: ServiceStatus[]; ccSwitch: ManualCcSwitchStatus;
  activeContext: ActiveContext | null; taskStatus: "idle"; overlay: OverlayView;
}
export type OverlayAction =
  | { action: "expand" | "collapse" | "enable" | "disable" | "open_main" | "exit" | "repair_position" }
  | { action: "set_shortcut"; shortcut: string }
  | { action: "draft"; draft: string };
export interface ImplementedCommandMap {
  projects_add_from_picker: CommandMap["projects_add_from_picker"];
  project_get: CommandMap["project_get"];
  project_remove: CommandMap["project_remove"];
  providers_list: CommandMap["providers_list"];
  provider_save: CommandMap["provider_save"];
  provider_set_enabled: CommandMap["provider_set_enabled"];
  provider_delete: CommandMap["provider_delete"];
  provider_models: CommandMap["provider_models"];
  provider_test: CommandMap["provider_test"];
  provider_switch: CommandMap["provider_switch"];
  operation_get: CommandMap["operation_get"];
  operation_cancel: CommandMap["operation_cancel"];
  framework_probe: { params: { message: string }; data: ProbeReply };
  desktop_initialize: { params: Record<string, never>; data: DesktopReady };
  app_get_status: { params: Record<string, never>; data: AppStatus };
  overlay_get: { params: Record<string, never>; data: OverlayView };
  overlay_control: { params: OverlayAction; data: null };
  settings_get: { params: SettingsScope; data: SettingsView };
  settings_update: { params: SettingsScope & { expectedRevision: number; changes: SettingChange[] }; data: SettingsView };
  // Main-window only; logs_clear returns the authoritative status after cleanup.
  logs_get_status: { params: Record<string, never>; data: LogStatus };
  logs_clear: { params: Record<string, never>; data: LogStatus };
  credential_put: { params: { profileId: string; expectedRevision: number; secret: string }; data: { credential: CredentialState; configRevision: number } };
  credential_remove: { params: { profileId: string; expectedRevision: number }; data: { credential: CredentialState; configRevision: number } };
}
type Assert<T extends true> = T;
export type FutureProjectCommandsRemainUnimplemented = Assert<
  Extract<keyof ImplementedCommandMap, "projects_list" | "project_select" | "project_open_directory" | "project_open_file" | "project_pick_file" | "project_git_summary"> extends never ? true : false
>;

// All mutations use optimistic revisions; operation receipts are acceptance, not success.
export interface CommandMap {
  framework_probe: { params: { message: string }; data: ProbeReply };
  app_get_status: ImplementedCommandMap["app_get_status"];
  logs_get_status: ImplementedCommandMap["logs_get_status"];
  logs_clear: ImplementedCommandMap["logs_clear"];
  settings_get: { params: SettingsScope; data: SettingsView | ProjectSettingsView };
  settings_update: { params: SettingsScope & { expectedRevision: number; changes: SettingChange[] }; data: SettingsView };
  providers_list: { params: Record<string, never>; data: { profiles: ProviderProfile[]; active: AiSelection | null; configState: ConfigState } };
  provider_save: { params: { profileId: string | null; expectedRevision: number | null; name: string; endpoint: string; defaultModel: string | null; defaultEffort: string | null }; data: ProviderProfile };
  provider_set_enabled: { params: { profileId: string; expectedRevision: number; enabled: boolean }; data: ProviderProfile };
  provider_delete: { params: { profileId: string; expectedRevision: number }; data: { removed: boolean } };
  provider_models: { params: { profileId: string; expectedRevision: number; refresh: boolean }; data: { capabilities: ModelCapability[] } };
  credential_put: { params: { profileId: string; expectedRevision: number; secret: string }; data: { credential: CredentialState; configRevision: number } };
  credential_remove: { params: { profileId: string; expectedRevision: number }; data: { credential: CredentialState; configRevision: number } };
  provider_test: { params: AiSelection; data: OperationReceipt };
  provider_switch: { params: AiSelection; data: OperationReceipt };
  operation_cancel: { params: { operationId: string }; data: { accepted: boolean } };
  operation_get: { params: { operationId: string }; data: OperationView };
  session_create: { params: { projectId: string }; data: { sessionId: string } };
  task_start: { params: { sessionId: string; selection: AiSelection; prompt: string }; data: TaskView };
  task_get: { params: { taskId: string }; data: TaskView };
  task_cancel: { params: { taskId: string }; data: { accepted: boolean; status: TaskStatus } };
  approval_decide: { params: { approvalId: string; taskId: string; operationFingerprint: string; decision: "approve" | "deny" }; data: { decision: "approved" | "denied" | "expired" } };
  // Planned project commands: types are not a runtime allow-list.
  projects_add_from_picker: { params: Record<string, never>; data: { outcome: "registered"; project: ProjectView } | { outcome: "cancelled" } };
  projects_list: { params: { query?: string; cursor?: string; limit?: number }; data: ProjectListPage };
  project_get: { params: { projectId: string }; data: ProjectView };
  project_select: { params: { projectId: string }; data: { activeProject: ActiveProject } };
  project_remove: { params: { projectId: string; expectedRevision: number }; data: { removed: true; retainedForHistory: boolean } };
  project_open_directory: { params: { projectId: string }; data: { systemAccepted: true } };
  project_pick_file: { params: { projectId: string }; data: { relativePath: string } | { outcome: "cancelled" } };
  project_open_file: { params: { projectId: string; relativePath: string }; data: { systemAccepted: true } };
  project_git_summary: { params: { projectId: string; refresh?: boolean }; data: GitSummary };
}
export type EventPayload =
  | { type: "app_state_changed" }
  | { type: "ai_config_changed"; configState: ConfigState; active: AiSelection | null; reason: "local_change" | "external_change" | "credential_change" }
  | { type: "operation_progress"; operationId: string; phase: "checking" | "waiting_for_idle" | "applying" | "rolling_back"; affectedTaskIds: string[] }
  | { type: "operation_completed"; operationId: string; result: ConnectionTest | SwitchResult | null; error: AppError | null }
  | { type: "task_status"; task: TaskView }
  | { type: "task_delta"; taskId: string; channel: "reply" | "progress"; text: string }
  | { type: "approval_required"; approvalId: string; taskId: string; operationKind: "command" | "file_change" | "unknown"; operationFingerprint: string; displaySummary: string; expiresAt: number };
export interface AppEvent {
  apiVersion: 1; eventId: string; streamId: string; sequence: number; occurredAt: number;
  correlationId: string; payload: EventPayload;
}
