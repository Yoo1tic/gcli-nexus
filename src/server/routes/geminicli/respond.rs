use crate::error::GeminiCliError;
use crate::server::router::PolluxState;
use axum::{
    Json,
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use eventsource_stream::Eventsource;
use futures::{Stream, TryStreamExt, future};
use pollux_schema::{gemini::GeminiResponseBody, geminicli::GeminiCliResponseBody};
use std::time::Duration;
use tokio_stream::StreamExt;
use tracing::{error, warn};

/// Build JSON response from upstream CLI response.
pub async fn build_json_response(
    upstream_resp: reqwest::Response,
    state: &PolluxState,
) -> Result<(StatusCode, Json<GeminiResponseBody>), GeminiCliError> {
    let status = upstream_resp.status();
    let response_body = transform_nostream(upstream_resp).await?;
    let mut sniffer = state.providers.geminicli_thoughtsig.build_sniffer();
    state
        .providers
        .geminicli_thoughtsig
        .sniff_response(&response_body, &mut sniffer);
    Ok((status, Json(response_body)))
}

/// Build SSE stream response with timeout and protocol mapping.
pub fn build_stream_response(
    upstream_resp: reqwest::Response,
    state: PolluxState,
) -> impl IntoResponse {
    let sniffer = state.providers.geminicli_thoughtsig.build_sniffer();
    let raw_stream = upstream_resp.bytes_stream().eventsource();
    let record_stream = transform_stream(raw_stream, state.clone(), sniffer);
    let timed_stream = record_stream
        .timeout(Duration::from_secs(60))
        .map(move |item| match item {
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

/// Convert upstream SSE events into SSE `Event`s and record thought signatures.
fn transform_stream<I, E>(
    s: I,
    state: PolluxState,
    mut sniffer: pollux_thoughtsig_core::SignatureSniffer,
) -> impl Stream<Item = Result<Event, E>>
where
    I: Stream<Item = Result<eventsource_stream::Event, E>>,
{
    s.try_filter_map(move |upstream_event| {
        let state = state.clone();

        let out = {
            if upstream_event.data.is_empty()
                || upstream_event.data == "[DONE]"
                || upstream_event.event == "done"
            {
                Ok(None)
            } else {
                let Some(gemini_resp) = parse_sse_payload(&upstream_event.data) else {
                    return future::ready(Ok(None));
                };

                state
                    .providers
                    .geminicli_thoughtsig
                    .sniff_response(&gemini_resp, &mut sniffer);

                match Event::default().json_data(gemini_resp) {
                    Ok(ev) => Ok(Some(ev)),
                    Err(e) => {
                        warn!("Failed to serialize GeminiResponse: {}", e);
                        Ok(None)
                    }
                }
            }
        };

        future::ready(out)
    })
}

fn parse_sse_payload(data: &str) -> Option<GeminiResponseBody> {
    let Ok(cli_resp) = serde_json::from_str::<GeminiCliResponseBody>(data) else {
        warn!("Skipping invalid SSE JSON data: {:.50}...", data);
        return None;
    };

    Some(cli_resp.into())
}

/// Convert non-streaming CLI envelope into `GeminiResponse`.
pub async fn transform_nostream(
    upstream_resp: reqwest::Response,
) -> Result<GeminiResponseBody, GeminiCliError> {
    let envelope = upstream_resp.json::<GeminiCliResponseBody>().await?;
    Ok(envelope.into())
}
