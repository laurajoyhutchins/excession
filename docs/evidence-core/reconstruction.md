# Automatic C reconstruction (bounded)

`excession reconstruct auto` is an evidence-guided synthesis loop over one
C ABI function `int name(int x)`. It is intentionally a bounded reference engine,
not a general-purpose decompiler or a recovery guarantee.

The tool reads an ELF relocatable object's .text section once, discovers
the unique named text symbol with `nm`, gathers disassembly cues using
`objdump`, generates expressions using a fixed versioned C grammar, and
compiles them using the existing pinned C object recipe. Each compiled candidate
is compared to the pinned target's .text bytes, with early termination on exact
text agreement. The target binary is never executed.

```sh
cc -O2 -fno-stack-protector -fno-asynchronous-unwind-tables -fno-ident \
  -c -o unknown.o original.c
excession reconstruct auto \
  --db evidence.sqlite --target unknown.o --out reconstructed.c \
  --max-candidates 64 --time-budget-seconds 30 --trusted-local true
excession reconstruct searches --db evidence.sqlite --target unknown.o
```

The generated C source is the best candidate **in the enumerated grammar**, which
is not necessarily the original source or semantically equivalent. When no exact
match is found it still writes a candidate, but reports
`CANDIDATE_BUDGET_EXHAUSTED` or `TIME_BUDGET_EXHAUSTED`. Search completeness is
never asserted, even when the selected candidates were all tried.

Candidate code is strictly synthesized from a fixed expression grammar, with no
header imports or user-defined instructions. Nevertheless the system compiler,
`nm`, `objcopy`, and `objdump` run directly on the host without a
sandbox. The explicit `--trusted-local true` gate means only trusted artifacts
and tools should be used. Input sizes, candidate counts, and tool timeouts are
bounded. An existing output path is never overwritten.

This first increment intentionally **does not** run candidate objects or the
target executable. Matching .text bytes does not cover symbol and relocation
tables, external calls, global data, ABI behavior, or complete binary identity.
The immutable SQLite search receipt records all candidate expressions, their
machine-code digests, bounded mismatch samples, and uncertainty. It shares
the canonical artifact identity and REA dossier, but it does not claim semantic
equivalence, independently attested build execution, or complete REA coverage.

Efficiencies: one-time target extraction; one-time disassembly; observed-constant
and operation prioritization; deduplication before compilation; per-run bounded
results; earliest possible exact-match termination. This does **not** yet cache
compiler results across runs or infer arbitrary control flow, types, or data.

## Reuse before synthesis

The search now uses an indexed, immutable SQLite catalog of previously successful
`EXACT_TEXT_MATCH` receipts. If a new object file has the same
`.text` digest, Excession retrieves the prior grammar expression and places
it at the front of the candidate queue. The target function may have a different
name or a different complete object-file identity. The proposed source is
regenerated with the **current** target name, and Excession compiles and
compares it again; previously stored evidence is never treated as proof
that the new source compiled correctly.

The report distinguishes `candidate_selection=REUSED_EXACT_TEXT` and
`GENERATED_GRAMMAR` and includes the content-audited originating
`reused_from_search_id` when applicable. Retrieval refuses modified
or inconsistent saved source receipts. An exact code-section match alone
is not a claim about relocation semantics, ABI, types, or program behavior.

This first indexed retrieval works across existing search reports in the same
SQLite store. Future function recognition should use upstream Ghidra Function ID,
BSim, and BinDiff producers, not another homegrown fuzzy signature engine.
