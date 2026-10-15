-- Additive migration: schema v1 and its checksum remain unchanged.
CREATE TABLE settings_revisions (
    scope TEXT PRIMARY KEY NOT NULL CHECK (scope = 'global'),
    revision INTEGER NOT NULL CHECK (revision >= 1 AND revision <= 9007199254740991)
) STRICT;
INSERT INTO settings_revisions SELECT 'global', max(1, coalesce(max(revision), 1)) FROM settings WHERE scope = 'global';

CREATE TABLE resident_preferences (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    shortcut TEXT NOT NULL CHECK (length(shortcut) BETWEEN 1 AND 80),
    x INTEGER,
    y INTEGER,
    monitor TEXT CHECK (length(monitor) <= 256),
    scale_factor REAL,
    CHECK ((x IS NULL AND y IS NULL AND monitor IS NULL AND scale_factor IS NULL) OR
           (x IS NOT NULL AND y IS NOT NULL AND scale_factor BETWEEN 0.25 AND 8))
) STRICT;
