# 在程序运行的过程中把它所依附的控制台改大,给 scripts/win-tui-resize.py 用。
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File win-tui-grow.ps1 `
#       100 30 180 50 10 en C:\dev\target\release\squid.exe `
#       C:\Users\kangwj\report-1700.txt C:\Users\kangwj\trace-1700.log
#
# 报告/日志路径每轮都不同(测试台在文件名里带上这一轮的时刻)。上一轮这一点是
# 靠"启动前先 del 掉"保证的,结果 del 不掉的那一轮读到的是上一轮的报告:
# 文件里只剩孤零零一行 MARK_QUIT,看着像"改尺寸根本没发生",其实是另一条
# 还卡在 WaitForExit 的僵尸进程在替这一轮说话。名字唯一,事实就不会串台。
#
# 四条用真机换来的规矩:
#
# 1) 不能用 mode con。在 Windows OpenSSH 给的 ConPTY 上 `mode con cols=180 lines=50`
#    会静默失败(返回 0,什么都没改)。
#
# 2) [Console]::WindowWidth 也不行 —— 至少在界面起来以后不行。crossterm 进备用屏是
#    CreateConsoleScreenBuffer + SetConsoleActiveScreenBuffer,换掉的是"当前正在显示
#    的那块屏幕缓冲区",而本进程的 stdout 句柄还指着进来时那块。实测:程序起来之后
#    对 [Console]::WindowWidth 赋值,读回来纹丝不动 —— 改的是一块没人看的屏幕。
#    CONOUT$ 这个名字每次打开都解析成"当前活动的那块缓冲区",所以本脚本自己
#    P/Invoke CreateFileW("CONOUT$"),改的才是界面真正依附的那一块。
#
# 3) 改窗口之前必须先改缓冲区。conhost 的 dwMaximumWindowSize 就等于缓冲区尺寸,
#    窗口永远不能比缓冲区大:直接把窗口设成 180 会被拒绝,于是"改大"根本没发生,
#    测的是一条根本没变过的尺寸。所以先把缓冲区撑到两种尺寸里最大的那个,再把窗口
#    收到小的,运行一段时间后才放大窗口。
#
# 4) 界面跑起来以后,一个字都不能往这个控制台里写。测试台自己的输出也是画在界面上的:
#    一行 Write-Output 会落在界面正在画的区域里,末尾的换行还会让 conhost 整体上滚
#    一行 —— 渲染器的 diff 从此和真实屏幕差一行,屏幕上到处是上一帧的残影。
#    所以启动之后的事实全部写进报告文件,由测试台另外一条 ssh 连接读回来。
param(
    [Parameter(Mandatory = $true)][int]$SmallColumns,
    [Parameter(Mandatory = $true)][int]$SmallRows,
    [Parameter(Mandatory = $true)][int]$BigColumns,
    [Parameter(Mandatory = $true)][int]$BigRows,
    [Parameter(Mandatory = $true)][int]$GrowAfter,
    [Parameter(Mandatory = $true)][string]$Lang,
    [Parameter(Mandatory = $true)][string]$Bin,
    [Parameter(Mandatory = $true)][string]$Report,
    [string]$Trace = ""
)

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace Squid {
public static class ConsoleProbe {
    [StructLayout(LayoutKind.Sequential)] public struct COORD { public short X; public short Y; }
    [StructLayout(LayoutKind.Sequential)] public struct SMALL_RECT {
        public short Left; public short Top; public short Right; public short Bottom;
    }
    [StructLayout(LayoutKind.Sequential)] public struct INFO {
        public COORD dwSize; public COORD dwCursorPosition; public ushort wAttributes;
        public SMALL_RECT srWindow; public COORD dwMaximumWindowSize;
    }

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr security,
                                     uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetConsoleScreenBufferInfo(IntPtr handle, out INFO info);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool SetConsoleScreenBufferSize(IntPtr handle, COORD size);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool SetConsoleWindowInfo(IntPtr handle, bool absolute, ref SMALL_RECT rect);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);

    // 0 是 NULL:脚本自己不是控制台程序时也拿不到句柄,调用方负责报错。
    public static IntPtr Open() {
        IntPtr handle = CreateFileW("CONOUT$", 0xC0000000, 0x00000003, IntPtr.Zero,
                                    3, 0, IntPtr.Zero);
        return handle.ToInt64() == -1 ? IntPtr.Zero : handle;
    }

    public static uint LastError() { return (uint)Marshal.GetLastWin32Error(); }

    public static string Size(IntPtr handle) {
        INFO info;
        if (!GetConsoleScreenBufferInfo(handle, out info)) { return "unknown"; }
        int windowColumns = info.srWindow.Right - info.srWindow.Left + 1;
        int windowRows = info.srWindow.Bottom - info.srWindow.Top + 1;
        return windowColumns + "x" + windowRows + " in " +
               info.dwSize.X + "x" + info.dwSize.Y;
    }

    /// 先撑缓冲区,再撑窗口。返回 false 时通过 LastError 说明为什么。
    public static bool Resize(IntPtr handle, int columns, int rows,
                              out int bufferColumns, out int bufferRows) {
        INFO info;
        bufferColumns = 0; bufferRows = 0;
        if (!GetConsoleScreenBufferInfo(handle, out info)) { return false; }
        int wide = columns > info.dwSize.X ? columns : info.dwSize.X;
        int tall = rows > info.dwSize.Y ? rows : info.dwSize.Y;
        if (wide != info.dwSize.X || tall != info.dwSize.Y) {
            COORD want = new COORD(); want.X = (short)wide; want.Y = (short)tall;
            if (!SetConsoleScreenBufferSize(handle, want)) { return false; }
            if (!GetConsoleScreenBufferInfo(handle, out info)) { return false; }
        }
        // 只改大小,不动位置:窗口左上角留在原地,才不会把用户正在看的内容卷走。
        SMALL_RECT window = info.srWindow;
        window.Right = (short)(window.Left + columns - 1);
        window.Bottom = (short)(window.Top + rows - 1);
        if (window.Right >= info.dwSize.X) { window.Right = (short)(info.dwSize.X - 1); }
        if (window.Bottom >= info.dwSize.Y) { window.Bottom = (short)(info.dwSize.Y - 1); }
        if (!SetConsoleWindowInfo(handle, true, ref window)) { return false; }
        if (!GetConsoleScreenBufferInfo(handle, out info)) { return false; }
        bufferColumns = info.dwSize.X; bufferRows = info.dwSize.Y;
        return true;
    }
}
}
'@

