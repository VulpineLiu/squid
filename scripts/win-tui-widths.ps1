# 量一量"这个控制台认为每个字形占几格",把结果写进文件,不需要任何人盯着屏幕看。
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File win-tui-widths.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File win-tui-widths.ps1 C:\Users\kangwj\widths.txt
#
# 为什么需要它:渲染器按 Unicode 的 East Asian Width 决定"这一行字占几列",
# conhost 按它自己的规矩决定"这个字形占几格"。两边一旦不一致,本该正好顶到右边框的
# 行就多出一格,右边框被挤到下一行开头,下面每一行跟着错位 —— 用户看到的就是花屏、
# 右边被切掉、页脚重复出现。中文控制台最容易撞这件事:·…—×↑↓→←“”●○≥
# 在 GBK 里都是双字节字符,而 Unicode 说它们是"窄"的;─│ 这类制表符同理。
#
# 判据不靠肉眼:把光标放到已知列,写一个字符,再读回光标停在第几列 ——
# 光标走过的格数就是这个字符在这个控制台里的真实宽度。同时把那一格里真正存下来的
# 字符读回来,于是"宽度不符"和"字符被码页换掉了"这两件事能一次分开。
#
# 两条写入路径都量一遍,因为它们真的不一样:
#   W = WriteConsoleW(UTF-16 直接进缓冲区)。界面在真控制台里走的就是这一条。
#   A = 程序往 stdout 写编码后的字节,由控制台自己解码。ConPTY/管道走这一条。
# 输出被重定向时 A 这条路根本到不了屏幕,那种轮次直接跳过,不量出一条假数据。
#
# 副作用:探测要在窗口倒数第二、第三行上写字,那两行原来的内容会被盖掉。
# 脚本先把那两行读下来,量完原样写回,再把光标放回去 —— 但宽字符是"一个字符占两格",
# 还原时第二格只能填回空格,所以还原完那两行可能有点毛刺。按回车或直接 clear 就行。
param(
    [string]$Out = "$env:USERPROFILE\win-tui-widths.txt"
)

