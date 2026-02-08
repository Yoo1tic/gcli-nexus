use crate::config::AntigravityResolvedConfig;
use crate::db::{AntigravityCreate, AntigravityPatch};
use crate::error::{OauthError, PolluxError};
use crate::providers::antigravity::client::oauth::{
    endpoints::AntigravityOauthEndpoints,
    ops::{AntigravityOauthOps, LoadCodeAssistResponse},
};
use chrono::{Duration as ChronoDuration, Utc};
use futures::stream::StreamExt;
use governor::{Quota, RateLimiter};
use oauth2::TokenResponse;
use reqwest::header::{CONNECTION, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub(crate) struct AntigravityRefreshTokenSeed {
    refresh_token: String,
}

impl AntigravityRefreshTokenSeed {
    pub fn new(refresh_token: String) -> Option<Self> {
        let refresh_token = refresh_token.trim().to_string();
        if refresh_token.is_empty() {
            return None;
        }
        Some(Self { refresh_token })
    }

    pub fn refresh_token(&self) -> &str {
        self.refresh_token.as_str()
    }
}

#[derive(Debug)]
pub(crate) enum RefreshOutcome {
    /// Refresh an existing DB credential (project_id must already exist).
    RefreshCredential {
        id: u64,
        patch: AntigravityPatch,
        result: Result<(), PolluxError>,
    },

    /// Refresh a 0-trust seed and discover a project_id.
    OnboardSeed {
        seed: AntigravityRefreshTokenSeed,
        result: Result<AntigravityCreate, PolluxError>,
    },
}

#[derive(Debug)]
enum RefreshTask {
    RefreshCredential { id: u64, refresh_token: String },
    OnboardSeed { seed: AntigravityRefreshTokenSeed },
}

impl RefreshTask {
    async fn execute(
        self,
        cfg: Arc<AntigravityResolvedConfig>,
        client: reqwest::Client,
    ) -> RefreshOutcome {
        match self {
            Self::RefreshCredential { id, refresh_token } => {
                let result = refresh_existing(cfg, client, refresh_token.as_str()).await;
                match result {
                    Ok(patch) => RefreshOutcome::RefreshCredential {
                        id,
                        patch,
                        result: Ok(()),
                    },
                    Err(e) => RefreshOutcome::RefreshCredential {
                        id,
                        patch: AntigravityPatch::default(),
                        result: Err(e),
                    },
                }
            }
            Self::OnboardSeed { seed } => {
                let result = refresh_and_discover(cfg, client, &seed).await;
                RefreshOutcome::OnboardSeed { seed, result }
            }
        }
    }
}

/// Handle for submitting refresh/onboarding tasks.
#[derive(Clone, Debug)]
pub(crate) struct AntigravityRefresherHandle {
    job_tx: mpsc::Sender<RefreshTask>,
}

impl AntigravityRefresherHandle {
    pub(crate) async fn submit_refresh(
        &self,
        id: u64,
        refresh_token: String,
    ) -> Result<(), PolluxError> {
        self.job_tx
            .send(RefreshTask::RefreshCredential { id, refresh_token })
            .await
            .map_err(|_| {
                PolluxError::RactorError("antigravity refresh job queue is closed".to_string())
            })
    }

    pub(crate) async fn submit_onboard_seed(
        &self,
        seed: AntigravityRefreshTokenSeed,
    ) -> Result<(), PolluxError> {
        self.job_tx
            .send(RefreshTask::OnboardSeed { seed })
            .await
            .map_err(|_| {
                PolluxError::RactorError("antigravity refresh job queue is closed".to_string())
            })
    }
}

/// Spawn a background refresher pipeline for Antigravity refresh/onboarding.
///
/// This mirrors the geminicli/codex refresher pipeline shape:
/// - governor rate limiter (oauth_tps)
/// - buffer_unordered concurrency
/// - deterministic retry policy inside the ops layer
pub(crate) fn spawn_pipeline(
    cfg: Arc<AntigravityResolvedConfig>,
) -> (AntigravityRefresherHandle, mpsc::Receiver<RefreshOutcome>) {
    let (job_tx, job_rx) = mpsc::channel::<RefreshTask>(1000);
    let (out_tx, out_rx) = mpsc::channel::<RefreshOutcome>(1000);

    let mut headers = HeaderMap::new();
    let mut builder = reqwest::Client::builder()
        .user_agent("antigravity-oauth/1.0".to_string())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30));

    if let Some(proxy_url) = cfg.proxy.clone() {
        let proxy =
            reqwest::Proxy::all(proxy_url.as_str()).expect("invalid proxy url for reqwest client");
        builder = builder.proxy(proxy);
    }

    if !cfg.enable_multiplexing {
        headers.insert(CONNECTION, HeaderValue::from_static("close"));
        builder = builder
            .http1_only()
            .pool_max_idle_per_host(0)
            .pool_idle_timeout(Duration::from_secs(0));
    } else {
        builder = builder.http2_adaptive_window(true);
    }

    let http = builder
        .default_headers(headers)
        .build()
        .expect("FATAL: initialize antigravity refresh HTTP client failed");

    let oauth_tps = cfg.oauth_tps.max(1);
    let oauth_tps_u32 = u32::try_from(oauth_tps).unwrap_or(u32::MAX);
    let burst_u32 = u32::try_from(oauth_tps.saturating_mul(2)).unwrap_or(u32::MAX);
    let limiter = Arc::new(RateLimiter::direct(
        Quota::per_second(std::num::NonZeroU32::new(oauth_tps_u32).unwrap())
            .allow_burst(std::num::NonZeroU32::new(burst_u32).unwrap()),
    ));

    let buffer_unordered = oauth_tps.saturating_mul(2).max(1);
    tokio::spawn({
        let cfg = cfg.clone();
        async move {
            info!(
                "Antigravity Refresh Pipeline Started: BufferUnordered={}, RateLimit={}/s, Burst={}",
                buffer_unordered, oauth_tps_u32, burst_u32
            );

            let mut pipeline = ReceiverStream::new(job_rx)
                .map(|task| {
                    let lim = limiter.clone();
                    let http = http.clone();
                    let cfg = cfg.clone();
                    async move {
                        lim.until_ready().await;
                        task.execute(cfg, http).await
                    }
                })
                .buffer_unordered(buffer_unordered);

            while let Some(outcome) = pipeline.next().await {
                if out_tx.send(outcome).await.is_err() {
                    warn!("Antigravity refresher outcome channel closed; worker stopping");
                    break;
                }
            }

            info!("Antigravity Refresh Pipeline Stopped");
        }
    });

    (AntigravityRefresherHandle { job_tx }, out_rx)
}

