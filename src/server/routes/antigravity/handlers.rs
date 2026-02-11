use super::{
    extract::AntigravityPreprocess,
    respond::{build_json_response, build_stream_response},
};
use crate::error::GeminiCliError;
use crate::providers::antigravity::AntigravityClient;
use crate::server::router::PolluxState;
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use pollux_schema::gemini::GeminiModelList;

pub async fn antigravity_proxy_handler(
    State(state): State<PolluxState>,
    AntigravityPreprocess(body, ctx): AntigravityPreprocess,
) -> Result<Response, GeminiCliError> {
    let caller = AntigravityClient::new(
        state.providers.antigravity_cfg.as_ref(),
        state.antigravity_client.clone(),
        None,
    );

    let upstream_resp = caller
        .call_antigravity(&state.providers.antigravity, &ctx, &body)
        .await
        .map_err(map_antigravity_error)?;

    if ctx.stream {
        Ok(build_stream_response(upstream_resp, state.clone()).into_response())
    } else {
        Ok(build_json_response(upstream_resp, &state)
            .await?
            .into_response())
    }
}

pub async fn antigravity_models_handler(
    State(state): State<PolluxState>,
) -> Result<Json<GeminiModelList>, GeminiCliError> {
    Ok(Json(GeminiModelList::from_model_names(
        state.providers.antigravity_cfg.model_list.iter().cloned(),
    )))
}

fn map_antigravity_error(err: crate::PolluxError) -> GeminiCliError {
    match err {
        crate::PolluxError::UpstreamStatus(status) => GeminiCliError::UpstreamFallbackError {
            status,
            body: String::new(),
        },
        other => other.into(),
    }
}
