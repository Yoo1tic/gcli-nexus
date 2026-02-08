//! Database module: models and schema for persistent storage.
//!
//! Layout:
//! - `models.rs`: Rust structs mirroring DB rows
//! - `schema.rs`: SQL DDL for initializing the database (SQLite-first)

pub mod actor;
pub mod models;
pub mod patch;
pub mod schema;
pub mod traits;

mod patch_impl;

pub use models::{DbAntigravityResource, DbCodexResource, DbGeminiCliResource};
pub use patch::{
    AntigravityCreate, AntigravityPatch, CodexCreate, CodexPatch, GeminiCliCreate, GeminiCliPatch,
    ProviderCreate, ProviderPatch,
};
pub use schema::SQLITE_INIT;

pub use actor::{DbActorHandle, spawn};
