use crate::db::models::{DbAntigravityResource, DbCodexResource, DbGeminiCliResource};
use crate::db::patch::{ProviderCreate, ProviderPatch};
use crate::db::schema::SQLITE_INIT;
use crate::db::traits::DbPatchable;
use crate::error::PolluxError;
use chrono::Utc;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::{str::FromStr, time::Duration};
use tracing::info;

#[derive(Debug)]
pub enum DbActorMessage {
    /// Create (or upsert) a provider record and return its id.
    Create(ProviderCreate, RpcReplyPort<Result<i64, PolluxError>>),

    /// Patch a provider record by id.
    Patch(ProviderPatch, RpcReplyPort<Result<(), PolluxError>>),

    /// List active Gemini CLI credentials (status=1).
    ListActiveGeminiCli(RpcReplyPort<Result<Vec<DbGeminiCliResource>, PolluxError>>),

    /// List active Codex keys (status=1).
    ListActiveCodex(RpcReplyPort<Result<Vec<DbCodexResource>, PolluxError>>),

    /// List active Antigravity credentials (status=1).
    ListActiveAntigravity(RpcReplyPort<Result<Vec<DbAntigravityResource>, PolluxError>>),

    /// Get Codex key by id.
    GetCodexById(i64, RpcReplyPort<Result<DbCodexResource, PolluxError>>),
}

#[derive(Clone)]
pub struct DbActorHandle {
    actor: ActorRef<DbActorMessage>,
}

impl DbActorHandle {
    pub async fn create(&self, create: ProviderCreate) -> Result<i64, PolluxError> {
        ractor::call!(self.actor, DbActorMessage::Create, create)
            .map_err(|e| PolluxError::RactorError(format!("DbActor Create RPC failed: {e}")))?
    }

    pub async fn patch(&self, patch: ProviderPatch) -> Result<(), PolluxError> {
        ractor::call!(self.actor, DbActorMessage::Patch, patch)
            .map_err(|e| PolluxError::RactorError(format!("DbActor Patch RPC failed: {e}")))?
    }

    pub async fn list_active_geminicli(&self) -> Result<Vec<DbGeminiCliResource>, PolluxError> {
        ractor::call!(self.actor, DbActorMessage::ListActiveGeminiCli).map_err(|e| {
            PolluxError::RactorError(format!("DbActor ListActiveGeminiCli RPC failed: {e}"))
        })?
    }

    pub async fn list_active_codex(&self) -> Result<Vec<DbCodexResource>, PolluxError> {
        ractor::call!(self.actor, DbActorMessage::ListActiveCodex).map_err(|e| {
            PolluxError::RactorError(format!("DbActor ListActiveCodex RPC failed: {e}"))
        })?
    }

    pub async fn list_active_antigravity(&self) -> Result<Vec<DbAntigravityResource>, PolluxError> {
        ractor::call!(self.actor, DbActorMessage::ListActiveAntigravity).map_err(|e| {
            PolluxError::RactorError(format!("DbActor ListActiveAntigravity RPC failed: {e}"))
        })?
    }

    pub async fn get_codex_by_id(&self, id: i64) -> Result<DbCodexResource, PolluxError> {
        ractor::call!(self.actor, DbActorMessage::GetCodexById, id).map_err(|e| {
            PolluxError::RactorError(format!("DbActor GetCodexById RPC failed: {e}"))
        })?
    }
}

struct DbActorState {
    pool: SqlitePool,
}

struct DbActor;

#[ractor::async_trait]
impl Actor for DbActor {
    type Msg = DbActorMessage;
    type State = DbActorState;
    type Arguments = String;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        database_url: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        let connect_opts = SqliteConnectOptions::from_str(database_url.as_str())
            .map_err(|e| ActorProcessingErr::from(format!("invalid database url: {e}")))?
            .create_if_missing(true)
            .busy_timeout(Duration::from_secs(5))
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);

        let pool = SqlitePoolOptions::new()
            .connect_with(connect_opts)
            .await
            .map_err(|e| ActorProcessingErr::from(format!("db connect failed: {e}")))?;

        apply_schema(&pool)
            .await
            .map_err(|e| ActorProcessingErr::from(format!("db schema init failed: {e}")))?;

        info!("DbActor initialized");
        Ok(DbActorState { pool })
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        message: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            DbActorMessage::Create(create, reply) => {
                let res = self.create_provider(&state.pool, create).await;
                let _ = reply.send(res);
            }
            DbActorMessage::Patch(patch, reply) => {
                let res = patch.apply_patch(&state.pool).await;
                let _ = reply.send(res);
            }
            DbActorMessage::ListActiveGeminiCli(reply) => {
                let res = self.list_active_geminicli(&state.pool).await;
                let _ = reply.send(res);
            }
            DbActorMessage::ListActiveCodex(reply) => {
                let res = self.list_active_codex(&state.pool).await;
                let _ = reply.send(res);
            }
            DbActorMessage::ListActiveAntigravity(reply) => {
                let res = self.list_active_antigravity(&state.pool).await;
                let _ = reply.send(res);
            }
            DbActorMessage::GetCodexById(id, reply) => {
                let res = self.get_codex_by_id(&state.pool, id).await;
                let _ = reply.send(res);
            }
        }
        Ok(())
    }
}

