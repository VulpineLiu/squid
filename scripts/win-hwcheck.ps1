# 真机验收脚本:只读检查 + 一个自建的回收站往返实验,不碰用户真实数据。
# 由 scripts/win-sync.sh 同步后单独执行:
#   ssh KANGWJ@host powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\kangwj\win-hwcheck.ps1
$ErrorActionPreference = 'Continue'
$null = & cmd /c 'chcp 65001 >nul'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_DIR = 'C:\dev\target'
$squid = 'C:\dev\target\release\squid.exe'
$scratch = Join-Path $env:USERPROFILE 'squid-rb-test'
$fails = @()
function Step($name) { Write-Output "`n=== $name ===" }
function Fail($m) { $script:fails += $m; Write-Output "!!! FAIL: $m" }
function Ok($m)     { Write-Output "ok  : $m" }

if (-not (Test-Path $squid)) { Write-Output "missing $squid"; exit 1 }

Step 'doctor'
& $squid doctor --lang en
Step 'status (json)'
$status = & $squid status --json | ConvertFrom-Json
Ok ("os={0} host={1} disks={2}" -f $status.os, $status.hostname, $status.disks.Count)

Step 'installed apps vs registry'
$apps = & $squid uninstall --list --json | ConvertFrom-Json
$reg = @()
foreach ($hive in 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall',
                  'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
                  'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall') {
    $reg += Get-ChildItem $hive -ErrorAction SilentlyContinue | ForEach-Object {
        $p = Get-ItemProperty $_.PSPath
        if ($p.DisplayName -and -not $p.SystemComponent) { $p.DisplayName }
    }
}
$regNames = $reg | Sort-Object -Unique
Ok ("squid={0} registry={1}" -f $apps.Count, $regNames.Count)
$missing = $regNames | Where-Object { $apps.name -notcontains $_ }
if ($missing) { Write-Output ("missing in squid: " + ($missing -join ' | ')) }

Step 'clean --dry-run (safety invariant)'
$report = & $squid clean --dry-run --json | ConvertFrom-Json
Ok ("items={0} freed={1} unreadable={2}" -f $report.items.Count, $report.freed_on_disk.human, $report.unreadable)
$report.items | Group-Object category | Sort-Object Count -Descending | ForEach-Object {
    Write-Output ("  {0,-14} {1}" -f $_.Name, $_.Count)
}
# 核心安全不变量:清理项绝不能落在系统目录里。
$forbidden = @('C:\Windows', 'C:\Program Files', 'C:\Program Files (x86)', '$Recycle.Bin')
$bad = $report.items | Where-Object {
    $p = $_.path.ToLower()
    foreach ($f in $forbidden) { if ($p.StartsWith($f.ToLower())) { return $true } }
    return $false
}
if ($bad) { Fail ("clean offered {0} system-dir item(s): " -f $bad.Count + (($bad | Select-Object -First 3).path -join ' | ')) }
else { Ok 'no clean item inside system directories' }

Step 'recycle-bin round trip'
Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
$pkg = Join-Path $scratch 'node_modules\pkg'
New-Item -ItemType Directory -Force -Path $pkg | Out-Null
[IO.File]::WriteAllBytes((Join-Path $pkg 'blob.bin'), (New-Object byte[] 2MB))
Ok ("fixture created: $scratch (2 MB)")
$before = (New-Object -ComObject Shell.Application).Namespace(0xA).Items().Count
& $squid purge $scratch --yes --trash --json
$after = (New-Object -ComObject Shell.Application).Namespace(0xA).Items().Count
if (Test-Path $scratch) { Fail 'scratch folder still on disk after trash' } else { Ok 'scratch folder left the disk' }
if ($after -le $before) { Fail "recycle bin did not grow ($before -> $after)" } else { Ok "recycle bin grew $before -> $after" }
$names = (New-Object -ComObject Shell.Application).Namespace(0xA).Items() | ForEach-Object { $_.Name }
if ($names -match 'squid-rb-test') { Ok 'original name recoverable from recycle bin' } else { Fail 'fixture not found in recycle bin by name' }
& $squid undo 2>&1 | Select-Object -First 4

Step 'result'
if ($fails.Count) { Write-Output ("FAILURES: " + $fails.Count); $fails | ForEach-Object { Write-Output " - $_" }; exit 1 }
Write-Output 'ALL CHECKS PASSED'
