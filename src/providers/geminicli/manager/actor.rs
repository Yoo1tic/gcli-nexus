use super::{
    ops::CredentialOps,
    scheduler::{CredentialId, CredentialManager},
};
use crate::config::GeminiCliResolvedConfig;
use crate::db::GeminiCliPatch;
use crate::error::{OauthError, PolluxError};
use crate::model_catalog::MODEL_REGISTRY;
use crate::providers::geminicli::client::oauth::endpoints::GoogleTokenResponse;
use crate::providers::geminicli::client::oauth::utils::attach_email_from_id_token;
use crate::providers::geminicli::resource::GeminiCliResource;
use crate::providers::geminicli::workers::{
    GeminiCliRefresherHandle, RefreshError, RefreshJob, RefreshResult, TaskType,
};
use crate::providers::geminicli::{SUPPORTED_MODEL_MASK, SUPPORTED_MODEL_NAMES};
use crate::providers::manifest::{GeminiCliLease, GeminiCliProfile};
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub(crate) struct GeminiCliRefreshTokenSeed {
    refresh_token: String,
}

impl GeminiCliRefreshTokenSeed {
    pub fn new(refresh_token: String) -> Option<Self> {
        let refresh_token = refresh_token.trim().to_string();
        if refresh_token.is_empty() {
            return None;
        }
        Some(Self { refresh_token })
    }
}

/// Public messages handled by the Gemini CLI actor.
pub enum GeminiCliActorMessage {
    /// Request one available credential for the given model mask. Err if none available.
    GetCredential(u64, RpcReplyPort<Option<GeminiCliLease>>),
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

    /// Submit a batch of credentials and trigger one refresh pass for each.
    SubmitCredentials(Vec<GeminiCliProfile>),
    /// Submit a trusted OAuth token response to the actor for onboarding + persistence.
    SubmitTrustedOauth(GoogleTokenResponse),
    /// Submit refresh tokens as 0-trust seeds. The actor will refresh, onboard, then persist+activate.
    SubmitUntrustedSeeds(Vec<GeminiCliRefreshTokenSeed>),

    // Internal messages (sent by the actor itself)
    /// Token refresh has completed; update stored credential and re-enqueue if ok.
    RefreshComplete { result: RefreshResult },
    /// A credential has been refreshed and stored; activate it in memory queues.
    ActivateCredential {
        id: CredentialId,
        credential: GeminiCliResource,
    },
}

/// Handle for interacting with the Gemini CLI actor.
#[derive(Clone)]
pub struct GeminiCliActorHandle {
    actor: ActorRef<GeminiCliActorMessage>,
}

impl GeminiCliActorHandle {
    /// Request a credential based on target model mask. Returns error if none available.
    pub async fn get_credential(
        &self,
        model_mask: u64,
    ) -> Result<Option<GeminiCliLease>, PolluxError> {
        ractor::call!(self.actor, GeminiCliActorMessage::GetCredential, model_mask)
            .map_err(|e| PolluxError::RactorError(format!("GetCredential RPC failed:: {e}")))
    }

    /// Report rate limit; the actor will cool down this credential before reuse.
    pub async fn report_rate_limit(&self, id: CredentialId, model_mask: u64, cooldown: Duration) {
        let _ = ractor::cast!(
            self.actor,
            GeminiCliActorMessage::ReportRateLimit {
                id,
                cooldown,
                model_mask
            }
        );
    }

    /// Report invalid/expired (401/403); the actor will refresh before reuse.
    pub async fn report_invalid(&self, id: CredentialId) {
        let _ = ractor::cast!(self.actor, GeminiCliActorMessage::ReportInvalid { id });
    }

    /// Report that a credential does not support a model (e.g. 400/404).
    pub async fn report_model_unsupported(&self, id: CredentialId, model_mask: u64) {
        let _ = ractor::cast!(
            self.actor,
            GeminiCliActorMessage::ReportModelUnsupported { id, model_mask }
        );
    }

    /// Report a credential as permanently banned/unusable; remove it entirely.
    pub async fn report_baned(&self, id: CredentialId) {
        let _ = ractor::cast!(self.actor, GeminiCliActorMessage::ReportBaned { id });
    }

