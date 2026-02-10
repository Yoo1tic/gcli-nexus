use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use std::time::Duration;

pub const UPSTREAM_BODY_PREVIEW_CHARS: usize = 500;

#[derive(Debug, PartialEq, Eq)]
pub enum ActionForError {
    RateLimit(Duration),
    Ban,
    Invalid,
    ModelUnsupported,
    None,
}

pub trait MappingAction: std::fmt::Debug + DeserializeOwned {
    fn try_match_rule(&self, status: StatusCode) -> Option<ActionForError>;

    fn action_from_status(status: StatusCode) -> ActionForError {
        match status {
            StatusCode::TOO_MANY_REQUESTS => ActionForError::RateLimit(Duration::from_secs(60)),
            StatusCode::FORBIDDEN => ActionForError::Ban,
            StatusCode::PAYMENT_REQUIRED => ActionForError::Ban,
            StatusCode::UNAUTHORIZED => ActionForError::Invalid,
            _ => ActionForError::None,
        }
    }
}

pub async fn classify_upstream_error<E, MappedError>(
    resp: reqwest::Response,
    map_raw: impl FnOnce(E) -> MappedError,
    map_status: impl FnOnce(StatusCode, String) -> MappedError,
) -> (ActionForError, MappedError)
where
    E: MappingAction,
{
    let status = resp.status();
    let bytes = resp.bytes().await.unwrap_or_default().to_vec();
    let raw_body = String::from_utf8_lossy(&bytes);
    let raw_body_owned = raw_body.into_owned();

    if let Ok(error) = serde_json::from_slice::<E>(&bytes) {
        if let Some(action) = error.try_match_rule(status) {
            tracing::debug!(
                %status,
                ?action,
                ?error,
                body = %format!("{:.len$}", raw_body_owned, len = UPSTREAM_BODY_PREVIEW_CHARS),
                "Upstream structured error matched mapping rule"
            );

            return (action, map_raw(error));
        }

        let action = E::action_from_status(status);

        tracing::debug!(
            %status,
            ?action,
            ?error,
            body = %format!("{:.len$}", raw_body_owned, len = UPSTREAM_BODY_PREVIEW_CHARS),
            "Upstream structured error fell back to status mapping"
        );

        return (action, map_status(status, raw_body_owned));
    }

    let action = E::action_from_status(status);

    tracing::debug!(
        %status,
        ?action,
        body = %format!("{:.len$}", raw_body_owned, len = UPSTREAM_BODY_PREVIEW_CHARS),
        "Upstream unstructured error"
    );

    (action, map_status(status, raw_body_owned))
}
