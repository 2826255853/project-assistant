-- S2-03: append only. Legacy rows have no grant and require a new picker selection.
ALTER TABLE projects ADD COLUMN directory_identity TEXT
    CHECK(directory_identity IS NULL OR (length(directory_identity)=48 AND directory_identity NOT GLOB '*[^0-9A-F]*'));
ALTER TABLE projects ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision BETWEEN 1 AND 9007199254740991);
ALTER TABLE projects ADD COLUMN authorization TEXT NOT NULL DEFAULT 'requires_reauthorization'
    CHECK(authorization IN ('granted','requires_reauthorization','revoked'));
ALTER TABLE projects ADD COLUMN availability TEXT NOT NULL DEFAULT 'identity_changed'
    CHECK(availability IN ('available','missing','inaccessible','identity_changed'));
ALTER TABLE projects ADD COLUMN removed_at INTEGER CHECK(removed_at IS NULL OR removed_at >= created_at);
CREATE UNIQUE INDEX idx_projects_identity ON projects(directory_identity) WHERE directory_identity IS NOT NULL AND removed_at IS NULL;
CREATE INDEX idx_projects_visible ON projects(removed_at, id);
CREATE TRIGGER project_grant_insert BEFORE INSERT ON projects
WHEN (NEW.authorization='granted' AND NEW.directory_identity IS NULL)
  OR ((NEW.authorization='revoked') != (NEW.removed_at IS NOT NULL))
BEGIN SELECT RAISE(ABORT,'PROJECT_GRANT_INVALID'); END;
CREATE TRIGGER project_grant_update BEFORE UPDATE ON projects
WHEN (NEW.authorization='granted' AND NEW.directory_identity IS NULL)
  OR ((NEW.authorization='revoked') != (NEW.removed_at IS NOT NULL))
BEGIN SELECT RAISE(ABORT,'PROJECT_GRANT_INVALID'); END;

-- Keep the existing, nullable history references and all old audit facts.
DROP INDEX idx_audit_recent;
DROP INDEX idx_audit_correlation;
ALTER TABLE audit_events RENAME TO audit_events_v4;
CREATE TABLE audit_events (
    id TEXT PRIMARY KEY NOT NULL,
    occurred_at INTEGER NOT NULL CHECK(occurred_at >= 0),
    correlation_id TEXT NOT NULL,
    event_kind TEXT NOT NULL CHECK(event_kind IN ('config_read','config_switch','connection_test','config_rollback','credential_change','approval_decision','task_transition','storage_recovery','project_grant','project_reauthorize','project_revoke','project_invalidated')),
    outcome TEXT NOT NULL CHECK(outcome IN ('success','failure','denied','interrupted')),
    error_code TEXT,
    project_id TEXT REFERENCES projects(id) ON DELETE SET NULL,
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    approval_id TEXT REFERENCES approvals(id) ON DELETE SET NULL,
    profile_id TEXT REFERENCES provider_profiles(id) ON DELETE SET NULL,
    model_id TEXT,
    effort TEXT
) STRICT;
INSERT INTO audit_events SELECT * FROM audit_events_v4;
DROP TABLE audit_events_v4;
CREATE INDEX idx_audit_recent ON audit_events(occurred_at DESC);
CREATE INDEX idx_audit_correlation ON audit_events(correlation_id, occurred_at);
CREATE INDEX idx_audit_project ON audit_events(project_id) WHERE project_id IS NOT NULL;
