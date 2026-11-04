-- S3-03: append-only task/session persistence. Existing migrations remain immutable.
ALTER TABLE sessions ADD COLUMN revision INTEGER NOT NULL DEFAULT 1
    CHECK (revision BETWEEN 1 AND 9007199254740991);
ALTER TABLE sessions ADD COLUMN client_request_id TEXT
    CHECK (client_request_id IS NULL OR (length(client_request_id) BETWEEN 1 AND 96));
CREATE UNIQUE INDEX idx_sessions_client_request
    ON sessions(client_request_id)
    WHERE client_request_id IS NOT NULL;

ALTER TABLE tasks ADD COLUMN revision INTEGER NOT NULL DEFAULT 1
    CHECK (revision BETWEEN 1 AND 9007199254740991);
ALTER TABLE tasks ADD COLUMN client_request_id TEXT
    CHECK (client_request_id IS NULL OR (length(client_request_id) BETWEEN 1 AND 96));
ALTER TABLE tasks ADD COLUMN selection_source TEXT
    CHECK (selection_source IS NULL OR selection_source IN ('task', 'project', 'global'));
ALTER TABLE tasks ADD COLUMN project_revision INTEGER
    CHECK (project_revision IS NULL OR project_revision BETWEEN 1 AND 9007199254740991);
ALTER TABLE tasks ADD COLUMN project_settings_revision INTEGER
    CHECK (project_settings_revision IS NULL OR project_settings_revision BETWEEN 1 AND 9007199254740991);
ALTER TABLE tasks ADD COLUMN global_settings_revision INTEGER
    CHECK (global_settings_revision IS NULL OR global_settings_revision BETWEEN 1 AND 9007199254740991);
ALTER TABLE tasks ADD COLUMN session_revision INTEGER
    CHECK (session_revision IS NULL OR session_revision BETWEEN 1 AND 9007199254740991);
ALTER TABLE tasks ADD COLUMN origin_window TEXT
    CHECK (origin_window IS NULL OR origin_window IN ('main', 'overlay'));
ALTER TABLE tasks ADD COLUMN last_event_sequence INTEGER NOT NULL DEFAULT 0
    CHECK (last_event_sequence BETWEEN 0 AND 9007199254740991);
ALTER TABLE tasks ADD COLUMN content_revision INTEGER NOT NULL DEFAULT 0
    CHECK (content_revision BETWEEN 0 AND 9007199254740991);
CREATE UNIQUE INDEX idx_tasks_client_request
    ON tasks(client_request_id)
    WHERE client_request_id IS NOT NULL;

ALTER TABLE approvals ADD COLUMN expires_at INTEGER
    CHECK (expires_at IS NULL OR expires_at > requested_at);
ALTER TABLE approvals ADD COLUMN revision INTEGER NOT NULL DEFAULT 1
    CHECK (revision BETWEEN 1 AND 9007199254740991);
ALTER TABLE approvals ADD COLUMN operation_fingerprint TEXT
    CHECK (operation_fingerprint IS NULL OR (length(operation_fingerprint) = 64 AND operation_fingerprint NOT GLOB '*[^0-9a-f]*'));
ALTER TABLE approvals ADD COLUMN window_label TEXT
    CHECK (window_label IS NULL OR window_label IN ('main', 'overlay'));
ALTER TABLE approvals ADD COLUMN consumption TEXT NOT NULL DEFAULT 'unconsumed'
    CHECK (consumption IN ('unconsumed', 'claimed', 'acknowledged', 'abandoned'));
ALTER TABLE approvals ADD COLUMN consume_request_id TEXT
    CHECK (consume_request_id IS NULL OR length(consume_request_id) BETWEEN 1 AND 96);
CREATE TRIGGER approval_consumption_request_guard BEFORE UPDATE OF consumption,consume_request_id ON approvals
WHEN (NEW.consumption='unconsumed' AND NEW.consume_request_id IS NOT NULL) OR
     (NEW.consumption!='unconsumed' AND NEW.consume_request_id IS NULL)