    /// Submit new credentials to the actor and trigger refresh for each.
    pub async fn submit_credentials(&self, creds: Vec<GeminiCliProfile>) {
        let _ = ractor::cast!(self.actor, GeminiCliActorMessage::SubmitCredentials(creds));
    }

    /// Submit a trusted OAuth token response to the actor for persistence + activation.
    pub(crate) async fn submit_trusted_oauth(&self, token_response: GoogleTokenResponse) {
        let _ = ractor::cast!(
            self.actor,
            GeminiCliActorMessage::SubmitTrustedOauth(token_response)
        );
    }

    /// Submit refresh tokens as 0-trust seeds. The actor will refresh, onboard, then persist+activate.
    pub(crate) async fn submit_refresh_tokens(&self, refresh_tokens: Vec<String>) {
        let seeds: Vec<GeminiCliRefreshTokenSeed> = refresh_tokens
            .into_iter()
            .filter_map(GeminiCliRefreshTokenSeed::new)
            .collect();

        if seeds.is_empty() {
            return;
        }

        let _ = ractor::cast!(
            self.actor,
            GeminiCliActorMessage::SubmitUntrustedSeeds(seeds)
        );
    }

    pub(in crate::providers::geminicli) fn send_refresh_complete(
        &self,
        result: RefreshResult,
    ) -> Result<(), PolluxError> {
        ractor::cast!(
            self.actor,
            GeminiCliActorMessage::RefreshComplete { result }
        )
        .map_err(|e| PolluxError::RactorError(format!("RefreshComplete cast failed: {e}")))
    }
}

/// Internal state held by ractor-driven Gemini CLI actor.
struct GeminiCliActorState {
    ops: CredentialOps,
    manager: CredentialManager,
    model_caps_all: u64,
    refresh_handle: GeminiCliRefresherHandle,
}

/// ractor-based Gemini CLI actor.
struct GeminiCliActor;

#[ractor::async_trait]
impl Actor for GeminiCliActor {
    type Msg = GeminiCliActorMessage;
    type State = GeminiCliActorState;
    type Arguments = (CredentialOps, Arc<GeminiCliResolvedConfig>);

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let (ops, cfg) = args;
        let refresh_handle = GeminiCliRefresherHandle::spawn(
            GeminiCliActorHandle {
                actor: _myself.clone(),
            },
            cfg.clone(),
        )
        .await?;

        let model_count = MODEL_REGISTRY.len();
        let model_caps_all = *SUPPORTED_MODEL_MASK;

        let mut manager = CredentialManager::new(model_count);

        let model_names = (*SUPPORTED_MODEL_NAMES).clone();
        info!(
            "GeminiCliActor initializing with supported models: {:?}",
            model_names
        );

        let rows = ops
            .load_active()
            .await
            .map_err(|e| ActorProcessingErr::from(format!("DB load active creds failed: {}", e)))?;

        for (id, cred) in rows {
            manager.add_credential(id, cred, model_caps_all);
        }

        info!(
            "GeminiCliActor started from DB: {} active creds loaded into {} queues",
            manager.total_creds(),
            model_count
        );

        info!(
            proxy = %cfg.proxy.as_ref().map(|u| u.as_str()).unwrap_or("<none>"),
            enable_multiplexing = cfg.enable_multiplexing,
            oauth_tps = cfg.oauth_tps,
            "GeminiCliActor runtime config loaded"
        );

