use crate::config::CONFIG;
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use crate::google_oauth::refresh_job::{JobInstruction, RefreshJobService, RefreshOutcome};
use crate::service::credential_manager::CredentialManager;
use crate::service::credential_manager::{AssignedCredential, CredentialId};
use crate::service::credential_ops::CredentialOps;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Public messages handled by the credentials actor.
#[derive(Debug)]
pub enum CredentialsActorMessage {
    /// Request one available credential for the given model. Err if none available.
    GetCredential(String, RpcReplyPort<Option<AssignedCredential>>),
    /// Report rate limiting; start cooldown with lazy re-enqueue.
    ReportRateLimit {
        id: CredentialId,
        cooldown: Duration,
        model_name: String,
    },
    /// Report unsupported model (e.g. 400/404); record per-credential blacklist.
    ReportModelUnsupported {
        id: CredentialId,
        model_name: String,
    },
    /// Report invalid/expired access (e.g. 401/403); refresh then re-enqueue.
    ReportInvalid { id: CredentialId },
    /// Report a credential as banned/unusable; remove from queues and storage.
    ReportBaned { id: CredentialId },

    /// Submit a batch of credentials and trigger one refresh pass for each.
    SubmitCredentials(Vec<GoogleCredential>),

    // Internal messages (sent by the actor itself)
    /// Token refresh has completed; update stored credential and re-enqueue if ok.
    RefreshComplete { outcome: RefreshOutcome },
    /// A credential has been refreshed and stored; activate it in memory queues.
    ActivateCredential {
        id: CredentialId,
        credential: GoogleCredential,
    },
}

/// Handle for interacting with the credentials actor.
#[derive(Clone)]
pub struct CredentialsHandle {
    actor: ActorRef<CredentialsActorMessage>,
}

impl CredentialsHandle {
    /// Request a credential based on target model. Returns error if none available.
    pub async fn get_credential(
        &self,
        model_name: impl AsRef<str>,
    ) -> Result<Option<AssignedCredential>, NexusError> {
        ractor::call!(
            self.actor,
            CredentialsActorMessage::GetCredential,
            model_name.as_ref().to_string()
        )
        .map_err(|e| NexusError::RactorError(format!("GetCredential RPC failed:: {e}")))
    }

    /// Report rate limit; the actor will cool down this credential before reuse.
    pub async fn report_rate_limit(
        &self,
        id: CredentialId,
        model_name: impl AsRef<str>,
        cooldown: Duration,
    ) {
        let _ = ractor::cast!(
            self.actor,
            CredentialsActorMessage::ReportRateLimit {
                id,
                cooldown,
                model_name: model_name.as_ref().to_string()
            }
        );
    }

    /// Report invalid/expired (401/403); the actor will refresh before reuse.
    pub async fn report_invalid(&self, id: CredentialId) {
        let _ = ractor::cast!(self.actor, CredentialsActorMessage::ReportInvalid { id });
    }

    /// Report that a credential does not support a model (e.g. 400/404).
    pub async fn report_model_unsupported(&self, id: CredentialId, model_name: String) {
        let _ = ractor::cast!(
            self.actor,
            CredentialsActorMessage::ReportModelUnsupported { id, model_name }
        );
    }

    /// Report a credential as permanently banned/unusable; remove it entirely.
    pub async fn report_baned(&self, id: CredentialId) {
        let _ = ractor::cast!(self.actor, CredentialsActorMessage::ReportBaned { id });
    }

    /// Submit new credentials to the actor and trigger refresh for each.
    pub async fn submit_credentials(&self, creds: Vec<GoogleCredential>) {
        let _ = ractor::cast!(
            self.actor,
            CredentialsActorMessage::SubmitCredentials(creds)
        );
    }

    pub fn send_refresh_complete(
        &self,
        outcome: RefreshOutcome,
    ) -> Result<(), ractor::MessagingErr<CredentialsActorMessage>> {
        self.actor
            .cast(CredentialsActorMessage::RefreshComplete { outcome })
    }
}

/// Internal state held by ractor-driven credentials actor
struct CredentialsActorState {
    ops: CredentialOps,
    manager: CredentialManager,
    queue_keys: Vec<String>,
    refresh_tx: mpsc::Sender<JobInstruction>,
}

/// ractor-based credentials actor
struct CredentialsActor;

