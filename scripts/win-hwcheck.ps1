# 真机验收脚本:只读检查 + 一个自建的回收站往返实验,不碰用户真实数据。
# 注意:含中文的 .ps1 必须带 UTF-8 BOM,否则 Windows PowerShell 5.1 会按系统
# ANSI 代码页(中文机器上是 GBK)解码,注释里的字节会吞掉相邻的引号,整份脚本
# 报"字符串缺少终止符"。chcp 65001 只影响控制台,不影响脚本文件的解码。
# 由 scripts/win-sync.sh 同步后单独执行:
#   ssh KANGWJ@host cmd /c "chcp 65001 >nul & powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\kangwj\win-hwcheck.ps1"
$ErrorActionPreference = 'Continue'
$null = & cmd /c 'chcp 65001 >nul'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_DIR = 'C:\dev\target'
$squid = 'C:\dev\target\release\squid.exe'
# 每轮换一个名字:回收站里可能有上一轮的同名条目,用名字判断"这一轮真的进去了"才成立。
$run = 'squid-rb-{0}' -f (Get-Date -Format 'HHmmss')
$scratch = Join-Path $env:USERPROFILE $run
$fails = @()
function Step($name) { Write-Output "`n=== $name ===" }
function Fail($m) { $script:fails += $m; Write-Output "!!! FAIL: $m" }
function Ok($m)     { Write-Output "ok  : $m" }

if (-not (Test-Path $squid)) { Write-Output "missing $squid"; exit 1 }

Step 'doctor'
$doctor = & $squid doctor --lang en 2>&1 | Out-String
Write-Output $doctor
# doctor 不能谎报回收站:Windows 上没有 ~/.Trash,但回收站是可用的。
if ($doctor -match 'fall back to permanent') {
    Fail 'doctor 说没有回收站(Windows 上应该是 Recycle Bin)'
} elseif ($doctor -match 'Recycle Bin') {
    Ok 'doctor 认出了 Recycle Bin'
} else {
    Write-Output 'note: doctor 的 trash 行既不是 Recycle Bin 也不是 fallback,人工确认'
}

Step 'status (json)'
$status = & $squid status --json | ConvertFrom-Json
Ok ("os={0} host={1} volumes={2} arch={3}" -f $status.host.os_version, $status.host.hostname,
    @($status.volumes).Count, $status.host.arch)
if (-not $status.host.os_version) { Fail 'status --json 没有报告操作系统' }
if (@($status.volumes).Count -lt 1) { Fail 'status --json 没有报告任何卷' }

Step 'installed apps vs registry'
# 2>$null 不能省:进度走 stderr,PowerShell 会把它们变成 ErrorRecord 混进输出流,
# 下面的 Count 就数成了 1。
$apps = @(& $squid uninstall --list --json 2>$null | ConvertFrom-Json)
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
Ok ("squid={0} registry={1}" -f $apps.Count, @($regNames).Count)
$missing = @($regNames | Where-Object { $apps.name -notcontains $_ })
if ($missing.Count) { Write-Output ("  missing in squid: " + (($missing | Select-Object -First 12) -join ' | ')) }

Step 'clean --dry-run (safety invariant)'
$report = & $squid clean --dry-run --json | ConvertFrom-Json
$items = @($report.items)
# 汇总由条目累加:JSON 里没有现成的 freed 总计字段。
$freed = 0
if ($items.Count) { $freed = [double](($items | Measure-Object -Property allocated -Sum).Sum) }
Ok ("items={0} freed={1:N0} B unreadable={2}" -f $items.Count, $freed, $report.unreadable)
# 每条清理建议都必须落在锚点之内,且 allocated 不能是 0(否则 freed 数是假的)。
$noanchor = @($items | Where-Object { -not $_.anchors -or @($_.anchors).Count -eq 0 })
if ($noanchor.Count) { Fail ("{0} 个清理项没有 anchor" -f $noanchor.Count) } else { Ok '每个清理项都有 anchor' }
$zero = @($items | Where-Object { $_.size -gt 0 -and $_.allocated -eq 0 })
if ($zero.Count) { Fail ("{0} 个清理项 size>0 但 allocated=0" -f $zero.Count) } else { Ok 'size>0 的清理项都量到了 allocated' }
$items | Group-Object category | Sort-Object Count -Descending | ForEach-Object {
    Write-Output ("  {0,-14} {1}" -f $_.Name, $_.Count)
}
# 核心安全不变量:系统目录里的条目可以"被看见",但绝不能"被选中"。
# C:\Windows\SoftwareDistribution\Download 和 Prefetch 是正当的清理目标
# (磁盘清理工具也清它们),所以禁止的不是路径本身,而是"未提权时被预选"。
# 真机上这一条救回了误判:280 条 SoftwareDistribution 全部 selected=false
# 且 lock=protected,note 里写明了要提权 —— 行为是对的。
$forbidden = @('c:\windows', 'c:\program files', 'c:\program files (x86)', '\$recycle.bin')
function In-SystemDir($path) {
    $p = ("$path").ToLower()
    foreach ($f in $forbidden) { if ($p.StartsWith($f)) { return $true } }
    return $false
}
$armed = @($items | Where-Object { In-SystemDir $_.path })
$live = @($armed | Where-Object { $_.selected -or -not $_.lock })
Write-Output ("  系统目录条目: {0} 条,其中可选中的 {1} 条" -f $armed.Count, $live.Count)
if ($live.Count) {
    Fail ("clean 把 {0} 个系统目录条目设成了可删: {1}" -f $live.Count, (($live | Select-Object -First 3).path -join ' | '))
} else { Ok '系统目录条目一律 protected + 未选中' }

