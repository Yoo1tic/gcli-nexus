use axum::{
    Json, Router,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode, header},
    routing::post,
};
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;
use tower::ServiceExt;
use url::Url;

#[derive(Clone, Default)]
struct CaptureState {
    reqs: Arc<Mutex<Vec<Captured>>>,
}

#[derive(Debug, Clone)]
struct Captured {
    path: String,
    query: Option<String>,
    headers: HeaderMap,
    body: Vec<u8>,
}

fn unique_sqlite_path(prefix: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_nanos();

    let mut temp_path = std::env::temp_dir();
    temp_path.push(format!(
        "pollux-{prefix}-{}-{}.sqlite",
        std::process::id(),
        nanos
    ));
    temp_path
}

async fn spawn_test_server(app: Router) -> Url {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    let base = Url::parse(&format!("http://{}", addr)).expect("valid base url");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server run");
    });

    base
}

async fn generate_handler(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    state.reqs.lock().unwrap().push(Captured {
        path: "/v1internal:generateContent".to_string(),
        query: None,
        headers,
        body: body.to_vec(),
    });

    (
        StatusCode::OK,
        Json(json!({
            "response": {
                "candidates": [{}]
            }
        })),
    )
}

async fn stream_handler(
    State(state): State<CaptureState>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    state.reqs.lock().unwrap().push(Captured {
        path: "/v1internal:streamGenerateContent".to_string(),
        query,
        headers,
        body: body.to_vec(),
    });

    let sse = r#"data: {\"response\":{\"candidates\":[{}]}}\n\n"#;
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/event-stream")],
        sse,
    )
}

#[tokio::test]
async fn antigravity_proxy_posts_expected_upstream_url_headers_and_envelope_for_generate_and_stream()
 {
    let captured = CaptureState::default();
    let upstream = Router::new()
        .route("/v1internal:generateContent", post(generate_handler))
        .route("/v1internal:streamGenerateContent", post(stream_handler))
        .with_state(captured.clone());
    let upstream_base = spawn_test_server(upstream).await;

    let temp_path = unique_sqlite_path("antigravity-upstream-shape");
    let database_url = format!("sqlite:{}", temp_path.display());
    let db = pollux::db::spawn(&database_url).await;

    // Insert an active credential before spawning providers so the actor loads it.
    let _id = db
        .create(pollux::db::ProviderCreate::Antigravity(
            pollux::db::AntigravityCreate {
                email: None,
                sub: None,
                project_id: "project-1".to_string(),
                refresh_token: "refresh-1".to_string(),
                access_token: Some("access-1".to_string()),
                expiry: Utc::now() + ChronoDuration::minutes(60),
            },
        ))
        .await
        .expect("insert antigravity credential");

    let mut cfg = pollux::config::Config::default();
    cfg.basic.pollux_key = "pwd".to_string();
    cfg.providers.antigravity.api_url = upstream_base;

    let model = "gemini-2.5-pro".to_string();
    cfg.providers.antigravity.model_list = vec![model.clone()];

    let providers = pollux::providers::Providers::spawn(db.clone(), &cfg).await;
    let pollux_key: Arc<str> = Arc::from(cfg.basic.pollux_key.clone());
    let state = pollux::server::router::PolluxState::new(
        providers,
        pollux_key.clone(),
        cfg.basic.insecure_cookie,
    );
    let app = pollux::server::router::pollux_router(state);

    let valid_body = r#"{"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#;
    let request_json: Value = serde_json::from_str(valid_body).expect("valid request json");

    // 1) Unary generateContent
    let uri = format!("/antigravity/v1beta/models/{}:generateContent", model);
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&uri)
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(axum::body::Body::from(valid_body))
                .expect("build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);

    // 2) Streaming streamGenerateContent
    let uri = format!("/antigravity/v1beta/models/{}:streamGenerateContent", model);
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&uri)
                .header("content-type", "application/json")
                .header("x-goog-api-key", pollux_key.as_ref())
                .body(axum::body::Body::from(valid_body))
                .expect("build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);

    let reqs = captured.reqs.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2, "expected exactly two upstream requests");

    let unary = reqs
        .iter()
        .find(|r| r.path == "/v1internal:generateContent")
        .expect("missing generateContent request");
    let stream = reqs
        .iter()
        .find(|r| r.path == "/v1internal:streamGenerateContent")
        .expect("missing streamGenerateContent request");

    // URL shape
    assert!(
        stream.query.as_deref().unwrap_or("").contains("alt=sse"),
        "expected stream request query to include alt=sse, got: {:?}",
        stream.query
    );

    // Required headers
    for r in [unary, stream] {
        let auth = r
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(auth, "Bearer access-1");
    }

    // JSON envelope body
    for r in [unary, stream] {
        let body_json: Value = serde_json::from_slice(&r.body).expect("upstream request body json");
        assert_eq!(body_json.get("model"), Some(&Value::String(model.clone())));
        assert_eq!(
            body_json.get("project"),
            Some(&Value::String("project-1".to_string()))
        );

        let request_id = body_json
            .get("requestId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            request_id.starts_with("agent/"),
            "expected requestId in body to start with agent/, got: {request_id:?}"
        );

        let mut segments = request_id.split('/');
        let seg0 = segments.next().unwrap_or_default();
        let seg1 = segments.next().unwrap_or_default();
        let seg2 = segments.next().unwrap_or_default();
        assert_eq!(seg0, "agent");
        assert!(
            seg1.parse::<u128>().is_ok(),
            "expected requestId middle segment to be timestamp ms, got: {request_id:?}"
        );
        assert_eq!(seg2.len(), 36, "expected UUID segment, got: {request_id:?}");

        assert_eq!(
            body_json.get("userAgent"),
            Some(&Value::String("antigravity".to_string())),
            "expected userAgent field in body"
        );

        assert_eq!(
            body_json.get("requestType"),
            Some(&Value::String("agent".to_string())),
            "expected requestType field in body"
        );
        let request_val = body_json.get("request").expect("request field missing");

        let session_id = request_val
            .get("sessionId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            session_id.starts_with('-'),
            "expected request.sessionId to start with '-', got: {session_id:?}"
        );
        assert!(
            session_id
                .trim_start_matches('-')
                .chars()
                .all(|character| character.is_ascii_digit()),
            "expected request.sessionId numeric payload, got: {session_id:?}"
        );

        assert_eq!(
            request_val.get("contents"),
            request_json.get("contents"),
            "expected request.contents to equal original pollux body contents"
        );
    }

    let _ = fs::remove_file(&temp_path);
}
