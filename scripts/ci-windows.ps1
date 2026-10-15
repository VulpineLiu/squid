# Build and test squid on a real Windows machine (runner or dev box).
# Usage:  pwsh -File scripts/ci-windows.ps1 [-Static]
param(
    # Statically link the CRT so the exe needs no VC++ redistributable.
    [switch]$Static
)

$ErrorActionPreference = 'Stop'

if ($Static) {
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
}

rustup target add x86_64-pc-windows-msvc | Out-Null
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p squid-cli --target x86_64-pc-windows-msvc

$exe = "target\x86_64-pc-windows-msvc\release\squid.exe"
if (-not (Test-Path $exe)) { throw "missing $exe" }
& $exe --version
Write-Host "windows CI OK: $((Get-Item $exe).Length) bytes" -ForegroundColor Green
