# Pre-commit checks for Windows (same gates as scripts/check.sh).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
#
# Requires the MSVC Rust toolchain. Git for Windows / bash is not required.
# e2e needs a live IronForge and is not part of this script. cargo-audit is
# optional; the step is skipped with a warning when it is not installed.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function Step([string]$Name) {
    Write-Host ""
    Write-Host "==> $Name" -ForegroundColor Cyan
}

function Assert-LastExit([string]$What) {
    if ($LASTEXITCODE -ne 0) {
        throw "$What failed with exit code $LASTEXITCODE"
    }
}

function Get-FileHashes([string]$Root) {
    $map = @{}
    Get-ChildItem -LiteralPath $Root -Recurse -File | ForEach-Object {
        $relative = $_.FullName.Substring($Root.Length).TrimStart('\', '/')
        $map[$relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
    }
    return $map
}

Step 'cargo fmt --check'
cargo fmt --all --check
Assert-LastExit 'cargo fmt --all --check'

Step 'cargo clippy'
cargo clippy --workspace --all-targets --locked -- -D warnings
Assert-LastExit 'cargo clippy'

Step 'man pages are generated'
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("ifg-man-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    cargo run -q -p ifg --example gen_man -- $tmp | Out-Null
    Assert-LastExit 'gen_man'
    $expected = Get-FileHashes (Join-Path (Get-Location).Path 'man')
    $actual = Get-FileHashes $tmp
    $problems = @()
    foreach ($name in $expected.Keys) {
        if (-not $actual.ContainsKey($name)) {
            $problems += "missing: $name"
        } elseif ($actual[$name] -ne $expected[$name]) {
            $problems += "changed: $name"
        }
    }
    foreach ($name in $actual.Keys) {
        if (-not $expected.ContainsKey($name)) {
            $problems += "extra: $name"
        }
    }
    if ($problems.Count -gt 0) {
        $problems | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
        throw 'man/ is out of date; run scripts/gen-man.sh'
    }
} finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force
}

Step 'cargo test'
cargo test --workspace --locked
Assert-LastExit 'cargo test'

Step 'cargo audit'
if (Get-Command cargo-audit -ErrorAction SilentlyContinue) {
    cargo audit
    Assert-LastExit 'cargo audit'
} else {
    Write-Warning 'cargo-audit not installed; skipped (cargo install cargo-audit --locked)'
}

Step 'ifg-client dependency boundary'
$tree = cargo tree -p ifg-client -e normal --prefix none --locked
Assert-LastExit 'cargo tree'
$violations = $tree | Where-Object { $_ -match '^(clap|comfy-table|toml|keyring|rpassword) ' }
if ($violations) {
    $violations | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
    throw 'ifg-client depends on CLI-only crates'
}

Step 'all checks passed'
