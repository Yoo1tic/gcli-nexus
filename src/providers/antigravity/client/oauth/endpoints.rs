use crate::config::AntigravityResolvedConfig;
use crate::error::{OauthError, PolluxError};
use crate::oauth_utils::{OauthTokenResponse, build_oauth2_client};
use oauth2::{
    AuthorizationCode, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RefreshToken,
    Scope, TokenResponse,
};
use tracing::info;

/// Stateless Antigravity OAuth endpoints built from resolved config.
///
/// Antigravity OAuth parameters are resolved from built-in defaults (and may be
/// overridden in tests via `AntigravityResolvedConfig`), so we build a fresh
/// oauth2 client per request from [`AntigravityResolvedConfig`].
pub struct AntigravityOauthEndpoints;

impl AntigravityOauthEndpoints {
    fn build_client(
        cfg: &AntigravityResolvedConfig,
    ) -> Result<crate::oauth_utils::StandardOauth2Client, PolluxError> {
        let redirect = RedirectUrl::new(cfg.oauth_redirect_url.to_string())?;
        build_oauth2_client(
            &cfg.oauth_client_id,
            Some(&cfg.oauth_client_secret),
            cfg.oauth_auth_url.as_str(),
            cfg.oauth_token_url.as_str(),
            redirect,
        )
    }

    /// Build an auth URL with configured scopes and PKCE challenge preset.
    pub(crate) fn build_authorize_url(
        cfg: &AntigravityResolvedConfig,
        pkce_challenge: PkceCodeChallenge,
    ) -> Result<(url::Url, CsrfToken), PolluxError> {
        let client = Self::build_client(cfg)?;
        let mut req = client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge)
            // Google-style OAuth knobs (matches gcli2api behavior).
            .add_extra_param("access_type", "offline")
            .add_extra_param("prompt", "consent");

        for scope in cfg.oauth_scopes.iter() {
            req = req.add_scope(Scope::new(scope.to_string()));
        }

        Ok(req.url())
    }

    /// Exchange an authorization code (PKCE) for tokens.
    pub(crate) async fn exchange_authorization_code(
        cfg: &AntigravityResolvedConfig,
        code: AuthorizationCode,
        verifier: PkceCodeVerifier,
        http_client: reqwest::Client,
    ) -> Result<OauthTokenResponse, OauthError> {
        let client = Self::build_client(cfg).map_err(|e| OauthError::Other {
            message: format!("failed to build oauth2 client: {e}"),
        })?;

        let token_result: OauthTokenResponse = client
            .exchange_code(code)
            .set_pkce_verifier(verifier)
            .request_async(&http_client)
            .await?;

        // `TokenResponse` trait is used by callers to access refresh_token etc.
        let _ = token_result.access_token();
        info!("Antigravity OAuth2 code exchange completed successfully");
        Ok(token_result)
    }

    /// Refresh an access token using a refresh token.
    pub(crate) async fn refresh_access_token(
        cfg: &AntigravityResolvedConfig,
        refresh_token: &str,
        http_client: reqwest::Client,
    ) -> Result<OauthTokenResponse, OauthError> {
        let client = Self::build_client(cfg).map_err(|e| OauthError::Other {
            message: format!("failed to build oauth2 client: {e}"),
        })?;

        let token_result: OauthTokenResponse = client
            .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
            .request_async(&http_client)
            .await?;
        Ok(token_result)
    }

    /// Refresh an access token, returning a JSON value for testability.
    ///
    /// This avoids exposing crate-private oauth2 response types across the public API.
    pub async fn refresh_access_token_raw(
        cfg: &AntigravityResolvedConfig,
        refresh_token: &str,
        http_client: reqwest::Client,
    ) -> Result<serde_json::Value, OauthError> {
        let token = Self::refresh_access_token(cfg, refresh_token, http_client).await?;
        serde_json::to_value(&token).map_err(|e| OauthError::Other {
            message: format!("failed to serialize oauth token response: {e}"),
        })
    }
}
