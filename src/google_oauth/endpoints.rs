use crate::config::{GCLI_CLIENT_ID, GCLI_CLIENT_SECRET, GOOGLE_AUTH_URL, GOOGLE_TOKEN_URI};
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;

use oauth2::{
    AuthUrl, Client as OAuth2Client, ClientId, ClientSecret, EndpointNotSet, EndpointSet,
    ExtraTokenFields, RefreshToken, StandardRevocableToken, StandardTokenResponse, TokenUrl,
    basic::{
        BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
        BasicTokenType,
    },
};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Stateless Google OAuth Endpoints.
pub(super) struct GoogleOauthEndpoints;

impl GoogleOauthEndpoints {
    /// Refresh the access token using the current refresh token.
    pub(super) async fn refresh_access_token(
        creds: &GoogleCredential,
        http_client: reqwest::Client,
    ) -> Result<GoogleTokenResponse, NexusError> {
        let client = build_oauth2_client()?;
        let token_result: GoogleTokenResponse = client
            .exchange_refresh_token(&RefreshToken::new(creds.refresh_token.clone()))
            .request_async(&http_client)
            .await?;
        info!(
            "Project_ID: {}, Access token refreshed successfully",
            creds.project_id
        );
        Ok(token_result)
    }
}

/// Build the Google OAuth2 client from credentials.
fn build_oauth2_client() -> Result<GoogleOauth2Client, NexusError> {
    let client = OAuth2Client::new(ClientId::new(GCLI_CLIENT_ID.to_string()))
        .set_client_secret(ClientSecret::new(GCLI_CLIENT_SECRET.to_string()))
        .set_auth_uri(AuthUrl::new(GOOGLE_AUTH_URL.as_str().to_string())?)
        .set_token_uri(TokenUrl::new(GOOGLE_TOKEN_URI.as_str().to_string())?);
    Ok(client)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct GoogleTokenField {
    #[serde(rename = "id_token")]
    pub id_token: Option<String>,
}
impl ExtraTokenFields for GoogleTokenField {}

pub(super) type GoogleTokenResponse = StandardTokenResponse<GoogleTokenField, BasicTokenType>;

pub(super) type GoogleOauth2Client = OAuth2Client<
    BasicErrorResponse,
    GoogleTokenResponse,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
>;
