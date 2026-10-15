# 按指定尺寸开好控制台,再在里面启动 squid,给 scripts/win-tui-capture.sh 用。
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File win-tui-run.ps1 `
#       100 30 C:\dev\target\release\squid.exe state --lang zh
#
# 为什么不能用 mode con:在 Windows OpenSSH 给的 ConPTY 上,
# `mode con cols=100 lines=30` 会静默失败(控制台仍是 120 列),所以以前所有
# "按 100 列抓的图"其实都是 120 列画出来后又被仿真器重排的产物。
# [Console]::WindowWidth 走 SetConsoleWindowInfo,实测真的能改。
#
# 尺寸改完把真实尺寸打印出来:测试要按控制台实际给的大小仿真,而不是按我们请求的大小。
param(
    [Parameter(Mandatory = $true)][int]$Columns,
    [Parameter(Mandatory = $true)][int]$Rows,
    [Parameter(Mandatory = $true)][string]$Bin,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments
)

chcp 65001 > $null
try {
    [Console]::WindowWidth = $Columns
    [Console]::WindowHeight = $Rows
}
catch {
    "MARK_SIZE_FAILED $($_.Exception.Message)"
}
"MARK_STARTED $([Console]::WindowWidth)x$([Console]::WindowHeight)"

& $Bin @Arguments
"MARK_QUIT $LASTEXITCODE"
