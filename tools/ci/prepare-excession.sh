#!/usr/bin/env bash
# Prepare the actual Excession CLI and locked TypeScript sidecar on GitHub Actions.
# The source is either the workflow checkout or the independently pinned
# Excession 'tool' checkout used by the reusable evidence workflow.
set -euo pipefail

source_root="${1:-.}"
profile="${2:-release}"
case "$source_root" in
  .|tool) ;;
  *) echo "unsupported Excession tool checkout: $source_root" >&2; exit 2 ;;
esac
case "$profile" in
  release|debug) ;;
  *) echo "unsupported build profile: $profile" >&2; exit 2 ;;
esac
if [[ ! -f "$source_root/core/Cargo.lock" || ! -f "$source_root/core/Cargo.toml" ]]; then
  echo "Pinned Excession Cargo manifest or lockfile missing" >&2
  exit 2
fi
test -f "$source_root/package-lock.json" || {
  echo "locked TypeScript sidecar missing" >&2
  exit 2
}

rustup toolchain install 1.99.1 --profile minimal
cargo +1.99.1 fetch --locked --manifest-path "$source_root/core/Cargo.toml"
if [[ "$profile" == release ]]; then
  cargo +1.99.1 build --offline --locked --release --manifest-path "$source_root/core/Cargo.toml" --bin excession
else
  cargo +1.99.1 build --offline --locked --manifest-path "$source_root/core/Cargo.toml" --bin excession
fi
npm ci --prefix "$source_root" --ignore-scripts --no-audit --no-fund

binary="$source_root/core/target/$profile/excession"
test -x "$binary" || { echo "actual Rust CLI was not built" >&2; exit 2; }
test -f "$source_root/node_modules/typescript/lib/typescript.js" || {
  echo "actual locked TypeScript sidecar missing" >&2
  exit 2
}
"$binary" --version
