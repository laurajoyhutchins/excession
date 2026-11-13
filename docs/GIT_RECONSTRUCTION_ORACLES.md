# Source-bound executable oracles for Git reconstruction

`excession reconstruct git-oracles` is the **trusted-local, unsandboxed** execution layer above
`reconstruct git` and `reconstruct git-evidence`. It does not replace either read-only verifier.

```sh
excession reconstruct git-oracles \
  --repo /absolute/path/to/git/repository \
  --plan /absolute/path/to/reconstruction-plan.json \
  --db /absolute/path/to/existing/evidence.sqlite \
  --evidence /absolute/path/to/build-rea-bindings.json \
  --checks /absolute/path/to/oracle-checks.json \
  --trusted-local true
```

The verifier **refuses** absent or false `--trusted-local`. Use only on software and independent
oracles that you trust to execute with the full permissions of your user. This is **not a
sandbox**. A timeout does not contain grandchild processes, network access, filesystem writes,
or other side effects of malicious source code. Do not run this on untrusted agent changes without
an external isolation boundary.

The oracle manifest uses `excession.git-reconstruction-oracles.v1` with bounded `checks`.
Each check declares its `phase` (`INTRODUCTION` or `INTEGRATION`), exact `checkpoint` commit,
`contracts`, exact clean checked-out `checkout` directory, absolute `program` executable path,
`oracle_source` file path, both SHA-256 hashes, `args`, `timeout_ms` (100..15000), and
`expected_stdout_sha256`. `args[0]` must be exactly `oracle_source` so a trusted interpreter
runs the independently stored script directly rather than accepting an ignored source marker.
For example, an oracle may be a Python script outside the target repository and any checkout,
invoked by an absolute interpreter path. Avoid scripts importing untrusted modules on an
unprotected machine.

Excession first verifies the entire declared Git history and every existing source-closed
build/REA evidence binding. Every obligation must be bound; missing build/REA evidence is a
refusal, not a silent opt-out. At a contract-introduction checkpoint, the check must name exactly
the newly introduced contracts. At a two-parent integration node, the check must name **all**
retained contracts together and execute from the exact merged commit. Each required group is
checked once. The manifest can neither omit a group nor add unbacked checks.

For each invocation, Excession checks the oracle and interpreter bytes, the clean checkout
HEAD and Git tree identity, the checked oracle stdout digest and zero exit status, and source
unchanged after the run. It limits oracle output to 64 KiB and kills a timed-out direct child.
It binds the observed output, original Git source-blob closure and independently replayed
build/REA report identity into append-only receipts using Excession's existing SQLite evidence
store. The receipt explicitly records `TRUSTED_LOCAL_UNSANDBOXED`, `NOT_ESTABLISHED` environment
attestation, and `NOT_ESTABLISHED` behavioral equivalence. Same inputs and output yield the same
receipt identity; elapsed wall time is deliberately excluded.

**Scope of the result:** The finite asserted test cases passed against the pinned source tree.
It does not prove that source dependencies are complete, that the oracle is independent or
correct, that the entire software system is equivalent, that the build was executed securely,
or that any Git history reflects the original development chronology. The complete source
closure and build/REA receipt prerequisites are checked, but their semantics remain bounded.

The checked local fixtures include a true two-parent merge of independent reader/writer
branches, separate checks at feature introduction, a joint reader/writer oracle at the merge,
and a public-interface oracle at the final stage. A writer mutation that remained structurally
valid failed the joint behavioral oracle. The merged shape is not inferred to be safe merely
because the Git object identities agree.
