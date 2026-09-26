#!/usr/bin/env bash
# The inner loop on Linux/macOS: formatting, types and clippy for the whole
# workspace, or `-p <crate>` for a type check of one crate. Never packages.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

package=""
if [ "${1:-}" = "-p" ]; then package="${2:?crate name}"; fi

echo "== rust fmt =="
cargo fmt --all -- --check

if [ -n "$package" ]; then
  echo "== cargo check -p $package =="
  cargo check --locked -p "$package" --all-targets --features compuquiet/fake-platform
else
  echo "== clippy (workspace) =="
  cargo clippy --locked --workspace --all-targets --features compuquiet/fake-platform -- -D warnings
  echo "== frontend types =="
  npx --no-install tsc --noEmit
fi
printf '\nFAST CHECK PASSED\n'
