use crate::error::NexusError;
use chrono::Duration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleCredential {
    pub email: Option<String>,
    pub project_id: String,
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expiry: DateTime<Utc>,
}

impl Default for GoogleCredential {
    fn default() -> Self {
        Self {
            email: None,
            project_id: String::new(),
            refresh_token: String::new(),
            access_token: None,
            expiry: Utc::now(),
        }
    }
}

impl GoogleCredential {
    /// Return true if current time is within 5 minutes of expiry (inclusive).
    /// This early-expiry buffer avoids edge cases during requests.
    pub fn is_expired(&self) -> bool {
        Utc::now() + Duration::minutes(5) >= self.expiry
    }

    /// Merge updates from any JSON-serializable payload into this credential.
    /// - Accepts any `T: Serialize` and converts to `serde_json::Value` internally.
    /// - Supports both OAuth token response (access_token, expires_in)
    ///   and full credential JSON (project_id, expiry, etc.).
    /// - Only updates fields present in the JSON; others remain unchanged.
    pub fn update_credential(&mut self, payload: impl Serialize) -> Result<(), NexusError> {
        #[derive(Debug, Default, Deserialize)]
        struct CredentialPatch {
            email: Option<String>,
            project_id: Option<String>,
            refresh_token: Option<String>,
            access_token: Option<String>,
            expiry: Option<DateTime<Utc>>,
            expires_in: Option<i64>,
        }

        let value = serde_json::to_value(payload)?;
        let patch: CredentialPatch = serde_json::from_value(value)?;

        macro_rules! set_plain {
            ($field:ident) => {
                if let Some(v) = patch.$field {
                    self.$field = v;
                }
            };
        }
        macro_rules! set_opt {
            ($field:ident) => {
                if let Some(v) = patch.$field {
                    self.$field = Some(v);
                }
            };
        }

        set_opt!(email);
        set_plain!(project_id);
        set_plain!(refresh_token);
        set_opt!(access_token);

        if let Some(secs) = patch.expires_in {
            self.expiry = Utc::now() + Duration::seconds(secs);
        } else if let Some(dt) = patch.expiry {
            self.expiry = dt;
        }

        debug!(
            "Project_ID {}, Credentials updated successfully",
            self.project_id
        );
        Ok(())
    }

    /// Build a credential from any JSON-like payload by applying updates to a default struct.
    pub fn from_payload(payload: impl Serialize) -> Result<Self, NexusError> {
        let mut cred = GoogleCredential::default();
        cred.update_credential(payload)?;
        Ok(cred)
    }
}
