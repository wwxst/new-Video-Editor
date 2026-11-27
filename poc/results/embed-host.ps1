# Native child-window embedding harness.
#
# Creates a real Win32 host window (the "Agent client"), hands its panel HWND to slint-poc, and
# verifies with GetParent() that the Slint window became a child of that panel. The screenshot is
# taken from the screen, so it contains the host chrome and the embedded child, not a Slint-only
# snapshot.

param(
    [string]$Exe = (Resolve-Path "..\ui-validation\target\release\slint-poc.exe").Path,
    [string]$Out = "embed-host.png",
    [string]$Report = "embed-host.json",
    [int]$Seconds = 8,
    [int]$Width = 1180,
    [int]$Height = 760
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class EmbedWin32 {
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr GetParent(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool SetFocus(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$form = New-Object System.Windows.Forms.Form
$form.Text = "Arcline Agent Client — Slint child host"
$form.Size = New-Object System.Drawing.Size(1220, 880)
$form.StartPosition = "CenterScreen"
$form.BackColor = [System.Drawing.Color]::FromArgb(18, 19, 22)

$header = New-Object System.Windows.Forms.Label
$header.Text = "AGENT CLIENT  /  native child-window host"
$header.Dock = "Top"
$header.Height = 44
$header.ForeColor = [System.Drawing.Color]::FromArgb(230, 232, 236)
$header.BackColor = [System.Drawing.Color]::FromArgb(29, 31, 37)
$header.Font = New-Object System.Drawing.Font("Segoe UI", 10, [System.Drawing.FontStyle]::Bold)
$header.TextAlign = "MiddleLeft"
$header.Padding = New-Object System.Windows.Forms.Padding(16, 0, 0, 0)
$form.Controls.Add($header)

$panel = New-Object System.Windows.Forms.Panel
$panel.Dock = "Fill"
$panel.BackColor = [System.Drawing.Color]::FromArgb(10, 11, 14)
$form.Controls.Add($panel)

$form.Show()
[System.Windows.Forms.Application]::DoEvents()
$hostHwnd = $panel.Handle

$resolvedOut = if ([IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path (Get-Location) $Out }
$resolvedReport = if ([IO.Path]::IsPathRooted($Report)) { $Report } else { Join-Path (Get-Location) $Report }
$childReport = Join-Path $PSScriptRoot "embed-child.json"
$childExe = (Resolve-Path $Exe).Path

$childOut = "$resolvedReport.child.out"
$childErr = "$resolvedReport.child.err"
Remove-Item $childReport -ErrorAction SilentlyContinue
Write-Host "child report: $childReport"
$proc = Start-Process -FilePath $childExe -ArgumentList @(
    "--embed-parent", $hostHwnd.ToInt64(),
    "--embed-seconds", "$Seconds",
    "--width", "$Width",
    "--height", "$Height",
    "--frames", "0",
    "--video-mode", "native",
    "--result", "embed-child.json"
) -PassThru -RedirectStandardOutput $childOut -RedirectStandardError $childErr -WorkingDirectory $PSScriptRoot

$deadline = (Get-Date).AddSeconds(8)
while (-not (Test-Path $childReport) -and (Get-Date) -lt $deadline) {
    [System.Windows.Forms.Application]::DoEvents()
    Start-Sleep -Milliseconds 100
}

if (Test-Path $childReport) {
    $embed = Get-Content $childReport -Raw | ConvertFrom-Json
    $childHwnd = [IntPtr][Convert]::ToInt64(($embed.child_hwnd -replace '^0x', ''), 16)
    [EmbedWin32]::SetFocus($childHwnd) | Out-Null
} else {
    $embed = $null
    Write-Host "--- child stdout ---"; Get-Content $childOut -ErrorAction SilentlyContinue
    Write-Host "--- child stderr ---"; Get-Content $childErr -ErrorAction SilentlyContinue
}

# Keep the host alive long enough for a stable screen capture and for the child input queue to settle.
$until = (Get-Date).AddSeconds([Math]::Max(2, $Seconds - 2))
while ((Get-Date) -lt $until) {
    [System.Windows.Forms.Application]::DoEvents()
    Start-Sleep -Milliseconds 50
}

$form.Activate() | Out-Null
[System.Windows.Forms.Application]::DoEvents()
$bmp = New-Object System.Drawing.Bitmap($form.Width, $form.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($form.Left, $form.Top, 0, 0, (New-Object System.Drawing.Size($form.Width, $form.Height)))
$bmp.Save($resolvedOut, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()
$bmp.Dispose()

$result = [ordered]@{
    host_hwnd = ("0x{0:x}" -f $hostHwnd.ToInt64())
    host_title = $form.Text
    child_exe = $childExe
    child_report = $embed
    screenshot = $resolvedOut
    host_screenshot = $true
}
$result | ConvertTo-Json -Depth 8 | Set-Content $resolvedReport

$form.Close()
[System.Windows.Forms.Application]::DoEvents()
if (-not $proc.HasExited) {
    try { $proc.Kill() } catch {}
}
Write-Output ($result | ConvertTo-Json -Depth 8)
