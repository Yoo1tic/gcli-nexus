use super::IsRetryable;
use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;
use thiserror::Error as ThisError;

use crate::providers::{ActionForError, MappingAction, UPSTREAM_BODY_PREVIEW_CHARS};

#[derive(Debug, ThisError)]
pub enum GeminiCliError {
    #[error("Request rejected")]
    RequestRejected {
        status: StatusCode,
        body: GeminiErrorObject,
        debug_message: Option<String>,
    },

    /// No usable credential is currently available.
    #[error("No available credential")]
    NoAvailableCredential,

    /// Upstream error that matched a provider mapping rule.
    #[error("Upstream mapped error: status={status} body={body:?}")]
    UpstreamMappedError {
        status: StatusCode,
        body: GeminiCliErrorBody,
    },

    /// Upstream fallback error (rule unmatched or body unstructured).
    #[error("Upstream fallback error: status={status}, body={body:.200}")]
    UpstreamFallbackError {
        status: StatusCode,
        /// Raw upstream body is preserved for internal diagnostics/logging only.
        body: String,
    },

    /// Transport-level failure (DNS, connect, timeouts, etc).
    #[error("HTTP request error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Stream protocol error: {0}")]
    StreamProtocolError(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<JsonRejection> for GeminiCliError {
    fn from(rejection: JsonRejection) -> Self {
        let debug_message = rejection.to_string();
        match rejection {
            JsonRejection::BytesRejection(_) => GeminiCliError::RequestRejected {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                body: GeminiErrorObject::for_status(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "PAYLOAD_TOO_LARGE",
                    "request body too large",
                ),
                debug_message: Some(debug_message),
            },
            JsonRejection::JsonSyntaxError(_) => GeminiCliError::RequestRejected {
                status: StatusCode::BAD_REQUEST,
                body: GeminiErrorObject::for_status(
                    StatusCode::BAD_REQUEST,
                    "INVALID_ARGUMENT",
                    "invalid JSON",
                ),
                debug_message: Some(debug_message),
            },
            _ => GeminiCliError::RequestRejected {
                status: StatusCode::BAD_REQUEST,
                body: GeminiErrorObject::for_status(
                    StatusCode::BAD_REQUEST,
                    "INVALID_ARGUMENT",
                    "invalid request",
                ),
                debug_message: Some(debug_message),
            },
        }
    }
}

impl IntoResponse for GeminiCliError {
    fn into_response(self) -> Response {
        let (status, error_body) = match self {
            GeminiCliError::RequestRejected {
                status,
                body,
                debug_message,
            } => {
                if let Some(debug_message) = debug_message {
                    tracing::warn!(
                        status = %status,
                        code = body.code,
                        err_status = %body.status,
                        message = %body.message,
                        debug_message = %debug_message,
                        "Gemini request rejected"
                    );
                } else {
                    tracing::warn!(
                        status = %status,
                        code = body.code,
                        err_status = %body.status,
                        message = %body.message,
                        "Gemini request rejected"
                    );
                }
                (status, body)
            }

            GeminiCliError::UpstreamMappedError { status, body } => {
                let cleaned = GeminiErrorBody::from(body).inner;
                tracing::warn!(
                    status = %status,
                    code = cleaned.code,
                    err_status = %cleaned.status,
                    message = %cleaned.message,
                    "Gemini upstream mapped error"
                );
                (status, cleaned)
            }

            GeminiCliError::UpstreamFallbackError { status, body } => {
                let status_str = match status {
                    StatusCode::TOO_MANY_REQUESTS => "RESOURCE_EXHAUSTED",
                    StatusCode::UNAUTHORIZED => "UNAUTHENTICATED",
                    StatusCode::FORBIDDEN => "PERMISSION_DENIED",
                    StatusCode::NOT_FOUND => "NOT_FOUND",
                    _ => "UNKNOWN",
                };
                tracing::warn!(
                    status = %status,
                    raw_body = %format!("{:.len$}", body, len = UPSTREAM_BODY_PREVIEW_CHARS),
                    "Gemini upstream fallback error"
                );
                (
                    status,
                    GeminiErrorObject::for_status(
                        status,
                        status_str,
                        format!("Upstream returned {status}"),
                    ),
                )
            }

            GeminiCliError::NoAvailableCredential => (
                StatusCode::SERVICE_UNAVAILABLE,
                GeminiErrorObject::for_status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "UNAVAILABLE",
                    "No available credentials to process the request.",
                ),
            ),

            GeminiCliError::Reqwest(e) => {
                tracing::warn!(error = %e, status = ?e.status(), "Gemini reqwest error");
                (
                    StatusCode::BAD_GATEWAY,
                    GeminiErrorObject::for_status(
                        StatusCode::BAD_GATEWAY,
                        "UNAVAILABLE",
                        "Upstream service error.",
                    ),
                )
            }

            GeminiCliError::StreamProtocolError(e) => {
                tracing::warn!(error = %e, "Gemini stream protocol error");
                (
                    StatusCode::BAD_GATEWAY,
                    GeminiErrorObject::for_status(
                        StatusCode::BAD_GATEWAY,
                        "UNAVAILABLE",
                        "Upstream stream protocol error.",
                    ),
                )
            }

            GeminiCliError::Internal(e) => {
                tracing::error!(error = %e, "Gemini internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    GeminiErrorObject::for_status(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "INTERNAL",
                        "An internal server error occurred.",
                    ),
                )
            }
        };

        let resp_json = GeminiErrorBody { inner: error_body };
        (status, Json(resp_json)).into_response()
    }
}

