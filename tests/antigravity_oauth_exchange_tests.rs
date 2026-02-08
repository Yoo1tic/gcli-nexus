use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    routing::post,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
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

fn cookie_header_from_set_cookie_headers(headers: &axum::http::HeaderMap) -> String {
    let mut pairs: Vec<String> = Vec::new();
    for v in headers.get_all(header::SET_COOKIE).iter() {
        let s = v.to_str().expect("set-cookie header was not valid utf-8");
        let first = s.split(';').next().unwrap_or("");
        let mut parts = first.splitn(2, '=');
        let name = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        if !name.trim().is_empty() {
            pairs.push(format!("{}={}", name.trim(), value));
        }
    }
    pairs.join("; ")
}

async fn token_handler(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    state.reqs.lock().unwrap().push(Captured {
        path: "/token".to_string(),
        headers,
        body: body.to_vec(),
    });

    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    let grant_type = form.get("grant_type").map(String::as_str).unwrap_or("");

    match grant_type {
        "authorization_code" => (
            StatusCode::OK,
            Json(json!({
                "access_token": "access-from-code",
                "token_type": "bearer",
                "expires_in": 3600,
                "refresh_token": "refresh-from-code"
            })),
        ),
        "refresh_token" => (
            StatusCode::OK,
            Json(json!({
                "access_token": "access-from-refresh",
                "token_type": "bearer",
                "expires_in": 3600
            })),
        ),
        _ => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "unsupported_grant_type",
                "grant_type": grant_type,
            })),
        ),
    }
}

async fn load_code_assist_handler(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    state.reqs.lock().unwrap().push(Captured {
        path: "/v1internal:loadCodeAssist".to_string(),
        headers,
        body: body.to_vec(),
    });

    (
        StatusCode::OK,
        Json(json!({
            "cloudaicompanionProject": "project-1",
            "allowedTiers": [{ "id": "FREE", "isDefault": true }]
        })),
    )
}

async fn onboard_user_handler(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    state.reqs.lock().unwrap().push(Captured {
        path: "/v1internal:onboardUser".to_string(),
        headers,
        body: body.to_vec(),
    });

    (
        StatusCode::OK,
        Json(json!({
            "done": true,
            "response": {
                "cloudaicompanionProject": { "id": "project-1" }
            }
        })),
    )
}

#[tokio::test]
async fn antigravity_oauth_callback_exchanges_code_against_mock_token_endpoint_and_returns_202() {
    let captured = CaptureState::default();
    let mock = Router::new()
        .route("/token", post(token_handler))
        .route("/v1internal:loadCodeAssist", post(load_code_assist_handler))
        .route("/v1internal:onboardUser", post(onboard_user_handler))
        .with_state(captured.clone());
    let base = spawn_test_server(mock).await;
    let token_url = base.join("/token").expect("token url");

    let temp_path = unique_sqlite_path("antigravity-oauth-exchange");
    let database_url = format!("sqlite:{}", temp_path.display());
    let db = pollux::db::spawn(&database_url).await;

    let mut cfg = pollux::config::Config::default();
    cfg.basic.pollux_key = "pwd".to_string();

    cfg.providers.antigravity.api_url = base;

    let mut providers = pollux::providers::Providers::spawn(db.clone(), &cfg).await;

    // Keep OAuth endpoints deterministic and local for tests.
    let antigravity_cfg = Arc::make_mut(&mut providers.antigravity_cfg);
    antigravity_cfg.oauth_auth_url =
        Url::parse("http://oauth.test/authorize").expect("valid auth url");
    antigravity_cfg.oauth_token_url = token_url;
    antigravity_cfg.oauth_redirect_url =
        Url::parse("http://localhost:8188").expect("valid redirect url");
    let pollux_key: Arc<str> = Arc::from(cfg.basic.pollux_key.clone());
    let state =
        pollux::server::router::PolluxState::new(providers, pollux_key, cfg.basic.insecure_cookie);
    let app = pollux::server::router::pollux_router(state);

    // 1) Start OAuth flow to set cookies.
    let entry_resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/antigravity/auth")
                .body(axum::body::Body::empty())
                .expect("build request"),
        )
        .await
        .expect("request failed");
    assert!(entry_resp.status().is_redirection());

    let location = entry_resp
        .headers()
        .get(header::LOCATION)
        .expect("missing location header")
        .to_str()
        .expect("location header was not utf-8");
    let auth_url = Url::parse(location).expect("location was not a valid url");
    let state = auth_url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_k, v)| v.to_string())
        .expect("missing state query param in auth redirect");

    let cookie_header = cookie_header_from_set_cookie_headers(entry_resp.headers());

    // 2) Callback hits token endpoint.
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri(format!("/?code=code-1&state={}", state))
                .header(header::COOKIE, cookie_header)
                .body(axum::body::Body::empty())
                .expect("build request"),
        )
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let reqs = captured.reqs.lock().unwrap().clone();
    assert!(
        reqs.iter().any(|r| r.path == "/token"),
        "expected mock token endpoint to be hit"
    );

    for r in reqs.iter().filter(|r| r.path.starts_with("/v1internal:")) {
        let auth = r
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            auth.starts_with("Bearer "),
            "expected auth header on {} request, got: {:?}",
            r.path,
            auth
        );
    }

    let redirect_uri = "http://localhost:8188/";
    let mut saw_code_exchange = false;
    for r in reqs.iter().filter(|r| r.path == "/token") {
        let form: HashMap<String, String> =
            url::form_urlencoded::parse(&r.body).into_owned().collect();
        if form.get("grant_type").map(String::as_str) == Some("authorization_code") {
            saw_code_exchange = true;
            assert_eq!(form.get("code").map(String::as_str), Some("code-1"));
            assert_eq!(
                form.get("redirect_uri").map(String::as_str),
                Some(redirect_uri)
            );
            assert!(
                form.get("code_verifier").map(|s| !s.trim().is_empty()) == Some(true),
                "expected PKCE code_verifier to be present"
            );
        }
    }
    assert!(
        saw_code_exchange,
        "expected at least one authorization_code exchange request"
    );

    let _ = fs::remove_file(&temp_path);
}
