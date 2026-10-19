-- Additive migration: schema v1/v2 and their checksums remain unchanged.
ALTER TABLE provider_profiles ADD COLUMN credential_status TEXT NOT NULL DEFAULT 'missing'
    CHECK (credential_status IN ('missing', 'configured_unverified', 'verified', 'invalid', 'external_unavailable'));
UPDATE provider_profiles
SET credential_status = CASE WHEN credential_ref IS NULL THEN 'missing' ELSE 'configured_unverified' END;

-- Credential mutations share the existing minimal audit boundary; no secret
-- material, origin, or endpoint is copied into audit history.
DROP INDEX idx_audit_recent;
DROP INDEX idx_audit_correlation;
ALTER TABLE audit_events RENAME TO audit_events_v2;
CREATE TABLE audit_events (
    id TEXT PRIMARY KEY NOT NULL,
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    correlation_id TEXT NOT NULL,
    event_kind TEXT NOT NULL CHECK (event_kind IN ('config_read', 'config_switch', 'connection_test', 'config_rollback', 'credential_change', 'approval_decision', 'task_transition', 'storage_recovery')),
    outcome TEXT NOT NULL CHECK (outcome IN ('success', 'failure', 'denied', 'interrupted')),
    error_code TEXT,
    project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    approval_id TEXT REFERENCES approvals(id) ON DELETE SET NULL,
    profile_id TEXT REFERENCES provider_profiles(id) ON DELETE SET NULL,
    model_id TEXT,
    effort TEXT
) STRICT;
INSERT INTO audit_events (id, occurred_at, correlation_id, event_kind, outcome, error_code, project_id, task_id, approval_id, profile_id, model_id, effort)
SELECT id, occurred_at, correlation_id, event_kind, outcome, error_code, project_id, task_id, approval_id, profile_id, model_id, effort
FROM audit_events_v2;
DROP TABLE audit_events_v2;
CREATE INDEX idx_audit_recent ON audit_events(occurred_at DESC);
CREATE INDEX idx_audit_correlation ON audit_events(correlation_id, occurred_at);
