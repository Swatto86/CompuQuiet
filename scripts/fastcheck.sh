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
  # File-size guideline (the agent-standards engineering skill): a code file over 400 lines needs a
  # reason on record or a split; files already over it are listed in scripts/file-size-baseline.txt
  # and may not grow.
  echo "== file size =="
  size_check="$HOME/.agents/scripts/check-file-size.ps1"
  if [ -f "$size_check" ] && command -v pwsh >/dev/null 2>&1; then
    pwsh -NoProfile -File "$size_check" -Root "$root"
  else
    echo "skip - file size check: ~/.agents/scripts/check-file-size.ps1 or pwsh not found"
  fi
  echo "== clippy (workspace) =="
  cargo clippy --locked --workspace --all-targets --features compuquiet/fake-platform -- -D warnings
  echo "== frontend types =="
  npx --no-install tsc --noEmit
  npx --no-install tsc --noEmit -p e2e
fi
printf '\nFAST CHECK PASSED\n'