impl From<crate::PolluxError> for GeminiCliError {
    fn from(err: crate::PolluxError) -> Self {
        match err {
            crate::PolluxError::NoAvailableCredential => GeminiCliError::NoAvailableCredential,
            crate::PolluxError::ReqwestError(e) => GeminiCliError::Reqwest(e),
            crate::PolluxError::StreamProtocolError(s) => GeminiCliError::StreamProtocolError(s),
            other => GeminiCliError::Internal(other.to_string()),
        }
    }
}

impl IsRetryable for GeminiCliError {
    fn is_retryable(&self) -> bool {
        match self {
            // Transport errors are already retried inside GeminiApi.
            GeminiCliError::Reqwest(_) => false,

            GeminiCliError::UpstreamFallbackError { status, .. } => matches!(
                *status,
                StatusCode::TOO_MANY_REQUESTS
                    | StatusCode::UNAUTHORIZED
                    | StatusCode::FORBIDDEN
                    | StatusCode::NOT_FOUND
            ),

            GeminiCliError::UpstreamMappedError { status, .. } => matches!(
                *status,
                StatusCode::TOO_MANY_REQUESTS
                    | StatusCode::UNAUTHORIZED
                    | StatusCode::FORBIDDEN
                    | StatusCode::NOT_FOUND
            ),

            _ => false,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct GeminiErrorBody {
    #[serde(rename = "error")]
    pub inner: GeminiErrorObject,
}

#[derive(Debug, Serialize)]
pub struct GeminiErrorObject {
    pub code: u16,
    pub message: String,
    pub status: String,
}

impl GeminiErrorObject {
    pub(crate) fn for_status(
        code: StatusCode,
        status: &'static str,
        message: impl Into<String>,
    ) -> Self {
        GeminiErrorObject {
            code: code.as_u16(),
            message: message.into(),
            status: status.to_string(),
        }
    }
}

impl From<GeminiCliErrorBody> for GeminiErrorBody {
    fn from(upstream_err: GeminiCliErrorBody) -> Self {
        let GeminiCliErrorBody { inner } = upstream_err;
        let GeminiCliErrorObject {
            code,
            message,
            status,
            details: _,
            extra: _,
        } = inner;
        GeminiErrorBody {
            inner: GeminiErrorObject {
                code: code.unwrap_or(0),
                message: message.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
                    "Upstream error (check server logs for details).".to_string()
                }),
                status: status
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "UNKNOWN".to_string()),
            },
        }
    }
}

