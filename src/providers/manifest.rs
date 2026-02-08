use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    GeminiCli,
    Codex,
    Antigravity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCliProfile {
    pub refresh_token: String,
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexProfile {
    pub account_id: String,
    pub sub: String,
    pub refresh_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chatgpt_plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityProfile {
    pub refresh_token: String,
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum ProfileInner {
    GeminiCli(GeminiCliProfile),
    Codex(CodexProfile),
    Antigravity(AntigravityProfile),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub inner: ProfileInner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCliLease {
    pub id: u64,
    pub access_token: String,
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexLease {
    pub id: u64,
    pub access_token: String,
    pub account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityLease {
    pub id: u64,
    pub access_token: String,
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum ProviderLease {
    GeminiCli(GeminiCliLease),
    Codex(CodexLease),
    Antigravity(AntigravityLease),
}

impl ProviderLease {
    pub fn kind(&self) -> ProviderKind {
        match self {
            ProviderLease::GeminiCli(_) => ProviderKind::GeminiCli,
            ProviderLease::Codex(_) => ProviderKind::Codex,
            ProviderLease::Antigravity(_) => ProviderKind::Antigravity,
        }
    }

    pub fn id(&self) -> u64 {
        match self {
            ProviderLease::GeminiCli(l) => l.id,
            ProviderLease::Codex(l) => l.id,
            ProviderLease::Antigravity(l) => l.id,
        }
    }
}
