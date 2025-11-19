use crate::config::{CLI_USER_AGENT, CONFIG};
use crate::service::credentials_actor::CredentialsHandle;
use axum::{Router, middleware, routing::any};
use reqwest::header::{CONNECTION, HeaderMap, HeaderValue};
use std::time::Duration;

#[derive(Clone)]
pub struct NexusState {
    pub handle: CredentialsHandle,
    pub client: reqwest::Client,
}

impl NexusState {
    pub fn new(handle: CredentialsHandle) -> Self {
        let mut headers = HeaderMap::new();

        let mut builder = reqwest::Client::builder()
            .user_agent(CLI_USER_AGENT.as_str())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(15 * 60));

        if let Some(proxy_url) = CONFIG.proxy.clone() {
            let proxy = reqwest::Proxy::all(proxy_url.as_str())
                .expect("invalid PROXY url for reqwest client");
            builder = builder.proxy(proxy);
        }

        if !CONFIG.enable_multiplexing {
            headers.insert(CONNECTION, HeaderValue::from_static("close"));

            builder = builder.http1_only().pool_max_idle_per_host(0);
        } else {
            builder = builder.http2_adaptive_window(true);
        }

        let client = builder
            .default_headers(headers)
            .build()
            .expect("failed to build reqwest client for proxy");

        Self { handle, client }
    }
}

pub fn nexus_router(state: NexusState) -> Router {
    use crate::middleware::auth::RequireKeyAuth;
    use crate::middleware::gemini_request::gemini_cli_handler;
    Router::new()
        .route("/v1beta/models/{*path}", any(gemini_cli_handler))
        .layer(middleware::from_extractor::<RequireKeyAuth>())
        .with_state(state)
}
