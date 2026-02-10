use crate::error::GeminiCliError;
use axum::{
    Json,
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use eventsource_stream::Eventsource;
use futures::{Stream, TryStreamExt};
use pollux_schema::{gemini::GeminiResponseBody, geminicli::GeminiCliResponseBody};
use std::time::Duration;
use tokio_stream::StreamExt;
use tracing::{error, warn};

pub async fn build_json_response(
    upstream_resp: reqwest::Response,
) -> Result<(StatusCode, Json<GeminiResponseBody>), GeminiCliError> {
    let status = upstream_resp.status();
    let response_body = transform_nostream(upstream_resp).await?;
    Ok((status, Json(response_body)))
}

pub fn build_stream_response(upstream_resp: reqwest::Response) -> impl IntoResponse {
    let raw_stream = upstream_resp.bytes_stream().eventsource();
    let timed_stream = transform_stream(raw_stream)
        .timeout(Duration::from_secs(60))
        .map(|item| match item {
            Ok(Ok(event)) => Ok(event),
            Ok(Err(e)) => Err(GeminiCliError::StreamProtocolError(e.to_string())),
            Err(_) => {
                error!("Upstream SSE stream timed out (idle > 60s)");
                Err(GeminiCliError::StreamProtocolError(
                    "Stream idle timeout".to_string(),
                ))
            }
        });

    Sse::new(timed_stream).keep_alive(KeepAlive::default())
}

fn transform_stream<I, E>(s: I) -> impl Stream<Item = Result<Event, E>>
where
    I: Stream<Item = Result<eventsource_stream::Event, E>>,
{
    s.try_filter_map(|upstream_event| async move {
        if upstream_event.data.is_empty() {
            return Ok(None);
        }

        if upstream_event.data == "[DONE]" {
            return Ok(None);
        }

        let Ok(cli_resp) = serde_json::from_str::<GeminiCliResponseBody>(&upstream_event.data)
        else {
            warn!(
                "Skipping invalid SSE JSON data: {:.50}...",
                upstream_event.data
            );
            return Ok(None);
        };

        let gemini_resp: GeminiResponseBody = cli_resp.into();
        match Event::default().json_data(gemini_resp) {
            Ok(ev) => Ok(Some(ev)),
            Err(e) => {
                warn!("Failed to serialize GeminiResponse: {}", e);
                Ok(None)
            }
        }
    })
}

async fn transform_nostream(
    upstream_resp: reqwest::Response,
) -> Result<GeminiResponseBody, GeminiCliError> {
    let envelope = upstream_resp.json::<GeminiCliResponseBody>().await?;
    Ok(envelope.into())
}
