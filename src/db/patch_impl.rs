//! ProviderPatch -> DbPatchable implementation.
//!
//! This sits in the `db` module because it contains SQL/table knowledge.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;
use tracing::debug;

use crate::error::PolluxError;
use crate::patches::{AntigravityPatch, CodexPatch, DbPatchable, GeminiCliPatch, ProviderPatch};

#[async_trait]
impl DbPatchable for ProviderPatch {
    async fn apply_patch(&self, pool: &SqlitePool) -> Result<(), PolluxError> {
        match self {
            ProviderPatch::GeminiCli { id, patch } => {
                let id = i64::try_from(*id).map_err(|_| {
                    PolluxError::UnexpectedError(format!("Invalid GeminiCli id {id}"))
                })?;

                let GeminiCliPatch {
                    email,
                    refresh_token,
                    access_token,
                    expiry,
                    status,
                } = patch.clone();

                let email_set = email.is_some();
                let refresh_token_set = refresh_token.is_some();
                let access_token_set = access_token.is_some();
                let expiry_set = expiry.is_some();
                let status_set = status.is_some();
                let updated_at = Utc::now();

                let res = sqlx::query!(
                    r#"
                    UPDATE gemini_cli
                    SET
                        email = COALESCE(?, email),
                        refresh_token = COALESCE(?, refresh_token),
                        access_token = COALESCE(?, access_token),
                        expiry = COALESCE(?, expiry),
                        status = COALESCE(?, status),
                        updated_at = ?
                    WHERE id = ?
                    "#,
                    email,
                    refresh_token,
                    access_token,
                    expiry,
                    status,
                    updated_at,
                    id,
                )
                .execute(pool)
                .await?;

                let affected = res.rows_affected();
                debug!(
                    provider = "gemini_cli",
                    id,
                    affected,
                    updated_at = %updated_at,
                    email_set,
                    refresh_token_set,
                    access_token_set,
                    expiry_set,
                    status_set,
                    "db patch applied"
                );

                if affected == 0 {
                    return Err(PolluxError::UnexpectedError(format!(
                        "GeminiCli credential not found for id={id}"
                    )));
                }

                Ok(())
            }

            ProviderPatch::Codex { id, patch } => {
                let id = i64::try_from(*id)
                    .map_err(|_| PolluxError::UnexpectedError(format!("Invalid Codex id {id}")))?;

                let CodexPatch {
                    email,
                    account_id,
                    sub,
                    refresh_token,
                    access_token,
                    expiry,
                    chatgpt_plan_type,
                    status,
                } = patch.clone();

                let email_set = email.is_some();
                let account_id_set = account_id.is_some();
                let sub_set = sub.is_some();
                let refresh_token_set = refresh_token.is_some();
                let access_token_set = access_token.is_some();
                let expiry_set = expiry.is_some();
                let chatgpt_plan_type_set = chatgpt_plan_type.is_some();
                let status_set = status.is_some();
                let updated_at = Utc::now();

                // Use the non-macro query API so we don't have to keep SQLx's offline cache in sync.
                let res = sqlx::query(
                    r#"
                    UPDATE codex
                    SET
                        email = COALESCE(?, email),
                        account_id = COALESCE(?, account_id),
                        sub = COALESCE(?, sub),
                        refresh_token = COALESCE(?, refresh_token),
                        access_token = COALESCE(?, access_token),
                        expiry = COALESCE(?, expiry),
                        chatgpt_plan_type = COALESCE(?, chatgpt_plan_type),
                        status = COALESCE(?, status),
                        updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(email)
                .bind(account_id)
                .bind(sub)
                .bind(refresh_token)
                .bind(access_token)
                .bind(expiry)
                .bind(chatgpt_plan_type)
                .bind(status)
                .bind(updated_at)
                .bind(id)
                .execute(pool)
                .await?;

                let affected = res.rows_affected();
                debug!(
                    provider = "codex",
                    id,
                    affected,
                    updated_at = %updated_at,
                    email_set,
                    account_id_set,
                    sub_set,
                    refresh_token_set,
                    access_token_set,
                    expiry_set,
                    chatgpt_plan_type_set,
                    status_set,
                    "db patch applied"
                );

                if affected == 0 {
                    return Err(PolluxError::UnexpectedError(format!(
                        "Codex key not found for id={id} (create first)"
                    )));
                }

                Ok(())
            }

            ProviderPatch::Antigravity { id, patch } => {
                let id = i64::try_from(*id).map_err(|_| {
                    PolluxError::UnexpectedError(format!("Invalid Antigravity id {id}"))
                })?;

                let AntigravityPatch {
                    email,
                    refresh_token,
                    access_token,
                    expiry,
                    status,
                } = patch.clone();

                let email_set = email.is_some();
                let refresh_token_set = refresh_token.is_some();
                let access_token_set = access_token.is_some();
                let expiry_set = expiry.is_some();
                let status_set = status.is_some();
                let updated_at = Utc::now();

                // Use bind query API to avoid SQLx offline cache requirements.
                let res = sqlx::query(
                    r#"
                    UPDATE antigravity
                    SET
                        email = COALESCE(?, email),
                        refresh_token = COALESCE(?, refresh_token),
                        access_token = COALESCE(?, access_token),
                        expiry = COALESCE(?, expiry),
                        status = COALESCE(?, status),
                        updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(email)
                .bind(refresh_token)
                .bind(access_token)
                .bind(expiry)
                .bind(status)
                .bind(updated_at)
                .bind(id)
                .execute(pool)
                .await?;

                let affected = res.rows_affected();
                debug!(
                    provider = "antigravity",
                    id,
                    affected,
                    updated_at = %updated_at,
                    email_set,
                    refresh_token_set,
                    access_token_set,
                    expiry_set,
                    status_set,
                    "db patch applied"
                );

                if affected == 0 {
                    return Err(PolluxError::UnexpectedError(format!(
                        "Antigravity credential not found for id={id}"
                    )));
                }

                Ok(())
            }
        }
    }
}
