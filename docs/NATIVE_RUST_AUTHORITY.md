# Rust-owned native dependency resolution

`excession evidence discover-native` moves revision selection, full snapshot
staging, source validation, byte and file bounds, toolchain identity, output
validation and input-declaration construction to `core/src/native_discovery.rs`.
`tools/native_language_adapter.py` remains a bounded **language-native resolver**:
it calls `go list`, the installed TypeScript compiler API, or `rustc` and
reports source and configuration paths. It does not read Git or issue claims.

```bash
cargo build --offline --manifest-path core/Cargo.toml --bin excession
excession evidence discover-native \
  --repo /checkout --rev COMMIT_SHA1 --entry src/lib.rs \
  --language rust --trusted-local true > declaration-and-stats.json
```

The CLI response includes `declaration` (the existing
`excession.input-declaration.v1` JSON object) and `stats` (staged and declared
file counts and bytes, commit, wall-clock resolution time). Use the **nested
`declaration`**, not the envelope, for `excession evidence contract-plan`.
The original `tools/excession_native.py` command and `discover()` Python API
remain a thin compatibility wrapper returning the declaration directly, plus
optional `--stats` output, so existing benchmark and test callers can migrate
without changing the contract schema.

## Trust and unsupported conditions

- Local Git SHA-1 objects are authoritative. The clean working tree is never a
  dependency, so historical commits can be inspected without a checkout.
- A temporary private snapshot is reconstructed from **regular, tracked Git
  blobs**. Symlinks and gitlinks are refused rather than resolved on the host.
- Stage limits: 4,096 files / 96 MiB. Declared input limits: 512 files / 32 MiB.
  Native resolver processes have bounded output and a timeout. The staged
  snapshot is audited after the resolver returns; added, deleted or modified
  source files cause refusal.
- `--trusted-local true` is compulsory. A compiler or language server is **not
  a sandbox** for hostile source. Compilers may launch other processes or parse
  arbitrary source content. This mode is not remote attestation.
- Toolchain executable bytes, version, Python adapter executable bytes/version,
  effective TypeScript compiler API hash and relevant language options are
  recorded as explicit assumptions. This **does not** fully capture system
  libraries, toolchain sysroots, ambient environment, or runtime effects.
- Go still refuses unsupported local/external package resolution and cgo;
  TypeScript refuses ambiguous or dynamic resolution; Rust refuses unsupported
  workspaces, build scripts, dynamic compile-time environments and other
  unresolved dependencies. No successful declaration implies completeness of
  execution dependencies or behavior verification.

Existing manifests and SQLite transport records are unchanged. Rust remains the
only authority that turns declared paths into Git-backed contracts and audits
historical comparisons. `DECLARED_INPUTS_ONLY` remains the coverage marker.

## Test coverage

`tests/test_native_rust_authority.py` exercises Go transitive source discovery
against dirty working trees, TypeScript path aliases and dynamic-import
refusal, Rust `include_str!` dependency tracking, symlink refusal, and trust
opt-in. Existing native-resolver tests continue to use the Python compatibility
API. Run with `make check`; optional Go and TypeScript cases skip when their
locally installed native toolchains are unavailable.
