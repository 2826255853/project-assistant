-- Immutable once released. IDs are application-generated UUID strings;
-- timestamps are UTC Unix milliseconds. No credential payload or arbitrary JSON.
CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL,
    canonical_path TEXT NOT NULL,
    path_key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    display_name TEXT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE project_usage (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    last_opened_at INTEGER NOT NULL CHECK (last_opened_at >= 0),
    open_count INTEGER NOT NULL DEFAULT 0 CHECK (open_count >= 0),
    last_source TEXT NOT NULL CHECK (last_source IN ('project_list', 'search', 'codex', 'system_open'))
) STRICT;
CREATE INDEX idx_project_usage_recent ON project_usage(last_opened_at DESC, project_id);

CREATE TABLE provider_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('official', 'relay')),
    adapter_kind TEXT NOT NULL CHECK (adapter_kind IN ('self_managed', 'cc_switch')),
    external_profile_id TEXT,
    endpoint TEXT NOT NULL CHECK (endpoint LIKE 'https://%' AND instr(endpoint, '?') = 0 AND instr(endpoint, '#') = 0 AND instr(endpoint, '@') = 0),
    endpoint_origin TEXT NOT NULL,
    wire_api TEXT NOT NULL CHECK (wire_api IN ('responses')),
    auth_kind TEXT NOT NULL CHECK (auth_kind IN ('api_key', 'external_login')),
    credential_ref TEXT,
    credential_revision INTEGER NOT NULL DEFAULT 0 CHECK (credential_revision >= 0),
    default_model TEXT,
    default_effort TEXT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    config_revision INTEGER NOT NULL CHECK (config_revision >= 1),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK ((auth_kind = 'api_key' AND credential_ref IS NOT NULL) OR (auth_kind = 'external_login' AND credential_ref IS NULL)),
    CHECK (credential_ref IS NULL OR credential_ref = 'credential:' || id || ':' || credential_revision),
    CHECK (default_effort IS NULL OR default_model IS NOT NULL)
) STRICT;

CREATE TABLE model_capabilities (
    profile_id TEXT NOT NULL REFERENCES provider_profiles(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL,
    effort TEXT NOT NULL, -- empty string denotes no explicit effort override
    context_window INTEGER CHECK (context_window > 0),
    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('cli_catalog', 'profile_declaration', 'connection_probe')),
    source_version TEXT NOT NULL,
    profile_revision INTEGER NOT NULL CHECK (profile_revision >= 1),
    observed_at INTEGER NOT NULL CHECK (observed_at >= 0),
    expires_at INTEGER NOT NULL CHECK (expires_at > observed_at),
    PRIMARY KEY (profile_id, model_id, effort)
) STRICT;
CREATE INDEX idx_model_capabilities_expiry ON model_capabilities(expires_at);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    external_session_id TEXT UNIQUE,
    summary_redacted TEXT CHECK (length(summary_redacted) <= 512),
    status TEXT NOT NULL CHECK (status IN ('active', 'closed', 'interrupted')),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;
CREATE INDEX idx_sessions_project_recent ON sessions(project_id, updated_at DESC);

CREATE TABLE ai_config_snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    profile_id TEXT NOT NULL REFERENCES provider_profiles(id) ON DELETE RESTRICT,
    profile_name TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('official', 'relay')),
    endpoint TEXT NOT NULL CHECK (endpoint LIKE 'https://%' AND instr(endpoint, '?') = 0 AND instr(endpoint, '#') = 0 AND instr(endpoint, '@') = 0),
    endpoint_origin TEXT NOT NULL,
    wire_api TEXT NOT NULL CHECK (wire_api IN ('responses')),
    auth_kind TEXT NOT NULL CHECK (auth_kind IN ('api_key', 'external_login')),
    credential_ref TEXT,
    credential_revision INTEGER NOT NULL CHECK (credential_revision >= 0),
    model_id TEXT NOT NULL,
    effort TEXT, -- NULL = no explicit override, discovered values are not hardcoded
    config_revision INTEGER NOT NULL CHECK (config_revision >= 1),
    capability_source TEXT NOT NULL CHECK (capability_source IN ('cli_catalog', 'profile_declaration', 'connection_probe')),
    capability_version TEXT NOT NULL,
    adapter_version TEXT NOT NULL,
    routing_scope TEXT NOT NULL CHECK (routing_scope IN ('owned_process', 'shared_global')),
    effective_at INTEGER NOT NULL CHECK (effective_at >= 0),
    captured_at INTEGER NOT NULL CHECK (captured_at >= effective_at),
    CHECK ((auth_kind = 'api_key' AND credential_ref IS NOT NULL) OR (auth_kind = 'external_login' AND credential_ref IS NULL)),
    CHECK (credential_ref IS NULL OR credential_ref = 'credential:' || profile_id || ':' || credential_revision)
) STRICT;
CREATE INDEX idx_snapshots_profile ON ai_config_snapshots(profile_id);
CREATE TRIGGER snapshot_immutable BEFORE UPDATE ON ai_config_snapshots
BEGIN SELECT RAISE(ABORT, 'SNAPSHOT_IMMUTABLE'); END;

