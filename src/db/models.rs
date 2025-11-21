use crate::google_oauth::credentials::GoogleCredential;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Persistent representation of a Google credential row.
/// Includes an auto-increment `id` primary key and a `status` flag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct DbCredential {
    pub id: i64,
    pub email: Option<String>,
    pub project_id: String,
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expiry: DateTime<Utc>,
    pub status: bool,
}

impl From<GoogleCredential> for DbCredential {
    fn from(g: GoogleCredential) -> Self {
        Self {
            id: 0, // to be set by DB AUTOINCREMENT on insert
            email: g.email,
            project_id: g.project_id,
            refresh_token: g.refresh_token,
            access_token: g.access_token,
            expiry: g.expiry,
            status: true,
        }
    }
}

impl From<DbCredential> for GoogleCredential {
    fn from(d: DbCredential) -> Self {
        GoogleCredential {
            email: d.email,
            project_id: d.project_id,
            refresh_token: d.refresh_token,
            access_token: d.access_token,
            expiry: d.expiry,
        }
    }
}
