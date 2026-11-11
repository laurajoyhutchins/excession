//! Excession's Rust core. Git is authoritative for objects, SQLite for relations.
//! Evidence here is bounded and local, not an unrestricted proof or signed attestation.
#![deny(unsafe_op_in_unsafe_fn)]

pub mod adapter;
pub mod benchmark;
pub mod contracts;
pub mod digest;
pub mod effects;
pub mod git;
pub mod graph_baseline;
pub mod historical;
pub mod identity;
pub mod impact;
pub mod index;
pub mod integration;
pub mod json;
pub mod native_boundary;
pub mod optimizer;
pub mod overlap;
pub mod preflight;
pub mod projection;
pub mod refactor;
pub mod scip_refactor;
pub mod scoped;
pub mod security;
pub mod security_discovery;
pub mod security_ingest;
pub mod security_survey;
pub mod semantic;
pub mod sqlite;
pub mod syntax;
pub mod syntax_resolution;
pub mod transactions;
pub mod transform;
pub mod views;

pub type Result<T> = std::result::Result<T, String>;

pub fn refuse<T>(message: impl Into<String>) -> Result<T> {
    Err(message.into())
}
