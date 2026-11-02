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