# 规矩:Add-Type 用 -TypeDefinition(整份编译单元),不要用 -MemberDefinition。
# -MemberDefinition 会把成员塞进一个生成的类体里,放在最前面的 using 就变成
# "类成员位置上的 using",PS 5.1 直接编译失败(CS1529),而它报的是 Add-Type
# 那一行,很容易误判成"这台机器不让 P/Invoke"。PS 5.1 也没有 -Usings 参数。
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace Squid {
public static class CellProbe {
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
                                     uint disposition, uint flags, IntPtr templateFile);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetConsoleScreenBufferInfo(IntPtr handle, out INFO info);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool SetConsoleCursorPosition(IntPtr handle, COORD coordinate);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern bool WriteConsoleW(IntPtr handle, string buffer, uint count,
                                            out uint written, IntPtr reserved);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern bool ReadConsoleOutputCharacterW(IntPtr handle, StringBuilder text,
                                                          uint count, COORD from, out uint read);
    [DllImport("kernel32.dll")] public static extern uint GetConsoleOutputCP();
    [DllImport("kernel32.dll")] public static extern uint GetConsoleCP();
    [DllImport("kernel32.dll")] public static extern bool GetConsoleMode(IntPtr handle, out uint mode);
    [DllImport("kernel32.dll")] public static extern IntPtr GetStdHandle(int index);
    [DllImport("kernel32.dll")] public static extern uint GetFileType(IntPtr handle);

    public static IntPtr Open() {
        IntPtr handle = CreateFileW("CONOUT$", 0xC0000000, 0x00000003, IntPtr.Zero, 3, 0, IntPtr.Zero);
        return handle.ToInt64() == -1 ? IntPtr.Zero : handle;
    }

    public static INFO Read(IntPtr handle) {
        INFO info;
        GetConsoleScreenBufferInfo(handle, out info);
        return info;
    }

    // 别在成员里写 uint:short 隐式转不成 uint(CS0266)。量光标列用 int 就够。
    public static int CursorColumn(IntPtr handle) { return (int)Read(handle).dwCursorPosition.X; }

    public static bool Place(IntPtr handle, int column, int row) {
        COORD where = new COORD();
        where.X = (short)column;
        where.Y = (short)row;
        return SetConsoleCursorPosition(handle, where);
    }

    public static bool PutWide(IntPtr handle, string text) {
        uint written;
        return WriteConsoleW(handle, text, (uint)text.Length, out written, IntPtr.Zero);
    }

    /// 那一格里真正存下来的第一个 UTF-16 单元。<0 表示什么都没读到。
    public static int CellAt(IntPtr handle, int column, int row) {
        StringBuilder text = new StringBuilder(2);
        COORD where = new COORD();
        where.X = (short)column;
        where.Y = (short)row;
        uint read = 0;
        if (!ReadConsoleOutputCharacterW(handle, text, 1, where, out read) || read == 0) { return -1; }
        return (int)text[0];
    }

    public static string RowAt(IntPtr handle, int column, int row, int count) {
        if (count <= 0) { return ""; }
        StringBuilder text = new StringBuilder(count + 1);
        COORD where = new COORD();
        where.X = (short)column;
        where.Y = (short)row;
        uint read = 0;
        if (!ReadConsoleOutputCharacterW(handle, text, (uint)count, where, out read)) { return ""; }
        return text.ToString(0, (int)read);
    }

    public static uint OutputCP() { return GetConsoleOutputCP(); }
    public static uint InputCP() { return GetConsoleCP(); }
    public static uint Mode(IntPtr handle) {
        uint mode = 0;
        GetConsoleMode(handle, out mode);
        return mode;
    }
    /// stdout 这个句柄是什么:0 未知 / 1 磁盘 / 2 字符设备(真控制台或 ConPTY) / 3 管道。
    public static uint StdoutKind() { return GetFileType(GetStdHandle(-11)); }
    /// stdout 自己是不是控制台(能不能 GetConsoleMode)。crossterm 就是靠这个决定
    /// 走 Windows 控制台 API 还是走 ANSI 转义序列,两条路的"宽度由谁说了算"不一样。
    public static bool StdoutIsConsole() {
        uint mode = 0;
        return GetConsoleMode(GetStdHandle(-11), out mode);
    }
}
}
'@

# 这些码位不是随便挑的:每一个都是界面里真的会画到屏幕上的字符,而且都是
# "GBK 认为宽、Unicode East Asian Width 认为窄"这一类。A 和 4E2D 是对照组:
# 一个必窄,一个必宽,量出来的数不对就说明脚本自己错了。
$codePoints = @(
    0x0041, 0x4E2D, 0x0020,
    0x00B7, 0x2026, 0x2014, 0x00D7, 0x201C, 0x201D, 0x2018, 0x2019,
    0x2190, 0x2191, 0x2192, 0x2193, 0x21D2, 0x21B3, 0x2265, 0x2713,
    0x25CF, 0x25CB, 0x25BE, 0x25A0, 0x2500, 0x2502, 0x250C, 0x2510,
    0x2514, 0x2518, 0x2550, 0x2503, 0x2460, 0x2461, 0x2462
)

function Hex([int]$value) { return ('{0:X4}' -f $value) }

$lines = New-Object 'System.Collections.Generic.List[string]'
$handle = [Squid.CellProbe]::Open()
if ($handle -eq [IntPtr]::Zero) {
    $lines.Add('ERROR CONOUT$ could not be opened - run this in the console window itself')
    [void][System.IO.File]::WriteAllLines($Out, $lines)
    Write-Output "WROTE $Out"
    return
}

