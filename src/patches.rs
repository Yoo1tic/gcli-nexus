//! Crate-private patch types shared across DB/providers.
//!
//! The `db` module re-exports these so external paths remain stable
//! (e.g. `pollux::db::ProviderPatch`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Abstraction for applying a patch payload/envelope to the database.
///
/// This is intentionally kept in a neutral crate-private module so DB actors,
/// providers, and higher-level orchestrators can share the same contract.
#[async_trait]
pub trait DbPatchable {
    async fn apply_patch(&self, pool: &SqlitePool) -> Result<(), crate::error::PolluxError>;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeminiCliPatch {
    /// `None` => do not change; `Some(v)` => update
    pub email: Option<String>,
    pub refresh_token: Option<String>,
    /// `None` => do not change; `Some(v)` => update
    pub access_token: Option<String>,
    pub expiry: Option<DateTime<Utc>>,
    pub status: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexPatch {
    /// `None` => do not change; `Some(v)` => update
    pub email: Option<String>,
    /// `None` => do not change; `Some(v)` => update
    pub account_id: Option<String>,
    /// `None` => do not change; `Some(v)` => update
    pub sub: Option<String>,
    pub refresh_token: Option<String>,
    /// `None` => do not change; `Some(v)` => update
    pub access_token: Option<String>,
    pub expiry: Option<DateTime<Utc>>,
    /// `None` => do not change; `Some(v)` => update
    pub chatgpt_plan_type: Option<String>,
    pub status: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AntigravityPatch {
    /// `None` => do not change; `Some(v)` => update
    pub email: Option<String>,
    pub refresh_token: Option<String>,
    /// `None` => do not change; `Some(v)` => update
    pub access_token: Option<String>,
    pub expiry: Option<DateTime<Utc>>,
    pub status: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum ProviderPatch {
    GeminiCli { id: u64, patch: GeminiCliPatch },
    Codex { id: u64, patch: CodexPatch },
    Antigravity { id: u64, patch: AntigravityPatch },
}

impl ProviderPatch {
    pub fn id(&self) -> u64 {
        match self {
            ProviderPatch::GeminiCli { id, .. } => *id,
            ProviderPatch::Codex { id, .. } => *id,
            ProviderPatch::Antigravity { id, .. } => *id,
        }
    }
}
