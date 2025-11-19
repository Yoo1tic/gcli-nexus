use super::endpoints::GoogleOauthEndpoints;
use crate::config::CONFIG;
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use backon::{ExponentialBuilder, Retryable};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::stream::{self, StreamExt};
use reqwest::StatusCode;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

// Refresh pipeline tuning moved to Config.refresh_concurrency.

/// Service layer to compose Google OAuth operations.
pub struct GoogleOauthService {
    client: reqwest::Client,
    retry_policy: ExponentialBuilder,
    refresh_tx: mpsc::UnboundedSender<RefreshJob>,
}

impl GoogleOauthService {
    /// Create a new service with a preconfigured HTTP client.
    pub fn new() -> Self {
        let mut builder = reqwest::Client::builder()
            .user_agent("geminicli-oauth/1.0".to_string())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15));
        if let Some(proxy_url) = CONFIG.proxy.clone() {
            let proxy = reqwest::Proxy::all(proxy_url.as_str())
                .expect("invalid PROXY url for GoogleOauthService");
            builder = builder.proxy(proxy);
        }
        if !CONFIG.enable_multiplexing {
            builder = builder.http1_only().pool_max_idle_per_host(0);
        }
        let client = builder
            .build()
            .expect("FATAL: initialize GoogleOauthService HTTP client failed");
        let retry_policy = ExponentialBuilder::default()
            .with_min_delay(Duration::from_secs(3))
            .with_max_delay(Duration::from_secs(5))
            .with_max_times(2);

        // Refresh pipeline: unbounded channel + concurrent worker
        let (refresh_tx, refresh_rx) = mpsc::unbounded_channel::<RefreshJob>();

        // Spawn background refresh worker using buffer_unordered semantics.
        // Extra refresh requests will queue in the channel (unbounded).
        let client_cloned = client.clone();
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

        Self {
            client: client_cloned,
            retry_policy,
            refresh_tx,
        }
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

    /// Refresh access token and update the given credentials.
    pub async fn refresh_credentials(
        &self,
        creds: &mut GoogleCredential,
    ) -> Result<(), NexusError> {
        refresh_inner(self.client.clone(), self.retry_policy, creds).await
    }

    /// Fetch userinfo and update the credential's email field using `update_credential`.
    pub async fn update_email_from_userinfo(
        &self,
        creds: &mut GoogleCredential,
    ) -> Result<(), NexusError> {
        let resp =
            (|| async { GoogleOauthEndpoints::fetch_userinfo(creds, self.client.clone()).await })
                .retry(self.retry_policy)
                .when(|e: &NexusError| match e {
                    // Do NOT retry on 401/403; retry on other reqwest errors
                    NexusError::ReqwestError(err) => !matches!(
                        err.status(),
                        Some(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
                    ),
                    // Retry all other errors
                    _ => true,
                })
                .await?;
        let status = resp.status();
        let body = resp.text().await?;
        debug!(status = %status, body = %body, "Userinfo response");
        let v: serde_json::Value = serde_json::from_str(&body)?;
        let email = v
            .get("email")
            .and_then(|x| x.as_str())
            .ok_or(crate::error::NexusError::MissingEmailInUserinfo)?
            .to_string();

        let payload = serde_json::json!({"email": email});
        creds.update_credential(&payload)?;
        Ok(())
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
            .when(|e: &NexusError| match e {
                // reqwest::Error retryable
                NexusError::ReqwestError(_) => true,
                NexusError::UnexpectedError(_) => true,
                // ServerResponse do not retry
                NexusError::Oauth2Server { .. } => false,
                // other errors do not retry
                _ => false,
            })
            .await?;
    let mut payload: Value = serde_json::to_value(&payload)?;
    debug!("Token response payload: {}", payload);
    if let Some(email) = payload
        .get("id_token")
        .and_then(|t| t.as_str())
        .and_then(|token| token.split('.').nth(1))
        .and_then(|payload_b64| URL_SAFE_NO_PAD.decode(payload_b64).ok())
        .and_then(|decoded| serde_json::from_slice::<Value>(&decoded).ok())
        .and_then(|payload_json| {
            payload_json
                .get("email")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        })
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert("email".to_string(), Value::String(email));
    }
    creds.update_credential(&payload)?;
    Ok(())
}

impl Default for GoogleOauthService {
    fn default() -> Self {
        Self::new()
    }
}
