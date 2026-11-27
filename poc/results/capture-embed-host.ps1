# Capture the in-process native host created by `slint-poc --embed-host`.
# The host is a real Win32 window; its client area contains the Slint child created with
# winit's `with_parent_window`. The screenshot therefore shows the host chrome plus the child.

param(
    [string]$Exe = (Resolve-Path "..\ui-validation\target\release\slint-poc.exe").Path,
    [string]$Out = "embed-host.png",
    [string]$Report = "embed-host.json",
    [int]$Seconds = 8,
    [int]$Width = 1200,
    [int]$Height = 800
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class EmbedCapture {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
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
}
"@

$outPath = if ([IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path (Get-Location) $Out }
$reportPath = if ([IO.Path]::IsPathRooted($Report)) { $Report } else { Join-Path (Get-Location) $Report }
$childReport = Join-Path (Get-Location) "embed-native-child.json"
$childOut = Join-Path (Get-Location) "embed-native-child.out"
$childErr = Join-Path (Get-Location) "embed-native-child.err"
Remove-Item $reportPath,$childReport -ErrorAction SilentlyContinue
$exePath = (Resolve-Path $Exe).Path

$proc = Start-Process -FilePath $exePath -ArgumentList @(
    "--embed-host", "1",
    "--embed-seconds", "$Seconds",
    "--width", "$Width",
    "--height", "$Height",
    "--frames", "0",
    "--video-mode", "native",
    "--result", "embed-native-child.json"
) -PassThru -WorkingDirectory (Get-Location) -RedirectStandardOutput $childOut -RedirectStandardError $childErr

$deadline = (Get-Date).AddSeconds(10)
$hwnd = [IntPtr]::Zero
while ($hwnd -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 200
    $hwnd = [EmbedCapture]::FindForProcess([uint32]$proc.Id)
}
if ($hwnd -eq [IntPtr]::Zero) {
    throw "native host window was not created"
}
[EmbedCapture]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Milliseconds 2500

$rect = New-Object EmbedCapture+RECT
[EmbedCapture]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
$bmp = New-Object System.Drawing.Bitmap(($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top))
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bmp.Size)
$bmp.Save($outPath, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()
$bmp.Dispose()

$child = $null
$childDeadline = (Get-Date).AddSeconds(4)
while (-not (Test-Path $childReport) -and (Get-Date) -lt $childDeadline) {
    Start-Sleep -Milliseconds 100
}
if (Test-Path $childReport) {
    $child = Get-Content $childReport -Raw | ConvertFrom-Json
} else {
    Write-Host "--- child stderr ---"; Get-Content $childErr -ErrorAction SilentlyContinue
    Write-Host "--- child stdout ---"; Get-Content $childOut -ErrorAction SilentlyContinue
}

$result = [ordered]@{
    host_title = "Arcline Agent Client — Slint child host"
    host_hwnd = ("0x{0:x}" -f $hwnd.ToInt64())
    child = $child
    screenshot = $outPath
}
$result | ConvertTo-Json -Depth 8
$result | ConvertTo-Json -Depth 8 | Set-Content $reportPath
if (-not $proc.HasExited) {
    try { $proc.Kill() } catch {}
}
