//! Excession's Rust core. Git is authoritative for objects, SQLite for relations.
//! Evidence here is bounded and local, not an unrestricted proof or signed attestation.
#![deny(unsafe_op_in_unsafe_fn)]

#[path = "observe/adapter.rs"]
pub mod adapter;
#[path = "research/benchmark.rs"]
pub mod benchmark;
#[path = "evidence/contracts.rs"]
pub mod contracts;
#[path = "source/digest.rs"]
pub mod digest;
#[path = "research/effects.rs"]
pub mod effects;
#[path = "graph/evidence_query.rs"]
pub mod evidence_query;
#[path = "source/git.rs"]
pub mod git;
#[path = "graph/graph_baseline.rs"]
pub mod graph_baseline;
#[path = "research/historical.rs"]
pub mod historical;
#[path = "source/identity.rs"]
pub mod identity;
#[path = "analysis/impact.rs"]
pub mod impact;
#[path = "graph/index.rs"]
pub mod index;
#[path = "research/integration.rs"]
pub mod integration;
pub mod json;
#[path = "observe/native_boundary.rs"]
pub mod native_boundary;
#[path = "observe/native_discovery.rs"]
pub mod native_discovery;
#[path = "research/optimizer.rs"]
pub mod optimizer;
#[path = "research/overlap.rs"]
pub mod overlap;
#[path = "evidence/preflight.rs"]
pub mod preflight;
#[path = "views/projection.rs"]
pub mod projection;
#[path = "analysis/refactor.rs"]
pub mod refactor;
#[path = "analysis/scip_refactor.rs"]
pub mod scip_refactor;
#[path = "views/scoped.rs"]
pub mod scoped;
#[path = "analysis/security.rs"]
pub mod security;
#[path = "analysis/security_discovery.rs"]
pub mod security_discovery;
#[path = "analysis/security_ingest.rs"]
pub mod security_ingest;
#[path = "analysis/security_survey.rs"]
pub mod security_survey;
#[path = "observe/semantic.rs"]
pub mod semantic;
#[path = "graph/sqlite.rs"]
pub mod sqlite;
#[path = "observe/syntax.rs"]
pub mod syntax;
#[path = "observe/syntax_resolution.rs"]
pub mod syntax_resolution;
#[path = "research/transactions.rs"]
pub mod transactions;
#[path = "research/transform.rs"]
pub mod transform;
#[path = "views/views.rs"]
pub mod views;

pub type Result<T> = std::result::Result<T, String>;

pub fn refuse<T>(message: impl Into<String>) -> Result<T> {
    Err(message.into())
}
