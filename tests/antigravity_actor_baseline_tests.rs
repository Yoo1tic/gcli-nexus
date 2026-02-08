use std::time::{SystemTime, UNIX_EPOCH};

#[tokio::test]
async fn antigravity_actor_get_credential_returns_none_with_empty_db() {
    // NOTE: `pollux::db::spawn()` registers a singleton ractor actor by name within a process.
    // Keep this test file to a single test.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_nanos();

    let mut temp_path = std::env::temp_dir();
    temp_path.push(format!(
        "pollux-antigravity-actor-baseline-{}-{}.sqlite",
        std::process::id(),
        nanos
    ));
    let database_url = format!("sqlite:{}", temp_path.display());
    let db = pollux::db::spawn(&database_url).await;

    let mut cfg = pollux::config::Config::default();
    cfg.basic.pollux_key = "pwd".to_string();

    let providers = pollux::providers::Providers::spawn(db, &cfg).await;

    let model_mask =
        pollux::model_catalog::mask("gemini-2.5-pro").expect("model present in registry");
    let lease = providers
        .antigravity
        .get_credential(model_mask)
        .await
        .expect("GetCredential should not error");

    assert!(lease.is_none(), "expected no credential in empty DB");

    let _ = tokio::fs::remove_file(&temp_path).await;
}