async fn refresh_existing(
    cfg: Arc<AntigravityResolvedConfig>,
    http_client: reqwest::Client,
    refresh_token: &str,
) -> Result<AntigravityPatch, PolluxError> {
    // This is for existing DB creds; project_id should already be set at creation.
    let token =
        AntigravityOauthEndpoints::refresh_access_token(&cfg, refresh_token, http_client).await?;

    let access_token = token.access_token().secret().to_string();

    let expires_in = token.expires_in().unwrap_or(Duration::from_secs(3600));
    let expiry = Utc::now()
        + ChronoDuration::from_std(expires_in).unwrap_or_else(|_| ChronoDuration::seconds(3600));

    Ok(AntigravityPatch {
        refresh_token: None,
        email: None,
        access_token: Some(access_token),
        expiry: Some(expiry),
        status: None,
    })
}

async fn refresh_and_discover(
    cfg: Arc<AntigravityResolvedConfig>,
    http_client: reqwest::Client,
    seed: &AntigravityRefreshTokenSeed,
) -> Result<AntigravityCreate, PolluxError> {
    let token = AntigravityOauthEndpoints::refresh_access_token(
        &cfg,
        seed.refresh_token(),
        http_client.clone(),
    )
    .await?;

    let access_token = token.access_token().secret().to_string();

    let expires_in = token.expires_in().unwrap_or(Duration::from_secs(3600));
    let expiry = Utc::now()
        + ChronoDuration::from_std(expires_in).unwrap_or_else(|_| ChronoDuration::seconds(3600));

    let project_id = ensure_project_id(access_token.as_str(), cfg.as_ref(), http_client).await?;

    Ok(AntigravityCreate {
        email: None,
        sub: None,
        project_id,
        refresh_token: seed.refresh_token().to_string(),
        access_token: Some(access_token),
        expiry,
    })
}

