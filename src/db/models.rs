use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct DbGeminiCliResource {
    pub id: i64,
    pub email: Option<String>,
    pub sub: String,
    pub project_id: String,
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expiry: DateTime<Utc>,
    pub status: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct DbCodexResource {
    pub id: i64,
    pub email: Option<String>,
    pub sub: String,
    pub account_id: String,
    pub refresh_token: String,
    pub access_token: String,
    pub expiry: DateTime<Utc>,
    pub chatgpt_plan_type: Option<String>,
    pub status: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct DbAntigravityResource {
    pub id: i64,
    pub email: Option<String>,
    /// Stable unique key (real subject if available, otherwise synthetic).
    pub sub: String,
    pub project_id: String,
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expiry: DateTime<Utc>,
    pub status: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
