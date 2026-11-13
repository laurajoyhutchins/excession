# Compiler-in-the-loop reconstruction

This is an **experimental, trusted-local C object-code feedback loop**, not a source-code
recovery claim. It builds on the native artifact evidence store and typed artifact identities.

## Try it

Compile a known target into an ELF object with the same compiler and flags:

```sh
cc -O2 -fno-stack-protector -fno-asynchronous-unwind-tables -fno-ident \
  -c -o target.o original.c
excession reconstruct evaluate --db evidence.sqlite --target target.o \
  --candidate hypothesis.c --trusted-local true
excession reconstruct history --db evidence.sqlite --target target.o
```

Use the mismatch offsets and byte counts to revise `hypothesis.c` and evaluate again.
The history is append-only and scoped to the original target's SHA-256 and length.
Matching code bytes only say the extracted `.text` bytes agree.
The comparison does **not** currently cover relocations, symbols, section layout,
data, ABI, branch targets, runtime behavior, complete binaries, or source originality.

The compiler and objcopy commands execute on the **host**, not in a sandbox.
Use this only for trusted candidates and known tools. The explicit `--trusted-local true`
gate prevents automatic execution by callers that have not opted in.
The recipe is fixed for the first C-object fixture; no arbitrary flags or shell
commands are accepted. It stores report metadata and digests in the same SQLite
file as REA evidence. It does not store complete source or machine code.
REA bundle counts are joined by the same typed artifact identity; native
REA predicates are not yet compared across candidate and target.

The report is an observation by Excession's local process, not independently
attested build execution. No provenance, semantic equivalence, or recovered
source is asserted. Artifact/REA, compiler tooling, and candidate isolation
have further acceptance work.
