# LLM-authored ontologies over pinned code evidence

The ontology author is the **agent**. Excession is not an ontology
generator, semantic oracle or autonomous architectural reviewer. It exposes
Git-checked source snippets and official SCIP-indexed relationships; the agent
supplies names, definitions and proposed memberships. Rust validates every
source identity and evidence reference, then can store only an unreviewed
proposal in the existing SQLite database.

There is no local or hosted LLM API dependency. The CLI and a minimal
**local, stdio MCP server** use the same Rust authoring implementation.

## Agent's normal workflow

1. Find relevant source paths with `search_code_sources` (or `concepts ontology-list`). Then inspect source and exact indexed evidence by path with
   `inspect_code_evidence` (or `concepts ontology-context`). Use the
   returned **normalized** `observation_id` values when composing your draft.
   The excerpt is capped at 40 lines, observations at 40 per path; request
   another first line or another path as needed. The report explicitly states
   if observation results are truncated.
2. Read `ontology_schema`. Construct `excession.concepts-ontology-proposal.v1`
   yourself. Supply a short, unique concept `key`, meaningful `name`, a
   falsifiable `definition`, a reasoned `rationale`, and evidence-backed
   source `members`. There is no requirement for reciprocal references.
3. `draft_ontology` (or `ontology-preview`) validates the proposal against
   the exact Git tree and SCIP source-index receipt. It yields a model,
   immutable snapshot manifest and the explicit status
   `UNREVIEWED_LLM_PROPOSAL`. Revise the draft based on gaps or counterevidence.
4. Optionally `publish_ontology` (or `ontology-publish`) saves this
   **unreviewed draft**, not an accepted ontology. The MCP host must have
   explicitly launched the server with `--allow-publish true`; without that
   flag the write tool is absent and invocations are refused.
5. Use `audit_ontology` to regenerate and compare the saved snapshot against
   the same indexed source and draft. Use `query_ontology` to retrieve
   concepts, their evidence, current answerability and explicit gaps.
   The existing separate `concepts review` command remains trusted-local and
   is **not** exposed through the agent MCP tools.

### Example CLI calls

```bash
excession concepts ontology-list \
  --repo "$REPO" --rev "$COMMIT" --index "$INDEX_JSON" \
  --search concepts > paths.json

excession concepts ontology-context \
  --repo "$REPO" --rev "$COMMIT" --index "$INDEX_JSON" \
  --path core/src/concepts_scip.rs --first-line 1 > context.json

excession concepts ontology-preview \
  --repo "$REPO" --rev "$COMMIT" --index "$INDEX_JSON" \
  --draft ontology-proposal.json > ontology-preview.json

excession concepts ontology-publish \
  --repo "$REPO" --rev "$COMMIT" --index "$INDEX_JSON" \
  --draft ontology-proposal.json --db excession.sqlite

excession concepts ontology-audit \
  --repo "$REPO" --rev "$COMMIT" --index "$INDEX_JSON" \
  --draft ontology-proposal.json --db excession.sqlite \
  --snapshot "$SNAPSHOT_ID"
```

For a native SCIP index generated inside a nested Rust crate, include
`--source-prefix core`, as with existing SCIP commands. Paths in proposals
must be full Git paths.

The proposal contract is at
`contracts/concepts/ontology-proposal.v1.schema.json`. The following is a
**structural example, not valid provenance**: the IDs are placeholders that
must be replaced with the exact values returned by `ontology-context`.

```json
{
  "schema": "excession.concepts-ontology-proposal.v1",
  "source_commit": "0000000000000000000000000000000000000000",
  "source_tree": "0000000000000000000000000000000000000000",
  "batch_id": "0000000000000000000000000000000000000000000000000000000000000000",
  "author_label": "unverified-agent-draft",
  "concepts": [
    {
      "key": "reference-normalization",
      "name": "SCIP source normalization",
      "definition": "Converts uniquely resolved SCIP references into source-bound candidate observations.",
      "rationale": "The authored definition is based on inspected source, not on a reciprocal-reference heuristic.",
      "members": [
        {
          "path": "core/src/concepts_scip.rs",
          "observations": ["0000000000000000000000000000000000000000000000000000000000000000"]
        }
      ]
    }
  ]
}
```

`relations` is optional. Initially only `POSSIBLY_SAME_RESPONSIBILITY`
and `SHARES_INTERFACE` can be proposed between two indexed source paths;
both require direct cited SCIP observations. These remain untested hypotheses.
The Rust author does **not** infer `ALIAS_OF`, `CONTRADICTS`, `IS_A`,
`PART_OF`, executable call edges, behavioral equivalence, or API
compatibility from native symbol references.

## MCP host setup

Local stdio MCP clients can launch Excession as one process:

```sh
excession mcp serve --repo "$REPO" --rev "$COMMIT" \
  --index "$INDEX_JSON" --db excession.sqlite
```

Append `--source-prefix core` for a nested index.
Append `--allow-publish true` **only** for an explicitly trusted local host
that may write unreviewed proposals. The default is read-only.

The server implements JSON-RPC/MCP `initialize`, `ping`,
`tools/list`, and `tools/call`. It speaks one JSON object per stdin/stdout
line, produces no general-purpose network listener, and exposes exactly the
curated tools: `inspect_code_evidence`, `ontology_schema`,
`draft_ontology`, `audit_ontology`, `query_ontology`,
`compare_native_baseline`, plus `publish_ontology` when explicitly
enabled. Tool results include JSON `structuredContent` and plain text
fallbacks. This is intended for LLM clients that support launching a local
MCP stdio server; it is **not** automatically connected to any ChatGPT
conversation or remote MCP client. An external compatible host is responsible
for registering/launching the server and assigning user permissions.

## Authority and failure boundaries

- **Source**: pinned Git commit/tree, exact tracked file OID, official native
  SCIP decoded index. No source edits or indexer execution are hidden.
- **Author**: `author_label` is an unverified string. It does not authenticate
  which human or model supplied the draft.
- **Evidence**: the agent may only cite existing normalized SCIP
  `observation_id` values. Rust checks each cited observation refers to each
  asserted member/source path. SCIP references support investigation, not
  the architectural conclusion itself.
- **Meaning**: every authored claim is `PROPOSED` and `UNTESTED`.
  Every concept is `UNREVIEWED`. Any later trusted review is scoped to
  an immutable concept revision, not a blanket proof.
- **Storage**: versioned immutable model and manifest, written to the
  existing SQLite concept tables. A differing Git/index/draft replay fails.
- **Limits**: 256 KiB draft input, at most 32 concepts, 32 source members
  each and 64 proposed relations. Source excerpts, observation results,
  and request frames are separately capped.
- **Coverage**: always partial. Unknown SCIP or compiler relationships are
  not treated as absent. Indexer completeness and the accuracy of the LLM's
  interpretation are not established by publication.

This follows the **propose → inspect/revise → certify separately** separation
in NVIDIA Auto Ontology, referenced at
`docs/concepts/NVIDIA_AUTO_ONTOLOGY_REFERENCE.md`, without copying its
Python service, LLM stack or separate storage architecture.
