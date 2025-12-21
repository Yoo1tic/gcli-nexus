use super::endpoints::GoogleOauthEndpoints;
use crate::config::OAUTH_RETRY_POLICY;
use crate::error::{IsRetryable, NexusError};
use crate::types::google_code_assist::UserTier;
use backon::Retryable;
use serde_json::Value;
use std::time::Duration;
use tracing::warn;

/// Stateless operations layer to compose Google OAuth requests.
pub struct GoogleOauthOps;

impl GoogleOauthOps {
    /// Call loadCodeAssist with network-aware retries.
    pub async fn load_code_assist_with_retry(
        access_token: impl AsRef<str>,
        http_client: reqwest::Client,
    ) -> Result<Value, NexusError> {
        let retry_policy = OAUTH_RETRY_POLICY.clone();

        (|| async {
            GoogleOauthEndpoints::load_code_assist(access_token.as_ref(), http_client.clone()).await
        })
        .retry(retry_policy)
        .when(|e: &NexusError| e.is_retryable())
        .notify(|err, dur: Duration| {
            warn!(
                "loadCodeAssist retrying after error {}, sleeping {:?}",
                err, dur
            );
        })
        .await
    }

    /// Provision a companion project with network-aware retries (no polling).
    pub async fn onboard_code_assist_with_retry(
        access_token: impl AsRef<str>,
        tier: UserTier,
        cloudaicompanion_project: Option<String>,
        http_client: reqwest::Client,
    ) -> Result<Value, NexusError> {
        let retry_policy = OAUTH_RETRY_POLICY.clone();

        (|| async {
            GoogleOauthEndpoints::onboard_code_assist(
                access_token.as_ref(),
                tier.clone(),
                cloudaicompanion_project.clone(),
                http_client.clone(),
            )
            .await
        })
        .retry(retry_policy)
        .when(|e: &NexusError| e.is_retryable())
        .notify(|err, dur: Duration| {
            warn!(
                "onboardCodeAssist retrying after error {}, sleeping {:?}",
                err, dur
            );
        })
        .await
    }
}
