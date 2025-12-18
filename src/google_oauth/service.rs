use super::endpoints::GoogleOauthEndpoints;
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use crate::google_oauth::utils::attach_email_from_id_token;
use crate::types::google_code_assist::UserTier;
use crate::{config::CONFIG, error::IsRetryable};
use backon::{ExponentialBuilder, Retryable};
use futures::stream::{self, StreamExt};
use reqwest::header::{CONNECTION, HeaderMap, HeaderValue};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

// Refresh pipeline tuning moved to Config.refresh_concurrency.

fn default_retry_policy() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_secs(1))
        .with_max_delay(Duration::from_secs(3))
        .with_max_times(3)
        .with_jitter()
}

/// Service layer to compose Google OAuth operations.
pub struct GoogleOauthService {
    refresh_tx: mpsc::UnboundedSender<RefreshJob>,
}

impl Default for GoogleOauthService {
    fn default() -> Self {
        Self::new()
    }
}

impl GoogleOauthService {
    /// Create a new service with a preconfigured HTTP client.
    pub fn new() -> Self {
        let mut headers = HeaderMap::new();
        let mut builder = reqwest::Client::builder()
            .user_agent("geminicli-oauth/1.0".to_string())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15));
        if let Some(proxy_url) = CONFIG.proxy.clone() {
            let proxy = reqwest::Proxy::all(proxy_url.as_str())
                .expect("invalid PROXY url for reqwest client");
            builder = builder.proxy(proxy);
        }
        if !CONFIG.enable_multiplexing {
            headers.insert(CONNECTION, HeaderValue::from_static("close"));

            builder = builder
                .http1_only()
                .pool_max_idle_per_host(0)
                .pool_idle_timeout(Duration::from_secs(0));
        } else {
            builder = builder.http2_adaptive_window(true);
        }
        let client = builder
            .default_headers(headers)
            .build()
            .expect("FATAL: initialize GoogleOauthService HTTP client failed");
        let retry_policy = default_retry_policy();

        // Refresh pipeline: unbounded channel + concurrent worker
        let (refresh_tx, refresh_rx) = mpsc::unbounded_channel::<RefreshJob>();

        // Spawn background refresh worker using buffer_unordered semantics.
        // Extra refresh requests will queue in the channel (unbounded).
        let refresh_concurrency = CONFIG.refresh_concurrency.max(1);
        tokio::spawn(async move {
            info!(
                unbounded = true,
                concurrency = refresh_concurrency,
                "Refresh worker started"
            );
            let stream = stream::unfold(refresh_rx, |mut rx| async move {
                let item = rx.recv().await;
                item.map(|job| (job, rx))
            });

            stream
                .map(move |job| {
                    let client = client.clone();
                    let policy = retry_policy;
                    async move {
                        let mut cred = job.cred;
                        let res = refresh_inner(client, policy, &mut cred).await.map(|_| cred);
                        let is_success = res.is_ok();
                        if let Err(e) = job.respond_to.send(res) {
                            warn!(?e, "refresh result receiver dropped");
                        }
                        if is_success {
                            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                        }
                    }
                })
                .buffer_unordered(refresh_concurrency)
                .for_each(|_| async {})
                .await;
            info!("Refresh worker stopped (channel closed)");
        });

        Self { refresh_tx }
    }

    /// Get a clone of the refresh job sender.
    pub fn refresh_tx(&self) -> mpsc::UnboundedSender<RefreshJob> {
        self.refresh_tx.clone()
    }

    /// Enqueue a refresh job and await the result.
    pub async fn queue_refresh(
        &self,
        cred: GoogleCredential,
    ) -> Result<GoogleCredential, NexusError> {
        let (tx, rx) = oneshot::channel();
        self.refresh_tx
            .send(RefreshJob {
                cred,
                respond_to: tx,
            })
            .map_err(|e| NexusError::RactorError(format!("send refresh job failed: {}", e)))?;
        rx.await
            .map_err(|e| NexusError::RactorError(format!("recv refresh result failed: {}", e)))?
    }

    /// Call loadCodeAssist with network-aware retries.
    pub async fn load_code_assist_with_retry(
        access_token: impl AsRef<str>,
        http_client: reqwest::Client,
    ) -> Result<Value, NexusError> {
        let retry_policy = default_retry_policy();

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
        let retry_policy = default_retry_policy();

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

/// Refresh request item used by the background refresh pipeline.
#[derive(Debug)]
pub struct RefreshJob {
    pub cred: GoogleCredential,
    pub respond_to: oneshot::Sender<Result<GoogleCredential, NexusError>>,
}

/// Shared refresh implementation so both direct calls and the background
/// worker use the same logic.
async fn refresh_inner(
    client: reqwest::Client,
    retry_policy: ExponentialBuilder,
    creds: &mut GoogleCredential,
) -> Result<(), NexusError> {
    let payload =
        (|| async { GoogleOauthEndpoints::refresh_access_token(creds, client.clone()).await })
            .retry(retry_policy)
            .when(|e: &NexusError| e.is_retryable())
            .notify(|err, dur: Duration| {
                error!(
                    "Google Oauth2 Retrying Error {} with sleeping {:?}",
                    err.to_string(),
                    dur
                );
            })
            .await?;
    let mut payload: Value = serde_json::to_value(&payload)?;
    debug!("Token response payload: {}", payload);
    attach_email_from_id_token(&mut payload);
    creds.update_credential(&payload)?;
    Ok(())
}
