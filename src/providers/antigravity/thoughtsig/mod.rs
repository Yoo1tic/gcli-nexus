//! Antigravity thought-signature pipeline.
//!
//! Provider-specific policy:
//! - `thought` part cache hit: keep the part and fill real signature.
//! - `thought` part cache miss: drop that thought part entirely.
//! - `functionCall` cache miss: keep the part and fill dummy signature.
//!
//! This intentionally differs from GeminiCLI behavior for thought parts.

mod adapter_request;
mod adapter_response;
mod service;

pub use service::AntigravityThoughtSigService;