        Ok(GeminiCliActorState {
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
            GeminiCliActorMessage::GetCredential(model_mask, rp) => {
                self.handle_get_credential(myself.clone(), state, rp, model_mask)
                    .await;
            }

            GeminiCliActorMessage::ReportRateLimit {
                id,
                cooldown,
                model_mask,
            } => {
                self.handle_report_rate_limit(state, id, cooldown, model_mask);
            }
            GeminiCliActorMessage::ReportModelUnsupported { id, model_mask } => {
                self.handle_report_model_unsupported(state, id, model_mask);
            }

            GeminiCliActorMessage::ReportInvalid { id } => {
                self.handle_report_invalid(myself.clone(), state, vec![id])
                    .await;
            }
            GeminiCliActorMessage::ReportBaned { id } => {
                self.handle_report_baned(state, id).await;
            }
            GeminiCliActorMessage::SubmitCredentials(creds_vec) => {
                self.handle_submit_credentials(state, creds_vec).await;
            }
            GeminiCliActorMessage::SubmitTrustedOauth(token_response) => {
                self.handle_submit_trusted_oauth(state, token_response)
                    .await;
            }
            GeminiCliActorMessage::SubmitUntrustedSeeds(seeds) => {
                self.handle_submit_untrusted_seeds(state, seeds).await;
            }
            GeminiCliActorMessage::RefreshComplete { result } => {
                self.handle_refresh_complete(myself.clone(), state, result)
                    .await;
            }
            GeminiCliActorMessage::ActivateCredential { id, credential } => {
                let project = credential.project_id().to_string();
                state
                    .manager
                    .add_credential(id, credential, state.model_caps_all);
                info!("ID: {id}, Project: {project}, submitted and activated");
            }
        }
        Ok(())
    }
}

