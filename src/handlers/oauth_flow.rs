use crate::google_oauth::credentials::GoogleCredential;
use crate::google_oauth::endpoints::GoogleOauthEndpoints;
use crate::google_oauth::service::GoogleOauthService;
use crate::google_oauth::utils::attach_email_from_id_token;
use crate::types::google_code_assist::{
    LoadCodeAssistResponse, OnboardOperationResponse, UserTier,
};
use crate::{NexusError, router::NexusState};
use axum::{
    Json,
    extract::{Query, State},
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::cookie::{Cookie, PrivateCookieJar, SameSite};
use oauth2::{AuthorizationCode, PkceCodeChallenge, PkceCodeVerifier};
use reqwest::Client;
use serde::Deserialize;
use time::Duration;
use tokio::time::{Duration as TokioDuration, sleep};
use tracing::{debug, error, info};

const CSRF_COOKIE: &str = "oauth_csrf_token";
const PKCE_COOKIE: &str = "oauth_pkce_verifier";

#[derive(Debug, Deserialize)]
pub struct AuthCallbackQuery {
    pub code: String,
    pub state: String,
}
/// GET /auth
pub async fn google_oauth_entry(jar: PrivateCookieJar) -> Result<impl IntoResponse, NexusError> {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_token) = GoogleOauthEndpoints::build_authorize_url(challenge);

    let jar = jar
        .add(build_cookie(CSRF_COOKIE, csrf_token.secret().to_string()))
        .add(build_cookie(PKCE_COOKIE, verifier.secret().to_string()));

    info!("Dispatching OAuth redirect to: {}", auth_url);

    Ok((jar, Redirect::temporary(auth_url.as_ref())).into_response())
}

/// GET /auth/callback
pub async fn google_oauth_callback(
    State(state): State<NexusState>,
    Query(query): Query<AuthCallbackQuery>,
    jar: PrivateCookieJar,
) -> impl IntoResponse {
    let (jar, session_data) = take_oauth_cookies(jar);

    let result = process_oauth_exchange(&state, &query, session_data).await;

    match result {
        Ok(credential) => {
            info!("OAuth callback stored credential successfully");
            (jar, Json(credential)).into_response()
        }
        Err(err) => {
            error!("OAuth failure: {:?}", err);
            (jar, err.into_response()).into_response()
        }
    }
}

async fn process_oauth_exchange(
    state: &NexusState,
    query: &AuthCallbackQuery,
    session_data: Option<(String, String)>,
) -> Result<GoogleCredential, NexusError> {
    let (pkce_verifier, csrf_token) = session_data.ok_or_else(|| NexusError::OauthFlowError {
        code: "OAUTH_SESSION_MISSING".to_string(),
        message: "Missing OAuth session cookies".to_string(),
        details: None,
    })?;

    if query.state != csrf_token {
        return Err(NexusError::OauthFlowError {
            code: "CSRF_MISMATCH".to_string(),
            message: "CSRF token mismatch".to_string(),
            details: None,
        });
    }

    let token_response = GoogleOauthEndpoints::exchange_authorization_code(
        AuthorizationCode::new(query.code.clone()),
        PkceCodeVerifier::new(pkce_verifier),
        state.client.clone(),
    )
    .await
    .map_err(|e| NexusError::OauthFlowError {
        code: "TOKEN_EXCHANGE_FAILED".to_string(),
        message: format!("Token exchange failed: {}", e),
        details: None,
    })?;

    let mut token_value = serde_json::to_value(&token_response).map_err(NexusError::JsonError)?;

    attach_email_from_id_token(&mut token_value);

    let mut credential = GoogleCredential::from_payload(&token_value)?;

    if credential.refresh_token.is_empty() {
        return Err(NexusError::OauthFlowError {
            code: "MISSING_REFRESH_TOKEN".to_string(),
            message: "Missing refresh_token (check access_type=offline)".to_string(),
            details: None,
        });
    }
    let access_token = credential
        .access_token
        .clone()
        .ok_or(NexusError::MissingAccessToken)?;

    let (project_id, tier) = ensure_companion_project(access_token.as_str(), &state.client).await?;

    credential.project_id = project_id.clone();
    info!(
        project_id = %project_id,
        tier = %tier.as_str(),
        "loadCodeAssist resolved companion project id"
    );

    state
        .handle
        .submit_credentials(vec![credential.clone()])
        .await;

    Ok(credential)
}

