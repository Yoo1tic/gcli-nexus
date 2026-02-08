use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use std::{
    fs,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;

#[tokio::test]
async fn antigravity_route_requires_key_rejects_bad_json_and_maps_no_credentials_to_503() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_nanos();

    let mut temp_path = std::env::temp_dir();
    temp_path.push(format!(
        "pollux-antigravity-basic-{}-{}.sqlite",
        std::process::id(),
        nanos
    ));

    let database_url = format!("sqlite:{}", temp_path.display());
    let db = pollux::db::spawn(&database_url).await;

    let mut cfg = pollux::config::Config::default();
    cfg.basic.pollux_key = "pwd".to_string();

    // Keep test behavior stable regardless of the repo's runtime `config.toml`.
    let model = pollux::config::CONFIG
        .antigravity()
        .model_list
        .first()
        .cloned()
        .unwrap_or_else(|| "gemini-2.5-pro".to_string());
    cfg.providers.antigravity.model_list = vec![model.clone()];

    // No Antigravity credentials inserted => valid requests should yield 503 (UNAVAILABLE).
    let providers = pollux::providers::Providers::spawn(db.clone(), &cfg).await;
    let pollux_key: Arc<str> = Arc::from(cfg.basic.pollux_key.clone());
    let state = pollux::server::router::PolluxState::new(
        providers,
        pollux_key.clone(),
        cfg.basic.insecure_cookie,
    );
    let app = pollux::server::router::pollux_router(state);

    let uri = format!("/antigravity/v1beta/models/{}:generateContent", model);
    let valid_body = r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;

    // 1) no key -> 401
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header("content-type", "application/json")
                .body(Body::from(valid_body))
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 2) correct key + invalid JSON -> 400
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::from("not-json"))
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 3) correct key + valid request -> 503 (no credentials configured)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::from(valid_body))
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = std::str::from_utf8(&body).expect("response body was not utf-8");
    assert_eq!(
        body_str,
        r#"{"error":{"code":503,"message":"No available credentials to process the request.","status":"UNAVAILABLE"}}"#
    );

    let _ = fs::remove_file(&temp_path);
}
