<#
.SYNOPSIS
  The inner loop: formatting, types, and clippy for the whole workspace, or a
  type check alone for one crate. Never packages, never runs WebDriver.
#>
[CmdletBinding()]
param(
    # Scope to one crate: `cargo check` only, no clippy or tests.
    [string]$Package
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    Write-Host '== rust fmt ==' -ForegroundColor Cyan
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt found unformatted files (run: cargo fmt --all)' }

    if ($Package) {
        Write-Host "== cargo check -p $Package ==" -ForegroundColor Cyan
        cargo check --locked -p $Package --all-targets --features compuquiet/fake-platform
        if ($LASTEXITCODE -ne 0) { throw "cargo check failed for $Package" }
    }
    else {
        Write-Host '== clippy (workspace) ==' -ForegroundColor Cyan
        cargo clippy --locked --workspace --all-targets --features compuquiet/fake-platform -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'clippy failed' }

        Write-Host '== frontend types ==' -ForegroundColor Cyan
        npx --no-install tsc --noEmit
        if ($LASTEXITCODE -ne 0) { throw 'tsc failed' }
    }
    Write-Host "`nFAST CHECK PASSED" -ForegroundColor Green
}
finally {
    Pop-Location
}
