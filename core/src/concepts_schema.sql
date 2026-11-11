-- Immutable semantic projections inside Excession's existing evidence SQLite database.
-- Canonical model and manifest are the only snapshot payloads; child tables are audited projections.
CREATE TABLE IF NOT EXISTS concept_snapshot (
    snapshot_id TEXT PRIMARY KEY,
    model_id TEXT NOT NULL,
    source_commit TEXT NOT NULL,
    source_tree TEXT NOT NULL,
    model_json TEXT NOT NULL,
    manifest_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS concept_subject (
    snapshot_id TEXT NOT NULL REFERENCES concept_snapshot(snapshot_id),
    subject_id TEXT NOT NULL,
    canonical_json TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, subject_id)
);
CREATE TABLE IF NOT EXISTS concept_claim (
    snapshot_id TEXT NOT NULL REFERENCES concept_snapshot(snapshot_id),
    claim_id TEXT NOT NULL,
    canonical_json TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, claim_id)
);
CREATE TABLE IF NOT EXISTS concept_support (
    snapshot_id TEXT NOT NULL,
    claim_id TEXT NOT NULL,
    receipt_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('SUPPORTS', 'CONTRADICTS')),
    kind TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, claim_id, receipt_id, role, kind),
    FOREIGN KEY (snapshot_id,claim_id) REFERENCES concept_claim(snapshot_id, claim_id)
);
CREATE TABLE IF NOT EXISTS concept_definition (
    snapshot_id TEXT NOT NULL REFERENCES concept_snapshot(snapshot_id),
    concept_id TEXT NOT NULL,
    revision_id TEXT NOT NULL,
    canonical_json TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, concept_id),
    UNIQUE (snapshot_id, revision_id)
);
CREATE TABLE IF NOT EXISTS concept_membership (
    snapshot_id TEXT NOT NULL,
    concept_id TEXT NOT NULL,
    claim_id TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, concept_id, claim_id),
    FOREIGN KEY (snapshot_id,concept_id) REFERENCES concept_definition(snapshot_id,concept_id),
    FOREIGN KEY (snapshot_id,claim_id) REFERENCES concept_claim(snapshot_id,claim_id)
);
CREATE TABLE IF NOT EXISTS concept_coverage_receipt (
    snapshot_id TEXT NOT NULL REFERENCES concept_snapshot(snapshot_id),
    receipt_id TEXT NOT NULL,
    PRIMARY KEY (snapshot_id,receipt_id)
);