BEGIN SELECT RAISE(ABORT, 'APPROVAL_CONSUMPTION_REQUEST_INVALID'); END;
CREATE TRIGGER approval_binding_fixed BEFORE UPDATE OF task_id,external_request_id,operation_kind,expires_at,operation_fingerprint,window_label ON approvals
WHEN NEW.task_id IS NOT OLD.task_id OR NEW.external_request_id IS NOT OLD.external_request_id OR
     NEW.operation_kind IS NOT OLD.operation_kind OR NEW.expires_at IS NOT OLD.expires_at OR
     NEW.operation_fingerprint IS NOT OLD.operation_fingerprint OR NEW.window_label IS NOT OLD.window_label
BEGIN SELECT RAISE(ABORT, 'APPROVAL_BINDING_FIXED'); END;
CREATE UNIQUE INDEX idx_approvals_consume_request
    ON approvals(consume_request_id) WHERE consume_request_id IS NOT NULL;
CREATE INDEX idx_approvals_expiry ON approvals(expires_at) WHERE decision = 'pending';

CREATE TABLE task_events (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence >= 1 AND sequence <= 9007199254740991),
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    event_kind TEXT NOT NULL CHECK (event_kind IN ('status', 'content_changed', 'approval', 'completed', 'failed')),
    status TEXT CHECK (status IS NULL OR status IN ('preparing', 'running', 'approval_required', 'cancelling', 'completed', 'failed', 'cancelled', 'interrupted')),
    revision INTEGER CHECK (revision IS NULL OR revision BETWEEN 1 AND 9007199254740991),
    error_code TEXT,
    approval_id TEXT REFERENCES approvals(id) ON DELETE RESTRICT,
    channel TEXT CHECK (channel IS NULL OR channel IN ('reply', 'progress')),
    content_revision INTEGER CHECK (content_revision IS NULL OR content_revision BETWEEN 1 AND 9007199254740991),
    PRIMARY KEY (task_id, sequence),
    CHECK ((event_kind = 'status' AND status IS NOT NULL AND status IN ('preparing', 'running', 'approval_required', 'cancelling') AND revision IS NOT NULL) OR
           (event_kind = 'content_changed' AND channel IS NOT NULL AND content_revision IS NOT NULL) OR
           (event_kind = 'approval' AND approval_id IS NOT NULL) OR
           (event_kind = 'completed' AND status IS NOT NULL AND status IN ('completed', 'cancelled', 'interrupted') AND revision IS NOT NULL) OR
           (event_kind = 'failed' AND status = 'failed' AND error_code IS NOT NULL AND revision IS NOT NULL))
) STRICT;
CREATE INDEX idx_task_events_recent ON task_events(task_id, sequence);

CREATE TRIGGER task_event_sequence_guard BEFORE INSERT ON task_events
BEGIN
    SELECT CASE WHEN NEW.sequence != (SELECT last_event_sequence + 1 FROM tasks WHERE id = NEW.task_id)
        THEN RAISE(ABORT, 'TASK_EVENT_SEQUENCE_INVALID') END;
    SELECT CASE WHEN NEW.approval_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM approvals WHERE id = NEW.approval_id AND task_id = NEW.task_id
    ) THEN RAISE(ABORT, 'TASK_EVENT_APPROVAL_MISMATCH') END;
END;
CREATE TRIGGER task_event_advance_sequence AFTER INSERT ON task_events
BEGIN
    UPDATE tasks SET last_event_sequence = NEW.sequence WHERE id = NEW.task_id;
END;
CREATE TRIGGER task_event_immutable BEFORE UPDATE ON task_events
BEGIN SELECT RAISE(ABORT, 'TASK_EVENT_IMMUTABLE'); END;

CREATE TRIGGER session_revision_guard BEFORE UPDATE ON sessions
WHEN NEW.revision != OLD.revision AND NEW.revision != OLD.revision + 1
BEGIN SELECT RAISE(ABORT, 'SESSION_REVISION_INVALID'); END;
CREATE TRIGGER task_revision_guard BEFORE UPDATE ON tasks
WHEN NEW.revision != OLD.revision AND NEW.revision != OLD.revision + 1
BEGIN SELECT RAISE(ABORT, 'TASK_REVISION_INVALID'); END;
CREATE TRIGGER approval_revision_guard BEFORE UPDATE ON approvals
WHEN NEW.revision != OLD.revision AND NEW.revision != OLD.revision + 1
BEGIN SELECT RAISE(ABORT, 'APPROVAL_REVISION_INVALID'); END;

