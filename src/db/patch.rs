use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// Re-export patch payload/envelope types from the neutral crate-private module.
// This keeps `pollux::db::{ProviderPatch, GeminiCliPatch, CodexPatch}` stable,
// and also preserves `pollux::db::patch::ProviderPatch`.
pub use crate::patches::{AntigravityPatch, CodexPatch, GeminiCliPatch, ProviderPatch};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCliCreate {
    pub email: Option<String>,
    pub sub: String,
    pub project_id: String,
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expiry: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCreate {
    pub email: Option<String>,
    pub sub: String,
    pub account_id: String,
    pub refresh_token: String,
    pub access_token: String,
    pub expiry: DateTime<Utc>,
    pub chatgpt_plan_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityCreate {
    pub email: Option<String>,
    /// May be missing depending on upstream/OAuth flow; DbActor will synthesize a stable value.
    pub sub: Option<String>,
    pub project_id: String,
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expiry: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum ProviderCreate {
    GeminiCli(GeminiCliCreate),
    Codex(CodexCreate),
    Antigravity(AntigravityCreate),
}