impl GeminiCliActor {
    fn handle_report_model_unsupported(
        &self,
        state: &mut GeminiCliActorState,
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

        // Scheduler is pure logic; log the state transition at the actor boundary.
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
                "GeminiCli credential id={} project={} now supports no models after disabling {} (mask=0x{:016x}); caps 0x{:016x} -> 0x{:016x}",
                id, project_id, disabled_names, model_mask, before_bits, after_bits
            );
        } else {
            info!(
                "GeminiCli credential id={} project={} disabled models {} (mask=0x{:016x}); caps 0x{:016x} -> 0x{:016x}",
                id, project_id, disabled_names, model_mask, before_bits, after_bits
            );
        }
    }

    async fn handle_get_credential(
        &self,
        myself: ActorRef<GeminiCliActorMessage>,
        state: &mut GeminiCliActorState,
        reply_port: RpcReplyPort<Option<GeminiCliLease>>,
        model_mask: u64,
    ) {
        let assignment = state.manager.get_assigned(model_mask);

        if !assignment.refresh_ids.is_empty() {
            self.handle_report_invalid(myself, state, assignment.refresh_ids)
                .await;
        }

        if let Some(assigned) = assignment.assigned {
            info!(
                "Get credential: ID: {}, Project: {}, model_mask=0x{:016x}, queue_len={}",
                assigned.id,
                assigned.project_id,
                model_mask,
                state.manager.queue_len(model_mask)
            );
            let _ = reply_port.send(Some(assigned));
            return;
        }

        warn!(
            "No credential available for model_mask=0x{:016x}, queue_len={}, cooldowns={}, refreshing={}",
            model_mask,
            state.manager.queue_len(model_mask),
            state.manager.cooldown_len(),
            state.manager.refreshing_len()
        );
        let _ = reply_port.send(None);
    }

    fn handle_report_rate_limit(
        &self,
        state: &mut GeminiCliActorState,
        id: CredentialId,
        cooldown: Duration,
        model_mask: u64,
    ) {
        if !state.manager.contains(id) {
            return;
        }
        state.manager.report_rate_limit(id, model_mask, cooldown);

        info!(
            "ID: {id}, Credential starting cooldown for model_mask=0x{:016x}, lazy re-enqueue after {} secs",
            model_mask,
            cooldown.as_secs(),
        );
    }

    // handle_report_invalid, handle_report_baned, handle_submit_credentials
    async fn handle_report_invalid(
        &self,
        myself: ActorRef<GeminiCliActorMessage>,
        state: &mut GeminiCliActorState,
        ids: Vec<CredentialId>,
    ) {
        let mut jobs_to_send = Vec::new();
        for id in ids {
            if state.manager.is_refreshing(id) {
                debug!("ID: {id} in batch already refreshing, skipping.");
                continue;
            }
            if let Some(current) = state.manager.get_full_credential_copy(id) {
                state.manager.mark_refreshing(id);

                info!(
                    "ID: {}, Project: {}, batch invalid reported.",
                    id,
                    current.project_id()
                );

                jobs_to_send.push((id, current));
            }
        }
        if jobs_to_send.is_empty() {
            return;
        }
        let refresh_handle = state.refresh_handle.clone();

        for (id, cred) in jobs_to_send {
            let task = RefreshJob {
                cred,
                r#type: TaskType::Refresh(id),
            };
            if let Err(e) = refresh_handle.submit_refresh(task.clone()) {
                warn!("ID: {id} Batch refresh enqueue failed. Rolling back.");

                let _ = myself.cast(GeminiCliActorMessage::RefreshComplete {
                    result: Err(RefreshError {
                        original_job: task,
                        error: PolluxError::RactorError(format!(
                            "Failed to enqueue refresh job: {e}"
                        )),
                    }),
                });
            } else {
                debug!("ID: {id} Batch refresh enqueued.");
            }
        }
    }

    async fn handle_report_baned(&self, state: &mut GeminiCliActorState, id: CredentialId) {
        let project = state
            .manager
            .project_id_of(id)
            .unwrap_or_else(|| "-".to_string());
        let removed_cred = state.manager.contains(id);

        state.manager.delete_credential(id);

        let ops = state.ops.clone();
        let project_for_db = project.clone();
        tokio::spawn(async move {
            if let Err(e) = ops.set_status(id, false).await {
                warn!(
                    "ID: {id}, Project: {project_for_db}, ban report failed to update DB status: {}",
                    e
                );
            }
        });
        info!(
            "ID: {id}, Project: {project}, banned. removed_from_mem={}",
            removed_cred
        );
    }

    async fn handle_submit_credentials(
        &self,
        state: &mut GeminiCliActorState,
        creds_vec: Vec<GeminiCliProfile>,
    ) {
        let count = creds_vec.len();
        info!(count, "Batch submit received, dispatching...");
        let refresh_handle = state.refresh_handle.clone();
        tokio::spawn(async move {
            for profile in creds_vec {
                let pid = profile.project_id.to_string();
                let cred = GeminiCliResource::from(profile);
                let job = RefreshJob {
                    cred,
                    r#type: TaskType::Onboard,
                };
                if let Err(e) = refresh_handle.submit_onboard(job) {
                    warn!(
                        "Project: {pid}, failed to enqueue onboarding refresh: {}",
                        e
                    );
                    break;
                }
            }
        });
    }

    async fn handle_submit_trusted_oauth(
        &self,
        state: &mut GeminiCliActorState,
        token_response: GoogleTokenResponse,
    ) {
        info!("Trusted OAuth submit received, dispatching onboarding...");
        let refresh_handle = state.refresh_handle.clone();
        tokio::spawn(async move {
            let mut token_value = match serde_json::to_value(&token_response) {
                Ok(v) => v,
                Err(e) => {
                    warn!("Trusted OAuth submit ignored: token JSON encode failed: {e}");
                    return;
                }
            };
            attach_email_from_id_token(&mut token_value);

            let mut cred = GeminiCliResource::default();
            if let Err(e) = cred.update_credential(token_value) {
                warn!("Trusted OAuth submit ignored: token JSON error: {e}");
                return;
            }
            let job = RefreshJob {
                cred,
                r#type: TaskType::Onboard,
            };
            if let Err(e) = refresh_handle.submit_onboard(job) {
                warn!("Trusted OAuth submit enqueue failed: {}", e);
            }
        });
    }

    async fn handle_submit_untrusted_seeds(
        &self,
        state: &mut GeminiCliActorState,
        seeds: Vec<GeminiCliRefreshTokenSeed>,
    ) {
        let count = seeds.len();
        info!(
            count,
            "0-trust seed submit received, dispatching onboarding..."
        );
        let refresh_handle = state.refresh_handle.clone();
        tokio::spawn(async move {
            for seed in seeds {
                let mut cred = GeminiCliResource::default();
                if let Err(e) =
                    cred.update_credential(json!({ "refresh_token": seed.refresh_token }))
                {
                    warn!("0-trust seed discarded: JSON error: {e}");
                    continue;
                }

                let job = RefreshJob {
                    cred,
                    r#type: TaskType::Onboard,
                };
                if let Err(e) = refresh_handle.submit_onboard(job) {
                    warn!("0-trust seed enqueue failed: {}", e);
                    break;
                }
            }
        });
    }

    async fn handle_refresh_complete(
        &self,
        myself: ActorRef<GeminiCliActorMessage>,
        state: &mut GeminiCliActorState,
        result: RefreshResult,
    ) {
        // If the result is for a refresh task, check if the credential is still in refreshing state.
        if let Some(id) = match &result {
            Ok(success) => &success.r#type,
            Err(failed) => &failed.original_job.r#type,
        }
        .credential_id()
            && !state.manager.is_refreshing(id)
        {
            debug!("ID: {id} Refresh completed/failed after removal; skipping.");
            return;
        }

        // Process the refresh result: if success, update credential and re-enqueue; if failure, decide based on error type.
        match result {
            Ok(success) => {
                let pid = success.cred.project_id().to_string();
                let cred = success.cred;
                match success.r#type {
                    TaskType::Refresh(id) => {
                        state
                            .manager
                            .add_credential(id, cred.clone(), state.model_caps_all);
                        let ops = state.ops.clone();
                        tokio::spawn(async move {
                            let patch = GeminiCliPatch {
                                email: cred.email().map(ToString::to_string),
                                access_token: cred.access_token().map(ToString::to_string),
                                expiry: Some(cred.expiry()),
                                ..Default::default()
                            };
                            if let Err(e) = ops.update_by_id(id, patch).await {
                                warn!("ID: {id} DB update failed: {}", e);
                            }
                        });
                    }
                    TaskType::Onboard => {
                        info!("Project: {pid} Onboard success. Inserting to DB.");
                        let ops = state.ops.clone();
                        let myself = myself.clone();
                        tokio::spawn(async move {
                            let cred_for_db = cred.clone();
                            match ops.upsert(cred_for_db).await {
                                Ok(new_id) => {
                                    if let Err(e) =
                                        myself.cast(GeminiCliActorMessage::ActivateCredential {
                                            id: new_id,
                                            credential: cred,
                                        })
                                    {
                                        warn!("Project: {pid} ActivateCredential failed: {}", e);
                                    }
                                }
                                Err(e) => warn!("Project: {pid} DB upsert failed: {}", e),
                            }
                        });
                    }
                }
            }
            Err(failed) => {
                let job = failed.original_job;
                let err = failed.error;
                let pid = job.cred.project_id().to_string();
                warn!("RefreshTask failed for project {}: {}", pid, err);
                match job.r#type {
                    TaskType::Refresh(id) => match err {
                        PolluxError::Oauth(OauthError::ServerResponse { .. }) => {
                            error!("ID: {id} Refresh failed: {}. Removing.", err);

                            state.manager.delete_credential(id);
                            let ops = state.ops.clone();
                            tokio::spawn(async move {
                                if let Err(e) = ops.set_status(id, false).await {
                                    warn!("ID: {id} DB set_status failed: {}", e);
                                }
                            });
                        }
                        _ => {
                            warn!(
                                "ID: {id} Refresh failed due to transient error: {}. Keeping credential.",
                                err
                            );
                            state
                                .manager
                                .add_credential(id, job.cred, state.model_caps_all);
                        }
                    },
                    TaskType::Onboard => {
                        warn!(
                            "Project: {} Onboard failed: {}. Discarding.",
                            job.cred.project_id(),
                            err
                        );
                    }
                }
            }
        }
    }
}

/// Async spawn of the Gemini CLI actor and return a handle.
pub(in crate::providers) async fn spawn(
    db: crate::db::DbActorHandle,
    gemini_cfg: Arc<GeminiCliResolvedConfig>,
) -> GeminiCliActorHandle {
    let ops = CredentialOps::new(db);

    let (actor, _jh) = Actor::spawn(
        Some("GeminiCliMain".to_string()),
        GeminiCliActor,
        (ops, gemini_cfg),
    )
    .await
    .expect("failed to spawn GeminiCliActor");
    GeminiCliActorHandle { actor }
}
