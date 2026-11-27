# UI POC measurement harness: runs one binary, samples CPU and peak working set from outside the
# process, and stores the JSON the binary wrote plus the external figures.
#
#   pwsh -File run-ui.ps1 -Exe ..\ui-validation\target\release\egui-poc.exe -Label egui -Clips 1000 `
#        -Frames 600 -VideoMode none -Interact scroll -Tag egui-1000-none-scroll
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Label,
    [Parameter(Mandatory = $true)][int]$Clips,
    [int]$Frames = 600,
    [string]$VideoMode = "none",
    [string]$Interact = "scroll",
    [Parameter(Mandatory = $true)][string]$Tag,
    [string]$OutDir = ".",
    [string]$Zoom = "60"
)

$out = Join-Path $OutDir "$Tag.json"
$shot = Join-Path $OutDir "$Tag.ppm"
$args = @(
    "--clips", "$Clips",
    "--frames", "$Frames",
    "--video-mode", $VideoMode,
    "--interact", $Interact,
    "--zoom", "$Zoom",
    "--out", $out
)
if ($Label -eq "slint") { $args += @("--shot", $shot) }

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $Exe -ArgumentList $args -PassThru -WindowStyle Normal
$peak = 0
$samples = 0
while (-not $p.HasExited) {
    try {
        $p.Refresh()
        $ws = $p.WorkingSet64
        if ($ws -gt $peak) { $peak = $ws }
        $samples++
    } catch {}
    Start-Sleep -Milliseconds 40
}
$sw.Stop()
$cpu = $p.TotalProcessorTime.TotalSeconds
$code = $p.ExitCode
$json = $null
if (Test-Path $out) { $json = Get-Content $out -Raw | ConvertFrom-Json }
$summary = [ordered]@{
    tag         = $Tag
    label       = $Label
    clips       = $Clips
    frames      = $Frames
    video       = $VideoMode
    interact    = $Interact
    zoom        = [double]$Zoom
    exit        = $code
    wall_s      = [math]::Round($sw.Elapsed.TotalSeconds, 3)
    cpu_s       = [math]::Round($cpu, 3)
    peak_rss_mb = [math]::Round($peak / 1MB, 1)
    samples     = $samples
    result      = $json
}
$summary | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $OutDir "$Tag.summary.json")
Write-Output ("{0}: exit={1} wall={2:N2}s cpu={3:N2}s peak={4:N1}MB" -f $Tag, $code, $sw.Elapsed.TotalSeconds, $cpu, ($peak / 1MB))
if ($json) {
    Write-Output ("  steady p50={0}us p95={1}us max={2}us fps(mean)={3:N0}" -f $json.steady_us_p50, $json.steady_us_p95, $json.steady_us_max, $json.fps_from_mean)
}
