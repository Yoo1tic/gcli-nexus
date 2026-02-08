use serde::{Deserialize, Serialize};
use url::Url;

use super::ProviderDefaults;

/// Claude system preamble for Antigravity upstream strict-match validation.
///
/// Default preamble (repository baseline):
/// ```text
/// You are Antigravity, a powerful agentic AI coding assistant designed by the Google Deepmind team working on Advanced Agentic Coding. You are pair programming with a USER to solve their coding task. The task may require creating a new codebase, modifying or debugging an existing codebase, or simply answering a question.**Absolute paths only****Proactiveness**
/// ```
///
/// This value is sourced from `CLAUDE_SYSTEM_PREAMBLE` and can be overridden
/// by environment-variable injection during build/CI.
///
/// WARNING: Antigravity applies strict text matching. Any character change
/// (including missing spaces) may fail validation and trigger HTTP 429.
pub const CLAUDE_SYSTEM_PREAMBLE: &str = env!("CLAUDE_SYSTEM_PREAMBLE");

/// Antigravity provider configuration managed by Figment.
///
/// Notes:
/// - Provider defaults (proxy/multiplexing/retry) follow the same fallback semantics as other
///   providers: provider-level overrides win, otherwise `providers.defaults.*`.
/// - OAuth endpoints/client credentials are intentionally fixed to built-in defaults
///   (not configurable via `config.toml`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AntigravityConfig {
    /// Base API URL for the antigravity upstream.
    /// TOML: `providers.antigravity.api_url`.
    /// Default: `https://daily-cloudcode-pa.googleapis.com`.
    #[serde(default = "default_api_url")]
    pub api_url: Url,

    /// Optional upstream HTTP proxy. If set, used for reqwest clients.
    /// TOML: `providers.antigravity.proxy`. Example: `http://127.0.0.1:1080`.
    /// Falls back to `providers.defaults.proxy` when unset.
    #[serde(default)]
    pub proxy: Option<Url>,

    /// OAuth refresh requests per second (TPS) for the refresh worker.
    /// TOML: `providers.antigravity.oauth_tps`. Default: `5`.
    #[serde(default = "default_oauth_tps")]
    pub oauth_tps: usize,

    /// List of supported model names (allowlist). Each name maps to a bit in the global model
    /// catalog and corresponds to an independent credential queue.
    /// TOML: `providers.antigravity.model_list`.
    #[serde(default = "default_model_list")]
    pub model_list: Vec<String>,

    /// Allow HTTP/2 multiplexing for reqwest clients; disabled forces HTTP/1.
    /// TOML: `providers.antigravity.enable_multiplexing`.
    /// Falls back to `providers.defaults.enable_multiplexing`.
    #[serde(default)]
    pub enable_multiplexing: Option<bool>,

    /// Max retry attempts for antigravity upstream calls.
    /// TOML: `providers.antigravity.retry_max_times`.
    /// Falls back to `providers.defaults.retry_max_times`.
    #[serde(default)]
    pub retry_max_times: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct AntigravityResolvedConfig {
    pub api_url: Url,
    pub proxy: Option<Url>,
    pub oauth_tps: usize,
    pub model_list: Vec<String>,
    pub enable_multiplexing: bool,
    pub retry_max_times: usize,
    pub oauth_auth_url: Url,
    pub oauth_token_url: Url,
    pub oauth_redirect_url: Url,
    pub oauth_client_id: String,
    pub oauth_client_secret: String,
    pub oauth_scopes: Vec<String>,
}

impl AntigravityConfig {
    pub fn resolve(&self, defaults: &ProviderDefaults) -> AntigravityResolvedConfig {
        AntigravityResolvedConfig {
            api_url: self.api_url.clone(),
            proxy: self.proxy.clone().or_else(|| defaults.proxy.clone()),
            oauth_tps: self.oauth_tps,
            model_list: self.model_list.clone(),
            enable_multiplexing: self
                .enable_multiplexing
                .unwrap_or(defaults.enable_multiplexing),
            retry_max_times: self.retry_max_times.unwrap_or(defaults.retry_max_times),
            oauth_auth_url: default_oauth_auth_url(),
            oauth_token_url: default_oauth_token_url(),
            oauth_redirect_url: default_oauth_redirect_url(),
            oauth_client_id: default_oauth_client_id(),
            oauth_client_secret: default_oauth_client_secret(),
            oauth_scopes: default_oauth_scopes(),
        }
    }
}

impl Default for AntigravityConfig {
    fn default() -> Self {
        Self {
            api_url: default_api_url(),
            proxy: None,
            oauth_tps: default_oauth_tps(),
            model_list: default_model_list(),
            enable_multiplexing: None,
            retry_max_times: None,
        }
    }
}

fn default_api_url() -> Url {
    Url::parse("https://daily-cloudcode-pa.googleapis.com")
        .expect("default antigravity api_url must be a valid URL")
}

fn default_oauth_tps() -> usize {
    5
}

fn default_model_list() -> Vec<String> {
    vec!["gemini-3-flash".to_string()]
}

fn default_oauth_auth_url() -> Url {
    Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
        .expect("default oauth_auth_url must be a valid URL")
}

fn default_oauth_token_url() -> Url {
    Url::parse("https://oauth2.googleapis.com/token")
        .expect("default oauth_token_url must be a valid URL")
}

fn default_oauth_redirect_url() -> Url {
    Url::parse("http://localhost:8188").expect("default oauth_redirect_url must be a valid URL")
}

fn default_oauth_client_id() -> String {
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com".to_string()
}

fn default_oauth_client_secret() -> String {
    "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".to_string()
}

fn default_oauth_scopes() -> Vec<String> {
    vec![
        "https://www.googleapis.com/auth/cloud-platform".to_string(),
        "https://www.googleapis.com/auth/userinfo.email".to_string(),
        "https://www.googleapis.com/auth/userinfo.profile".to_string(),
        "https://www.googleapis.com/auth/cclog".to_string(),
        "https://www.googleapis.com/auth/experimentsandconfigs".to_string(),
    ]
}
