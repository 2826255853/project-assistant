-- S1-07. Existing migrations and their checksums are immutable.
CREATE TABLE ai_default_revisions (
    setting_id TEXT PRIMARY KEY REFERENCES settings(id) ON DELETE CASCADE,
    profile_revision INTEGER NOT NULL CHECK(profile_revision >= 1)
) STRICT;
-- Legacy defaults remain declarations. Backfill their revision, never ready evidence.
INSERT INTO ai_default_revisions(setting_id,profile_revision)
SELECT s.id,p.config_revision FROM settings s JOIN provider_profiles p ON p.id=s.profile_id WHERE s.key='ai_defaults';
CREATE TABLE ai_runtime (
    id INTEGER PRIMARY KEY CHECK(id=1),
    blocked INTEGER NOT NULL DEFAULT 0 CHECK(blocked IN (0,1)),
    profile_id TEXT REFERENCES provider_profiles(id) ON DELETE RESTRICT,
    profile_revision INTEGER, model_id TEXT, effort TEXT,
    credential_revision INTEGER, adapter_version TEXT, tested_at INTEGER,
    capability_version TEXT, effective_effort TEXT,
    CHECK((profile_id IS NULL AND profile_revision IS NULL AND model_id IS NULL AND effort IS NULL AND credential_revision IS NULL AND adapter_version IS NULL AND tested_at IS NULL) OR
          (profile_id IS NOT NULL AND profile_revision >= 1 AND model_id IS NOT NULL AND credential_revision >= 1 AND adapter_version IS NOT NULL AND tested_at >= 0))
) STRICT;
INSERT INTO ai_runtime(id) VALUES(1);
CREATE TABLE provider_operations (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('test','switch')),
    profile_id TEXT NOT NULL, profile_revision INTEGER NOT NULL CHECK(profile_revision >= 1),
    model_id TEXT NOT NULL, effort TEXT, correlation_id TEXT NOT NULL,
    phase TEXT NOT NULL CHECK(phase IN ('checking','waiting_for_idle','applying','rolling_back','completed','failed','cancelled')),
    outcome TEXT CHECK(outcome IN ('applied','unchanged','rolled_back','rollback_failed')),
    error_code TEXT, tested_at INTEGER, credential_revision INTEGER, adapter_version TEXT,
    capability_version TEXT, effective_effort TEXT,
    active_profile_id TEXT, active_revision INTEGER, active_model_id TEXT, active_effort TEXT,
    config_state TEXT CHECK(config_state IN ('unconfigured','checking','ready','switching','degraded','error')),
    settings_revision INTEGER NOT NULL CHECK(settings_revision >= 1),
    created_at INTEGER NOT NULL CHECK(created_at >= 0), finished_at INTEGER
) STRICT;
CREATE INDEX idx_provider_operations_pending ON provider_operations(phase,created_at);
