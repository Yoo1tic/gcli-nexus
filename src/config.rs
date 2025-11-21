use figment::{
    Figment,
    providers::{Env, Serialized},
};
use reqwest;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::LazyLock,
};
use url::Url;

/// Application configuration managed by Figment.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// HTTP server listen address (e.g., "0.0.0.0", "127.0.0.1").
    /// Env: `LISTEN_ADDR`. Default: `0.0.0.0`.
    #[serde(default = "default_listen_ip")]
    pub listen_addr: IpAddr,

    /// HTTP server listen port.
    /// Env: `LISTEN_PORT`. Default: `8188`.
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,

    /// Database URL for SQLite.
    /// Env: `DATABASE_URL`. Default: `sqlite://data.db`.
    #[serde(default)]
    pub database_url: String,

    /// Log level for tracing subscriber initialization (e.g., "error", "warn", "info", "debug", "trace").
    /// Env: `LOGLEVEL`. Default: `info`.
    #[serde(default)]
    pub loglevel: String,

    /// Optional upstream HTTP proxy. If set, used for reqwest clients.
    /// Env: `PROXY`. Example: `http://127.0.0.1:1080`.
    #[serde(default)]
    pub proxy: Option<Url>,

    /// Authentication key for inbound request validation (required, non-empty).
    /// Env: `NEXUS_KEY`. Must be provided.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_string_lax")]
    pub nexus_key: String,

    /// Max concurrent Google OAuth refreshes processed by the worker.
    /// Env: `REFRESH_CONCURRENCY`. Default: `10`.
    #[serde(default)]
    pub refresh_concurrency: usize,

    /// List of Gemini model names treated as "big" models.
    /// Env: `BIGMODEL_LIST`. Default: empty.
    #[serde(default)]
    pub bigmodel_list: Vec<String>,

    /// Optional directory containing credential files to preload at startup.
    /// Env: `CRED_PATH`. Example: `./credentials`. Default: unset (skip preload).
    #[serde(default)]
    pub cred_path: Option<PathBuf>,

    /// Allow HTTP/2 multiplexing for reqwest clients; disabled forces HTTP/1.
    /// Env: `ENABLE_MULTIPLEXING`. Default: `false`.
    #[serde(default)]
    pub enable_multiplexing: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_ip(),
            listen_port: default_listen_port(),
            database_url: "sqlite://data.db".to_string(),
            loglevel: "info".to_string(),
            proxy: None,
            nexus_key: "pwd".to_string(),
            refresh_concurrency: 10,
            bigmodel_list: Vec::new(),
            cred_path: None,
            enable_multiplexing: false,
        }
    }
}

impl Config {
    /// Builds a Figment that merges defaults and environment variables.
    /// Uses raw env mapping, so field names map to env vars in UPPER_SNAKE_CASE.
    pub fn figment() -> Figment {
        Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Env::raw())
    }

    /// Loads configuration from the environment (with defaults) and validates required fields.
    pub fn from_env() -> Self {
        let cfg: Self = Self::figment()
            .extract()
            .expect("failed to extract configuration via Figment");
        if cfg.nexus_key.trim().is_empty() {
            panic!("NEXUS_KEY must be set and non-empty");
        }
        cfg
    }
}

fn deserialize_string_lax<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let v = Value::deserialize(deserializer)?;

    match v {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(serde::de::Error::custom(
            "expected a string or a number for NEXUS_KEY",
        )),
    }
}

/// Global, lazily-initialized configuration instance.
pub static CONFIG: LazyLock<Config> = LazyLock::new(Config::from_env);

/// Google OAuth endpoints (constants).
pub static GOOGLE_AUTH_URL: LazyLock<Url> = LazyLock::new(|| {
    Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("valid Google OAuth auth URL")
});

pub static GOOGLE_TOKEN_URI: LazyLock<Url> = LazyLock::new(|| {
    Url::parse("https://oauth2.googleapis.com/token").expect("valid Google OAuth token URI")
});

pub static GOOGLE_USERINFO_URI: LazyLock<Url> = LazyLock::new(|| {
    Url::parse("https://www.googleapis.com/oauth2/v3/userinfo")
        .expect("valid Google OAuth2 userinfo URI")
});

pub const GCLI_CLIENT_ID: &str = env!("GCLI_CLIENT_ID");
pub const GCLI_CLIENT_SECRET: &str = env!("GCLI_CLIENT_SECRET");

pub const CLI_VERSION: &str = "0.16.0";
pub static CLI_USER_AGENT: LazyLock<String> =
    LazyLock::new(|| format!("GeminiCLI/{v} (Linux; x64)", v = CLI_VERSION));

// Cloud Code Gemini endpoints
pub static GEMINI_GENERATE_URL: LazyLock<reqwest::Url> = LazyLock::new(|| {
    reqwest::Url::parse("https://cloudcode-pa.googleapis.com/v1internal:generateContent")
        .expect("valid Cloud Code generateContent URL")
});

pub static GEMINI_STREAM_URL: LazyLock<reqwest::Url> = LazyLock::new(|| {
    reqwest::Url::parse(
        "https://cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse",
    )
    .expect("valid Cloud Code streamGenerateContent URL with alt=sse")
});

/// Default IP address for the HTTP server listen address.
pub fn default_listen_ip() -> IpAddr {
    Ipv4Addr::new(0, 0, 0, 0).into()
}

/// Default port for the HTTP server.
pub fn default_listen_port() -> u16 {
    8188
}
