pub mod endpoints;
pub mod ops;

use backon::ExponentialBuilder;
use std::{sync::LazyLock, time::Duration};

/// Shared retry policy for Antigravity OAuth + upstream discovery calls.
///
/// Kept small and deterministic (mirrors other providers).
pub(crate) static OAUTH_RETRY_POLICY: LazyLock<ExponentialBuilder> = LazyLock::new(|| {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_secs(1))
        .with_max_delay(Duration::from_secs(3))
        .with_max_times(3)
        .with_jitter()
});
