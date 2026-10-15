# 清掉上一轮测试留在机器上的进程。测试台在每次运行前后各调一次。
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File win-tui-clean.ps1
#
# 为什么需要:测试台是通过 ssh 起的,本地 Ctrl-C 或 ssh 断掉并不会让远端那条
# PowerShell 结束 —— 它会一直卡在 WaitForExit 上,而它的子进程(squid.exe)
# 则一直占着那块控制台和一个 release 版 exe 文件句柄。后果有两个:
# 编译产物覆盖不了,以及旧进程会往报告文件里补写 MARK_QUIT,
# 让下一轮读到一份不属于这一轮的事实。
#
# 只杀两类,且都按命令行匹配,不碰用户自己开的窗口:
#   * squid.exe(被测程序本身);
#   * 命令行里带 win-tui-grow.ps1 的 powershell.exe(测试台自己),本进程除外。
$targets = @()
$binaries = Get-Process -Name squid -ErrorAction SilentlyContinue
foreach ($process in $binaries) {
    $targets += [pscustomobject]@{ Id = $process.Id; Why = "squid.exe" }
}

$growers = Get-CimInstance -ClassName Win32_Process -Filter "Name = 'powershell.exe'" |
    Where-Object { $_.ProcessId -ne $PID -and $_.CommandLine -match 'win-tui-grow\.ps1' }
foreach ($process in $growers) {
    $targets += [pscustomobject]@{ Id = $process.ProcessId; Why = "win-tui-grow.ps1" }
}

foreach ($target in $targets) {
    Write-Output "KILLED $($target.Why) pid=$($target.Id)"
    Stop-Process -Id $target.Id -Force -ErrorAction SilentlyContinue
}
if ($targets.Count -eq 0) { Write-Output "CLEAN nothing to kill" }
