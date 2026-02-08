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
async fn codex_response_route_rejects_bad_requests_and_requires_key() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_nanos();

    let mut temp_path = std::env::temp_dir();
    temp_path.push(format!(
        "pollux-codex-basic-{}-{}.sqlite",
        std::process::id(),
        nanos
    ));

    let database_url = format!("sqlite:{}", temp_path.display());
    let db = pollux::db::spawn(&database_url).await;

    let mut cfg = pollux::config::Config::default();
    cfg.basic.pollux_key = "pwd".to_string();
    // Keep test behavior stable regardless of the repo's runtime `config.toml`.
    let model = pollux::config::CONFIG
        .codex()
        .model_list
        .first()
        .cloned()
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    cfg.providers.codex.model_list = vec![model.clone()];

    // No Codex keys inserted => valid requests should yield 503 (NO_CREDENTIAL).
    let providers = pollux::providers::Providers::spawn(db.clone(), &cfg).await;
    let pollux_key: Arc<str> = Arc::from(cfg.basic.pollux_key.clone());
    let state = pollux::server::router::PolluxState::new(
        providers,
        pollux_key.clone(),
        cfg.basic.insecure_cookie,
    );
    let app = pollux::server::router::pollux_router(state);

    // 1) no key -> 401
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codex/v1/responses")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"gpt-4o-mini"}"#))
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
                .uri("/codex/v1/responses")
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::from("not-json"))
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 3) correct key + {} -> 400
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codex/v1/responses")
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::from("{}"))
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 4) correct key + {"model":"any"} -> 503 (no upstream keys configured)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codex/v1/responses")
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::from(format!(r#"{{ "model": "{model}", "foo": 1 }}"#)))
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
        r#"{"error":{"code":"NO_CREDENTIAL","message":"No available credentials to process the request.","type":"NO_CREDENTIAL"}}"#
    );

    // 5) correct key + 30 MiB JSON body -> 503 (Codex endpoint limit is higher than 30 MiB)
    let oversized_input = "a".repeat(30 * 1024 * 1024 + 1024);
    let oversized_payload = format!(r#"{{"model":"{model}","input":"{oversized_input}"}}"#);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codex/v1/responses")
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::from(oversized_payload))
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
        r#"{"error":{"code":"NO_CREDENTIAL","message":"No available credentials to process the request.","type":"NO_CREDENTIAL"}}"#
    );

    // 6) GET /codex/v1/models: no key -> 401
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/codex/v1/models")
                .body(Body::empty())
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 7) GET /codex/v1/models: correct key -> 200 with JSON list
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/codex/v1/models")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::empty())
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = std::str::from_utf8(&body).expect("response body was not utf-8");

    // Should be a Codex-style model list (object=list) and include the default model name.
    assert!(body_str.contains("\"object\":\"list\""));
    assert!(body_str.contains(&format!("\"id\":\"{}\"", model)));

    // 8) POST /codex/resource:add: no key -> 401
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codex/resource:add")
                .header("content-type", "application/json")
                .body(Body::from("[]"))
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 9) POST /codex/resource:add: correct key + non-array payload -> 400
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codex/resource:add")
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::from(r#"{"refresh_tokens":["rt_01..."]}"#))
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = std::str::from_utf8(&body).expect("response body was not utf-8");
    assert!(body_str.contains("数据格式不允许"));

    // 10) POST /codex/resource:add: correct key + array payload -> 202 + Success
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/codex/resource:add")
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(Body::from("[]"))
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let body = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = std::str::from_utf8(&body).expect("response body was not utf-8");
    assert_eq!(body_str, "Success");

    let _ = fs::remove_file(&temp_path);
}
