use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use crate::google_oauth::service::GoogleOauthService;
use crate::google_oauth::service::refresh_inner;
use crate::service::credential_manager::CredentialId;
use crate::types::google_code_assist::LoadCodeAssistResponse;
use backon::ExponentialBuilder;
use std::time::Duration;

#[derive(Debug)]
pub enum JobInstruction {
    Maintain {
        id: CredentialId,
        cred: GoogleCredential,
    },
    Onboard {
        cred: GoogleCredential,
    },
}

#[derive(Debug)]
pub enum RefreshOutcome {
    Success(JobInstruction),
    Failed(JobInstruction, NexusError),
}

impl JobInstruction {
    pub async fn execute(&mut self, client: reqwest::Client) -> Result<(), NexusError> {
        let retry_policy = ExponentialBuilder::default()
            .with_min_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(3))
            .with_max_times(3)
            .with_jitter();

        match self {
            Self::Maintain { cred, .. } => {
                refresh_inner(client, retry_policy, cred).await?;
            }

            Self::Onboard { cred } => {
                refresh_inner(client.clone(), retry_policy, cred).await?;
                let token_str = cred.access_token.as_deref().ok_or_else(|| {
                    NexusError::RactorError("Refresh success but token is None".to_string())
                })?;
                let load_json =
                    GoogleOauthService::load_code_assist_with_retry(token_str, client).await?;
                let load_resp: LoadCodeAssistResponse =
                    serde_json::from_value(load_json).map_err(NexusError::JsonError)?;
                if let Some(existing_project_id) = load_resp.cloudaicompanion_project {
                    tracing::info!("Onboard: Found Project ID {}", existing_project_id);
                    cred.project_id = existing_project_id;
                } else {
                    return Err(NexusError::UnexpectedError(
                        "Onboard: missing cloudaicompanion_project".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}
