use crate::config::{
    AntigravityResolvedConfig, CodexResolvedConfig, Config, GeminiCliResolvedConfig,
};
use crate::db::DbActorHandle;
use crate::providers::antigravity::AntigravityActorHandle;
use crate::providers::antigravity::AntigravityThoughtSigService;
use crate::providers::codex::CodexActorHandle;
use crate::providers::geminicli::{GeminiCliActorHandle, GeminiThoughtSigService};
use std::sync::Arc;
use tracing::info;

/// Aggregates handles for all enabled providers.
///
/// Keep this as a simple struct (vs. a dynamic registry) to preserve
/// compile-time ergonomics and avoid over-abstracting too early.
#[derive(Clone)]
pub struct Providers {
    pub geminicli: GeminiCliActorHandle,
    pub geminicli_cfg: Arc<GeminiCliResolvedConfig>,
    pub geminicli_thoughtsig: GeminiThoughtSigService,
    pub codex: CodexActorHandle,
    pub codex_cfg: Arc<CodexResolvedConfig>,
    pub antigravity: AntigravityActorHandle,
    pub antigravity_cfg: Arc<AntigravityResolvedConfig>,
    pub antigravity_thoughtsig: AntigravityThoughtSigService,
}

impl Providers {
    pub async fn spawn(db: DbActorHandle, cfg: &Config) -> Self {
        let provider_defaults = &cfg.providers.defaults;
        let geminicli_cfg = Arc::new(cfg.geminicli());
        let codex_cfg = Arc::new(cfg.codex());
        let antigravity_cfg = Arc::new(cfg.antigravity());

        // Log resolved provider configs here so `main` stays wiring-only.
        info!(
            providers_defaults_proxy = %provider_defaults.proxy.as_ref().map(|u| u.as_str()).unwrap_or("<none>"),
            providers_defaults_enable_multiplexing = provider_defaults.enable_multiplexing,
            providers_defaults_retry_max_times = provider_defaults.retry_max_times,
            "Provider defaults loaded"
        );
        info!(
            geminicli_proxy = %geminicli_cfg.proxy.as_ref().map(|u| u.as_str()).unwrap_or("<none>"),
            geminicli_enable_multiplexing = geminicli_cfg.enable_multiplexing,
            geminicli_retry_max_times = geminicli_cfg.retry_max_times,
            geminicli_oauth_tps = geminicli_cfg.oauth_tps,
            geminicli_model_list = ?geminicli_cfg.model_list,
            "Gemini CLI config (effective)"
        );

        info!(
            codex_proxy = %codex_cfg.proxy.as_ref().map(|u| u.as_str()).unwrap_or("<none>"),
            codex_enable_multiplexing = codex_cfg.enable_multiplexing,
            codex_retry_max_times = codex_cfg.retry_max_times,
            codex_oauth_tps = codex_cfg.oauth_tps,
            codex_responses_url = %crate::providers::codex::CODEX_RESPONSES_URL.as_str(),
            codex_model_list = ?codex_cfg.model_list,
            "Codex config (effective)"
        );

        info!(
            antigravity_api_url = %antigravity_cfg.api_url.as_str(),
            antigravity_proxy = %antigravity_cfg.proxy.as_ref().map(|u| u.as_str()).unwrap_or("<none>"),
            antigravity_enable_multiplexing = antigravity_cfg.enable_multiplexing,
            antigravity_retry_max_times = antigravity_cfg.retry_max_times,
            antigravity_oauth_tps = antigravity_cfg.oauth_tps,
            antigravity_model_list = ?antigravity_cfg.model_list,
            "Antigravity config (effective)"
        );

        let geminicli = crate::providers::geminicli::spawn(db.clone(), geminicli_cfg.clone()).await;
        let geminicli_thoughtsig = GeminiThoughtSigService::new();
        let codex = crate::providers::codex::spawn(db.clone(), codex_cfg.clone()).await;
        let antigravity = crate::providers::antigravity::spawn(db, antigravity_cfg.clone()).await;
        let antigravity_thoughtsig = AntigravityThoughtSigService::new();

        Self {
            geminicli,
            geminicli_cfg,
            geminicli_thoughtsig,
            codex,
            codex_cfg,
            antigravity,
            antigravity_cfg,
            antigravity_thoughtsig,
        }
    }
}
