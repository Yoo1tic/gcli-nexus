use super::{extract::CodexPreprocess, respond};
use crate::error::CodexError;
use crate::providers::codex::client::CodexClient;
use crate::server::router::PolluxState;
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use pollux_schema::CodexRequestBody;
use pollux_schema::openai::OpenaiModelList;
use tracing::debug;

pub(super) async fn codex_response_handler(
    State(state): State<PolluxState>,
    CodexPreprocess(body, ctx): CodexPreprocess,
) -> Result<Response, CodexError> {
    let codex_body: CodexRequestBody = body.into();

    debug!(
        model = %ctx.model,
        client_stream = ctx.stream,
        upstream_stream = codex_body.stream,
        model_mask = format_args!("0x{:016x}", ctx.model_mask),
        "Incoming Codex request"
    );

    let caller = CodexClient::new(
        state.providers.codex_cfg.as_ref(),
        state.codex_client.clone(),
        None,
    );

    let upstream_resp = caller
        .call_codex(
            &state.providers.codex,
            ctx.model.as_str(),
            ctx.model_mask,
            ctx.stream,
            &codex_body,
        )
        .await?;

    if ctx.stream {
        Ok(respond::build_stream_response(upstream_resp).into_response())
    } else {
        let (status, body) = respond::build_json_response_from_stream(upstream_resp).await?;
        Ok((status, body).into_response())
    }
}

pub(super) async fn codex_models_handler() -> Result<Json<OpenaiModelList>, CodexError> {
    Ok(Json(super::CODEX_MODEL_LIST.clone()))
}
