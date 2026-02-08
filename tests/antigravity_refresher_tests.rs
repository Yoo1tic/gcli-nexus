use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    routing::post,
};
use base64::Engine as _;
use pollux::config::AntigravityResolvedConfig;
use pollux::providers::antigravity::client::oauth::{
    endpoints::AntigravityOauthEndpoints, ops::AntigravityOauthOps,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;
use url::Url;

#[derive(Clone, Default)]
struct CaptureState {
    bodies: Arc<Mutex<Vec<Captured>>>,
}

#[derive(Debug, Clone)]
struct Captured {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
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

fn make_cfg(token_url: Url, api_url: Url) -> AntigravityResolvedConfig {
    AntigravityResolvedConfig {
        api_url,
        proxy: None,
        oauth_tps: 5,
        model_list: vec!["gemini-2.5-pro".to_string()],
        enable_multiplexing: true,
        retry_max_times: 3,
        oauth_auth_url: Url::parse("http://oauth.test/authorize").unwrap(),
        oauth_token_url: token_url,
        oauth_redirect_url: Url::parse("http://localhost:8188").unwrap(),
        oauth_client_id: "client-id".to_string(),
        oauth_client_secret: "client-secret".to_string(),
        oauth_scopes: vec!["openid".to_string()],
    }
}

#[tokio::test]
async fn refresh_grant_posts_expected_form_fields() {
    let captured = CaptureState::default();

    let app = Router::new()
        .route("/token", post(token_handler))
        .with_state(captured.clone());

    let base = spawn_test_server(app).await;
    let token_url = base.join("/token").unwrap();

    let cfg = make_cfg(token_url, Url::parse("http://api.test").unwrap());
    let http = reqwest::Client::new();

    let _ = AntigravityOauthEndpoints::refresh_access_token_raw(&cfg, "refresh-token-1", http)
        .await
        .expect("refresh token exchange should succeed");

    let reqs = captured.bodies.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "expected exactly one token request");
    let first = &reqs[0];
    assert_eq!(first.path, "/token");

    let form: HashMap<String, String> = url::form_urlencoded::parse(&first.body)
        .into_owned()
        .collect();

    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("refresh_token")
    );
    assert_eq!(
        form.get("refresh_token").map(String::as_str),
        Some("refresh-token-1")
    );

    // Some OAuth clients send client credentials in the body, others use HTTP Basic auth.
    let has_body_creds = form.get("client_id").map(String::as_str) == Some("client-id")
        && form.get("client_secret").map(String::as_str) == Some("client-secret");

    let has_basic_auth = first
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Basic "))
        .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
        .and_then(|raw| String::from_utf8(raw).ok())
        .as_deref()
        == Some("client-id:client-secret");

    assert!(
        has_body_creds || has_basic_auth,
        "expected client credentials via body or basic auth; form keys: {:?}, auth header: {:?}",
        form.keys().collect::<Vec<_>>(),
        first.headers.get(header::AUTHORIZATION)
    );
}

async fn token_handler(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    state.bodies.lock().unwrap().push(Captured {
        path: "/token".to_string(),
        headers,
        body: body.to_vec(),
    });

    (
        StatusCode::OK,
        Json(json!({
            "access_token": "access-1",
            "token_type": "bearer",
            "expires_in": 3600
        })),
    )
}

#[tokio::test]
async fn load_code_assist_posts_expected_json_body() {
    let captured = CaptureState::default();

    let app = Router::new()
        .route("/v1internal:loadCodeAssist", post(load_code_assist_handler))
        .with_state(captured.clone());

    let base = spawn_test_server(app).await;
    let api_url = base;
    let token_url = Url::parse("http://oauth.test/token").unwrap();
    let cfg = make_cfg(token_url, api_url);

    let http = reqwest::Client::new();
    let _ = AntigravityOauthOps::load_code_assist(&cfg, "access-token-1", http)
        .await
        .expect("loadCodeAssist should succeed");

    let reqs = captured.bodies.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "expected exactly one loadCodeAssist request");
    let first = &reqs[0];
    assert_eq!(first.path, "/v1internal:loadCodeAssist");

    let body_json: Value = serde_json::from_slice(&first.body).expect("request body json");
    assert_eq!(body_json, AntigravityOauthOps::load_code_assist_body_json());
}

async fn load_code_assist_handler(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> (StatusCode, Json<Value>) {
    state.bodies.lock().unwrap().push(Captured {
        path: "/v1internal:loadCodeAssist".to_string(),
        headers,
        body: body.to_vec(),
    });

    (
        StatusCode::OK,
        Json(json!({
            "allowedTiers": [{ "id": "FREE", "isDefault": true }]
        })),
    )
}
