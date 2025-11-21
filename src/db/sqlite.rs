use crate::db::models::DbCredential;
use crate::db::schema::SQLITE_INIT;
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use chrono::{DateTime, Utc};
use sqlx::{Pool, Sqlite};

pub type SqlitePool = Pool<Sqlite>;

#[derive(Clone)]
pub struct CredentialsStorage {
    pool: SqlitePool,
}

impl CredentialsStorage {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Initialize the schema by executing the bundled DDL.
    pub async fn init_schema(&self) -> Result<(), NexusError> {
        self.apply_schema().await
    }

    /// Upsert by unique project_id. Returns the row id.
    /// Uses SQLite `INSERT ... ON CONFLICT(project_id) DO UPDATE`.
    pub async fn upsert(&self, cred: GoogleCredential, status: bool) -> Result<i64, NexusError> {
        let record = sqlx::query!(
            r#"
            INSERT INTO credentials (
                email, project_id, refresh_token, access_token, expiry, status
            ) 
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(project_id) DO UPDATE SET
                email=excluded.email,
                refresh_token=excluded.refresh_token,
                access_token=excluded.access_token,
                expiry=excluded.expiry,
                status=excluded.status
            RETURNING id
            "#,
            cred.email,
            cred.project_id,
            cred.refresh_token,
            cred.access_token,
            cred.expiry,
            status
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(record.id)
    }

    pub async fn get_by_id(&self, id: i64) -> Result<DbCredential, NexusError> {
        let record = sqlx::query_as!(
            DbCredential,
            r#"
            SELECT
                id,
                email,
                project_id,
                refresh_token,
                access_token,
                expiry as "expiry!: DateTime<Utc>",
                status as "status!: bool"
            FROM credentials
            WHERE id = ?
            "#,
            id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn get_by_project_id(&self, project_id: &str) -> Result<DbCredential, NexusError> {
        let record = sqlx::query_as!(
            DbCredential,
            r#"
            SELECT
                id,
                email,
                project_id,
                refresh_token,
                access_token,
                expiry as "expiry!: DateTime<Utc>",
                status as "status!: bool"
            FROM credentials
            WHERE project_id = ?
            "#,
            project_id
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(record)
    }

    pub async fn list_active(&self) -> Result<Vec<DbCredential>, NexusError> {
        let records = sqlx::query_as!(
            DbCredential,
            r#"
            SELECT
                id,
                email,
                project_id,
                refresh_token,
                access_token,
                expiry as "expiry!: DateTime<Utc>",
                status as "status!: bool"
            FROM credentials
            WHERE status = 1
            ORDER BY id
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }

    pub async fn set_status(&self, id: i64, status: bool) -> Result<(), NexusError> {
        sqlx::query!(
            r#"
            UPDATE credentials
            SET status = ?
            WHERE id = ?
            "#,
            status,
            id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Update all credential fields by id (except id itself).
    pub async fn update_by_id(
        &self,
        id: i64,
        cred: GoogleCredential,
        status: bool,
    ) -> Result<(), NexusError> {
        sqlx::query!(
            r#"UPDATE credentials SET
                email = ?,
                project_id = ?,
                refresh_token = ?,
                access_token = ?,
                expiry = ?,
                status = ?
              WHERE id = ?"#,
            cred.email,
            cred.project_id,
            cred.refresh_token,
            cred.access_token,
            cred.expiry,
            status,
            id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn apply_schema(&self) -> Result<(), NexusError> {
        for stmt in SQLITE_INIT.split(';') {
            let s = stmt.trim();
            if s.is_empty() {
                continue;
            }
            sqlx::query(s).execute(&self.pool).await?;
        }
        Ok(())
    }
}
