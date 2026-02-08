use super::ops::CredentialOps;
use super::scheduler::{CredentialId, CredentialManager};
use crate::config::AntigravityResolvedConfig;
use crate::db::{AntigravityCreate, AntigravityPatch};
use crate::error::{OauthError, PolluxError};
use crate::model_catalog::MODEL_REGISTRY;
use crate::oauth_utils::OauthTokenResponse;
use crate::providers::antigravity::resource::AntigravityResource;
use crate::providers::antigravity::workers::refresher::{
    AntigravityRefreshTokenSeed, RefreshOutcome,
};
use crate::providers::manifest::AntigravityLease;
use oauth2::TokenResponse;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use std::{sync::Arc, time::Duration};
use tracing::{debug, error, info, warn};

/// Public messages handled by the Antigravity actor.
#[derive(Debug)]
pub enum AntigravityActorMessage {
    /// Request one available credential for the given model mask. `None` if none available.
    GetCredential(u64, RpcReplyPort<Option<AntigravityLease>>),

    /// Report rate limiting for a model mask; start cooldown with lazy re-enqueue.
    ReportRateLimit {
        id: CredentialId,
        cooldown: Duration,
        model_mask: u64,
    },

    /// Report unsupported model (e.g. 400/404); clear capability bits for this credential.
    ReportModelUnsupported { id: CredentialId, model_mask: u64 },

    /// Report invalid/expired access (e.g. 401/403); refresh then re-enqueue.
    ReportInvalid { id: CredentialId },

    /// Report a credential as banned/unusable; remove from queues and storage.
    ReportBaned { id: CredentialId },

    /// Submit a trusted OAuth token response to the actor for onboarding + persistence.
    SubmitTrustedOauth(OauthTokenResponse),

    /// Submit refresh tokens as 0-trust seeds. The actor will refresh, onboard, then persist+activate.
    SubmitUntrustedSeeds(Vec<AntigravityRefreshTokenSeed>),

    // Internal messages (sent by the actor itself)
    /// Token refresh/onboarding has completed; update stored credential and re-enqueue if ok.
    RefreshComplete { outcome: RefreshOutcome },

    /// A credential has been refreshed/onboarded and stored; activate it in memory queues.
    ActivateCredential {
        id: CredentialId,
        credential: AntigravityResource,
    },
}

/// Handle for interacting with the Antigravity actor.
#[derive(Clone, Debug)]
pub struct AntigravityActorHandle {
    actor: ActorRef<AntigravityActorMessage>,
}

impl AntigravityActorHandle {
    /// Request a credential based on target model mask.
    pub async fn get_credential(
        &self,
        model_mask: u64,
    ) -> Result<Option<AntigravityLease>, PolluxError> {
        ractor::call!(
            self.actor,
            AntigravityActorMessage::GetCredential,
            model_mask
        )
        .map_err(|e| PolluxError::RactorError(format!("GetCredential RPC failed: {e}")))
    }

    pub async fn report_rate_limit(&self, id: CredentialId, model_mask: u64, cooldown: Duration) {
        let _ = ractor::cast!(
            self.actor,
            AntigravityActorMessage::ReportRateLimit {
                id,
                cooldown,
                model_mask
            }
        );
    }

    pub async fn report_invalid(&self, id: CredentialId) {
        let _ = ractor::cast!(self.actor, AntigravityActorMessage::ReportInvalid { id });
    }

    pub async fn report_model_unsupported(&self, id: CredentialId, model_mask: u64) {
        let _ = ractor::cast!(
            self.actor,
            AntigravityActorMessage::ReportModelUnsupported { id, model_mask }
        );
    }

    pub async fn report_baned(&self, id: CredentialId) {
        let _ = ractor::cast!(self.actor, AntigravityActorMessage::ReportBaned { id });
    }

    /// Submit a trusted OAuth token response to the actor.
    pub(crate) async fn submit_trusted_oauth(&self, token_response: OauthTokenResponse) {
        let _ = ractor::cast!(
            self.actor,
            AntigravityActorMessage::SubmitTrustedOauth(token_response)
        );
    }

    /// Submit refresh tokens as 0-trust seeds.
    pub(crate) async fn submit_refresh_tokens(&self, refresh_tokens: Vec<String>) {
        let seeds: Vec<AntigravityRefreshTokenSeed> = refresh_tokens
            .into_iter()
            .filter_map(AntigravityRefreshTokenSeed::new)
            .collect();

        if seeds.is_empty() {
            return;
        }

        let _ = ractor::cast!(
            self.actor,
            AntigravityActorMessage::SubmitUntrustedSeeds(seeds)
        );
    }
}

