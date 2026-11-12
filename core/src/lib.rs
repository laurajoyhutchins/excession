//! Excession's Rust core. Git is authoritative for objects, SQLite for relations.
//! Evidence here is bounded and local, not an unrestricted proof or signed attestation.
#![deny(unsafe_op_in_unsafe_fn)]

pub mod adapter;
pub mod benchmark;
pub mod compiler_clang;
pub mod concepts;
pub mod concepts_analysis;
pub mod concepts_author;
pub mod concepts_compile;
pub mod concepts_mcp;
pub mod concepts_normalize;
pub mod concepts_query;
pub mod concepts_review;
pub mod concepts_scip;
pub mod concepts_store;
pub mod concepts_value;
pub mod configured_store;
pub mod contracts;
pub mod digest;
pub mod doctor;
pub mod effects;
pub mod evidence_query;
pub mod git;
pub mod graph;
pub mod graph_assembly;
pub mod graph_baseline;
pub mod historical;
pub mod identity;
pub mod impact;
pub mod index;
pub mod integration;
pub mod json;
pub mod kbuild_graph;
pub mod kernel;
pub mod kernel_coverage;
pub mod kernel_links;
pub mod native_boundary;
pub mod native_discovery;
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
pub mod semantic_contract;
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