Step 'recycle-bin round trip'
# 观察回收站有两种办法,任一有效即可断定 IFileOperation 成功:
#   1) Shell.Application 枚举(在非交互 SSH 会话里能不能用,本身就是待验证项);
#   2) squid 自己的账本 —— 只有 IFileOperation 返回成功才会记下这次会话。
# 注意函数里绝不能 Write-Output 诊断文本:它会混进返回值,$before/$after 就废了。
$script:bin_err = $null
function Bin-Items {
    $script:bin_err = $null
    try {
        $shell = New-Object -ComObject Shell.Application
        return ,@($shell.Namespace(0xA).Items() | ForEach-Object { $_.Name })
    } catch {
        $script:bin_err = $_.Exception.Message
        return $null
    }
}
$pkg = Join-Path $scratch 'node_modules\pkg'
New-Item -ItemType Directory -Force -Path $pkg | Out-Null
[IO.File]::WriteAllBytes((Join-Path $pkg 'blob.bin'), (New-Object byte[] 2MB))
$artefact = Join-Path $scratch 'node_modules'
Ok ("fixture created: $artefact (2 MB)")

# 回收站的落地判据:量 C:\$Recycle.Bin 的字节数,别信 COM 枚举。
# 就是这条量出了真 bug —— IFileOperation 少了 FOF_ALLOWUNDO 时,
# 文件从磁盘消失、账本写 mode=trash、回收站一字节没多。
function Bin-Bytes {
    $t = 0
    foreach ($d in Get-ChildItem 'C:\$Recycle.Bin' -Force -Directory -ErrorAction SilentlyContinue) {
        # 用 .NET 枚举:Get-ChildItem -Recurse 撞到别人 SID 的 ACL 会直接报错刷屏。
        try {
            foreach ($f in [IO.Directory]::EnumerateFiles($d.FullName, '*', 'AllDirectories')) {
                try { $t += (New-Object IO.FileInfo $f).Length } catch {}
            }
        } catch {}
    }
    return $t
}
$bytesBefore = Bin-Bytes
Write-Output ("  bin bytes before = {0:N0}" -f $bytesBefore)

$before = Bin-Items
if ($null -eq $before) { Write-Output ("  Shell.Application 不可用: " + $script:bin_err) }
# --min-age 0 是必须的:purge 默认只碰 7 天没动的文件,刚建的 fixture 会被跳过。
# 进度走 stderr、结果走 stdout:把 2>&1 并起来会被 PowerShell 变成 ErrorRecord,
# JSON 就没了,所以这里分开收。
$purge = & $squid purge $scratch --yes --min-age 0 --json 2>$null | Out-String
if (-not $purge) { Fail 'purge --json 没有给出任何 stdout' }
$verdict = $null
try { $verdict = $purge | ConvertFrom-Json } catch { Fail "purge --json 解析失败: $_" }
if ($verdict) {
    Write-Output ("  removed={0} failed={1} mode={2} freed={3:N0} B" -f
        $verdict.removed, $verdict.failed, $verdict.mode, $verdict.freed_allocated)
    if ($verdict.mode -ne 'trash') { Fail ("mode={0},fixture 应为 trash 模式" -f $verdict.mode) }
    if ($verdict.failed -gt 0) { Fail ("purge 报告 {0} 条失败" -f $verdict.failed) }
}

# 断言 artefact 自身被移走:purge 只删 node_modules,它的父目录本来就该留着。
if (Test-Path $artefact) { Fail 'trash 之后 node_modules 仍在磁盘上' } else { Ok 'node_modules 已离开磁盘' }

$bytesAfter = Bin-Bytes
$delta = $bytesAfter - $bytesBefore
Write-Output ("  bin bytes after  = {0:N0} (delta {1:N0})" -f $bytesAfter, $delta)
if ($delta -lt 1MB) {
    Fail ("回收站只多了 {0:N0} 字节 —— 删除绕过了回收站,mode=trash 是假的" -f $delta)
} else { Ok ("回收站真的收到了 {0:N0} 字节" -f $delta) }

$after = Bin-Items
if ($null -eq $before -or $null -eq $after) {
    Write-Output 'note: 无法从当前会话枚举回收站,改由账本判定'
} elseif (@($after).Count -le @($before).Count) {
    Fail ("回收站条目数没有增加 ({0} -> {1})" -f @($before).Count, @($after).Count)
} else {
    Ok ("回收站增加 {0} -> {1}" -f @($before).Count, @($after).Count)
}
if ($after -and (@($after) -match [regex]::Escape($run))) { Ok "回收站里能看到原名 $run" }
elseif ($after) { Write-Output ("note: 回收站里按原名找不到 {0},内容: {1}" -f $run, ((@($after) | Select-Object -First 8) -join ' | ')) }

Step 'undo'
# Windows 上回收站不记录落地路径,undo 应给出"去回收站里还原"的指引而不是假装恢复。
$undo = & $squid undo 2>&1 | Out-String
if ($undo) { Write-Output (($undo.Trim() -split "`n" | Select-Object -First 8) -join "`n") }
if ($undo -match 'no trashed session') { Fail 'purge 之后账本里没有这次回收会话' } else { Ok 'undo 认出了刚才的会话' }

Step 'result'
if ($fails.Count) { Write-Output ("FAILURES: " + $fails.Count); $fails | ForEach-Object { Write-Output " - $_" }; exit 1 }
Write-Output 'ALL CHECKS PASSED'
