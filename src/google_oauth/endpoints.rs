use crate::config::{
    GCLI_CLIENT_ID, GCLI_CLIENT_SECRET, GOOGLE_AUTH_URL, GOOGLE_TOKEN_URI, OAUTH_CALLBACK_URL,
};
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;

use oauth2::{
    AuthUrl, AuthorizationCode, Client as OAuth2Client, ClientId, ClientSecret, CsrfToken,
    EndpointNotSet, EndpointSet, ExtraTokenFields, PkceCodeChallenge, PkceCodeVerifier,
    RefreshToken, Scope, StandardRevocableToken, StandardTokenResponse, TokenUrl,
    basic::{
        BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
        BasicTokenType,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::LazyLock;
use tracing::info;

/// Stateless Google OAuth Endpoints.
pub(crate) struct GoogleOauthEndpoints;

const LOAD_CODE_ASSIST_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";

pub(crate) static DEFAULT_SCOPES: LazyLock<Vec<Scope>> = LazyLock::new(|| {
    vec![
        Scope::new("https://www.googleapis.com/auth/cloud-platform".to_string()),
        Scope::new("https://www.googleapis.com/auth/userinfo.email".to_string()),
        Scope::new("https://www.googleapis.com/auth/userinfo.profile".to_string()),
    ]
});

pub(crate) static OAUTH_CLIENT: LazyLock<GoogleOauth2Client> =
    LazyLock::new(|| build_oauth2_client().expect("valid Google OAuth2 client with redirect"));

impl GoogleOauthEndpoints {
    /// Return the shared Google OAuth2 client with redirect configured.
    pub(crate) fn client() -> &'static GoogleOauth2Client {
        &OAUTH_CLIENT
    }

    /// Build an auth URL with default scopes and PKCE challenge preset.
    pub(crate) fn build_authorize_url(pkce_challenge: PkceCodeChallenge) -> (url::Url, CsrfToken) {
        let mut req = Self::client()
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge)
            .add_extra_param("access_type", "offline")
            .add_extra_param("prompt", "consent");

        for scope in DEFAULT_SCOPES.iter() {
            req = req.add_scope(scope.clone());
        }

        req.url()
    }

    /// Refresh the access token using the current refresh token.
    pub(crate) async fn refresh_access_token(
        creds: &GoogleCredential,
        http_client: reqwest::Client,
    ) -> Result<GoogleTokenResponse, NexusError> {
        let token_result: GoogleTokenResponse = Self::client()
            .exchange_refresh_token(&RefreshToken::new(creds.refresh_token.clone()))
            .request_async(&http_client)
            .await?;
        info!(
            "Project_ID: {}, Access token refreshed successfully",
            creds.project_id
        );
        Ok(token_result)
    }

    /// Exchange an authorization code (PKCE) for tokens.
    pub(crate) async fn exchange_authorization_code(
        code: AuthorizationCode,
        verifier: PkceCodeVerifier,
        http_client: reqwest::Client,
    ) -> Result<GoogleTokenResponse, NexusError> {
        let token_result: GoogleTokenResponse = Self::client()
            .exchange_code(code)
            .set_pkce_verifier(verifier)
            .request_async(&http_client)
            .await?;
        info!("OAuth2 code exchange completed successfully");
        Ok(token_result)
    }

    /// Call Cloud Code's loadCodeAssist to fetch subscription metadata and the companion project.
    pub(crate) async fn load_code_assist(
        access_token: impl AsRef<str>,
        http_client: reqwest::Client,
    ) -> Result<Value, NexusError> {
        let resp = http_client
            .post(LOAD_CODE_ASSIST_URL)
            .bearer_auth(access_token.as_ref())
            .json(&json!({}))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(NexusError::UpstreamStatus(resp.status()));
        }

        let body: Value = resp.json().await?;
        Ok(body)
    }
}

/// Build the Google OAuth2 client from credentials.
fn build_oauth2_client() -> Result<GoogleOauth2Client, NexusError> {
    let client = OAuth2Client::new(ClientId::new(GCLI_CLIENT_ID.to_string()))
        .set_client_secret(ClientSecret::new(GCLI_CLIENT_SECRET.to_string()))
        .set_auth_uri(AuthUrl::new(GOOGLE_AUTH_URL.as_str().to_string())?)
        .set_token_uri(TokenUrl::new(GOOGLE_TOKEN_URI.as_str().to_string())?)
        .set_redirect_uri(OAUTH_CALLBACK_URL.clone());
    Ok(client)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct GoogleTokenField {
    #[serde(rename = "id_token")]
    pub id_token: Option<String>,
}
impl ExtraTokenFields for GoogleTokenField {}

pub(crate) type GoogleTokenResponse = StandardTokenResponse<GoogleTokenField, BasicTokenType>;

pub(crate) type GoogleOauth2Client<
    HasAuthUrl = EndpointSet,
    HasDeviceAuthUrl = EndpointNotSet,
    HasIntrospectionUrl = EndpointNotSet,
    HasRevocationUrl = EndpointNotSet,
    HasTokenUrl = EndpointSet,
> = OAuth2Client<
    BasicErrorResponse,
    GoogleTokenResponse,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
    HasAuthUrl,
    HasDeviceAuthUrl,
    HasIntrospectionUrl,
    HasRevocationUrl,
    HasTokenUrl,
>;
