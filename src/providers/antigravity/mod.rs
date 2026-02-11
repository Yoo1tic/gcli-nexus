use crate::config::AntigravityResolvedConfig;
use crate::db::DbActorHandle;
use std::sync::Arc;

pub mod client;
pub mod manager;
pub mod resource;
mod thoughtsig;
pub mod workers;

/// Fixed Antigravity-style User-Agent string.
pub(crate) const ANTIGRAVITY_USER_AGENT: &str = "antigravity/1.15.8 (Windows; AMD64)";

pub use client::{AntigravityClient, AntigravityContext};
pub use manager::actor::AntigravityActorHandle;
pub use thoughtsig::AntigravityThoughtSigService;

pub(in crate::providers) async fn spawn(
    db: DbActorHandle,
    cfg: Arc<AntigravityResolvedConfig>,
) -> AntigravityActorHandle {
    manager::spawn(db, cfg).await
}
