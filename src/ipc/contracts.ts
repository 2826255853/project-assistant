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
  id: string; name: string; sourceKind: SourceKind; adapterKind: "self_managed";
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
export interface ServiceStatus { id: "desktop" | "ipc" | "events" | "overlay" | "storage" | "settings" | "logging" | "credentials" | "providers" | "codex" | "projects" | "tasks"; state: "ready" | "unavailable" | "degraded"; reasonCode: string | null }
export interface AppStatus {
  apiVersion: 1; streamId: string; sequence: number; configState: "unconfigured";
  services: ServiceStatus[]; ccSwitch: ManualCcSwitchStatus;
  activeContext: ActiveContext | null; taskStatus: "idle"; overlay: OverlayView;
}
export type OverlayAction =
  | { action: "expand" | "collapse" | "enable" | "disable" | "open_main" | "exit" | "repair_position" }
  | { action: "set_shortcut"; shortcut: string }
  | { action: "draft"; draft: string };
export interface ImplementedCommandMap {
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
}

// All mutations use optimistic revisions; operation receipts are acceptance, not success.
export interface CommandMap {
  framework_probe: { params: { message: string }; data: ProbeReply };
  app_get_status: ImplementedCommandMap["app_get_status"];
  logs_get_status: ImplementedCommandMap["logs_get_status"];
  logs_clear: ImplementedCommandMap["logs_clear"];
  settings_get: { params: SettingsScope; data: SettingsView };
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
