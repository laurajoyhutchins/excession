# Contributing to Excession

Excession is a Git-grounded, auditable code-intelligence CLI. Contributions should improve demonstrable correctness and usability, not create parallel evidence authorities.

## Start

Read [README](README.md), [development setup](docs/DEVELOPMENT.md), [contracts](contracts/README.md) and [product onboarding plan](docs/product/10-contributor-onboarding.md).

Use `mise install && mise run setup && mise run check`, or invoke the underlying Rust/Python/Node checks directly. Dependencies should be installed from the repo's pinned version/lock files. Ordinary local verification should remain focused; run expensive external fixture, native compiler or cross-platform suites when specifically relevant.

## Pull requests

- State the user-visible behavior and exact semantic/evidence claim.
- Add a failing or explicitly skipped future test before implementation. **Never count skipped tests as passed behavior.**
- Preserve Git source authority, deterministic receipts, typed coverage gaps and independent replay. Avoid duplicating language-native semantics already provided by SCIP/Clang/native tools.
- Include independent positive/negative fixtures, measured costs for performance changes and a precise unsupported list.
- Keep changes small and based on current main. Avoid merging old feature stacks wholesale; see issue #60.

## Experimental work

Concept intelligence and ontology work in PRs #88–#95 is currently proposal scaffolding. An ontology concept does not establish an import, function call, runtime dependency, vulnerability or safe rewrite.

See [SECURITY.md](SECURITY.md) for security disclosure.
