use axum::{Json, http::StatusCode, response::IntoResponse};
use chrono::{DateTime, Utc};
use oauth2::basic::BasicErrorResponseType;
use oauth2::reqwest::Error as ReqwestClientError;
use oauth2::{HttpClientError, RequestTokenError, StandardErrorResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Error as SqlxError;
use std::collections::HashMap;
use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
pub enum NexusError {
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("HTTP request error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Missing access token; refresh first")]
    MissingAccessToken,

    #[error("missing email in userinfo response")]
    MissingEmailInUserinfo,

    #[error("Unexpected error: {0}")]
    UnexpectedError(String),

    #[error("OAuth2 server error: {error}")]
    Oauth2Server { error: String },

    #[error("No available credential")]
    NoAvailableCredential,

    #[error("Ractor error: {0}")]
    RactorError(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] SqlxError),

    #[error("Upstream error with status: {0}")]
    UpstreamStatus(StatusCode),

    #[error("Gemini API error: {0:?}")]
    GeminiServerError(GeminiError),
}

impl NexusError {}
type PkgsRequestTokenError = RequestTokenError<
    HttpClientError<ReqwestClientError>,
    StandardErrorResponse<BasicErrorResponseType>,
>;
impl From<PkgsRequestTokenError> for NexusError {
    fn from(e: PkgsRequestTokenError) -> Self {
        match e {
            RequestTokenError::ServerResponse(err) => NexusError::Oauth2Server {
                error: err.error().to_string(),
            },
            RequestTokenError::Request(wrapper) => match wrapper {
                oauth2::HttpClientError::Reqwest(real_err) => NexusError::ReqwestError(*real_err),
                other => NexusError::UnexpectedError(format!("HttpClientError: {:?}", other)),
            },
            RequestTokenError::Parse(parse_err, _body) => NexusError::Json(parse_err.into_inner()),
            RequestTokenError::Other(s) => NexusError::UnexpectedError(s),
        }
    }
}
impl IntoResponse for NexusError {
    fn into_response(self) -> axum::response::Response {
        let (status, error_body) = match self {
            NexusError::GeminiServerError(gemini_err) => {
                let status = StatusCode::from_u16(gemini_err.error.code as u16)
                    .unwrap_or(StatusCode::BAD_REQUEST);

                let body = ApiErrorBody {
                    code: gemini_err.error.status,
                    message: gemini_err.error.message,
                };
                (status, body)
            }
            NexusError::DatabaseError(_) | NexusError::RactorError(_) => {
                let status = StatusCode::INTERNAL_SERVER_ERROR;
                let body = ApiErrorBody {
                    code: "INTERNAL_ERROR".to_string(),
                    message: "An internal server error occurred.".to_string(),
                };
                (status, body)
            }
            NexusError::Json(_)
            | NexusError::UnexpectedError(_)
            | NexusError::Oauth2Server { .. }
            | NexusError::MissingAccessToken
            | NexusError::MissingEmailInUserinfo => {
                let status = StatusCode::UNAUTHORIZED;
                let body = ApiErrorBody {
                    code: "UNAUTHORIZED".to_string(),
                    message: "Authentication error.".to_string(),
                };
                (status, body)
            }
            NexusError::NoAvailableCredential => {
                let status = StatusCode::SERVICE_UNAVAILABLE; // 503
                let body = ApiErrorBody {
                    code: "NO_CREDENTIAL".to_string(),
                    message: "No available credentials to process the request.".to_string(),
                };
                (status, body)
            }
            NexusError::ReqwestError(_) | NexusError::UrlParse(_) => {
                let status = StatusCode::BAD_GATEWAY;
                let body = ApiErrorBody {
                    code: "BAD_GATEWAY".to_string(),
                    message: "Upstream service is unavailable.".to_string(),
                };
                (status, body)
            }
            NexusError::UpstreamStatus(code) => {
                let (err_code, msg) = match code {
                    StatusCode::TOO_MANY_REQUESTS => {
                        ("RATE_LIMIT", "Upstream rate limit exceeded.")
                    }
                    StatusCode::UNAUTHORIZED => ("UNAUTHORIZED", "Upstream authentication failed."),
                    StatusCode::FORBIDDEN => ("FORBIDDEN", "Upstream permission denied."),
                    StatusCode::NOT_FOUND => ("NOT_FOUND", "Upstream resource not found."),
                    _ => ("UPSTREAM_ERROR", "An upstream error occurred."),
                };

                (
                    code,
                    ApiErrorBody {
                        code: err_code.to_string(),
                        message: msg.to_string(),
                    },
                )
            }
        };
        (status, Json(ApiErrorResponse { error: error_body })).into_response()
    }
}

/// Standardized API error response body
#[derive(Serialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct ApiErrorResponse {
    pub error: ApiErrorBody,
}

/// Gemini API error response structure
#[derive(Deserialize, Debug)]
pub struct GeminiError {
    pub error: GeminiErrorBody,
}

#[derive(Deserialize, Debug)]
pub struct GeminiErrorBody {
    pub code: u32,
    pub message: String,
    pub status: String,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl GeminiError {
    pub fn quota_reset_delay(&self) -> Option<u64> {
        self.error
            .extra
            .get("details")?
            .as_array()?
            .iter()
            .filter_map(|detail| {
                detail
                    .get("metadata")
                    .and_then(|m| m.get("quotaResetTimeStamp"))
                    .and_then(|ts| ts.as_str())
                    .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            })
            .filter_map(|reset_dt| {
                let reset = reset_dt.with_timezone(&Utc);
                let now = Utc::now();
                let diff_secs = (reset - now).num_seconds();
                (diff_secs > 0).then_some(diff_secs as u64)
            })
            .next()
    }
}

pub trait Retryable {
    fn is_retryable(&self) -> bool;
}

impl Retryable for NexusError {
    fn is_retryable(&self) -> bool {
        match self {
            NexusError::ReqwestError(_) => true,
            NexusError::UnexpectedError(_) => false,

            _ => false,
        }
    }
}
