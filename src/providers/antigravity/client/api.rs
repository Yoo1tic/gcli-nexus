use backon::{ExponentialBuilder, Retryable};
use chrono::Utc;
use rand::Rng as _;
use tracing::error;
use uuid::Uuid;

pub struct AntigravityApi;

const REQUEST_ID_PREFIX: &str = "agent";
const SESSION_ID_MAX_EXCLUSIVE: i64 = 9_000_000_000_000_000_000;
const ANTIGRAVITY_GENERATE_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:generateContent";
const ANTIGRAVITY_STREAM_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse";

impl AntigravityApi {
    pub fn request_id_from_parts(timestamp_ms: i64, request_uuid: Uuid) -> String {
        format!("{REQUEST_ID_PREFIX}/{timestamp_ms}/{request_uuid}")
    }

    pub fn generate_request_id() -> String {
        Self::request_id_from_parts(Utc::now().timestamp_millis(), Uuid::new_v4())
    }

    pub fn session_id_from_int(value: i64) -> String {
        format!("-{value}")
    }

    pub fn generate_session_id() -> String {
        let value = rand::rng().random_range(0..SESSION_ID_MAX_EXCLUSIVE);
        Self::session_id_from_int(value)
    }

    pub async fn try_post<T>(
        client: reqwest::Client,
        token: impl AsRef<str>,
        stream: bool,
        retry_policy: ExponentialBuilder,
        body: &T,
    ) -> Result<reqwest::Response, reqwest::Error>
    where
        T: serde::Serialize,
    {
        let url = if stream {
            ANTIGRAVITY_STREAM_URL
        } else {
            ANTIGRAVITY_GENERATE_URL
        };

        (|| async {
            let resp = client
                .post(url)
                .header("user-agent", "antigravity/1.16.5 linux/amd64")
                .bearer_auth(token.as_ref())
                .json(body)
                .send()
                .await?;
            if resp.status().is_server_error() {
                let status = resp.status();
                let err = resp.error_for_status().unwrap_err();
                error!("Antigravity upstream server error (will retry): {}", status);
                return Err(err);
            }
            Ok(resp)
        })
        .retry(retry_policy)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_uses_agent_timestamp_uuid_shape() {
        let id = AntigravityApi::request_id_from_parts(
            1234,
            Uuid::parse_str("00000000-0000-4000-8000-000000000000").unwrap(),
        );
        assert_eq!(id, "agent/1234/00000000-0000-4000-8000-000000000000");
    }

    #[test]
    fn fixed_upstream_urls_are_expected_literals() {
        assert_eq!(
            ANTIGRAVITY_GENERATE_URL,
            "https://daily-cloudcode-pa.googleapis.com/v1internal:generateContent"
        );
        assert_eq!(
            ANTIGRAVITY_STREAM_URL,
            "https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn session_id_is_negative_decimal_string() {
        assert_eq!(AntigravityApi::session_id_from_int(42), "-42");
        assert_eq!(AntigravityApi::session_id_from_int(0), "-0");
    }
}
