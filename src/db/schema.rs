//! SQL DDL for initializing the credential storage.
//! SQLite-first design; can be adapted for other RDBMS.

/// SQLite schema with:
/// - `id` INTEGER PRIMARY KEY AUTOINCREMENT
/// - Core fields mirrored from `GoogleCredential` (client keys are constants and not stored)
/// - `project_id` UNIQUE (creates an index implicitly)
/// - `status` BOOLEAN (stored as INTEGER 0/1)
/// - Separate index on `project_id` kept for clarity/perf (redundant with UNIQUE)
pub const SQLITE_INIT: &str = r#"
CREATE TABLE IF NOT EXISTS credentials (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    email TEXT NULL,
    project_id TEXT NOT NULL UNIQUE,
    refresh_token TEXT NOT NULL,
    access_token TEXT NULL,
    expiry TEXT NOT NULL, -- RFC3339
    status INTEGER NOT NULL DEFAULT 1
);

-- Redundant non-unique index on project_id (UNIQUE already creates one).
-- Kept to meet the explicit requirements on indexing clarity.
CREATE INDEX IF NOT EXISTS idx_credentials_project_id ON credentials(project_id);
"#;
