use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use std::{
    fs,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;

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

async fn build_app_for_oauth_tests() -> (axum::Router, std::path::PathBuf) {
    let temp_path = unique_sqlite_path("antigravity-oauth");
    let database_url = format!("sqlite:{}", temp_path.display());
    let db = pollux::db::spawn(&database_url).await;

    let mut cfg = pollux::config::Config::default();
    cfg.basic.pollux_key = "pwd".to_string();

    let mut providers = pollux::providers::Providers::spawn(db.clone(), &cfg).await;

    // Keep OAuth endpoints deterministic and off the public internet for tests.
    let antigravity_cfg = Arc::make_mut(&mut providers.antigravity_cfg);
    antigravity_cfg.oauth_auth_url =
        url::Url::parse("http://oauth.test/authorize").expect("valid auth url");
    antigravity_cfg.oauth_token_url =
        url::Url::parse("http://oauth.test/token").expect("valid token url");
    antigravity_cfg.oauth_redirect_url =
        url::Url::parse("http://localhost:8188").expect("valid redirect url");
    let pollux_key: Arc<str> = Arc::from(cfg.basic.pollux_key.clone());
    let state =
        pollux::server::router::PolluxState::new(providers, pollux_key, cfg.basic.insecure_cookie);
    let app = pollux::server::router::pollux_router(state);
    (app, temp_path)
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

#[tokio::test]
async fn antigravity_oauth_routes_set_cookies_and_return_explicit_flow_errors() {
    // NOTE: `pollux::db::spawn()` registers a singleton ractor actor by name within a process.
    // Keep all OAuth route assertions in one test to avoid multiple spawns in this test binary.
    let (app, temp_path) = build_app_for_oauth_tests().await;

    // 1) GET /antigravity/auth returns redirect and sets provider-specific cookies.
    let entry_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/antigravity/auth")
                .body(Body::empty())
                .expect("failed to build request"),
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
    assert!(location.starts_with("http://oauth.test/authorize"));

    let set_cookies: Vec<String> = entry_resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap_or("").to_string())
        .collect();

    assert!(
        set_cookies
            .iter()
            .any(|c| c.starts_with("antigravity_oauth_csrf_token=")),
        "expected csrf cookie, got: {set_cookies:?}"
    );
    assert!(
        set_cookies
            .iter()
            .any(|c| c.starts_with("antigravity_oauth_pkce_verifier=")),
        "expected pkce cookie, got: {set_cookies:?}"
    );

    // 2) GET / callback without cookies => explicit flow error.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/?code=fake_code&state=fake_state")
                .body(Body::empty())
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = std::str::from_utf8(&body).expect("response body was not utf-8");
    assert!(body_str.contains("\"code\":\"OAUTH_SESSION_MISSING\""));

    // 3) GET / callback with cookies but mismatched state => explicit CSRF error.
    let cookie_header = cookie_header_from_set_cookie_headers(entry_resp.headers());
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/?code=fake_code&state=wrong_state")
                .header(header::COOKIE, cookie_header)
                .body(Body::empty())
                .expect("failed to build request"),
        )
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = std::str::from_utf8(&body).expect("response body was not utf-8");
    assert!(body_str.contains("\"code\":\"CSRF_MISMATCH\""));

    let _ = fs::remove_file(&temp_path);
}
