# Append-only evidence uncertainty history

The Rust implementation in `core/src/evidence/unknowns.rs` adds one
transactional table to Excession's existing SQLite database. It references
immutable `evidence_snapshot_v1` rows, and each event contains only receipts
already present in its snapshot.

The durable event JSON is governed by
[`unknown-event.v1.schema.json`](../../contracts/evidence-core/unknown-event.v1.schema.json).
That schema is necessary because canonical event documents are stored and
read back. It is not an additional graph, evidence database or producer.

## Implemented guarantees

- Deterministic question identity scoped to a snapshot; no invented Git
  object identity.
- Append-only event rows and SQL update/delete refusal.
- Atomic `expected_revision` checks through `BEGIN IMMEDIATE`.
- A complete replay of the event chain, row identities, content hashes,
  predecessor IDs, and exact receipt membership.
- Explicit transition rules for create, cumulative evidence update,
  proposed resolution and reopening with new counterevidence.
- Event-count and receipt-size limits; stale, missing, forged, duplicate
  or retracted evidence refuses the transaction.

`RESOLUTION_REPORTED_NOT_VERIFIED` records only a claim that a receipt
supports a resolution. It does not authenticate the source Git revision,
prove a producer executed or establish predicate-specific semantic truth.
A contradictory receipt reopens the question; the previous proposal remains
available in the immutable history.

## Acceptance

The actual Rust/SQLite event output is checked against the JSON Schema in
`core/tests/evidence_unknown_contract.rs`. Native unit tests exercise
transaction rollback, stale revisions, logical transitions, loss of earlier
events, canonical bytes, digest tampering and reopening.

## Remaining work

This module supplies the native persistence primitive only. Public CLI
integration, cross-snapshot reconciliation, independent producer attestations
and predicate-specific verification are still unimplemented. Integrate them
into existing query and agent interfaces; do not create an ontology service.

## CLI workflow

```sh
# Snapshot must already exist in the canonical SQLite evidence store.
excession evidence unknown-append --db /tmp/evidence.sqlite \
  --snapshot SNAPSHOT_SHA256 --question "Which target can be called?" \
  --expected 0 --action CREATE --support RECEIPT_SHA256

# An operator may record an *unverified* proposal.
excession evidence unknown-append --db /tmp/evidence.sqlite \
  --snapshot SNAPSHOT_SHA256 --question "Which target can be called?" \
  --expected 1 --action PROPOSE_RESOLUTION \
  --support RECEIPT_SHA256 --resolution RECEIPT_SHA256

# Replay the append-only history by the unknown_id returned above.
excession inspect unknown-history --db /tmp/evidence.sqlite --id UNKNOWN_SHA256
```

`ADD_EVIDENCE` preserves prior receipts. `REOPEN` requires new
counterevidence and a previous resolution proposal. Pass receipt IDs as a
comma-separated `--support` or `--counter` list; each must belong to the
same immutable snapshot. A write with a stale `--expected` revision refuses,
and no CLI operation can promote a proposal to verified semantic truth.

The CLI test exercises create, stale-write refusal, unsupported action
refusal, proposed resolution, reopen, and complete canonical readback through
the installed binary.