/// Internal state held by ractor-driven Antigravity actor.
struct AntigravityActorState {
    ops: CredentialOps,
    manager: CredentialManager,
    model_caps_all: u64,
    refresh_handle: crate::providers::antigravity::workers::refresher::AntigravityRefresherHandle,
}

struct AntigravityActor;

#[ractor::async_trait]
impl Actor for AntigravityActor {
    type Msg = AntigravityActorMessage;
    type State = AntigravityActorState;
    type Arguments = (CredentialOps, Arc<AntigravityResolvedConfig>);

    async fn pre_start(
        &self,
        myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let (ops, cfg) = args;

        let model_count = MODEL_REGISTRY.len();
        let model_caps_all = cfg
            .model_list
            .iter()
            .filter_map(|name| crate::model_catalog::mask(name))
            .fold(0u64, |acc, bit| acc | bit);

        info!(
            supported_models = ?cfg.model_list,
            supported_model_mask = format!("0x{:016x}", model_caps_all),
            "AntigravityActor initializing"
        );

        let mut manager = CredentialManager::new(model_count);
        let rows = ops
            .load_active()
            .await
            .map_err(|e| ActorProcessingErr::from(format!("DB load active creds failed: {e}")))?;
        for (id, cred) in rows {
            manager.add_credential(id, cred, model_caps_all);
        }

        info!(
            total_creds = manager.total_creds(),
            model_count, "AntigravityActor started from DB"
        );

        // Spawn refresher pipeline and wire outcomes back into this actor.
        let (refresh_handle, mut out_rx) =
            crate::providers::antigravity::workers::refresher::spawn_pipeline(cfg.clone());
        tokio::spawn({
            let myself = myself.clone();
            async move {
                while let Some(outcome) = out_rx.recv().await {
                    let _ = myself.cast(AntigravityActorMessage::RefreshComplete { outcome });
                }
            }
        });

        Ok(AntigravityActorState {
            ops,
            manager,
            model_caps_all,
            refresh_handle,
        })
    }

    async fn handle(
        &self,
        myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            AntigravityActorMessage::GetCredential(model_mask, rp) => {
                self.handle_get_credential(myself.clone(), state, rp, model_mask)
                    .await;
            }

            AntigravityActorMessage::ReportRateLimit {
                id,
                cooldown,
                model_mask,
            } => {
                self.handle_report_rate_limit(state, id, cooldown, model_mask);
            }
            AntigravityActorMessage::ReportModelUnsupported { id, model_mask } => {
                self.handle_report_model_unsupported(state, id, model_mask);
            }

            AntigravityActorMessage::ReportInvalid { id } => {
                self.handle_report_invalid(myself.clone(), state, vec![id])
                    .await;
            }

            AntigravityActorMessage::ReportBaned { id } => {
                self.handle_report_baned(state, id).await;
            }

            AntigravityActorMessage::SubmitTrustedOauth(token_response) => {
                self.handle_submit_trusted_oauth(state, token_response)
                    .await;
            }
            AntigravityActorMessage::SubmitUntrustedSeeds(seeds) => {
                self.handle_submit_untrusted_seeds(state, seeds).await;
            }

            AntigravityActorMessage::RefreshComplete { outcome } => {
                self.handle_refresh_complete(myself.clone(), state, outcome)
                    .await;
            }
            AntigravityActorMessage::ActivateCredential { id, credential } => {
                let project = credential.project_id().to_string();
                state
                    .manager
                    .add_credential(id, credential, state.model_caps_all);
                info!(id, project, "Antigravity credential activated");
            }
        }
        Ok(())
    }
}

impl AntigravityActor {
    fn handle_report_model_unsupported(
        &self,
        state: &mut AntigravityActorState,
        id: CredentialId,
        model_mask: u64,
    ) {
        if model_mask == 0 || !state.manager.contains(id) {
            return;
        }

        let project_id = state
            .manager
            .project_id_of(id)
            .unwrap_or_else(|| "-".to_string());

        let Some((before_bits, after_bits)) = state.manager.mark_model_unsupported(id, model_mask)
        else {
            return;
        };
        if before_bits == after_bits {
            return;
        }

        let disabled_names = crate::model_catalog::format_model_mask(model_mask);
        if after_bits == 0 {
            warn!(
                "Antigravity credential id={} project={} now supports no models after disabling {} (mask=0x{:016x}); caps 0x{:016x} -> 0x{:016x}",
                id, project_id, disabled_names, model_mask, before_bits, after_bits
            );
        } else {
            info!(
                "Antigravity credential id={} project={} disabled models {} (mask=0x{:016x}); caps 0x{:016x} -> 0x{:016x}",
                id, project_id, disabled_names, model_mask, before_bits, after_bits
            );
        }
    }

