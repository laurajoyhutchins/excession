# Microbenchmarks: single-query retrieval and projection costs

These historical calibration exercises live below the primary relational experiments. They do **not** measure agent task completion, independent graph completeness, or reusable multi-revision relationships.

## Exact file locations

On the pinned h11 Git fixture, compare `rg --files`, `git ls-tree`,
`excession view graph`, and `excession view paths --role focus`.
All arms receive four exact module names and return source path sets.
Report literal stdout bytes, child CPU, wall time, process exit status and
agreement with four independently declared file paths. A match is not proof
of completeness for general module resolution.

Run the isolated workflow or, after building the binary and indexing h11:

```sh
python3 benchmarks/micro/locate.py \
  --repo /path/to/clean/h11 --db /tmp/h11-index.sqlite \
  --bin ./core/target/debug/excession \
  --tasks benchmarks/calibration-h11.json --name h11 \
  --out /tmp/h11-file-retrieval
```

The Git arm enumerates the `h11` tree and filters after the timer; the
`rg` arm uses a filename glob over the clean working tree at HEAD.
Their raw outputs and different setup costs must be interpreted separately.

## Symbol definitions

`benchmarks/micro/symbol_compare.py` uses a tiny tracked C fixture containing one
definition and one call. A real LSP JSON-RPC `textDocument/definition`
request is compared to fixed literal `rg` and `git grep` searches.
Report returned location count, whether the explicit definition appears,
initialization time, definition-request time, wire bytes and CPU. The LSP
client uses UTF-16 offsets as specified by the language server protocol.

```sh
python3 benchmarks/micro/symbol_compare.py --repo . --out /tmp/symbol-probe
```

An absent `clangd` server is recorded as unsupported; it is never treated as
zero-time success. The LSP CPU measurement includes shutdown, while its
wall-time sample ends when the definition response arrives. Those are different
measurement windows. Excession's current module views do not resolve C symbols.

**Do not pool file lookup, graph closure, and symbol definition into a single
winner score.** This small C test is a measurement-protocol smoke, not
representative evidence for large codebases. No human grading is used.