/// Gemini API error response structure.
#[derive(Debug, Deserialize, Serialize)]
pub struct GeminiCliErrorBody {
    #[serde(rename = "error")]
    pub inner: GeminiCliErrorObject,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GeminiCliErrorObject {
    /// Numeric error code returned by upstream (often equals the HTTP status code, e.g. `429`/`404`).
    ///
    /// Example:
    /// - `429` with `status="RESOURCE_EXHAUSTED"`
    /// - `404` with `status="NOT_FOUND"`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<u16>,

    /// Human-readable error message from upstream.
    ///
    /// Example (`429`):
    /// - `"You have exhausted your capacity on this model. Your quota will reset after ..."`
    /// - `"No capacity available for model ..."`
    ///
    /// Example (`404`):
    /// - `"Requested entity was not found."`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Upstream status string (Google-style canonical status name).
    ///
    /// Example:
    /// - `"RESOURCE_EXHAUSTED"` (rate limit / capacity)
    /// - `"NOT_FOUND"` (model/resource not found)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    /// Structured error details array returned by upstream.
    ///
    /// This field is present on some errors (notably `429`), and may include objects like:
    /// - `google.rpc.ErrorInfo` with `metadata` (e.g. `model`, `quotaResetDelay`,
    ///   `quotaResetTimeStamp` RFC3339 timestamp)
    /// - `google.rpc.RetryInfo` with `retryDelay`
    ///
    /// We keep this as `Vec<Value>` for forward compatibility and only *optionally* extract
    /// `metadata.quotaResetTimeStamp` for cooldown calculation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<Value>>,

    /// Catch-all for any additional/unknown fields inside the upstream `error` object.
    ///
    /// This is intentionally kept to make deserialization "best effort" if upstream adds new
    /// fields, and to preserve raw information for internal logs/debugging.
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl GeminiCliErrorBody {
    pub fn quota_reset_delay(&self) -> Option<u64> {
        let details = self.inner.details.as_ref()?;

        details
            .iter()
            .filter_map(|detail| {
                detail
                    .get("metadata")
                    .and_then(|m| m.get("quotaResetTimeStamp"))
                    .and_then(Value::as_str)
                    .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            })
            .filter_map(|reset_dt| {
                let reset = reset_dt.with_timezone(&Utc);
                let now = Utc::now();
                let diff_secs = (reset - now).num_seconds();
                (diff_secs > 0).then_some((diff_secs as u64).saturating_add(1))
            })
            .next()
            .or_else(|| {
                details
                    .iter()
                    .any(|detail| {
                        detail.get("reason").and_then(Value::as_str)
                            == Some("MODEL_CAPACITY_EXHAUSTED")
                    })
                    .then_some(10 * 60)
            })
    }
}

impl MappingAction for GeminiCliErrorBody {
    fn try_match_rule(&self, status: StatusCode) -> Option<ActionForError> {
        match (status, self) {
            // 401: credential is invalid/expired.
            (StatusCode::UNAUTHORIZED, body)
                if body.inner.status.as_deref() == Some("UNAUTHENTICATED") =>
            {
                Some(ActionForError::Invalid)
            }

            // 403: account permission issue.
            (StatusCode::FORBIDDEN, body)
                if body.inner.status.as_deref() == Some("PERMISSION_DENIED") =>
            {
                Some(ActionForError::Ban)
            }

            // 404: requested model/resource not found.
            (StatusCode::NOT_FOUND, body) if body.inner.status.as_deref() == Some("NOT_FOUND") => {
                Some(ActionForError::ModelUnsupported)
            }

            // 429: quota/capacity exhausted.
            (StatusCode::TOO_MANY_REQUESTS, body)
                if body.inner.status.as_deref() == Some("RESOURCE_EXHAUSTED") =>
            {
                Some(ActionForError::RateLimit(Duration::from_secs(
                    body.quota_reset_delay().unwrap_or(90).max(1),
                )))
            }

            _ => None,
        }
    }

    fn action_from_status(status: StatusCode) -> ActionForError {
        match status {
            StatusCode::UNAUTHORIZED => ActionForError::Invalid,
            StatusCode::FORBIDDEN => ActionForError::None, // often WAF or transient; preserve cred
            StatusCode::NOT_FOUND => ActionForError::ModelUnsupported,
            StatusCode::TOO_MANY_REQUESTS => ActionForError::RateLimit(Duration::from_secs(60)),
            _ => ActionForError::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_and_map() {
        let e429_1 = GeminiCliErrorBody {
            inner: GeminiCliErrorObject {
                code: Some(429),
                message: Some("quota".to_string()),
                status: Some("RESOURCE_EXHAUSTED".to_string()),
                details: Some(vec![json!({
                    "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                    "reason": "QUOTA_EXHAUSTED",
                    "domain": "cloudcode-pa.googleapis.com",
                    "metadata": {
                        "uiMessage": "true",
                        "model": "gemini-2.5-pro",
                        "quotaResetDelay": "5h41m27.587942796s",
                        "quotaResetTimeStamp": "2999-01-01T00:00:00Z"
                    }
                })]),
                extra: BTreeMap::new(),
            },
        };
        assert_eq!(e429_1.inner.code, Some(429));
        assert_eq!(e429_1.inner.status.as_deref(), Some("RESOURCE_EXHAUSTED"));
        assert!(e429_1.inner.details.is_some());
        assert!(matches!(
            e429_1.try_match_rule(StatusCode::TOO_MANY_REQUESTS),
            Some(ActionForError::RateLimit(_))
        ));

        let e429_2 = GeminiCliErrorBody {
            inner: GeminiCliErrorObject {
                code: Some(429),
                message: Some("No capacity".to_string()),
                status: Some("RESOURCE_EXHAUSTED".to_string()),
                details: Some(vec![json!({
                    "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                    "domain": "cloudcode-pa.googleapis.com",
                    "metadata": { "model": "gemini-3-pro-preview" },
                    "reason": "MODEL_CAPACITY_EXHAUSTED"
                })]),
                extra: BTreeMap::new(),
            },
        };
        assert_eq!(e429_2.inner.code, Some(429));
        assert_eq!(e429_2.inner.status.as_deref(), Some("RESOURCE_EXHAUSTED"));
        assert!(e429_2.inner.details.is_some());
        assert_eq!(
            e429_2.try_match_rule(StatusCode::TOO_MANY_REQUESTS),
            Some(ActionForError::RateLimit(Duration::from_secs(60 * 60)))
        );

        let e404_1 = GeminiCliErrorBody {
            inner: GeminiCliErrorObject {
                code: Some(404),
                message: Some("Requested entity was not found.".to_string()),
                status: Some("NOT_FOUND".to_string()),
                details: None,
                extra: BTreeMap::new(),
            },
        };
        assert_eq!(e404_1.inner.code, Some(404));
        assert_eq!(e404_1.inner.status.as_deref(), Some("NOT_FOUND"));
        assert!(matches!(
            e404_1.try_match_rule(StatusCode::NOT_FOUND),
            Some(ActionForError::ModelUnsupported)
        ));
    }

    #[test]
    fn quota_reset_delay_uses_timestamp() {
        // Use far-future timestamp to keep the test stable regardless of runtime clock.
        let raw = r#"{
            "error": {
                "code": 429,
                "message": "quota",
                "status": "RESOURCE_EXHAUSTED",
                "details": [
                    { "metadata": { "quotaResetTimeStamp": "2999-01-01T00:00:00Z" } }
                ]
            }
        }"#;

        let parsed = serde_json::from_str::<GeminiCliErrorBody>(raw).expect("parse sample");
        assert!(parsed.quota_reset_delay().is_some());
    }
}
