-- S3-02: independent monotonic project settings revisions, including projects
-- without an override, and an explicit audit fact for project AI defaults.
CREATE TABLE project_settings_revisions (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991)
) STRICT;
INSERT INTO project_settings_revisions(project_id, revision)
SELECT p.id, max(1, coalesce(max(s.revision), 1))
FROM projects p
LEFT JOIN settings s ON s.scope='project' AND s.project_id=p.id
GROUP BY p.id;
CREATE TRIGGER project_settings_revision_insert AFTER INSERT ON projects
BEGIN
    INSERT INTO project_settings_revisions(project_id, revision) VALUES(NEW.id, 1);
END;

DROP INDEX idx_audit_recent;
DROP INDEX idx_audit_correlation;
ALTER TABLE audit_events RENAME TO audit_events_v5;
CREATE TABLE audit_events (
    id TEXT PRIMARY KEY NOT NULL,
    occurred_at INTEGER NOT NULL CHECK(occurred_at >= 0),
    correlation_id TEXT NOT NULL,
    event_kind TEXT NOT NULL CHECK(event_kind IN ('config_read','config_switch','connection_test','config_rollback','credential_change','approval_decision','task_transition','storage_recovery','project_grant','project_reauthorize','project_revoke','project_invalidated','project_ai_defaults_update')),
    outcome TEXT NOT NULL CHECK(outcome IN ('success','failure','denied','interrupted')),
    error_code TEXT,
    project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    approval_id TEXT REFERENCES approvals(id) ON DELETE SET NULL,
    profile_id TEXT REFERENCES provider_profiles(id) ON DELETE SET NULL,
    model_id TEXT,
    effort TEXT
) STRICT;
INSERT INTO audit_events SELECT * FROM audit_events_v5;
DROP TABLE audit_events_v5;
CREATE INDEX idx_audit_recent ON audit_events(occurred_at DESC);
CREATE INDEX idx_audit_correlation ON audit_events(correlation_id, occurred_at);
CREATE INDEX idx_audit_project ON audit_events(project_id) WHERE project_id IS NOT NULL;
