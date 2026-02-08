use pollux::PolluxError;
use pollux::error::OauthError;
use pollux::providers::geminicli::client::oauth::types::{LoadCodeAssistResponse, UserTier};

fn enabled_response_json() -> serde_json::Value {
    serde_json::json!({
        "currentTier": {
            "id": "standard-tier",
            "quotaTier": "standard-tier"
        },
        "allowedTiers": [
            { "id": "standard-tier", "isDefault": true, "quotaTier": "standard-tier" }
        ],
        "cloudaicompanionProject": "test-project"
    })
}

fn banned_response_json() -> serde_json::Value {
    serde_json::json!({
        "currentTier": null,
        "allowedTiers": [
            { "id": "standard-tier", "isDefault": true, "quotaTier": "standard-tier" }
        ],
        "ineligibleTiers": [
            {
                "reasonCode": "RESTRICTED_AGE",
                "reasonMessage": "Account is not eligible",
                "tierId": "standard-tier"
            }
        ]
    })
}

#[test]
fn reads_project_id_and_tier_from_current_tier() {
    let raw = enabled_response_json();
    let resp: LoadCodeAssistResponse = serde_json::from_value(raw).expect("parse enabled json");
    assert_eq!(
        resp.cloudaicompanion_project.as_deref(),
        Some("test-project")
    );
    assert_eq!(resp.resolve_effective_tier(), UserTier::Standard);
}

#[test]
fn falls_back_to_allowed_tiers_when_current_tier_missing() {
    let raw = banned_response_json();
    let resp: LoadCodeAssistResponse = serde_json::from_value(raw).expect("parse banned json");
    assert_eq!(resp.cloudaicompanion_project, None);
    assert_eq!(resp.resolve_effective_tier(), UserTier::Standard);
}

#[test]
fn ensure_eligible_returns_error_with_details() {
    let raw = banned_response_json();
    let resp: LoadCodeAssistResponse =
        serde_json::from_value(raw.clone()).expect("parse banned json");
    let err = resp
        .ensure_eligible(raw.clone())
        .expect_err("expected ineligible error");

    match err {
        PolluxError::Oauth(OauthError::Flow {
            code,
            message,
            details,
        }) => {
            assert_eq!(code, "RESTRICTED_AGE");
            assert!(message.contains("not eligible"));
            assert_eq!(details, Some(raw));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
