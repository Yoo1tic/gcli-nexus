use crate::server::router::PolluxState;
use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    http::header::CONTENT_LENGTH,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use tracing::debug;

pub mod extract;
pub mod handlers;
pub mod oauth;
pub mod resource;
pub mod respond;

use crate::providers::codex::SUPPORTED_MODEL_NAMES;
use pollux_schema::openai::OpenaiModelList;
use std::sync::LazyLock;

const CODEX_RESPONSES_BODY_LIMIT_BYTES: usize = 100 * 1024 * 1024;

pub static CODEX_MODEL_LIST: LazyLock<OpenaiModelList> = LazyLock::new(|| {
    OpenaiModelList::from_model_names(SUPPORTED_MODEL_NAMES.iter().cloned(), "codex".to_string())
});

#[derive(Debug, Clone)]
pub struct CodexContext {
    pub model: String,
    pub stream: bool,
    pub model_mask: u64,
}

async fn debug_codex_responses_body_size(req: Request, next: Next) -> Response {
    let content_length = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    match content_length {
        Some(bytes) => {
            debug!(
                content_length_bytes = bytes,
                content_length_mib = format_args!("{:.2}", bytes as f64 / (1024.0 * 1024.0)),
                "Incoming Codex /responses request body size"
            );
        }
        None => {
            debug!("Incoming Codex /responses request body size unknown (no Content-Length)");
        }
    }

    next.run(req).await
}

pub fn router() -> Router<PolluxState> {
    Router::new()
        .route(
            "/codex/v1/responses",
            post(handlers::codex_response_handler)
                .layer(DefaultBodyLimit::max(CODEX_RESPONSES_BODY_LIMIT_BYTES))
                .layer(middleware::from_fn(debug_codex_responses_body_size)),
        )
        .route("/codex/v1/models", get(handlers::codex_models_handler))
        .route("/codex/resource:add", post(resource::codex_resource_add))
}
