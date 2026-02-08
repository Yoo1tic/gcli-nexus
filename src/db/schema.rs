//! SQL DDL for initializing the database schema.
//! SQLite-first design; can be adapted for other RDBMS.

/// SQLite schema includes:
/// - `gemini_cli` table (Gemini CLI provider, one (sub, project_id) per row)
/// - `codex` table (Codex provider, one (sub, account_id) per row)
/// - `antigravity` table (Antigravity provider, one (sub, project_id) per row)
pub const SQLITE_INIT: &str = r#"
-- ---------------------------------------------------------------------------
-- Gemini CLI provider
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS gemini_cli (
    id INTEGER PRIMARY KEY NOT NULL,
    email TEXT NULL,
    sub TEXT NOT NULL,
    project_id TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    access_token TEXT NULL,
    expiry TEXT NOT NULL, -- RFC3339
    status INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL, -- RFC3339
    updated_at TEXT NOT NULL, -- RFC3339
    UNIQUE(sub, project_id)
);

CREATE INDEX IF NOT EXISTS idx_gemini_cli_status ON gemini_cli(status);

-- ---------------------------------------------------------------------------
-- Codex provider (one (sub, account_id) per row)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS codex (
    id INTEGER PRIMARY KEY NOT NULL,
    email TEXT NULL,
    sub TEXT NOT NULL,
    account_id TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    access_token TEXT NOT NULL,
    expiry TEXT NOT NULL, -- RFC3339
    chatgpt_plan_type TEXT NULL,
    status INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL, -- RFC3339
    updated_at TEXT NOT NULL, -- RFC3339
    UNIQUE(sub, account_id)
);

CREATE INDEX IF NOT EXISTS idx_codex_status ON codex(status);

-- ---------------------------------------------------------------------------
-- Antigravity provider (one (sub, project_id) per row)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS antigravity (
    id INTEGER PRIMARY KEY NOT NULL,
    email TEXT NULL,
    sub TEXT NOT NULL,
    project_id TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    access_token TEXT NULL,
    expiry TEXT NOT NULL, -- RFC3339
    status INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL, -- RFC3339
    updated_at TEXT NOT NULL, -- RFC3339
    UNIQUE(sub, project_id)
);

CREATE INDEX IF NOT EXISTS idx_antigravity_status ON antigravity(status);
"#;
