# Source layout

Excession deliberately remains one Rust package and one installed CLI. Public Rust modules keep their established names; `core/src/lib.rs` maps them to responsibility-based source directories.

- `source/` authenticates Git objects and source identities.
- `evidence/` owns contracts and preflight validation.
- `graph/` persists relational and indexed evidence in SQLite.
- `observe/` hosts language-native and syntax observation boundaries. Language adapters remain under `core/adapters/` until their native migration PRs finish.
- `views/` derives read-only projections and navigation.
- `analysis/` produces advisory security, impact, and refactoring hypotheses.
- `research/` preserves historical experiments, transactions, and reproducibility machinery.
- `json.rs` and `lib.rs` remain at the crate root as common internal infrastructure.
- `bin/excession.rs` remains stable until the independent clap CLI migration is integrated.

Moving modules does not establish complete dependencies or behavior preservation. The accompanying CI must compile and test the new paths, while self-analysis authenticates each pinned source and replays rust-analyzer SCIP observations. Historical fixture blobs and versioned receipts remain unchanged.

A later, separately verified step can lift the Cargo package and adapters to the repository root. This stage does not change installation paths or public Rust module names, avoiding simultaneous conflicts with active CLI, contract, and adapter migrations.