#[ractor::async_trait]
impl Actor for CredentialsActor {
    type Msg = CredentialsActorMessage;
    type State = CredentialsActorState;
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _arguments: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let refresh_service = RefreshJobService::new(CredentialsHandle {
            actor: _myself.clone(),
        });
        let refresh_tx = refresh_service.job_tx();
        let ops = CredentialOps::new()
            .await
            .map_err(|e| ActorProcessingErr::from(format!("Credential ops init failed: {}", e)))?;

        let mut manager = CredentialManager::new();

        let queue_keys = CONFIG.model_list.clone();

        info!(
            "CredentialsActor initializing with supported models: {:?}",
            queue_keys
        );

        let rows = ops
            .load_active()
            .await
            .map_err(|e| ActorProcessingErr::from(format!("DB load active creds failed: {}", e)))?;

        for (id, cred) in rows {
            manager.add_credential(id, cred, &queue_keys);
        }

        info!(
            "CredentialsActor started from DB: {} active creds loaded into {} queues",
            manager.total_creds(),
            queue_keys.len()
        );

        Ok(CredentialsActorState {
            ops,
            manager,
            queue_keys,
            refresh_tx,
        })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            CredentialsActorMessage::GetCredential(model_name, rp) => {
                self.handle_get_credential(state, rp, model_name).await;
            }

            CredentialsActorMessage::ReportRateLimit {
                id,
                cooldown,
                model_name,
            } => {
                self.handle_report_rate_limit(state, id, cooldown, model_name);
            }
            CredentialsActorMessage::ReportModelUnsupported { id, model_name } => {
                state.manager.mark_model_unsupported(id, &model_name);
            }

            CredentialsActorMessage::ReportInvalid { id } => {
                self.handle_report_invalid(state, id).await;
            }
            CredentialsActorMessage::ReportBaned { id } => {
                self.handle_report_baned(state, id).await;
            }
            CredentialsActorMessage::SubmitCredentials(creds_vec) => {
                self.handle_submit_credentials(state, creds_vec).await;
            }
            CredentialsActorMessage::RefreshComplete { outcome } => {
                self.handle_refresh_complete(state, outcome).await;
            }
            CredentialsActorMessage::ActivateCredential { id, credential } => {
                let project = credential.project_id.clone();
                state
                    .manager
                    .add_credential(id, credential, &state.queue_keys);
                info!("ID: {id}, Project: {project}, submitted and activated");
            }
        }
        Ok(())
    }
}

impl CredentialsActor {
    async fn handle_get_credential(
        &self,
        state: &mut CredentialsActorState,
        reply_port: RpcReplyPort<Option<AssignedCredential>>,
        model_name: impl AsRef<str>,
    ) {
        let query_key = model_name.as_ref();
        let assignment = state.manager.get_assigned(&query_key);

        for id in assignment.refresh_ids {
            self.handle_report_invalid(state, id).await;
        }

        if let Some(assigned) = assignment.assigned {
            debug!(
                "ID: {}, Project: {}, queue: {}, get credential",
                assigned.id, assigned.project_id, query_key
            );
            let _ = reply_port.send(Some(assigned));
            return;
        }

        warn!(
            "No credential available for queue={}, queue_len={}, cooldowns={}, refreshing={}",
            query_key,
            state.manager.queue_len(&query_key),
            state.manager.cooldown_len(),
            state.manager.refreshing_len()
        );
        let _ = reply_port.send(None);
    }

    fn handle_report_rate_limit(
        &self,
        state: &mut CredentialsActorState,
        id: CredentialId,
        cooldown: Duration,
        model_name: impl AsRef<str>,
    ) {
        if !state.manager.contains(id) {
            return;
        }
        let query_key = model_name.as_ref();
        state.manager.report_rate_limit(id, &query_key, cooldown);

        info!(
            "ID: {id}, Credential starting cooldown for {query_key} queue, lazy re-enqueue after {} secs",
            cooldown.as_secs(),
        );
    }

