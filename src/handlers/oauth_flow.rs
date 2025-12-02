use crate::config::CONFIG;
use crate::google_oauth::credentials::GoogleCredential;
use crate::google_oauth::endpoints::GoogleOauthEndpoints;
use crate::google_oauth::utils::attach_email_from_id_token;
use crate::{NexusError, router::NexusState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::cookie::{Cookie, PrivateCookieJar, SameSite};
use oauth2::{AuthorizationCode, PkceCodeChallenge, PkceCodeVerifier};
use serde::Deserialize;
use serde_json::Value;
use subtle::ConstantTimeEq;
use time::Duration;
use tracing::{debug, error, info};

const CSRF_COOKIE: &str = "oauth_csrf_token";
const PKCE_COOKIE: &str = "oauth_pkce_verifier";

#[derive(Debug, Deserialize)]
pub struct AuthCallbackQuery {
    pub code: String,
    pub state: String,
}

/// GET /auth/:secret
pub async fn google_oauth_entry(
    Path(secret): Path<String>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, NexusError> {
    if !bool::from(secret.as_bytes().ct_eq(CONFIG.nexus_key.as_bytes())) {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

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

    let load_resp =
        GoogleOauthEndpoints::load_code_assist(access_token, state.client.clone()).await?;
    debug!(body = %load_resp, "loadCodeAssist upstream body");

    if let Some(ineligible) = load_resp
        .get("ineligibleTiers")
        .and_then(Value::as_array)
        .and_then(|tiers| tiers.get(0))
    {
        let reason_code = ineligible
            .get("reasonCode")
            .and_then(Value::as_str)
            .unwrap_or("ACCOUNT_INELIGIBLE");
        let reason_message = ineligible
            .get("reasonMessage")
            .and_then(Value::as_str)
            .unwrap_or("Account is not eligible for Gemini Code Assist");
        return Err(NexusError::OauthFlowError {
            code: reason_code.to_string(),
            message: reason_message.to_string(),
            details: Some(load_resp.clone()),
        });
    }

    let project_id = load_resp
        .get("cloudaicompanionProject")
        .and_then(Value::as_str)
        .ok_or_else(|| NexusError::OauthFlowError {
            code: "MISSING_COMPANION_PROJECT".to_string(),
            message: "loadCodeAssist missing cloudaicompanionProject".to_string(),
            details: None,
        })?;

    let quota_tier = load_resp
        .get("allowedTiers")
        .and_then(Value::as_array)
        .and_then(|tiers| tiers.get(0))
        .and_then(|tier| tier.get("quotaTier"))
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");

    credential.project_id = project_id.to_string();
    info!(
        project_id = %project_id,
        quota_tier = %quota_tier,
        "loadCodeAssist resolved companion project id"
    );

    state
        .handle
        .submit_credentials(vec![credential.clone()])
        .await;

    Ok(credential)
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
        .same_site(SameSite::Lax)
        .max_age(Duration::minutes(15))
        .build()
}
