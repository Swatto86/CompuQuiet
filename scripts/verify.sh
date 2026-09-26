#!/usr/bin/env bash
# The full gate. One definition for every platform; verify.ps1 is a shim that
# runs this file through Git for Windows' Bash.
#
# Cheapest checks first, so a formatting slip fails in seconds rather than
# after a WebDriver run. Nothing here packages an installer: that is the
# release step (`--package`, gated on AGENT_RELEASE=1).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

package=0
[ "${1:-}" = "--package" ] && package=1

say() { printf '\n== %s ==\n' "$1"; }

say "pre-push hook"
bash "$root/scripts/install-hooks.sh"

say "rust fmt"
cargo fmt --all -- --check

say "clippy"
# The fake platform is compiled here so its code and the engine tests behind
# it are linted too; the release build never enables it (checked below).
cargo clippy --locked --workspace --all-targets --features compuquiet/fake-platform -- -D warnings

say "rust tests"
cargo test --locked --workspace --all-targets --features compuquiet/fake-platform

say "test-only code stays out of release builds"
grep -qE '^default = \[\]$' src-tauri/Cargo.toml || { echo "src-tauri default features must be empty"; exit 1; }
grep -qE '^fake = \[\]$' crates/cq-platform/Cargo.toml || { echo "cq-platform's fake feature must not be default"; exit 1; }
if grep -q 'fake-platform' src-tauri/tauri.conf.json; then echo "tauri.conf.json must not enable fake-platform"; exit 1; fi
echo "release features OK"

say "frontend types"
npm run --silent build

say "frontend formatting"
npx --no-install prettier --check "ui/src/**/*.ts" "e2e/**/*.ts"

say "frontend assets"
dist="ui/dist"
index="$dist/index.html"
[ -f "$index" ] || { echo "no built frontend at $index"; exit 1; }
referenced=$(grep -oE '(src|href)="[^"]+"' "$index" | sed -E 's/.*="([^"]+)".*/\1/' | grep -E '^/?assets/' || true)
[ -n "$referenced" ] || { echo "index.html references no bundled assets"; exit 1; }
count=0
while IFS= read -r asset; do
  [ -n "$asset" ] || continue
  path="$dist/${asset#/}"
  [ -f "$path" ] || { echo "index.html references $asset, which is not on disk"; exit 1; }
  count=$((count + 1))
done <<< "$referenced"
echo "frontend assets OK ($count referenced, all present)"

say "frontend tests"
npm test --silent

say "version agreement"
cargo_version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
node scripts/check-version-agreement.mjs "$cargo_version"
echo "version agreement OK ($cargo_version)"

say "script permissions"
notexec=0
while IFS= read -r entry; do
  mode=${entry%% *}
  file=${entry#*$'\t'}
  if [ "$mode" != "100755" ]; then
    echo "  not executable in git: $file (git update-index --chmod=+x '$file')"
    notexec=$((notexec + 1))
  fi
done < <(git ls-files -s -- '*.sh')
[ "$notexec" -eq 0 ] || exit 1
echo "script permissions OK"

say "file sizes"
# 400 lines is the hard cap for Rust and TypeScript sources. awk's NR counts an
# unterminated final line too, which `wc -l` does not.
hard=400
over=0
while IFS= read -r file; do
  case "$file" in
    ./target/*|./node_modules/*|./ui/dist/*|./.git/*|./src-tauri/gen/*) continue ;;
  esac
  lines=$(awk 'END { print NR }' "$file")
  if [ "$lines" -gt "$hard" ]; then
    echo "  OVER HARD CAP ($hard): $file — $lines lines"
    over=$((over + 1))
  fi
done < <(find . -type f \( -name '*.rs' -o -name '*.ts' \))
if [ "$over" -gt 0 ]; then
  echo ""
  echo "Split these by responsibility before committing."
  exit 1
fi
echo "file sizes OK (none over $hard lines)"

say "debug application (fake platform, no bundle)"
npx --no-install tauri build --debug --no-bundle --features fake-platform --config e2e/tauri.conf.json

case "$(uname -s)" in
  Darwin) echo "WebDriver is unavailable on macOS; Windows and Linux run the suite." ;;
  MINGW*|MSYS*|CYGWIN*)
    say "native WebDriver acceptance"
    export PATH="$root/.webdriver:$PATH"
    if [ "${GITHUB_ACTIONS:-}" = "true" ]; then
      powershell.exe -NoProfile -NonInteractive -File e2e/windows-ci.ps1
    else
      npm run --silent e2e:run
    fi
    ;;
  *)
    say "native WebDriver acceptance"
    export PATH="$root/.webdriver:$PATH"
    if [ -z "${DISPLAY:-}" ] && command -v xvfb-run >/dev/null 2>&1; then
      xvfb-run -a dbus-run-session -- npm run --silent e2e:run
    else
      npm run --silent e2e:run
    fi
    ;;
esac

if [ "$package" = "1" ]; then
  [ "${AGENT_RELEASE:-}" = "1" ] || {
    echo "Packaging is a separate release/handoff step; set AGENT_RELEASE=1 after debug verification."
    exit 1
  }
  say "release packaging"
  npx --no-install tauri build
fi

printf '\nALL CHECKS PASSED\n'
