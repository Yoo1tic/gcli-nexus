mod basic;
mod providers;

pub use basic::BasicConfig;
pub use providers::{
    AntigravityConfig, AntigravityResolvedConfig, CLAUDE_SYSTEM_PREAMBLE, CodexConfig,
    CodexResolvedConfig, GeminiCliConfig, GeminiCliResolvedConfig, ProviderDefaults,
    ProvidersConfig,
};

use figment::{
    Figment,
    providers::{Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::LazyLock};

/// Application configuration managed by Figment.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    /// Core server configuration (see `basic` table in config.toml).
    #[serde(default)]
    pub basic: BasicConfig,

    /// Provider and upstream settings (see `providers` table in config.toml).
    #[serde(default)]
    pub providers: ProvidersConfig,
}

const DEFAULT_CONFIG_FILE: &str = "config.toml";

impl Config {
    /// Builds a Figment that merges defaults and a config TOML file.
    pub fn figment() -> Figment {
        let figment = Figment::new().merge(Serialized::defaults(Config::default()));
        if PathBuf::from(DEFAULT_CONFIG_FILE).is_file() {
            figment.merge(Toml::file(DEFAULT_CONFIG_FILE))
        } else {
            figment
        }
    }

    /// Loads configuration by merging defaults and `config.toml` if present.
    ///
    /// Note: this does **not** validate required fields like `basic.pollux_key`. Binaries should
    /// call `Config::from_toml()` instead (or validate explicitly) to avoid running with insecure
    /// defaults.
    pub fn from_optional_toml() -> Self {
        Self::figment().extract().unwrap_or_else(|err| {
            panic!("failed to extract configuration (defaults + optional config.toml): {err}")
        })
    }

    /// Loads configuration from the TOML file (with defaults) and validates required fields.
    pub fn from_toml() -> Self {
        if !PathBuf::from(DEFAULT_CONFIG_FILE).is_file() {
            panic!("config file not found: {}", DEFAULT_CONFIG_FILE);
        }
        let cfg: Self = Self::figment().extract().unwrap_or_else(|err| {
            panic!(
                "failed to extract configuration from {}: {err}",
                DEFAULT_CONFIG_FILE
            )
        });
        if cfg.basic.pollux_key.trim().is_empty() {
            panic!("basic.pollux_key must be set and non-empty");
        }
        cfg
    }

    pub fn geminicli(&self) -> GeminiCliResolvedConfig {
        self.providers.geminicli.resolve(&self.providers.defaults)
    }

    pub fn codex(&self) -> CodexResolvedConfig {
        self.providers.codex.resolve(&self.providers.defaults)
    }

    pub fn antigravity(&self) -> AntigravityResolvedConfig {
        self.providers.antigravity.resolve(&self.providers.defaults)
    }
}

/// Global, lazily-initialized configuration instance.
pub static CONFIG: LazyLock<Config> = LazyLock::new(Config::from_optional_toml);
