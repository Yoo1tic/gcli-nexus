use crate::PolluxError;
use crate::error::OauthError;
use crate::providers::antigravity::client::oauth::endpoints::AntigravityOauthEndpoints;
use crate::server::router::PolluxState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::cookie::{Cookie, PrivateCookieJar, SameSite};
use oauth2::{AuthorizationCode, PkceCodeChallenge, PkceCodeVerifier, TokenResponse};
use std::collections::HashMap;
use time::Duration;
use tracing::{error, info};

const CSRF_COOKIE: &str = "antigravity_oauth_csrf_token";
const PKCE_COOKIE: &str = "antigravity_oauth_pkce_verifier";

/// GET /antigravity/auth
///
/// Starts the Antigravity OAuth2 PKCE flow and redirects the browser to the configured auth URL.
pub async fn antigravity_oauth_entry(
    State(state): State<PolluxState>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, PolluxError> {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_token) = AntigravityOauthEndpoints::build_authorize_url(
        &state.providers.antigravity_cfg,
        challenge,
    )?;

    let jar = jar
        .add(build_cookie(
            CSRF_COOKIE,
            csrf_token.secret().to_string(),
            !state.insecure_cookie,
        ))
        .add(build_cookie(
            PKCE_COOKIE,
            verifier.secret().to_string(),
            !state.insecure_cookie,
        ));

    info!("Dispatching Antigravity OAuth redirect to: {}", auth_url);
    Ok((jar, Redirect::temporary(auth_url.as_ref())).into_response())
}

/// GET /
///
/// Antigravity OAuth callback handler.
///
/// This handler is intentionally **guarded**: it only activates when both `code` and `state`
/// query parameters are present. Otherwise it returns 404, keeping `/` effectively not-found.
pub async fn antigravity_oauth_callback_root(
    State(state): State<PolluxState>,
    Query(params): Query<HashMap<String, String>>,
    jar: PrivateCookieJar,
) -> impl IntoResponse {
    let code = params
        .get("code")
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let state_param = params
        .get("state")
        .map(String::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let (Some(code), Some(state_param)) = (code, state_param) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let (jar, session_data) = take_oauth_cookies(jar);
    let result = process_oauth_exchange(&state, &code, &state_param, session_data).await;

    match result {
        Ok(token_response) => {
            state
                .providers
                .antigravity
                .submit_trusted_oauth(token_response)
                .await;
            info!("Antigravity OAuth callback accepted");
            (jar, (StatusCode::ACCEPTED, "Success")).into_response()
        }
        Err(err) => {
            error!("Antigravity OAuth failure: {:?}", err);
            (jar, err.into_response()).into_response()
        }
    }
}

async fn process_oauth_exchange(
    state: &PolluxState,
    code: &str,
    state_param: &str,
    session_data: Option<(String, String)>,
) -> Result<crate::oauth_utils::OauthTokenResponse, PolluxError> {
    let (pkce_verifier, csrf_token) = session_data.ok_or_else(|| OauthError::Flow {
        code: "OAUTH_SESSION_MISSING".to_string(),
        message: "Missing OAuth session cookies".to_string(),
        details: None,
    })?;

    if state_param != csrf_token {
        return Err(OauthError::Flow {
            code: "CSRF_MISMATCH".to_string(),
            message: "CSRF token mismatch".to_string(),
            details: None,
        }
        .into());
    }

    let token_response = AntigravityOauthEndpoints::exchange_authorization_code(
        &state.providers.antigravity_cfg,
        AuthorizationCode::new(code.to_string()),
        PkceCodeVerifier::new(pkce_verifier),
        state.antigravity_client.clone(),
    )
    .await
    .map_err(|e| OauthError::Flow {
        code: "TOKEN_EXCHANGE_FAILED".to_string(),
        message: format!("Token exchange failed: {e}"),
        details: None,
    })?;

    let refresh_token = token_response
        .refresh_token()
        .map(|t| t.secret().to_string())
        .unwrap_or_default();

    if refresh_token.trim().is_empty() {
        return Err(OauthError::Flow {
            code: "MISSING_REFRESH_TOKEN".to_string(),
            message: "Missing refresh_token (check access_type=offline)".to_string(),
            details: None,
        }
        .into());
    }

    Ok(token_response)
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

fn build_cookie(name: &'static str, value: String, secure: bool) -> Cookie<'static> {
    Cookie::build((name, value))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(Duration::minutes(15))
        .build()
}