    // handle_report_invalid, handle_report_baned, handle_submit_credentials
    async fn handle_report_invalid(&self, state: &mut CredentialsActorState, id: CredentialId) {
        if state.manager.is_refreshing(id) {
            debug!("ID: {id}, Already refreshing; skip duplicate");
            return;
        }
        let Some(current) = state.manager.get_full_credential_copy(id) else {
            return;
        };
        let pid = current.project_id.clone();
        info!("ID: {id}, Project: {pid}, invalid reported; starting refresh");

        state.manager.mark_refreshing(id);

        if let Err(e) = state
            .refresh_tx
            .send(JobInstruction::Maintain {
                id,
                cred: current.clone(),
            })
            .await
        {
            state.manager.add_credential(id, current, &state.queue_keys);
            warn!("ID: {id}, Failed to enqueue refresh job: {}", e);
            return;
        }
        debug!("ID: {id}, Credential refresh enqueued");
    }

    async fn handle_report_baned(&self, state: &mut CredentialsActorState, id: CredentialId) {
        let project = state
            .manager
            .project_id_of(id)
            .unwrap_or_else(|| "-".to_string());
        let removed_cred = state.manager.contains(id);

        state.manager.delete_credential(id);

        if let Err(e) = state.ops.set_status(id, false).await {
            warn!(
                "ID: {id}, Project: {project}, ban report failed to update DB status: {}",
                e
            );
            return;
        }
        info!(
            "ID: {id}, Project: {project}, banned. removed_from_mem={}",
            removed_cred
        );
    }

    async fn handle_submit_credentials(
        &self,
        state: &mut CredentialsActorState,
        creds_vec: Vec<GoogleCredential>,
    ) {
        let count = creds_vec.len();
        info!(count, "Batch submit received, dispatching...");
        let refresh_tx = state.refresh_tx.clone();
        tokio::spawn(async move {
            for cred in creds_vec {
                let pid = cred.project_id.clone();
                if let Err(e) = refresh_tx.send(JobInstruction::Onboard { cred }).await {
                    warn!(
                        "Project: {pid}, failed to enqueue onboarding refresh: {}",
                        e
                    );
                    break;
                }
            }
        });
    }

    async fn handle_refresh_complete(
        &self,
        state: &mut CredentialsActorState,
        outcome: RefreshOutcome,
    ) {
        match outcome {
            RefreshOutcome::Success(job) => match job {
                JobInstruction::Maintain { id, cred } => {
                    if !state.manager.is_refreshing(id) {
                        debug!("ID: {id} Refresh completed after removal; skipping.");
                        return;
                    }
                    debug!("ID: {id} Refresh success. Updating DB and Manager.");
                    state
                        .manager
                        .add_credential(id, cred.clone(), &state.queue_keys);
                    if let Err(e) = state.ops.update_by_id(id, cred, true).await {
                        warn!("ID: {id} DB update failed: {}", e);
                    }
                }

                JobInstruction::Onboard { cred } => {
                    let pid = cred.project_id.clone();
                    info!("Project: {pid} Onboard success. Inserting to DB.");

                    match state.ops.upsert(cred.clone(), true).await {
                        Ok(new_id) => {
                            state
                                .manager
                                .add_credential(new_id, cred, &state.queue_keys);
                            info!("Project: {pid} Activated with ID: {new_id}");
                        }
                        Err(e) => warn!("Project: {pid} DB upsert failed: {}", e),
                    }
                }
            },

            RefreshOutcome::Failed(job, err) => match job {
                JobInstruction::Maintain { id, cred } => {
                    if !state.manager.is_refreshing(id) {
                        debug!("ID: {id} Refresh failed after removal; skipping.");
                        return;
                    }
                    match err {
                        NexusError::Oauth2Server { .. } => {
                            error!("ID: {id} Refresh failed: {}. Removing.", err);

                            state.manager.delete_credential(id);
                            if let Err(e) = state.ops.set_status(id, false).await {
                                warn!("ID: {id} DB set_status failed: {}", e);
                            }
                        }
                        _ => {
                            warn!(
                                "ID: {id} Refresh failed due to transient error: {}. Keeping credential.",
                                err
                            );
                            state.manager.add_credential(id, cred, &state.queue_keys);
                        }
                    }
                }

                JobInstruction::Onboard { cred } => {
                    warn!(
                        "Project: {} Onboard failed: {}. Discarding.",
                        cred.project_id, err
                    );
                }
            },
        }
    }
}

/// Async spawn of the credentials actor and return a handle.
pub async fn spawn() -> CredentialsHandle {
    let (actor, _jh) = Actor::spawn(Some("CredentialsActor".to_string()), CredentialsActor, ())
        .await
        .expect("failed to spawn CredentialsActor");
    CredentialsHandle { actor }
}
