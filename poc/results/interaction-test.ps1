# Real Win32 pointer input against the Slint POC: splitter drag, clip click, zoom button, wheel.
# The app writes its final model state, so the assertions use the app's own view of the input.

param(
    [string]$Exe = (Resolve-Path "..\ui-validation\target\release\slint-poc.exe").Path,
    [string]$Report = "interaction.json",
    [int]$Seconds = 24,
    [int]$Width = 1600,
    [int]$Height = 1000
)

$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class UiWin32 {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int data, UIntPtr extra);
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
    public static void Drag(int x1, int y1, int x2, int y2) {
        SetCursorPos(x1, y1);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        for (int i = 1; i <= 10; i++) {
            SetCursorPos(x1 + (x2 - x1) * i / 10, y1 + (y2 - y1) * i / 10);
            System.Threading.Thread.Sleep(30);
        }
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
    }
    public static void Wheel(int x, int y, int notches) {
        SetCursorPos(x, y);
        mouse_event(0x0800, 0, 0, notches * 120, UIntPtr.Zero);
    }
}
"@

$reportPath = if ([IO.Path]::IsPathRooted($Report)) { $Report } else { Join-Path (Get-Location) $Report }
$childReport = Join-Path (Get-Location) "interaction-child.json"
$exePath = (Resolve-Path $Exe).Path
Remove-Item $reportPath,$childReport -ErrorAction SilentlyContinue

$proc = Start-Process -FilePath $exePath -ArgumentList @(
    "--frames", "0",
    "--idle-seconds", "$Seconds",
    "--video-mode", "native",
    "--scenario", "default",
    "--width", "$Width",
    "--height", "$Height",
    "--result", "interaction-child.json"
) -PassThru -WorkingDirectory (Get-Location)

$deadline = (Get-Date).AddSeconds(10)
$hwnd = [IntPtr]::Zero
while ($hwnd -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 200
    $hwnd = [UiWin32]::FindForProcess([uint32]$proc.Id)
}
if ($hwnd -eq [IntPtr]::Zero) { throw "Slint window not found" }
[UiWin32]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Milliseconds 800

$origin = New-Object UiWin32+POINT
[UiWin32]::ClientToScreen($hwnd, [ref]$origin) | Out-Null
function X([int]$x) { $origin.X + $x }
function Y([int]$y) { $origin.Y + $y }

# 1. Drag the left splitter 100 px to the right.
[UiWin32]::Drag((X 399), (Y 220), (X 499), (Y 220))
Start-Sleep -Milliseconds 500

# 2. Click across the first visible video clips; one lands inside a clip regardless of the
#    exact first-clip start in the deterministic scene.
foreach ($x in @(220, 280, 340, 400, 460)) {
    [UiWin32]::Click((X $x), (Y 762))
    Start-Sleep -Milliseconds 120
}
Start-Sleep -Milliseconds 300

# 3. Click the "放大" (zoom in) button in the timeline toolbar. A small sweep makes the
#    hit-area robust against a one-pixel layout difference between builds.
[UiWin32]::Click((X 530), (Y 676))
Start-Sleep -Milliseconds 150
[UiWin32]::Click((X 545), (Y 676))
Start-Sleep -Milliseconds 150
[UiWin32]::Click((X 560), (Y 676))
Start-Sleep -Milliseconds 600

# 4. Wheel down over the timeline lanes to scroll vertically.
[UiWin32]::Wheel((X 900), (Y 900), -5)
Start-Sleep -Milliseconds 250
[UiWin32]::Wheel((X 900), (Y 800), -5)
Start-Sleep -Milliseconds 800

# Close through the normal window path so the app writes its result.
[UiWin32]::PostMessage($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep -Seconds 2

$child = if (Test-Path $childReport) { Get-Content $childReport -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
if (-not $proc.HasExited) { try { $proc.Kill() } catch {} }
$extra = $child.extra
$checks = [ordered]@{
    left_panel_widened = ($extra.panel_fractions[0] -gt 0.26)
    clip_selected = ($extra.selected_id -ge 0)
    zoom_increased = ($extra.zoom -gt 60)
    vertical_scroll_moved = ($extra.view_scroll[1] -gt 1)
}
$result = [ordered]@{
    child = $child
    checks = $checks
    passed = ($checks.Values -notcontains $false)
}
$result | ConvertTo-Json -Depth 8
$result | ConvertTo-Json -Depth 8 | Set-Content $reportPath
