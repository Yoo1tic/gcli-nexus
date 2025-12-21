use crate::{
    config::{CONFIG, OAUTH_RETRY_POLICY},
    error::{IsRetryable, NexusError},
    google_oauth::{
        credentials::GoogleCredential, endpoints::GoogleOauthEndpoints, ops::GoogleOauthOps,
        utils::attach_email_from_id_token,
    },
    service::{credential_manager::CredentialId, credentials_actor::CredentialsHandle},
    types::google_code_assist::LoadCodeAssistResponse,
};
use backon::{ExponentialBuilder, Retryable};
use futures::stream::StreamExt;
use governor::{Quota, RateLimiter};
use reqwest::header::{CONNECTION, HeaderMap, HeaderValue};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info, warn};

#[derive(Debug)]
pub enum JobInstruction {
    Maintain {
        id: CredentialId,
        cred: GoogleCredential,
    },
    Onboard {
        cred: GoogleCredential,
    },
}

#[derive(Debug)]
pub enum RefreshOutcome {
    Success(JobInstruction),
    Failed(JobInstruction, NexusError),
}

impl JobInstruction {
    pub async fn execute(&mut self, client: reqwest::Client) -> Result<(), NexusError> {
        let retry_policy = OAUTH_RETRY_POLICY.clone();

        match self {
            Self::Maintain { cred, .. } => {
                refresh_inner(client, retry_policy, cred).await?;
            }

            Self::Onboard { cred } => {
                refresh_inner(client.clone(), retry_policy, cred).await?;
                let token_str = cred.access_token.as_deref().ok_or_else(|| {
                    NexusError::RactorError("Refresh success but token is None".to_string())
                })?;
                let load_json =
                    GoogleOauthOps::load_code_assist_with_retry(token_str, client).await?;
                let load_resp: LoadCodeAssistResponse =
                    serde_json::from_value(load_json).map_err(NexusError::JsonError)?;
                if let Some(existing_project_id) = load_resp.cloudaicompanion_project {
                    tracing::info!("Onboard: Found Project ID {}", existing_project_id);
                    cred.project_id = existing_project_id;
                } else {
                    return Err(NexusError::UnexpectedError(
                        "Onboard: missing cloudaicompanion_project".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

// Refresh pipeline tuning moved to Config.refresh_concurrency.
pub struct RefreshJobService {
    job_tx: mpsc::Sender<JobInstruction>,
}

impl RefreshJobService {
    /// Create a new refresh pipeline with a preconfigured HTTP client.
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
            .expect("FATAL: initialize refresh job HTTP client failed");
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
                .buffer_unordered(refresh_concurrency);

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
