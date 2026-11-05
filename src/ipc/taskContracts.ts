// S3-01 design contracts only. No invocation, service, permission or admission.
import type { AiSelection, AppError, PublicSnapshot } from "./contracts";

export type SessionStatus = "active" | "closed" | "interrupted";
// idle belongs to the app/composer, never to an admitted task.
export type TaskStatus = "preparing" | "running" | "approval_required" | "cancelling" | "completed" | "failed" | "cancelled" | "interrupted";
export type TaskSelectionSource = "task" | "project" | "global";
export interface SessionView {
  id: string; projectId: string; status: SessionStatus; revision: number;
  createdAt: number; updatedAt: number;
}
export interface SessionReceipt { session: SessionView; duplicate: boolean }
export interface TaskView {
  id: string; sessionId: string; projectId: string; clientRequestId: string;
  status: TaskStatus; revision: number; snapshot: PublicSnapshot;
  selectionSource: TaskSelectionSource; error: AppError | null;
  createdAt: number; startedAt: number | null; finishedAt: number | null;
  lastEventSequence: number; contentRevision: number; contentAvailable: boolean;
}
export interface TaskStartParams {
  projectId: string; sessionId: string; clientRequestId: string;
  expectedProjectRevision: number; expectedSessionRevision: number;
  expectedProjectSettingsRevision: number; expectedGlobalRevision: number;
  selection: AiSelection | null; prompt: string;
}
export interface TaskReceipt { task: TaskView; duplicate: boolean }
export interface CancelReceipt { accepted: boolean; status: TaskStatus; revision: number }
export type ApprovalDecision = "pending" | "approved" | "denied" | "expired" | "interrupted";
export type ApprovalConsumption = "unconsumed" | "claimed" | "acknowledged" | "abandoned";
export interface ApprovalView {
  id: string; taskId: string; sessionId: string; revision: number;
  operationKind: "command" | "file_change" | "unknown";
  operationFingerprint: string; displaySummary: string;
  decision: ApprovalDecision; consumption: ApprovalConsumption;
  requestedAt: number; expiresAt: number; decidedAt: number | null;
}
export interface ApprovalReceipt { approval: ApprovalView; duplicate: boolean }
export interface TaskContent {
  taskId: string; contentRevision: number; channel: "reply" | "progress";
  text: string; nextCursor: string | null; truncated: boolean;
  availability: "memory" | "unavailable";
}
// Sensitive events are returned only by an authorized pull command.
// These are NOT AppEvent.payload variants.
export type TaskEventData =
  | { type: "status"; status: TaskStatus; revision: number }
  | { type: "content_changed"; channel: "reply" | "progress"; contentRevision: number }
  | { type: "approval"; approvalId: string }
  | { type: "completed"; status: "completed" | "cancelled" | "interrupted"; revision: number }
  | { type: "failed"; code: string; revision: number };
export interface TaskEvent { taskId: string; sequence: number; occurredAt: number; data: TaskEventData }
export interface TaskEventPage {
  taskId: string; events: TaskEvent[]; nextCursor: string | null;
  throughSequence: number; resetRequired: boolean;
}
export interface TaskChange { id: string; kind: "added" | "modified" | "deleted"; relativePath: string; diffAvailable: boolean }
export interface TaskChangePage { taskId: string; items: TaskChange[]; nextCursor: string | null; contentAvailable: boolean }
export interface TaskDiff { taskId: string; changeId: string; text: string | null; unavailableReason: null | "expired" | "binary" | "too_large" | "unsafe_path" }
export interface ActiveTaskContext {
  contextRevision: number; projectId: string; projectName: string;
  sessionId: string; selection: AiSelection;
}
export interface OverlayTaskView {
  taskId: string; status: TaskStatus; revision: number;
  replyPreview: string; contentAvailable: boolean;
  approvalPending: boolean; errorCode: string | null;
}
export interface OverlayTaskReceipt { task: OverlayTaskView; duplicate: boolean }