impl DbActor {
    async fn create_provider(
        &self,
        pool: &SqlitePool,
        create: ProviderCreate,
    ) -> Result<i64, PolluxError> {
        match create {
            ProviderCreate::GeminiCli(c) => {
                let now = Utc::now();
                let id: i64 = sqlx::query_scalar(
                    r#"
                INSERT INTO gemini_cli (
                    email, sub, project_id, refresh_token, access_token, expiry, status, created_at, updated_at
                )
                VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?)
                ON CONFLICT(sub, project_id) DO UPDATE SET
                    email=excluded.email,
                    refresh_token=excluded.refresh_token,
                    access_token=excluded.access_token,
                    expiry=excluded.expiry,
                    status=1,
                    updated_at=excluded.updated_at
                RETURNING id
                "#,
                )
                .bind(c.email)
                .bind(c.sub)
                .bind(c.project_id)
                .bind(c.refresh_token)
                .bind(c.access_token)
                .bind(c.expiry)
                .bind(now)
                .bind(now)
                .fetch_one(pool)
                .await?;

                Ok(id)
            }

            ProviderCreate::Codex(c) => {
                let now = Utc::now();

                let id: i64 = sqlx::query_scalar(
                    r#"
                INSERT INTO codex (
                    email, sub, account_id, refresh_token, access_token, expiry, chatgpt_plan_type, status, created_at, updated_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?)
                ON CONFLICT(sub, account_id) DO UPDATE SET
                    email = COALESCE(excluded.email, email),
                    refresh_token = excluded.refresh_token,
                    access_token = excluded.access_token,
                    expiry = excluded.expiry,
                    chatgpt_plan_type = COALESCE(excluded.chatgpt_plan_type, chatgpt_plan_type),
                    status = 1,
                    updated_at = excluded.updated_at
                RETURNING id
                "#,
                )
                .bind(c.email)
                .bind(c.sub)
                .bind(c.account_id)
                .bind(c.refresh_token)
                .bind(c.access_token)
                .bind(c.expiry)
                .bind(c.chatgpt_plan_type)
                .bind(now)
                .bind(now)
                .fetch_one(pool)
                .await?;

                Ok(id)
            }

            ProviderCreate::Antigravity(c) => {
                let now = Utc::now();
                let sub = c
                    .sub
                    .unwrap_or_else(|| synthetic_sub_from_refresh_token(&c.refresh_token));

                let id: i64 = sqlx::query_scalar(
                    r#"
                INSERT INTO antigravity (
                    email, sub, project_id, refresh_token, access_token, expiry, status, created_at, updated_at
                )
                VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?)
                ON CONFLICT(sub, project_id) DO UPDATE SET
                    email=excluded.email,
                    refresh_token=excluded.refresh_token,
                    access_token=excluded.access_token,
                    expiry=excluded.expiry,
                    status=1,
                    updated_at=excluded.updated_at
                RETURNING id
                "#,
                )
                .bind(c.email)
                .bind(sub)
                .bind(c.project_id)
                .bind(c.refresh_token)
                .bind(c.access_token)
                .bind(c.expiry)
                .bind(now)
                .bind(now)
                .fetch_one(pool)
                .await?;

                Ok(id)
            }
        }
    }

    async fn list_active_geminicli(
        &self,
        pool: &SqlitePool,
    ) -> Result<Vec<DbGeminiCliResource>, PolluxError> {
        let rows = sqlx::query_as::<_, DbGeminiCliResource>(
            r#"
        SELECT id, email, sub, project_id, refresh_token, access_token, expiry, status, created_at, updated_at
        FROM gemini_cli
        WHERE status = 1
        ORDER BY id
        "#,
        )
        .fetch_all(pool)
        .await?;

        Ok(rows)
    }

    async fn list_active_codex(
        &self,
        pool: &SqlitePool,
    ) -> Result<Vec<DbCodexResource>, PolluxError> {
        let rows = sqlx::query_as::<_, DbCodexResource>(
            r#"
        SELECT id, email, sub, account_id, refresh_token, access_token, expiry, chatgpt_plan_type, status, created_at, updated_at
        FROM codex
        WHERE status = 1
        ORDER BY id
        "#,
        )
        .fetch_all(pool)
        .await?;

        Ok(rows)
    }

    async fn list_active_antigravity(
        &self,
        pool: &SqlitePool,
    ) -> Result<Vec<DbAntigravityResource>, PolluxError> {
        let rows = sqlx::query_as::<_, DbAntigravityResource>(
            r#"
        SELECT id, email, sub, project_id, refresh_token, access_token, expiry, status, created_at, updated_at
        FROM antigravity
        WHERE status = 1
        ORDER BY id
        "#,
        )
        .fetch_all(pool)
        .await?;

        Ok(rows)
    }

    async fn get_codex_by_id(
        &self,
        pool: &SqlitePool,
        id: i64,
    ) -> Result<DbCodexResource, PolluxError> {
        let row = sqlx::query_as::<_, DbCodexResource>(
            r#"
        SELECT id, email, sub, account_id, refresh_token, access_token, expiry, chatgpt_plan_type, status, created_at, updated_at
        FROM codex
        WHERE id = ?
        "#,
        )
        .bind(id)
        .fetch_one(pool)
        .await?;

        Ok(row)
    }
}

fn synthetic_sub_from_refresh_token(refresh_token: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    refresh_token.hash(&mut h);
    format!("rt_hash:{:016x}", h.finish())
}

/// Spawn the database actor and return a cloneable handle.
pub async fn spawn(database_url: &str) -> DbActorHandle {
    let (actor, _jh) = ractor::Actor::spawn(
        Some("DbActor".to_string()),
        DbActor,
        database_url.to_string(),
    )
    .await
    .expect("failed to spawn DbActor");

    DbActorHandle { actor }
}

async fn apply_schema(pool: &SqlitePool) -> Result<(), PolluxError> {
    for stmt in SQLITE_INIT.split(';') {
        let s = stmt.trim();
        if s.is_empty() {
            continue;
        }
        sqlx::query(s).execute(pool).await?;
    }
    Ok(())
}
