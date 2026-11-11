-- Immutable review decisions are not modifications to source-bound claims.
CREATE TABLE IF NOT EXISTS concept_review_event (
    event_id TEXT PRIMARY KEY,
    snapshot_id TEXT NOT NULL REFERENCES concept_snapshot(snapshot_id),
    concept_id TEXT NOT NULL,
    revision_id TEXT NOT NULL,
    canonical_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS concept_review_by_target
    ON concept_review_event(snapshot_id, concept_id, event_id);
