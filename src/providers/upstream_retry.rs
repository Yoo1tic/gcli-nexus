use backon::{ExponentialBuilder, Retryable};
use reqwest::header::HeaderMap;
use std::sync::LazyLock;
use std::time::Duration;
use url::Url;

use crate::providers::UPSTREAM_BODY_PREVIEW_CHARS;

static NETWORK_RETRY_POLICY: LazyLock<ExponentialBuilder> = LazyLock::new(|| {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(100))
        .with_max_delay(Duration::from_millis(300))
        .with_max_times(2)
        .with_jitter()
});

pub(crate) async fn post_json_with_retry<T>(
    provider: &'static str,
    client: &reqwest::Client,
    url: &Url,
    headers: Option<HeaderMap>,
    body: &T,
) -> Result<reqwest::Response, reqwest::Error>
where
    T: serde::Serialize,
{
    (|| {
        let client = client.clone();
        let url = url.clone();
        let headers = headers.clone();

        async move {
            let mut request = client.post(url.clone());
            if let Some(headers) = &headers {
                request = request.headers(headers.clone());
            }

            let resp = request.json(body).send().await?;

            if resp.status().is_server_error() {
                let status = resp.status();
                let err = resp.error_for_status_ref().unwrap_err();

                let body_preview = match resp.bytes().await {
                    Ok(bytes) => {
                        let raw_body = String::from_utf8_lossy(&bytes);
                        format!("{:.len$}", raw_body, len = UPSTREAM_BODY_PREVIEW_CHARS)
                    }
                    Err(e) => format!("<failed to read body: {e}>"),
                };

                tracing::debug!(
                    provider,
                    %status,
                    url = %url,
                    body = %body_preview,
                    "[{provider}] Upstream server error (will retry)"
                );

                return Err(err);
            }

            Ok(resp)
        }
    })
    .retry(*NETWORK_RETRY_POLICY)
    .await
}