try {
    $before = [Squid.CellProbe]::Read($handle)
    $window = $before.srWindow
    $columns = [int]$window.Right - [int]$window.Left + 1
    $rows = [int]$window.Bottom - [int]$window.Top + 1
    $mode = [int][Squid.CellProbe]::Mode($handle)
    $outCp = [int][Squid.CellProbe]::OutputCP()
    $inCp = [int][Squid.CellProbe]::InputCP()
    $stdoutKind = [int][Squid.CellProbe]::StdoutKind()
    $stdoutConsole = [Squid.CellProbe]::StdoutIsConsole()
    $dotnet = [Console]::OutputEncoding.WebName
    $vt = ($mode -band 0x0004) -ne 0
    $redirected = [Console]::IsOutputRedirected

    # 一行一个事实,每件事都先落成变量再拼:多值 -f 一旦少个参数就整行报
    # FormatException,诊断脚本最怕的就是这种"整行没了"。
    $lines += "ENV outcp=$outCp incap=$inCp mode=0x$(Hex $mode) vtoutput=$vt"
    $lines += "ENV stdout_kind=$stdoutKind stdout_is_console=$stdoutConsole redirected=$redirected dotnet=$dotnet"
    $lines += "ENV window=${columns}x${rows} at=$($window.Left),$($window.Top) buffer=$($before.dwSize.X)x$($before.dwSize.Y)"
    $lines += "ENV cursor=$($before.dwCursorPosition.X),$($before.dwCursorPosition.Y) ps=$($PSVersionTable.PSVersion)"

    if ($rows -lt 5 -or $columns -lt 60) {
        $lines.Add("ERROR window is ${columns}x${rows} - need at least 60x6, resize the window and run again")
        return
    }

    # 探测用窗口里倒数第二、第三行:最后一行写字会触发滚动,谁都不想要这种副作用。
    $probeRows = @{ W = [int]$window.Bottom - 1; A = [int]$window.Bottom - 2 }
    foreach ($path in @('W', 'A')) {
        $row = $probeRows[$path]
        $saved = [Squid.CellProbe]::RowAt($handle, $window.Left, $row, $columns)
        if ($path -eq 'A' -and ($redirected -or -not $stdoutConsole)) {
            $lines += "SKIP path=A stdout is not a console screen - the byte path would not reach the screen"
            continue
        }
        $column = [int]$window.Left
        $stop = [int]$window.Right - 2
        foreach ($point in $codePoints) {
            if ($column -ge $stop) {
                $lines += "TRUNCATED path=$A column=$column - widen the window and run again"
                break
            }
            if (-not [Squid.CellProbe]::Place($handle, $column, $row)) {
                $lines += "PLACE_FAILED path=$path column=$column row=$row"
                break
            }
            $glyph = [string][char]$point
            $from = [Squid.CellProbe]::CursorColumn($handle)
            if ($path -eq 'W') {
                [void][Squid.CellProbe]::PutWide($handle, $glyph)
            } else {
                # 走 .NET 的编码链路:编码后的字节进 stdout 句柄,由控制台自己解码。
                [Console]::Write($glyph)
            }
            $to = [Squid.CellProbe]::CursorColumn($handle)
            $cell = [Squid.CellProbe]::CellAt($handle, $column, $row)
            $lines += "MEASURE path=$path cp=U+$(Hex $point) column=$column width=$($to - $from) cell=U+$(Hex ([Math]::Max($cell, 0)))"
            $column = $to
        }
        # 把这一行原本的内容写回去。宽字符还原不完美(第二格只能回空格),报告里说清楚了。
        [void][Squid.CellProbe]::Place($handle, $window.Left, $row)
        [void][Squid.CellProbe]::PutWide($handle, ($saved -replace '[^\x20-\x7E]', ' '))
    }

    # 光标回到脚本进来时的位置:探测不该留下痕迹。
    [void][Squid.CellProbe]::Place($handle, $before.dwCursorPosition.X, $before.dwCursorPosition.Y)
} catch {
    $lines += "FAILED $($_.Exception.GetType().Name) $($_.Exception.Message)"
} finally {
    [void][Squid.CellProbe]::CloseHandle($handle)
    [void][System.IO.File]::WriteAllLines($Out, $lines, [System.Text.Encoding]::UTF8)
    Write-Output "WROTE $Out lines=$($lines.Count)"
}
