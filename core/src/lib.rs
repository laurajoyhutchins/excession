//! Excession's Rust core. Git is authoritative for objects, SQLite for relations.
//! Evidence here is bounded and local, not an unrestricted proof or signed attestation.
#![deny(unsafe_op_in_unsafe_fn)]

pub mod adapter;
pub mod agent;
pub mod analyze;
pub mod benchmark;
pub mod change_inventory;
pub mod compiler_clang;
pub mod configured_store;
pub mod contracts;
pub mod digest;

pub mod discovery;
pub mod doctor;
pub mod effects;
pub mod evidence;
pub mod evidence_coverage;
pub mod evidence_query;
pub mod evidence_store;
pub mod explain;
pub mod git;
pub mod graph;
pub mod graph_assembly;
pub mod graph_baseline;
pub mod historical;
pub mod identity;
pub mod impact;
pub mod impact_query;
mod incremental_python;
pub mod index;
mod indexed_tree;
pub mod integration;
pub mod json;
pub mod kbuild_graph;
pub mod kernel;
pub mod kernel_coverage;
pub mod kernel_links;
mod module_identity;
pub mod native_boundary;
pub mod native_discovery;
pub mod native_syntax;
pub mod observed_relations;
pub mod optimizer;
pub mod overlap;
pub mod preflight;
pub mod projection;
pub mod refactor;
pub mod refactor_codegen;
mod refactor_module_api;
pub mod refactor_rust_analyzer;
pub mod refactor_rust_repair;
pub mod scip_evidence;
pub mod scip_refactor;
pub mod scoped;
pub mod security;
pub mod security_discovery;
pub mod security_ingest;
pub mod security_survey;
pub mod semantic;
pub mod semantic_contract;
pub mod source_capabilities;
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