const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const exact = (value: Record<string, unknown>, keys: string[]) => Object.keys(value).length === keys.length && keys.every(key => key in value);
const uuid = (value: unknown): value is string => typeof value === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
const revision = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 1;
const time = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 0;
const bounded = (value: unknown, limit: number) => typeof value === "string" && new TextEncoder().encode(value).length <= limit && !/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(value);
function isAppError(value: unknown): value is AppError {
  return record(value) && exact(value, ["category", "code", "message", "suggestion", "retryable"]) &&
    typeof value.code === "string" && /^PA_[A-Z0-9_]{1,96}$/.test(value.code) &&
    ["validation", "permission", "environment", "authentication", "compatibility", "conflict", "timeout", "cancelled", "external", "storage", "internal"].includes(String(value.category)) &&
    bounded(value.message, 512) && bounded(value.suggestion, 512) && typeof value.retryable === "boolean";
}
function isSnapshot(value: unknown): value is PublicSnapshot {
  return record(value) && exact(value, ["id", "profileId", "profileName", "sourceKind", "endpoint", "modelId", "effort", "configRevision", "credentialRevision", "effectiveAt", "routingScope"]) &&
    uuid(value.id) && /^[A-Za-z0-9_-]{1,128}$/.test(String(value.profileId)) && bounded(value.profileName, 512) &&
    (value.sourceKind === "relay" || value.sourceKind === "official") && bounded(value.endpoint, 2048) &&
    /^[A-Za-z0-9_.-]{1,128}$/.test(String(value.modelId)) &&
    (value.effort === null || (typeof value.effort === "string" && /^[A-Za-z0-9_.-]{1,128}$/.test(value.effort))) &&
    revision(value.configRevision) && Number.isSafeInteger(value.credentialRevision) && Number(value.credentialRevision) >= 0 &&
    time(value.effectiveAt) && value.routingScope === "owned_process";
}
export function isSessionView(value: unknown): value is SessionView {
  return record(value) && exact(value, ["id", "projectId", "status", "revision", "createdAt", "updatedAt"]) && uuid(value.id) && uuid(value.projectId) &&
    ["active", "closed", "interrupted"].includes(String(value.status)) && revision(value.revision) && time(value.createdAt) && time(value.updatedAt);
}
export function isSessionReceipt(value: unknown): value is SessionReceipt {
  return record(value) && exact(value, ["session", "duplicate"]) && isSessionView(value.session) && typeof value.duplicate === "boolean";
}
export function isSessionPage(value: unknown): value is { items: SessionView[]; nextCursor: string | null } {
  return record(value) && exact(value, ["items", "nextCursor"]) && Array.isArray(value.items) && value.items.length <= 100 && value.items.every(isSessionView) &&
    new Set(value.items.map(item => item.id)).size === value.items.length && (value.nextCursor === null || bounded(value.nextCursor, 512));
}
export function isTaskView(value: unknown): value is TaskView {
  return record(value) && exact(value, ["id", "sessionId", "projectId", "clientRequestId", "status", "revision", "snapshot", "selectionSource", "error", "createdAt", "startedAt", "finishedAt", "lastEventSequence", "contentRevision", "contentAvailable"]) &&
    uuid(value.id) && uuid(value.sessionId) && uuid(value.projectId) && uuid(value.clientRequestId) &&
    ["preparing", "running", "approval_required", "cancelling", "completed", "failed", "cancelled", "interrupted"].includes(String(value.status)) && revision(value.revision) && isSnapshot(value.snapshot) &&
    ["task", "project", "global"].includes(String(value.selectionSource)) && (value.error === null || isAppError(value.error)) && time(value.createdAt) &&
    (value.startedAt === null || time(value.startedAt)) && (value.finishedAt === null || time(value.finishedAt)) &&
    Number.isSafeInteger(value.lastEventSequence) && Number(value.lastEventSequence) >= 0 && Number.isSafeInteger(value.contentRevision) && Number(value.contentRevision) >= 0 && typeof value.contentAvailable === "boolean";
}
export function isTaskReceipt(value: unknown): value is TaskReceipt {
  return record(value) && exact(value, ["task", "duplicate"]) && isTaskView(value.task) && typeof value.duplicate === "boolean";
}
export function isTaskPage(value: unknown): value is { items: TaskView[]; nextCursor: string | null } {
  return record(value) && exact(value, ["items", "nextCursor"]) && Array.isArray(value.items) && value.items.length <= 100 && value.items.every(isTaskView) &&
    new Set(value.items.map(item => item.id)).size === value.items.length && (value.nextCursor === null || bounded(value.nextCursor, 512));
}
export function isCancelReceipt(value: unknown): value is CancelReceipt {
  return record(value) && exact(value, ["accepted", "status", "revision"]) && typeof value.accepted === "boolean" &&
    ["preparing", "running", "approval_required", "cancelling", "completed", "failed", "cancelled", "interrupted"].includes(String(value.status)) && revision(value.revision);
}
function isEventData(value: unknown): value is TaskEventData {
  if (!record(value) || typeof value.type !== "string") return false;
  if (value.type === "status") return exact(value, ["type", "status", "revision"]) && ["preparing", "running", "approval_required", "cancelling"].includes(String(value.status)) && revision(value.revision);
  if (value.type === "content_changed") return exact(value, ["type", "channel", "contentRevision"]) && ["reply", "progress"].includes(String(value.channel)) && revision(value.contentRevision);
  if (value.type === "approval") return exact(value, ["type", "approvalId"]) && uuid(value.approvalId);
  if (value.type === "completed") return exact(value, ["type", "status", "revision"]) && ["completed", "cancelled", "interrupted"].includes(String(value.status)) && revision(value.revision);
  if (value.type === "failed") return exact(value, ["type", "code", "revision"]) && typeof value.code === "string" && /^PA_[A-Z0-9_]{1,96}$/.test(value.code) && revision(value.revision);
  return false;
}
export function isTaskEventPage(value: unknown): value is TaskEventPage {
  return record(value) && exact(value, ["taskId", "events", "nextCursor", "throughSequence", "resetRequired"]) && uuid(value.taskId) && Array.isArray(value.events) && value.events.length <= 100 &&
    value.events.every(event => record(event) && exact(event, ["taskId", "sequence", "occurredAt", "data"]) && event.taskId === value.taskId && revision(event.sequence) && time(event.occurredAt) && isEventData(event.data)) &&
    (value.nextCursor === null || bounded(value.nextCursor, 512)) && Number.isSafeInteger(value.throughSequence) && Number(value.throughSequence) >= 0 && typeof value.resetRequired === "boolean";
}
export function isApprovalView(value: unknown): value is ApprovalView {
  return record(value) && exact(value, ["id", "taskId", "sessionId", "revision", "operationKind", "operationFingerprint", "displaySummary", "decision", "consumption", "requestedAt", "expiresAt", "decidedAt"]) &&
    uuid(value.id) && uuid(value.taskId) && uuid(value.sessionId) && revision(value.revision) &&
    ["command", "file_change", "unknown"].includes(String(value.operationKind)) &&
    typeof value.operationFingerprint === "string" && /^[0-9a-f]{64}$/.test(value.operationFingerprint) && bounded(value.displaySummary, 512) &&
    ["pending", "approved", "denied", "expired", "interrupted"].includes(String(value.decision)) &&
    ["unconsumed", "claimed", "acknowledged", "abandoned"].includes(String(value.consumption)) &&
    time(value.requestedAt) && time(value.expiresAt) && (value.decidedAt === null || time(value.decidedAt));
}
export function isApprovalPage(value: unknown): value is { items: ApprovalView[] } {
  return record(value) && exact(value, ["items"]) && Array.isArray(value.items) && value.items.length <= 100 && value.items.every(isApprovalView);
}
export interface TaskCommandMap {
  active_context_set: { params: { projectId: string; sessionId: string; selection: AiSelection; expectedContextRevision: number }; data: ActiveTaskContext };
  session_create: { params: { projectId: string; expectedProjectRevision: number; clientRequestId: string }; data: SessionReceipt };
  session_find_by_request: { params: { projectId: string; clientRequestId: string }; data: SessionView | null };
  sessions_list: { params: { projectId: string; cursor?: string; limit?: number }; data: { items: SessionView[]; nextCursor: string | null } };
  session_get: { params: { sessionId: string }; data: SessionView };
  session_resume: { params: { sessionId: string; expectedRevision: number; expectedProjectRevision: number }; data: SessionView };
  session_end: { params: { sessionId: string; expectedRevision: number }; data: SessionView };
  task_start: { params: TaskStartParams; data: TaskReceipt };
  task_find_by_request: { params: { projectId: string; sessionId: string; clientRequestId: string }; data: TaskView | null };
  tasks_list: { params: { sessionId: string; cursor?: string; limit?: number }; data: { items: TaskView[]; nextCursor: string | null } };
  task_get: { params: { taskId: string }; data: TaskView };
  task_cancel: { params: { taskId: string; expectedRevision: number }; data: CancelReceipt };
  task_content_get: { params: { taskId: string; channel: "reply" | "progress"; cursor?: string }; data: TaskContent };
  task_events_get: { params: { taskId: string; cursor?: string; limit?: number }; data: TaskEventPage };
  approvals_get: { params: { taskId: string }; data: { items: ApprovalView[] } };
  approval_decide: { params: { approvalId: string; taskId: string; expectedRevision: number; operationFingerprint: string; decision: "approve" | "deny" }; data: ApprovalReceipt };
  task_changes_get: { params: { taskId: string; cursor?: string; limit?: number }; data: TaskChangePage };
  task_change_get: { params: { taskId: string; changeId: string }; data: TaskDiff };
  // S3-07: bound to server-side active context; no arbitrary IDs/config/cwd.
  task_start_from_active: { params: { expectedContextRevision: number; clientRequestId: string; prompt: string }; data: OverlayTaskReceipt };
  overlay_task_get: { params: Record<string, never>; data: { context: ActiveTaskContext | null; task: OverlayTaskView | null } };
  overlay_task_find_by_request: { params: { expectedContextRevision: number; clientRequestId: string }; data: OverlayTaskView | null };
  overlay_task_cancel: { params: { expectedContextRevision: number; expectedTaskRevision: number }; data: CancelReceipt };
}

// S3-04 exposes the main-window session/task gateway and authoritative state
// journal only. Approval decisions, content bodies, and overlay commands stay closed.