    async fn handle_get_credential(
        &self,
        myself: ActorRef<AntigravityActorMessage>,
        state: &mut AntigravityActorState,
        reply_port: RpcReplyPort<Option<AntigravityLease>>,
        model_mask: u64,
    ) {
        let assignment = state.manager.get_assigned(model_mask);

        if !assignment.refresh_ids.is_empty() {
            self.handle_report_invalid(myself, state, assignment.refresh_ids)
                .await;
        }

        if let Some(assigned) = assignment.assigned {
            info!(
                "Get credential: id={}, project={}, model_mask=0x{:016x}, queue_len={}",
                assigned.id,
                assigned.project_id,
                model_mask,
                state.manager.queue_len(model_mask)
            );
            let _ = reply_port.send(Some(assigned));
            return;
        }

        warn!(
            "No Antigravity credential available for model_mask=0x{:016x}, queue_len={}, cooldowns={}, refreshing={}",
            model_mask,
            state.manager.queue_len(model_mask),
            state.manager.cooldown_len(),
            state.manager.refreshing_len()
        );
        let _ = reply_port.send(None);
    }

    fn handle_report_rate_limit(
        &self,
        state: &mut AntigravityActorState,
        id: CredentialId,
        cooldown: Duration,
        model_mask: u64,
    ) {
        if !state.manager.contains(id) {
            return;
        }
        state.manager.report_rate_limit(id, model_mask, cooldown);
        info!(
            id,
            model_mask = format!("0x{:016x}", model_mask),
            cooldown_secs = cooldown.as_secs(),
            "Credential starting cooldown"
        );
    }

    async fn handle_report_invalid(
        &self,
        myself: ActorRef<AntigravityActorMessage>,
        state: &mut AntigravityActorState,
        ids: Vec<CredentialId>,
    ) {
        let mut jobs_to_send: Vec<(CredentialId, String)> = Vec::new();
        for id in ids {
            if state.manager.is_refreshing(id) {
                debug!(id, "ID already refreshing, skipping");
                continue;
            }

            if let Some(current) = state.manager.get_full_credential_copy(id) {
                state.manager.mark_refreshing(id);
                jobs_to_send.push((id, current.refresh_token().to_string()));
            }
        }

        if jobs_to_send.is_empty() {
            return;
        }

        let refresh_handle = state.refresh_handle.clone();
        tokio::spawn(async move {
            for (id, refresh_token) in jobs_to_send {
                if let Err(e) = refresh_handle.submit_refresh(id, refresh_token).await {
                    warn!(id, "Antigravity refresh enqueue failed: {}", e);
                    let _ = myself.cast(AntigravityActorMessage::RefreshComplete {
                        outcome: RefreshOutcome::RefreshCredential {
                            id,
                            patch: AntigravityPatch::default(),
                            result: Err(e),
                        },
                    });
                }
            }
        });
    }

    async fn handle_report_baned(&self, state: &mut AntigravityActorState, id: CredentialId) {
        let project = state
            .manager
            .project_id_of(id)
            .unwrap_or_else(|| "-".to_string());
        let removed = state.manager.contains(id);
        state.manager.delete_credential(id);

        let ops = state.ops.clone();
        tokio::spawn(async move {
            if let Err(e) = ops.set_status(id, false).await {
                warn!(id, "ban report failed to update DB status: {}", e);
            }
        });

        info!(id, project, removed_from_mem = removed, "Credential banned");
    }

    async fn handle_submit_trusted_oauth(
        &self,
        state: &mut AntigravityActorState,
        token_response: OauthTokenResponse,
    ) {
        let refresh_token = token_response
            .refresh_token()
            .map(|t| t.secret().trim().to_string())
            .unwrap_or_default();

        let Some(seed) = AntigravityRefreshTokenSeed::new(refresh_token) else {
            warn!("Trusted OAuth submit ignored: missing refresh_token");
            return;
        };

        info!("Trusted OAuth submit received, dispatching seed onboarding...");
        let refresh_handle = state.refresh_handle.clone();
        tokio::spawn(async move {
            if let Err(e) = refresh_handle.submit_onboard_seed(seed).await {
                warn!("Trusted OAuth seed enqueue failed: {}", e);
            }
        });
    }