async fn ensure_project_id(
    access_token: &str,
    cfg: &AntigravityResolvedConfig,
    http_client: reqwest::Client,
) -> Result<String, PolluxError> {
    let load_json =
        AntigravityOauthOps::load_code_assist_with_retry(cfg, access_token, http_client.clone())
            .await?;
    debug!(body = %load_json, "antigravity loadCodeAssist upstream body");

    let load_resp: LoadCodeAssistResponse =
        serde_json::from_value(load_json.clone()).map_err(PolluxError::JsonError)?;

    if let Some(pid) = load_resp
        .cloudaicompanion_project
        .clone()
        .filter(|s| !s.trim().is_empty())
    {
        return Ok(pid);
    }

    let tier_id = load_resp
        .allowed_tiers
        .iter()
        .find(|t| t.is_default)
        .and_then(|t| t.id.clone())
        .unwrap_or_else(|| "LEGACY".to_string());

    perform_onboarding(access_token, cfg, tier_id.as_str(), http_client).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardUserOperation {
    #[serde(default)]
    done: bool,
    #[serde(default)]
    response: Option<OnboardUserResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardUserResponse {
    #[serde(rename = "cloudaicompanionProject")]
    project: Option<ProjectIdOrObject>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ProjectIdOrObject {
    String(String),
    Object { id: String },
}

impl ProjectIdOrObject {
    fn into_id(self) -> Option<String> {
        match self {
            ProjectIdOrObject::String(s) if !s.trim().is_empty() => Some(s),
            ProjectIdOrObject::Object { id } if !id.trim().is_empty() => Some(id),
            _ => None,
        }
    }
}

async fn perform_onboarding(
    access_token: &str,
    cfg: &AntigravityResolvedConfig,
    tier_id: &str,
    http_client: reqwest::Client,
) -> Result<String, PolluxError> {
    const MAX_ATTEMPTS: usize = 5;
    const RETRY_DELAY: Duration = Duration::from_secs(2);
    let mut last_resp: Option<Value> = None;

    for attempt in 1..=MAX_ATTEMPTS {
        let resp_json = AntigravityOauthOps::onboard_user_with_retry(
            cfg,
            access_token,
            tier_id,
            http_client.clone(),
        )
        .await?;
        debug!(body = %resp_json, "antigravity onboardUser upstream body");
        last_resp = Some(resp_json.clone());

        let op: OnboardUserOperation =
            serde_json::from_value(resp_json.clone()).map_err(PolluxError::JsonError)?;
        if op.done {
            return op
                .response
                .and_then(|r| r.project)
                .and_then(ProjectIdOrObject::into_id)
                .ok_or_else(|| {
                    OauthError::Flow {
                        code: "ONBOARD_FAILED".to_string(),
                        message: "Onboarding completed but returned no project ID".to_string(),
                        details: Some(resp_json),
                    }
                    .into()
                });
        }

        if attempt < MAX_ATTEMPTS {
            info!(
                "antigravity onboardUser pending (attempt {}/{}), retrying in {:?}...",
                attempt, MAX_ATTEMPTS, RETRY_DELAY
            );
            sleep(RETRY_DELAY).await;
        }
    }

    Err(OauthError::Flow {
        code: "ONBOARD_TIMEOUT".to_string(),
        message: "Project provisioning timed out".to_string(),
        details: last_resp,
    }
    .into())
}
