# 在 Windows 真机上跑一整套校验:fmt 之外的 test / clippy / release build。
# 由 scripts/win-sync.sh 上传并调用;也可以单独在 PowerShell 里直接跑。
$ErrorActionPreference = 'Continue'
# 中文输出必须 UTF-8,否则 SSH 那头看到的全是乱码。
$null = & cmd /c 'chcp 65001 >nul'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
# 增量构建产物放 D 盘之外无所谓,但统一放 C:\dev\target 以免被 tar 覆盖。
$env:CARGO_TARGET_DIR = 'C:\dev\target'
Set-Location C:\dev\nex_squid

$fail = 0
foreach ($step in @(
    @{ n = 'cargo test';    a = @('test') },
    @{ n = 'cargo clippy';  a = @('clippy', '--all-targets', '--', '-D', 'warnings') },
    @{ n = 'cargo build';   a = @('build', '--release', '-p', 'squid-cli') }
)) {
    Write-Output "`n==> $($step.n)"
    & cargo @($step.a)
    if ($LASTEXITCODE -ne 0) { Write-Output "!!! $($step.n) FAILED (exit $LASTEXITCODE)"; $fail = 1 }
}
if ($fail) { Write-Output "`nRESULT: FAIL" ; exit 1 }
Write-Output "`nRESULT: OK"