    async fn handle_submit_untrusted_seeds(
        &self,
        state: &mut AntigravityActorState,
        seeds: Vec<AntigravityRefreshTokenSeed>,
    ) {
        let count = seeds.len();
        info!(
            count,
            "0-trust seed submit received, dispatching onboarding..."
        );
        let refresh_handle = state.refresh_handle.clone();
        tokio::spawn(async move {
            for seed in seeds {
                if let Err(e) = refresh_handle.submit_onboard_seed(seed).await {
                    warn!("0-trust seed enqueue failed: {}", e);
                    break;
                }
            }
        });
    }

    async fn handle_refresh_complete(
        &self,
        myself: ActorRef<AntigravityActorMessage>,
        state: &mut AntigravityActorState,
        outcome: RefreshOutcome,
    ) {
        match outcome {
            RefreshOutcome::RefreshCredential { id, patch, result } => match result {
                Ok(()) => {
                    if !state.manager.is_refreshing(id) {
                        debug!(id, "refresh completed after removal; skipping");
                        return;
                    }

                    let Some(mut cred) = state.manager.get_full_credential_copy(id) else {
                        debug!(
                            id,
                            "refresh completed but credential missing in manager; skipping"
                        );
                        return;
                    };

                    if let Err(e) = cred.update_credential(&patch) {
                        warn!(
                            id,
                            "failed applying refresh patch to in-memory credential: {}", e
                        );
                    }
                    state.manager.add_credential(id, cred, state.model_caps_all);

                    let ops = state.ops.clone();
                    tokio::spawn(async move {
                        if let Err(e) = ops.update_by_id(id, patch).await {
                            warn!(id, "DB update failed: {}", e);
                        }
                    });
                }

                Err(err) => {
                    if !state.manager.is_refreshing(id) {
                        debug!(id, "refresh failed after removal; skipping");
                        return;
                    }

                    match err {
                        PolluxError::Oauth(OauthError::ServerResponse { .. }) => {
                            error!(id, "refresh failed permanently: {}. Disabling.", err);
                            state.manager.delete_credential(id);

                            let ops = state.ops.clone();
                            tokio::spawn(async move {
                                if let Err(e) = ops.set_status(id, false).await {
                                    warn!(id, "DB set_status failed: {}", e);
                                }
                            });
                        }

                        _ => {
                            warn!(
                                id,
                                "refresh failed due to transient error: {}. Keeping credential.",
                                err
                            );

                            if let Some(cred) = state.manager.get_full_credential_copy(id) {
                                state.manager.add_credential(id, cred, state.model_caps_all);
                            } else {
                                state.manager.delete_credential(id);
                            }
                        }
                    }
                }
            },

            RefreshOutcome::OnboardSeed { seed, result } => match result {
                Ok(create) => {
                    let pid = create.project_id.to_string();
                    info!(project_id = %pid, "Seed onboard success. Inserting to DB.");

                    let ops = state.ops.clone();
                    let myself = myself.clone();
                    tokio::spawn(async move {
                        let create_for_db: AntigravityCreate = create.clone();
                        let cred_for_mem = AntigravityResource::from(create);
                        match ops.upsert(create_for_db).await {
                            Ok(new_id) => {
                                if let Err(e) =
                                    myself.cast(AntigravityActorMessage::ActivateCredential {
                                        id: new_id,
                                        credential: cred_for_mem,
                                    })
                                {
                                    warn!(project_id = %pid, "ActivateCredential failed: {}", e);
                                }
                            }
                            Err(e) => warn!(project_id = %pid, "DB upsert failed: {}", e),
                        }
                    });
                }

                Err(err) => {
                    warn!(
                        refresh_token = %seed.refresh_token(),
                        "Seed onboard failed: {}. Discarding.",
                        err
                    );
                }
            },
        }
    }
}

/// Async spawn of the Antigravity actor and return a handle.
pub(in crate::providers) async fn spawn(
    db: crate::db::DbActorHandle,
    cfg: Arc<AntigravityResolvedConfig>,
) -> AntigravityActorHandle {
    let ops = CredentialOps::new(db);

    let (actor, _jh) = Actor::spawn(
        Some("AntigravityMain".to_string()),
        AntigravityActor,
        (ops, cfg),
    )
    .await
    .expect("failed to spawn AntigravityActor");

    AntigravityActorHandle { actor }
}
