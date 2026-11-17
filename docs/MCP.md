> **Current MCP transport is an evidence interface, not yet a
> persistent investigation engine.** Agents should use its six existing
> read-only tools to gather source-bound evidence and build their own
> question-led explanations. The proposed
> [investigation/transform/verify workflow](PRODUCT.md) does not add
> unimplemented MCP tool names. See [investigations](INVESTIGATIONS.md)
> and [verification](VERIFICATION.md).

# Coding-agent integration

This is the **local stdio integration** for an agent that can launch processes
inside a trusted checkout. For ChatGPT conversations using the GitHub connector,
use the [GitHub-native analysis request workflow](GITHUB_NATIVE_ANALYSIS.md)
instead. It executes on GitHub-hosted runners and delivers evidence through
workflow artifacts, with no remote MCP service to register.


Excession exposes a repository-scoped, read-only MCP server over standard
input/output using the [official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)
(`rmcp` 3.10.0). MCP is an adapter over the same Rust Git/SQLite analysis used by
`excession agent locate`, `agent context`, `impact`, `agent verify`,
`analyze`, and `tests`. It does not use a second language parser or a
separate semantic authority.

## Install

Install a release that includes the `excession mcp` subcommand or build
a source revision containing the official SDK integration. The original
v0.2.0 archive predates MCP; do not configure it as an MCP server.
Check `excession doctor` before using native language adapters.

In the target repository, inspect the proposed configuration:

```sh
excession agent init
```

Then explicitly create missing files:

```sh
excession agent init --write
```

Use `--client codex` or `--client claude` to target one client, or omit
`--client` for both. The command creates project-level
`.codex/config.toml` (Codex, for trusted projects) and `.mcp.json`
(Claude Code). They launch `excession mcp --repo /absolute/local/repo`.

**Existing configuration is never overwritten.** If either file exists,
`agent init` reports `EXISTING_CONFIG_PRESERVED` and prints the proposed
entry for a manual merge. An existing `AGENTS.md` or `CLAUDE.md` is
always left untouched. The generated paths are machine-specific; review
these local files before committing them. Run `agent init` again if you
move or clone the repository elsewhere. The executable must be reachable
on PATH for the client.

To run the server without agent-client configuration:

```sh
excession mcp --repo /path/to/repository
```

The official SDK handles MCP 2026-07-28 discovery/negotiation, older
initialize-based clients, stdio framing, JSON-RPC errors, and typed tool
schemas. Debug output must never be written to stdout.

## Exposed tools

| MCP tool | Existing computation | Notes |
| --- | --- | --- |
| `search_code` | `agent locate` | Git-pinned lexical matches plus bounded context; index coverage may be partial |
| `get_context` | `agent context` | Requires 1–16 exact tracked paths |
| `explain_impact` | `impact` | Possible impact between two Git revisions |
| `verify_change` | `agent verify` | Requires two distinct retained commits; never authorizes merge |
| `plan_tests` | `tests` | Plan only; no project code execution |
| `check_coverage` | `analyze` | Git-pinned static relationship coverage and explicit unknowns |

Tool inputs deliberately contain **no repository parameter**. Each server
is confined to the repository selected when the MCP process was started.
Clients cannot change it through a tool call. `--roots` and `--db`
may be set at server startup when needed; ordinary agents should not need
to supply them.

Use `search_code` with the task description, then `get_context` for
interesting paths. Use `check_coverage` before interpreting absent
relationships. Once a candidate is committed, call `explain_impact` and
`verify_change` with base and candidate commit IDs. `plan_tests` can
provide a no-execution plan beforehand.

## Safety and limitations

- MCP never edits source, runs compilers, starts tests, or approves merges.
  `plan_tests` only plans. Local cache writes, including the incremental
  lexical index, take place outside the checkout.
- The lookup, context, impact and verification tools read retained Git
  objects, **not** dirty working-tree contents. The existing
  `excession tests --json` workflow separately handles uncommitted changes.
  Do not present source evidence about HEAD as evidence about unstaged edits.
- Path names in context are exact Git paths, not arbitrary filesystem
  reads. Caller-supplied receipt file paths and shell execution are not
  exposed over MCP.
- Absent results, unsupported languages, truncated indexes and unverified
  native provenance remain explicit unknowns. A passing receipt is not a
  proof of correctness.
- The stdio server is **local**. ChatGPT cloud connections require a
  separately deployed, authenticated HTTP transport or proxy. No public
  endpoint or authorization is installed by `agent init`.
- The official SDK negotiates protocol versions. Excession exposes no
  subscriptions, shell-control methods or remote filesystem roots; its
  MCP transport remains local stdio. Git/SQLite analysis runs on blocking
  workers rather than blocking the protocol runtime.
- Tool results larger than 4 MiB return a tool-level refusal. No
  arbitrary-shell or repository selection arguments are exposed.

## Acceptance

`cargo test --locked --manifest-path core/Cargo.toml --test mcp_cli`
exercises the real binary and a Git fixture through initialize, tool
discovery, successful Git source lookup, and refused operations.
Unit tests cover protocol errors, bounds and create-only onboarding.

The end-to-end product test remains an agent-on-repository benchmark:
compare agents with and without the MCP tools on previously unseen
repositories, measuring correct source discovery, task success, latency,
cost and independent verification. Passing MCP protocol tests alone does
not establish those gains.
