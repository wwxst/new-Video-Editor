# Chinese text / IME interaction harness.
#
# `-Mode direct` injects Unicode characters into the focused TextInput.
# `-Mode ime` activates the installed Microsoft Pinyin IME, types "nihao", captures the pre-edit
# state, commits with Space, and captures the committed text.

param(
    [Parameter(Mandatory = $true)][ValidateSet("direct", "ime")][string]$Mode,
    [string]$Exe = (Resolve-Path "..\ui-validation\target\release\slint-poc.exe").Path,
    [string]$Out = "ime.png",
    [string]$Report = "ime.json",
    [int]$Seconds = 18,
    [int]$Width = 1600,
    [int]$Height = 1000
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class ImeWin32 {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr LoadKeyboardLayout(string id, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    public static IntPtr FindForProcess(uint target) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == target && IsWindowVisible(h)) {
                var t = new StringBuilder(512); GetWindowText(h, t, 512);
                if (t.Length > 0) { found = h; return false; }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
    }
    public static void Key(byte vk) {
        keybd_event(vk, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(40);
        keybd_event(vk, 0, 2, UIntPtr.Zero);
        System.Threading.Thread.Sleep(40);
    }
    public static void CtrlA() {
        keybd_event(0x11, 0, 0, UIntPtr.Zero);
        keybd_event(0x41, 0, 0, UIntPtr.Zero);
        keybd_event(0x41, 0, 2, UIntPtr.Zero);
        keybd_event(0x11, 0, 2, UIntPtr.Zero);
    }
}
"@

$outPath = if ([IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path (Get-Location) $Out }
$reportPath = if ([IO.Path]::IsPathRooted($Report)) { $Report } else { Join-Path (Get-Location) $Report }
$childReport = Join-Path (Get-Location) "ime-child.json"
$exePath = (Resolve-Path $Exe).Path
Remove-Item $reportPath,$childReport -ErrorAction SilentlyContinue

$proc = Start-Process -FilePath $exePath -ArgumentList @(
    "--frames", "0",
    "--idle-seconds", "$Seconds",
    "--video-mode", "native",
    "--scenario", "selected",
    "--width", "$Width",
    "--height", "$Height",
    "--result", "ime-child.json"
) -PassThru -WorkingDirectory (Get-Location)

$deadline = (Get-Date).AddSeconds(10)
$hwnd = [IntPtr]::Zero
while ($hwnd -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 200
    $hwnd = [ImeWin32]::FindForProcess([uint32]$proc.Id)
}
if ($hwnd -eq [IntPtr]::Zero) { throw "Slint window not found" }
[ImeWin32]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Milliseconds 500

$origin = New-Object ImeWin32+POINT
[ImeWin32]::ClientToScreen($hwnd, [ref]$origin) | Out-Null
$inputX = $origin.X + 1250
$inputY = $origin.Y + 285
[ImeWin32]::Click($inputX, $inputY)
Start-Sleep -Milliseconds 400
[ImeWin32]::CtrlA()
Start-Sleep -Milliseconds 100

if ($Mode -eq "direct") {
    # SendInput with KEYEVENTF_UNICODE: Chinese text without an IME conversion step.
    Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class UniInput {
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] public struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; [FieldOffset(8)] public MOUSEINPUT mi; }
    [DllImport("user32.dll", SetLastError=true)] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
    public static void Text(string s) {
        foreach (char ch in s) {
            var down = new INPUT { type = 1 };
            down.ki.wScan = (ushort)ch; down.ki.dwFlags = 0x0004;
            var up = new INPUT { type = 1 };
            up.ki.wScan = (ushort)ch; up.ki.dwFlags = 0x0004 | 0x0002;
            SendInput(1, new[] { down }, Marshal.SizeOf(typeof(INPUT)));
            SendInput(1, new[] { up }, Marshal.SizeOf(typeof(INPUT)));
        }
    }
}
"@
    $text = -join @([char]0x4E2D, [char]0x6587, [char]0x8F93, [char]0x5165, [char]0x6D4B, [char]0x8BD5)
    [UniInput]::Text($text)
    Start-Sleep -Milliseconds 1000
    $shot = "slint-chinese-direct.png"
} else {
    $hkl = [ImeWin32]::LoadKeyboardLayout("00000804", 1)
    [ImeWin32]::PostMessage($hwnd, 0x0050, [IntPtr]::Zero, $hkl) | Out-Null
    Start-Sleep -Milliseconds 500
    foreach ($vk in @(0x4E, 0x49, 0x48, 0x41, 0x4F)) { [ImeWin32]::Key([byte]$vk) }
    Start-Sleep -Milliseconds 1200
    $shot = "slint-ime-preedit.png"
    $rect = New-Object ImeWin32+RECT
    [ImeWin32]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
    $bmp = New-Object System.Drawing.Bitmap(($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top))
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bmp.Size)
    $bmp.Save((Join-Path (Get-Location) $shot), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    [ImeWin32]::Key(0x20)
    Start-Sleep -Milliseconds 1200
    $shot = "slint-ime-committed.png"
}

$rect = New-Object ImeWin32+RECT
[ImeWin32]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
$bmp = New-Object System.Drawing.Bitmap(($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top))
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bmp.Size)
$bmp.Save($outPath, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()

[ImeWin32]::PostMessage($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep -Seconds 2
Start-Sleep -Seconds 1
$child = if (Test-Path $childReport) { Get-Content $childReport -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
if (-not $proc.HasExited) { try { $proc.Kill() } catch {} }
$result = [ordered]@{
    mode = $Mode
    screenshot = $outPath
    child = $child
}
$result | ConvertTo-Json -Depth 8
$result | ConvertTo-Json -Depth 8 | Set-Content $reportPath
