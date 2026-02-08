use crate::providers::Providers;
use crate::providers::codex::CODEX_USER_AGENT;
use crate::providers::geminicli::GEMINICLI_USER_AGENT;
use crate::server::guards::auth::RequireKeyAuth;
use crate::server::routes::codex::oauth::{codex_oauth_callback, codex_oauth_entry};
use crate::server::routes::geminicli::oauth::{google_oauth_callback, google_oauth_entry};
use crate::server::routes::{codex, geminicli};

use axum::{
    Router,
    extract::{FromRef, Request},
    http::{HeaderName, StatusCode, Version, header::USER_AGENT},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use axum_extra::extract::cookie::Key;
use base64::Engine as _;
use rand::RngCore;
use reqwest::header::{CONNECTION, HeaderMap, HeaderValue};
use std::time::Instant;
use std::{sync::Arc, sync::LazyLock, time::Duration};
use tracing::{error, info, warn};

/// Global cookie signing/encryption key for PrivateCookieJar.
static COOKIE_KEY: LazyLock<Key> = LazyLock::new(Key::generate);

const MAX_REQUEST_ID_LEN: usize = 128;
const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

fn generate_request_id() -> String {
    // 96 bits => 16 chars base64url (no padding).
    let mut bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn format_http_version(version: Version) -> &'static str {
    match version {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2",
        Version::HTTP_3 => "HTTP/3",
        _ => "HTTP/?",
    }
}

#[derive(Clone)]
pub struct PolluxState {
    pub providers: Providers,
    pub client: reqwest::Client,
    pub codex_client: reqwest::Client,
    pub pollux_key: Arc<str>,
    pub insecure_cookie: bool,
}

impl PolluxState {
    pub fn new(providers: Providers, pollux_key: Arc<str>, insecure_cookie: bool) -> Self {
        let geminicli_cfg = providers.geminicli_cfg.clone();
        let codex_cfg = providers.codex_cfg.clone();

        fn build_client(
            user_agent: &str,
            proxy: Option<url::Url>,
            enable_multiplexing: bool,
        ) -> reqwest::Client {
            let mut headers = HeaderMap::new();

            let mut builder = reqwest::Client::builder()
                .user_agent(user_agent)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(10 * 60));

            if let Some(proxy_url) = proxy {
                let proxy = reqwest::Proxy::all(proxy_url.as_str())
                    .expect("invalid proxy url for reqwest client");
                builder = builder.proxy(proxy);
            }

            if !enable_multiplexing {
                headers.insert(CONNECTION, HeaderValue::from_static("close"));

                builder = builder
                    .http1_only()
                    .pool_max_idle_per_host(0)
                    .pool_idle_timeout(Duration::from_secs(0));
            } else {
                builder = builder.http2_adaptive_window(true);
            }

            builder
                .default_headers(headers)
                .build()
                .expect("failed to build reqwest client")
        }
        let client = build_client(
            GEMINICLI_USER_AGENT,
            geminicli_cfg.proxy.clone(),
            geminicli_cfg.enable_multiplexing,
        );
        let codex_client = build_client(
            CODEX_USER_AGENT,
            codex_cfg.proxy.clone(),
            codex_cfg.enable_multiplexing,
        );

        Self {
            providers,
            client,
            codex_client,
            pollux_key,
            insecure_cookie,
        }
    }
}

impl FromRef<PolluxState> for Key {
    fn from_ref(state: &PolluxState) -> Self {
        let _ = state; // state not used to fetch the static key
        COOKIE_KEY.clone()
    }
}

async fn not_found_handler() -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn access_log(req: Request, next: Next) -> Response {
    // Capture request metadata before moving `req` into the handler stack.
    let method = req.method().clone();
    let uri = req.uri().clone();
    let version = req.version();

    let request_id = req
        .headers()
        .get(X_REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= MAX_REQUEST_ID_LEN)
        .map(str::to_string)
        .unwrap_or_else(generate_request_id);

    let user_agent = req
        .headers()
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();

    let start = Instant::now();
    let mut resp = next.run(req).await;

    // Always reflect `x-request-id` for easier correlation, even if the client didn't send one.
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        resp.headers_mut().insert(X_REQUEST_ID, value);
    }

    let status = resp.status();
    let latency_ms = start.elapsed().as_millis() as u64;
    let path = uri.path();
    let protocol = format_http_version(version);

    // Note: for SSE/streaming responses, `latency_ms` is time-to-first-byte (handler return),
    // not the full stream duration.
    if status.is_server_error() {
        error!(
            "| {:>3} | {} | {:^7} | {:<8} | {} | {}ms | {}",
            status.as_u16(),
            request_id,
            method.as_str(),
            protocol,
            path,
            latency_ms,
            user_agent
        );
    } else if status.is_client_error() {
        warn!(
            "| {:>3} | {} | {:^7} | {:<8} | {} | {}ms | {}",
            status.as_u16(),
            request_id,
            method.as_str(),
            protocol,
            path,
            latency_ms,
            user_agent
        );
    } else {
        info!(
            "| {:>3} | {} | {:^7} | {:<8} | {} | {}ms | {}",
            status.as_u16(),
            request_id,
            method.as_str(),
            protocol,
            path,
            latency_ms,
            user_agent
        );
    }

    resp
}

pub fn pollux_router(state: PolluxState) -> Router {
    let gemini = geminicli::router()
        .layer(middleware::from_extractor_with_state::<RequireKeyAuth, _>(
            state.clone(),
        ));

    let codex = codex::router().layer(middleware::from_extractor_with_state::<RequireKeyAuth, _>(
        state.clone(),
    ));

    let oauth = Router::new()
        // Oauth Redirect path
        .route("/geminicli/auth", get(google_oauth_entry))
        .route("/codex/auth", get(codex_oauth_entry))
        // GeminiCli Callback paths
        .route("/oauth2callback", get(google_oauth_callback))
        // Codex Callback paths
        .route("/auth/callback", get(codex_oauth_callback));

    Router::new()
        .merge(oauth)
        .merge(gemini)
        .merge(codex)
        .fallback(not_found_handler)
        .with_state(state)
        .layer(middleware::from_fn(access_log))
}
