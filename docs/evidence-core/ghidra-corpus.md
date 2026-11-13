# Ghidra observations in the Excession software corpus

This is an artifact-first **adapter and corpus retrieval contract** for Ghidra
Function ID (FID) and BSim. It does not reimplement either matching algorithm.
Ghidra supplies native match observations and their scores; Excession stores
their bytes alongside a normalized, strictly validated record.

## Three operations

```sh
# Native Ghidra producer output in /tmp/ghidra-native.txt;
# adapter-normalized observations in /tmp/ghidra-matches.json.
excession evidence ghidra-import --db corpus.sqlite \
  --artifact ./unknown.o --report /tmp/ghidra-matches.json \
  --native /tmp/ghidra-native.txt

# Register an externally reviewed candidate from a pinned Git commit.
# candidate.json contains the exact "candidate" object from an imported match.
excession evidence ghidra-source-bind --db corpus.sqlite \
  --repo /path/to/checkout --rev 0123... --source src/function.c \
  --candidate /tmp/candidate.json --symbol example_function

# Retrieve all candidate observations for one function in one binary.
excession inspect ghidra-candidates --db corpus.sqlite \
  --artifact ./unknown.o --entry ram:00001000 --limit 16
```

A matching FID or BSim candidate key is a lookup *hint*. A bound Git source
is independently verified as the exact commit/tree/blob/mode/path but **never
promoted** to proof that it produced the binary. All source candidates are
returned, including conflicting ones; none is selected as authoritative.
Queries revalidate the stored match record against the preserved normalized
report and native-receipt hash, and replay each returned Git source identity.

## Native data and normalization

The `--native` input is a UTF-8 text receipt from an upstream Ghidra
query/export. The `--report` input is an *Excession adapter document*, not
a claimed native Ghidra JSON schema. Its `producer.native_receipt_sha256`
must hash the **exact original** `--native` bytes. Both are stored immutably
in the same SQLite authority used for other evidence.

The schema is `contracts/evidence-core/ghidra-matches.v1.schema.json`.
The report pins artifact SHA-256 and length, producer version, an
analysis-profile digest, and typed FID/BSim native match records.

FID candidates retain the database identity, library family/version/variant
and function name. FID evidence retains native overall score and hash-match
mode. BSim candidates retain its **native executable MD5 and function entry
identity**, the BSim database identity, and the matched function name. BSim
evidence retains similarity and confidence **as different measures**.
An MD5 in a BSim result is *not* upgraded into a SHA-256 artifact identity.

Database identity can be a content hash or explicitly `DECLARED_NAME`.
Declared database names are not independently authenticated. Match
observation correctness, Ghidra execution, binary/source correspondence,
and runtime semantics remain **NOT_ESTABLISHED**.

## Current execution boundary

This change implements ingestion, immutable indexing, cross-artifact candidate
retrieval and Git replay. It does **not** yet ship or verify a live
Ghidra headless producer. Fixtures use synthetic Ghidra-shaped native receipts
and cannot establish upstream Function ID or BSim fidelity. A native Ghidra
exporter must be tested against actual `FidService` and BSim
query APIs before calling this an end-to-end Ghidra integration.

Reuse is by exact **provider candidate identity** across any artifacts and
Git sources registered in the same SQLite store, not fuzzy matching inside
Excession. This avoids N×N rescanning of all compiled functions. Ghidra should
do the expensive similarity indexing and querying.

## Follow-on acceptance

- Native Ghidra producer outputs must have a versioned, tested cross-check with
  Ghidra's Function ID and BSim results, including thresholds, ambiguity,
  analysis settings and database identity.
- Connect retrieved Git files and SCIP symbols, track coverage and unresolved
  source candidates, and only allow compiler-in-the-loop validation to upgrade
  source/artifact correspondence under an explicit verified build contract.
- Add corpus-level change invalidation keyed to pinned Git blobs and artifact
  digest, without discarding independent evidence.