async fn ensure_companion_project(
    access_token: &str,
    client: &Client,
) -> Result<(String, UserTier), NexusError> {
    let load_json =
        GoogleOauthService::load_code_assist_with_retry(access_token, client.clone()).await?;
    debug!(body = %load_json, "loadCodeAssist upstream body");

    let load_resp: LoadCodeAssistResponse =
        serde_json::from_value(load_json.clone()).map_err(NexusError::JsonError)?;

    if let Some(ineligible) = load_resp.ineligible_tiers.first() {
        return Err(NexusError::OauthFlowError {
            code: ineligible
                .reason_code
                .clone()
                .unwrap_or_else(|| "ACCOUNT_INELIGIBLE".to_string()),
            message: ineligible
                .reason_message
                .clone()
                .unwrap_or_else(|| "Account is not eligible for Gemini Code Assist".to_string()),
            details: Some(load_json),
        });
    }

    let tier = load_resp.resolve_effective_tier();

    if let Some(existing_project_id) = load_resp.cloudaicompanion_project {
        return Ok((existing_project_id, tier));
    }

    info!("No existing companion project found, starting onboarding...");
    let new_project_id = perform_onboarding(access_token, tier.clone(), client).await?;

    Ok((new_project_id, tier))
}

fn take_oauth_cookies(jar: PrivateCookieJar) -> (PrivateCookieJar, Option<(String, String)>) {
    let csrf = jar.get(CSRF_COOKIE).map(|c| c.value().to_string());
    let pkce = jar.get(PKCE_COOKIE).map(|c| c.value().to_string());

    let jar = jar
        .remove(Cookie::from(CSRF_COOKIE))
        .remove(Cookie::from(PKCE_COOKIE));

    match (pkce, csrf) {
        (Some(p), Some(c)) => (jar, Some((p, c))),
        _ => (jar, None),
    }
}

fn build_cookie(name: &'static str, value: String) -> Cookie<'static> {
    Cookie::build((name, value))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(Duration::minutes(15))
        .build()
}

async fn perform_onboarding(
    access_token: &str,
    tier: UserTier,
    client: &Client,
) -> Result<String, NexusError> {
    const MAX_ATTEMPTS: usize = 5;
    const RETRY_DELAY: TokioDuration = TokioDuration::from_secs(5);
    let mut last_resp: Option<serde_json::Value> = None;

    for attempt in 1..=MAX_ATTEMPTS {
        let resp_json = GoogleOauthService::onboard_code_assist_with_retry(
            access_token,
            tier.clone(),
            None,
            client.clone(),
        )
        .await?;
        debug!(body = %resp_json, "onboardCodeAssist upstream body");

        last_resp = Some(resp_json.clone());
        let op_resp: OnboardOperationResponse =
            serde_json::from_value(resp_json.clone()).map_err(NexusError::JsonError)?;

        if op_resp.done {
            return op_resp
                .response
                .and_then(|r| r.project_details)
                .map(|p| p.id)
                .ok_or_else(|| NexusError::OauthFlowError {
                    code: "ONBOARD_FAILED".to_string(),
                    message: "Onboarding completed but returned no project ID".to_string(),
                    details: Some(resp_json),
                });
        }

        if attempt < MAX_ATTEMPTS {
            info!(
                "onboardCodeAssist pending (attempt {}/{}), retrying in {:?}...",
                attempt, MAX_ATTEMPTS, RETRY_DELAY
            );
            sleep(RETRY_DELAY).await;
        }
    }

    Err(NexusError::OauthFlowError {
        code: "ONBOARD_TIMEOUT".to_string(),
        message: "Companion project provisioning timed out".to_string(),
        details: last_resp,
    })
}
