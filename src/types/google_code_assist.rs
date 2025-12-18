use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TierInfo {
    pub id: String,
    pub name: Option<String>,
    pub quota_tier: Option<UserTier>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IneligibleReason {
    pub reason_code: Option<String>,
    pub reason_message: Option<String>,
    pub tier_id: Option<UserTier>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LoadCodeAssistResponse {
    pub current_tier: Option<TierInfo>,
    pub cloudaicompanion_project: Option<String>,
    #[serde(default)]
    pub allowed_tiers: Vec<TierInfo>,
    #[serde(default)]
    pub ineligible_tiers: Vec<IneligibleReason>,
}

impl LoadCodeAssistResponse {
    pub fn resolve_effective_tier(&self) -> UserTier {
        self.current_tier
            .as_ref()
            .and_then(|t| t.quota_tier.clone())
            .or_else(|| {
                self.allowed_tiers
                    .iter()
                    .find(|t| t.is_default)
                    .and_then(|t| t.quota_tier.clone())
            })
            .unwrap_or(UserTier::Legacy)
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectObject {
    pub id: String,
    pub name: Option<String>,
    pub project_number: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OnboardResultPayload {
    #[serde(rename = "cloudaicompanionProject")]
    pub project_details: Option<ProjectObject>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OnboardOperationResponse {
    pub name: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub response: Option<OnboardResultPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
pub enum UserTier {
    Free,
    Legacy,
    Standard,
    Other(String),
}

impl From<String> for UserTier {
    fn from(raw: String) -> Self {
        match raw.as_str() {
            "free-tier" => UserTier::Free,
            "legacy-tier" => UserTier::Legacy,
            "standard-tier" => UserTier::Standard,
            other => UserTier::Other(other.to_string()),
        }
    }
}

impl From<UserTier> for String {
    fn from(tier: UserTier) -> Self {
        match tier {
            UserTier::Free => "free-tier".to_string(),
            UserTier::Legacy => "legacy-tier".to_string(),
            UserTier::Standard => "standard-tier".to_string(),
            UserTier::Other(value) => value,
        }
    }
}

impl UserTier {
    pub fn as_str(&self) -> &str {
        match self {
            UserTier::Free => "free-tier",
            UserTier::Legacy => "legacy-tier",
            UserTier::Standard => "standard-tier",
            UserTier::Other(value) => value.as_str(),
        }
    }
}
