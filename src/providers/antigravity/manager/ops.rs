use super::scheduler::CredentialId;
use crate::db::{
    AntigravityCreate, AntigravityPatch, DbActorHandle, ProviderCreate, ProviderPatch,
};
use crate::error::PolluxError;
use crate::providers::antigravity::resource::AntigravityResource;

#[derive(Clone)]
pub struct CredentialOps {
    db: DbActorHandle,
}

impl CredentialOps {
    pub fn new(db: DbActorHandle) -> Self {
        Self { db }
    }

    pub async fn load_active(
        &self,
    ) -> Result<Vec<(CredentialId, AntigravityResource)>, PolluxError> {
        let rows = self.db.list_active_antigravity().await?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let id = u64::try_from(row.id).map_err(|_| {
                PolluxError::UnexpectedError(format!("Invalid credential id {}", row.id))
            })?;
            result.push((id, row.into()));
        }
        Ok(result)
    }

    pub async fn upsert(&self, create: AntigravityCreate) -> Result<CredentialId, PolluxError> {
        if create.project_id.trim().is_empty() {
            return Err(PolluxError::UnexpectedError(
                "Antigravity credential missing project_id".to_string(),
            ));
        }

        let id = self.db.create(ProviderCreate::Antigravity(create)).await?;
        u64::try_from(id)
            .map_err(|_| PolluxError::UnexpectedError(format!("Invalid credential id {}", id)))
    }

    pub async fn update_by_id(
        &self,
        id: CredentialId,
        patch: AntigravityPatch,
    ) -> Result<(), PolluxError> {
        // Keep the same validation semantics: the DB layer uses `i64` ids.
        let _ = i64::try_from(id)
            .map_err(|_| PolluxError::UnexpectedError(format!("Invalid credential id {}", id)))?;

        self.db
            .patch(ProviderPatch::Antigravity { id, patch })
            .await?;
        Ok(())
    }

    pub async fn set_status(&self, id: CredentialId, status: bool) -> Result<(), PolluxError> {
        let _ = i64::try_from(id)
            .map_err(|_| PolluxError::UnexpectedError(format!("Invalid credential id {}", id)))?;

        let patch = AntigravityPatch {
            status: Some(status),
            ..Default::default()
        };
        self.db
            .patch(ProviderPatch::Antigravity { id, patch })
            .await
    }
}