# 报告文件:每次运行从空白开始。用 AppendAllText 而不是 Add-Content,是因为
# 后者会走文件系统 provider,路径里一旦有它不认的东西就静默什么都不写。
function Tell([string]$line) {
    [void][System.IO.File]::AppendAllText($Report, "$line`r`n")
}

function Read-console([string]$when) {
    $handle = [Squid.ConsoleProbe]::Open()
    if ($handle -eq [IntPtr]::Zero) {
        Tell "NO_CONOUT $when win32=$([Squid.ConsoleProbe]::LastError())"
        return "unknown"
    }
    try { return [Squid.ConsoleProbe]::Size($handle) } finally {
        [void][Squid.ConsoleProbe]::CloseHandle($handle)
    }
}

function Set-console([int]$columns, [int]$rows, [string]$when) {
    $handle = [Squid.ConsoleProbe]::Open()
    if ($handle -eq [IntPtr]::Zero) {
        Tell "SIZE_FAILED $when CONOUT`$ unavailable win32=$([Squid.ConsoleProbe]::LastError())"
        Tell "$when unknown"
        return
    }
    try {
        $bufferColumns = 0
        $bufferRows = 0
        $worked = [Squid.ConsoleProbe]::Resize($handle, $columns, $rows,
                                               [ref]$bufferColumns, [ref]$bufferRows)
        if (-not $worked) {
            Tell "SIZE_FAILED $when win32=$([Squid.ConsoleProbe]::LastError())"
        }
        # 报告里写的是改完之后控制台自己承认的尺寸,不是我们请求的数字。
        $seen = [Squid.ConsoleProbe]::Size($handle)
        Tell "$when $seen"
        $script:seen = $seen
    } finally {
        [void][Squid.ConsoleProbe]::CloseHandle($handle)
    }
}

Set-Content -LiteralPath $Report -Value "" -Encoding ascii
if ($Trace -ne "") {
    Remove-Item -LiteralPath $Trace -ErrorAction SilentlyContinue
    $env:SQUID_CONSOLE_TRACE = $Trace
}
# 第一行先留下"这一轮是谁":两个进程抢同一份报告时,这行就是裁判。
Tell "MARK_ENTER pid=$PID bin=$Bin report=$Report"

Set-console $SmallColumns $SmallRows "MARK_STARTED"
# 启动之前打印一次是安全的:那时候界面上还没有任何东西需要保持原样。
# 测试台靠这一行对齐时钟 —— 改尺寸就发生在它之后的第 $GrowAfter 秒。
"MARK_STARTED $script:seen"

$arguments = "state"
if ($Lang -notin @('en', 'zh')) { $arguments = "state --lang $Lang" }
# 用 .NET 的 Process,不用 Start-Process:实测后者 -PassThru 给的对象的 ExitCode
# 是空的(报告里那行 "MARK_QUIT " 后面什么都没有),于是"界面到底怎么退的"没答案。
# UseShellExecute=false 才会继承本进程的控制台,后面改的才是界面依附的那块缓冲区。
$startInfo = New-Object System.Diagnostics.ProcessStartInfo
$startInfo.FileName = $Bin
$startInfo.Arguments = $arguments
$startInfo.UseShellExecute = $false
$interface = [System.Diagnostics.Process]::Start($startInfo)
Tell "MARK_RUNNING pid=$($interface.Id) arguments=$arguments"

Start-Sleep -Seconds $GrowAfter
# 顺序不能反:Resize() 内部已经是先撑缓冲区再撑窗口,反过来会被 conhost 拒绝。
Set-console $BigColumns $BigRows "MARK_RESIZED"
# 两秒后再读一次。ConPTY 的尺寸是由 ssh 客户端说了算的:如果它把我们改的又改回去,
# 这一行和上一行不一样,而界面看到的就是一场没人认领的抖动。
Start-Sleep -Seconds 2
Tell "MARK_STILL $(Read-console 'MARK_STILL')"

$interface.WaitForExit()
Tell "MARK_QUIT $($interface.ExitCode)"
# 界面已经退出,控制台重新归脚本所有,这一行才允许打印。
"MARK_QUIT $($interface.ExitCode)"
