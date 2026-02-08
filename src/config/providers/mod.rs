mod antigravity;
mod codex;
mod geminicli;

pub use antigravity::{AntigravityConfig, AntigravityResolvedConfig, CLAUDE_SYSTEM_PREAMBLE};
pub use codex::{CodexConfig, CodexResolvedConfig};
pub use geminicli::{GeminiCliConfig, GeminiCliResolvedConfig};

use serde::{Deserialize, Serialize};
use url::Url;

/// Global provider defaults (used when provider-level config is unset).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderDefaults {
    /// Optional upstream HTTP proxy. If set, used for reqwest clients.
    /// TOML: `providers.defaults.proxy`. Example: `http://127.0.0.1:1080`.
    #[serde(default)]
    pub proxy: Option<Url>,

    /// Allow HTTP/2 multiplexing for reqwest clients; disabled forces HTTP/1.
    /// TOML: `providers.defaults.enable_multiplexing`. Default: `false`.
    #[serde(default = "default_enable_multiplexing")]
    pub enable_multiplexing: bool,

    /// Max retry attempts for upstream calls.
    /// TOML: `providers.defaults.retry_max_times`. Default: `3`.
    #[serde(default = "default_retry_max_times")]
    pub retry_max_times: usize,
}

impl Default for ProviderDefaults {
    fn default() -> Self {
        Self {
            proxy: None,
            enable_multiplexing: default_enable_multiplexing(),
            retry_max_times: default_retry_max_times(),
        }
    }
}

/// All provider configurations.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ProvidersConfig {
    /// Global defaults for providers (overridden per provider if set).
    #[serde(default)]
    pub defaults: ProviderDefaults,

    /// Gemini CLI provider configuration.
    #[serde(default)]
    pub geminicli: GeminiCliConfig,

    /// Codex passthrough provider configuration.
    #[serde(default)]
    pub codex: CodexConfig,

    /// Antigravity provider configuration.
    #[serde(default)]
    pub antigravity: AntigravityConfig,
}

fn default_enable_multiplexing() -> bool {
    false
}

fn default_retry_max_times() -> usize {
    3
}
