use super::OAUTH_RETRY_POLICY;
use crate::config::AntigravityResolvedConfig;
use crate::error::{IsRetryable, OauthError};
use backon::Retryable;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tracing::warn;

/// Request metadata used by Antigravity's upstream project discovery.
///
/// This matches gcli2api's request shape so it's deterministic and testable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AntigravityMetadata {
    ide_type: &'static str,
    platform: &'static str,
    plugin_type: &'static str,
}

impl Default for AntigravityMetadata {
    fn default() -> Self {
        Self {
            ide_type: "ANTIGRAVITY",
            platform: "PLATFORM_UNSPECIFIED",
            plugin_type: "GEMINI",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadCodeAssistRequest {
    #[serde(default)]
    metadata: AntigravityMetadata,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OnboardUserRequest<'a> {
    tier_id: &'a str,
    #[serde(default)]
    metadata: AntigravityMetadata,
}

/// Minimal typed view of loadCodeAssist response needed for onboarding.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadCodeAssistResponse {
    pub cloudaicompanion_project: Option<String>,
    #[serde(default)]
    pub allowed_tiers: Vec<AllowedTier>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AllowedTier {
    pub id: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

/// Stateless operations layer for Antigravity OAuth + project discovery calls.
pub struct AntigravityOauthOps;

impl AntigravityOauthOps {
    pub fn load_code_assist_body_json() -> Value {
        // Keep this as a Value so tests can assert exact JSON.
        json!({
            "metadata": {
                "ideType": "ANTIGRAVITY",
                "platform": "PLATFORM_UNSPECIFIED",
                "pluginType": "GEMINI"
            }
        })
    }

    fn load_code_assist_url(cfg: &AntigravityResolvedConfig) -> String {
        format!(
            "{}/v1internal:loadCodeAssist",
            cfg.api_url.as_str().trim_end_matches('/')
        )
    }

    fn onboard_user_url(cfg: &AntigravityResolvedConfig) -> String {
        format!(
            "{}/v1internal:onboardUser",
            cfg.api_url.as_str().trim_end_matches('/')
        )
    }

    pub async fn load_code_assist(
        cfg: &AntigravityResolvedConfig,
        access_token: impl AsRef<str>,
        http_client: reqwest::Client,
    ) -> Result<Value, OauthError> {
        let url = Self::load_code_assist_url(cfg);
        let resp = http_client
            .post(url)
            .bearer_auth(access_token.as_ref())
            .json(&LoadCodeAssistRequest {
                metadata: AntigravityMetadata::default(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(OauthError::UpstreamStatus(resp.status()));
        }

        Ok(resp.json::<Value>().await?)
    }

    pub async fn onboard_user(
        cfg: &AntigravityResolvedConfig,
        access_token: impl AsRef<str>,
        tier_id: impl AsRef<str>,
        http_client: reqwest::Client,
    ) -> Result<Value, OauthError> {
        let url = Self::onboard_user_url(cfg);
        let req = OnboardUserRequest {
            tier_id: tier_id.as_ref(),
            metadata: AntigravityMetadata::default(),
        };

        let resp = http_client
            .post(url)
            .bearer_auth(access_token.as_ref())
            .json(&req)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(OauthError::UpstreamStatus(resp.status()));
        }

        Ok(resp.json::<Value>().await?)
    }

    /// loadCodeAssist with network-aware retries.
    pub async fn load_code_assist_with_retry(
        cfg: &AntigravityResolvedConfig,
        access_token: impl AsRef<str>,
        http_client: reqwest::Client,
    ) -> Result<Value, OauthError> {
        let retry_policy = *OAUTH_RETRY_POLICY;
        (|| async { Self::load_code_assist(cfg, access_token.as_ref(), http_client.clone()).await })
            .retry(retry_policy)
            .when(|e: &OauthError| e.is_retryable())
            .notify(|err, dur: Duration| {
                warn!(
                    "antigravity loadCodeAssist retrying after error {}, sleeping {:?}",
                    err, dur
                );
            })
            .await
    }

    /// onboardUser with network-aware retries (no polling here).
    pub async fn onboard_user_with_retry(
        cfg: &AntigravityResolvedConfig,
        access_token: impl AsRef<str>,
        tier_id: impl AsRef<str>,
        http_client: reqwest::Client,
    ) -> Result<Value, OauthError> {
        let retry_policy = *OAUTH_RETRY_POLICY;
        let tier_id = tier_id.as_ref().to_string();
        (|| async {
            Self::onboard_user(
                cfg,
                access_token.as_ref(),
                tier_id.as_str(),
                http_client.clone(),
            )
            .await
        })
        .retry(retry_policy)
        .when(|e: &OauthError| e.is_retryable())
        .notify(|err, dur: Duration| {
            warn!(
                "antigravity onboardUser retrying after error {}, sleeping {:?}",
                err, dur
            );
        })
        .await
    }
}
