use crate::server::router::PolluxState;
use axum::extract::rejection::JsonRejection;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
pub struct AntigravityResourceSeed {
    /// Only this field is used; all other fields are ignored.
    ///
    /// Aliases support common naming across other tools.
    #[serde(alias = "refreshToken")]
    pub refresh_token: Option<String>,
}

/// POST /antigravity/resource:add
///
/// 0-trust credential ingestion. Mirrors `/geminicli/resource:add` semantics.
pub async fn antigravity_resource_add(
    State(state): State<PolluxState>,
    payload: Result<Json<Vec<AntigravityResourceSeed>>, JsonRejection>,
) -> axum::response::Response {
    let Json(seeds) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "数据格式不允许：请求体必须是 JSON 数组，例如 [{\"refresh_token\":\"...\"}]",
            )
                .into_response();
        }
    };

    let mut seen: HashSet<String> = HashSet::new();
    let refresh_tokens: Vec<String> = seeds
        .into_iter()
        .filter_map(|s| s.refresh_token)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        // Deduplicate within this request to avoid redundant refresh work.
        .filter(|t| seen.insert(t.clone()))
        .collect();

    state
        .providers
        .antigravity
        .submit_refresh_tokens(refresh_tokens)
        .await;

    (StatusCode::ACCEPTED, "Success").into_response()
}