CREATE TABLE tasks (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE RESTRICT,
    external_turn_id TEXT,
    snapshot_id TEXT UNIQUE REFERENCES ai_config_snapshots(id) ON DELETE RESTRICT,
    prompt_summary_redacted TEXT CHECK (length(prompt_summary_redacted) <= 512),
    result_summary_redacted TEXT CHECK (length(result_summary_redacted) <= 512),
    status TEXT NOT NULL CHECK (status IN ('idle', 'preparing', 'running', 'approval_required', 'cancelling', 'completed', 'failed', 'cancelled', 'interrupted')),
    error_code TEXT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    started_at INTEGER CHECK (started_at >= created_at),
    finished_at INTEGER CHECK (finished_at >= created_at),
    CHECK (status NOT IN ('running', 'approval_required', 'completed') OR (snapshot_id IS NOT NULL AND started_at IS NOT NULL)),
    CHECK (started_at IS NULL OR snapshot_id IS NOT NULL),
    CHECK ((status IN ('completed', 'failed', 'cancelled', 'interrupted')) = (finished_at IS NOT NULL)),
    CHECK (finished_at IS NULL OR started_at IS NULL OR finished_at >= started_at),
    UNIQUE (session_id, external_turn_id)
) STRICT;
CREATE INDEX idx_tasks_session_recent ON tasks(session_id, created_at DESC);
CREATE INDEX idx_tasks_status ON tasks(status, created_at);
CREATE TRIGGER task_snapshot_fixed BEFORE UPDATE OF snapshot_id ON tasks
WHEN OLD.snapshot_id IS NOT NULL AND NEW.snapshot_id IS NOT OLD.snapshot_id
BEGIN SELECT RAISE(ABORT, 'TASK_SNAPSHOT_FIXED'); END;

CREATE TABLE approvals (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE RESTRICT,
    external_request_id TEXT NOT NULL,
    operation_kind TEXT NOT NULL CHECK (operation_kind IN ('command', 'file_change', 'unknown')),
    operation_summary_redacted TEXT CHECK (length(operation_summary_redacted) <= 512),
    decision TEXT NOT NULL DEFAULT 'pending' CHECK (decision IN ('pending', 'approved', 'denied', 'expired', 'interrupted')),
    requested_at INTEGER NOT NULL CHECK (requested_at >= 0),
    decided_at INTEGER CHECK (decided_at >= requested_at),
    CHECK ((decision = 'pending') = (decided_at IS NULL)),
    UNIQUE (task_id, external_request_id)
) STRICT;
CREATE INDEX idx_approvals_task ON approvals(task_id, requested_at);
CREATE INDEX idx_approvals_pending ON approvals(requested_at) WHERE decision = 'pending';

CREATE TABLE settings (
    id TEXT PRIMARY KEY NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('global', 'project')),
    project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
    key TEXT NOT NULL CHECK (key IN ('ai_defaults', 'search_engine', 'editor', 'project_sort', 'approval_policy', 'log_level', 'summary_retention_days')),
    value_text TEXT,
    value_integer INTEGER,
    profile_id TEXT REFERENCES provider_profiles(id) ON DELETE RESTRICT,
    model_id TEXT,
    effort TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    CHECK ((scope = 'global' AND project_id IS NULL) OR (scope = 'project' AND project_id IS NOT NULL)),
    CHECK ((key = 'ai_defaults' AND profile_id IS NOT NULL AND value_text IS NULL AND value_integer IS NULL AND (effort IS NULL OR model_id IS NOT NULL)) OR
           (key <> 'ai_defaults' AND profile_id IS NULL AND model_id IS NULL AND effort IS NULL AND ((value_text IS NULL) <> (value_integer IS NULL))))
) STRICT;
CREATE UNIQUE INDEX idx_settings_global ON settings(key) WHERE scope = 'global';
CREATE UNIQUE INDEX idx_settings_project ON settings(project_id, key) WHERE scope = 'project';

-- Audit rows survive history deletion; nullable foreign keys are intentionally detached.
CREATE TABLE audit_events (
    id TEXT PRIMARY KEY NOT NULL,
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    correlation_id TEXT NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('config_read', 'config_switch', 'connection_test', 'config_rollback', 'approval_decision', 'task_transition', 'storage_recovery')),
    outcome TEXT NOT NULL CHECK (outcome IN ('success', 'failure', 'denied', 'interrupted')),
    error_code TEXT,
    project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    approval_id TEXT REFERENCES approvals(id) ON DELETE SET NULL,
    profile_id TEXT REFERENCES provider_profiles(id) ON DELETE SET NULL,
    model_id TEXT,
    effort TEXT
) STRICT;
CREATE INDEX idx_audit_recent ON audit_events(occurred_at DESC);
CREATE INDEX idx_audit_correlation ON audit_events(correlation_id, occurred_at);