CREATE TRIGGER session_status_transition_guard BEFORE UPDATE OF status ON sessions
WHEN NEW.status != OLD.status AND NOT (
    (OLD.status = 'active' AND NEW.status IN ('closed', 'interrupted')) OR
    (OLD.status = 'interrupted' AND NEW.status = 'active')
)
BEGIN SELECT RAISE(ABORT, 'SESSION_STATUS_TRANSITION_INVALID'); END;

CREATE TRIGGER task_status_transition_guard BEFORE UPDATE OF status ON tasks
WHEN NEW.status != OLD.status AND NOT (
    (OLD.status = 'idle' AND NEW.status IN ('interrupted', 'failed', 'cancelled')) OR
    (OLD.status = 'preparing' AND NEW.status IN ('running', 'cancelling', 'failed', 'interrupted')) OR
    (OLD.status = 'running' AND NEW.status IN ('approval_required', 'cancelling', 'completed', 'failed', 'interrupted')) OR
    (OLD.status = 'approval_required' AND NEW.status IN ('running', 'cancelling', 'failed', 'interrupted')) OR
    (OLD.status = 'cancelling' AND NEW.status IN ('completed', 'failed', 'cancelled', 'interrupted'))
)
BEGIN SELECT RAISE(ABORT, 'TASK_STATUS_TRANSITION_INVALID'); END;

CREATE TRIGGER approval_decision_transition_guard BEFORE UPDATE OF decision ON approvals
WHEN NEW.decision != OLD.decision AND NOT (
    OLD.decision = 'pending' AND NEW.decision IN ('approved', 'denied', 'expired', 'interrupted')
)
BEGIN SELECT RAISE(ABORT, 'APPROVAL_DECISION_TRANSITION_INVALID'); END;
CREATE TRIGGER approval_consumption_transition_guard BEFORE UPDATE OF consumption ON approvals
WHEN NEW.consumption != OLD.consumption AND NOT (
    (OLD.consumption = 'unconsumed' AND NEW.consumption = 'claimed') OR
    (OLD.consumption = 'claimed' AND NEW.consumption IN ('acknowledged', 'abandoned'))
)
BEGIN SELECT RAISE(ABORT, 'APPROVAL_CONSUMPTION_TRANSITION_INVALID'); END;

CREATE TRIGGER session_binding_fixed BEFORE UPDATE OF project_id, client_request_id ON sessions
WHEN NEW.project_id IS NOT OLD.project_id OR NEW.client_request_id IS NOT OLD.client_request_id
BEGIN SELECT RAISE(ABORT, 'SESSION_BINDING_FIXED'); END;
CREATE TRIGGER task_admission_guard BEFORE INSERT ON tasks
WHEN NEW.status = 'preparing' AND (
    NEW.snapshot_id IS NULL OR NEW.client_request_id IS NULL OR
    NEW.selection_source IS NULL OR NEW.project_revision IS NULL OR
    NEW.project_settings_revision IS NULL OR NEW.global_settings_revision IS NULL OR NEW.session_revision IS NULL OR
    NEW.origin_window IS NULL
)
BEGIN SELECT RAISE(ABORT, 'TASK_ADMISSION_INCOMPLETE'); END;
CREATE TRIGGER task_binding_fixed BEFORE UPDATE OF session_id, snapshot_id, client_request_id,
    selection_source, project_revision, project_settings_revision, global_settings_revision, session_revision, origin_window ON tasks
WHEN NEW.session_id IS NOT OLD.session_id OR NEW.snapshot_id IS NOT OLD.snapshot_id OR
    NEW.client_request_id IS NOT OLD.client_request_id OR NEW.selection_source IS NOT OLD.selection_source OR
    NEW.project_revision IS NOT OLD.project_revision OR NEW.project_settings_revision IS NOT OLD.project_settings_revision OR
    NEW.global_settings_revision IS NOT OLD.global_settings_revision OR NEW.session_revision IS NOT OLD.session_revision OR
    NEW.origin_window IS NOT OLD.origin_window
BEGIN SELECT RAISE(ABORT, 'TASK_BINDING_FIXED'); END;
