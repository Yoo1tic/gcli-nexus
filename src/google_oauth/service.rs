use super::endpoints::GoogleOauthEndpoints;
use crate::google_oauth::credentials::GoogleCredential;
use crate::google_oauth::utils::attach_email_from_id_token;
use crate::{
    config::CONFIG,
    error::{IsRetryable, NexusError},
    service::credentials_actor::CredentialsHandle,
    types::google_code_assist::UserTier,
    types::job::{JobInstruction, RefreshOutcome},
};
use backon::{ExponentialBuilder, Retryable};
use futures::stream::StreamExt;
use governor::{Quota, RateLimiter};
use reqwest::header::{CONNECTION, HeaderMap, HeaderValue};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
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
    job_tx: mpsc::Sender<JobInstruction>,
}

impl GoogleOauthService {
    /// Create a new service with a preconfigured HTTP client.
    pub fn new(handle: CredentialsHandle) -> Self {
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
        let limiter = Arc::new(RateLimiter::direct(
            Quota::per_minute(std::num::NonZeroU32::new(30).unwrap())
                .allow_burst(std::num::NonZeroU32::new(10).unwrap()),
        ));

        let (job_tx, job_rx) = mpsc::channel::<JobInstruction>(1000);
        let handle = handle.clone();

        // Spawn background refresh worker using buffer_unordered semantics.
        // Extra refresh requests will queue in the channel (unbounded).
        let refresh_concurrency = CONFIG.refresh_concurrency.max(1);
        tokio::spawn(async move {
            info!(
                "Refresh Pipeline Started: Concurrency={}, RateLimit=10/min",
                refresh_concurrency
            );

            let mut pipeline = ReceiverStream::new(job_rx)
                .map(|mut instruction| {
                    let lim = limiter.clone();
                    let http = client.clone();
                    async move {
                        lim.until_ready().await;

                        match instruction.execute(http).await {
                            Ok(()) => RefreshOutcome::Success(instruction),
                            Err(e) => RefreshOutcome::Failed(instruction, e),
                        }
                    }
                })
                .buffer_unordered(refresh_concurrency); // C. 并发控制

            while let Some(outcome) = pipeline.next().await {
                if let Err(e) = handle.send_refresh_complete(outcome) {
                    warn!("Actor unreachable (channel closed), worker stopping: {}", e);
                    break;
                }
            }
            info!("Refresh Pipeline Stopped");
        });

        Self { job_tx }
    }

    pub fn job_tx(&self) -> mpsc::Sender<JobInstruction> {
        self.job_tx.clone()
    }

    pub async fn submit(&self, job: JobInstruction) {
        let tx = self.job_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = tx.send(job).await {
                warn!("Failed to submit refresh job (channel closed/full): {}", e);
            }
        });
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

/// Shared refresh implementation so both direct calls and the background
/// worker use the same logic.
pub async fn refresh_inner(
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
