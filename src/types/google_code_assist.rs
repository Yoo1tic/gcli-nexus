use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TierInfo {
    pub id: String,
    pub name: Option<String>,
    pub quota_tier: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IneligibleReason {
    pub reason_code: Option<String>,
    pub reason_message: Option<String>,
    pub tier_id: Option<String>,
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
