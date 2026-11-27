# Generic external measurement: run a probe binary, sample working set and CPU from outside,
# store its stdout/stderr and the timings next to the JSON it wrote.
#
#   pwsh -File measure.ps1 -Exe ..\media-validation\target\release\shim-probe.exe `
#        -Arguments "--file X --mode video-seq --frames 300" -Tag shim-video
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Arguments = "",
    [Parameter(Mandatory = $true)][string]$Tag,
    [string]$OutDir = ".",
    [int]$TimeoutSeconds = 300
)

$stdout = Join-Path $OutDir "$Tag.stdout.txt"
$stderr = Join-Path $OutDir "$Tag.stderr.txt"
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $Exe -ArgumentList $Arguments -PassThru -NoNewWindow -RedirectStandardOutput $stdout -RedirectStandardError $stderr
$peak = 0
while (-not $p.HasExited) {
    try {
        $p.Refresh()
        $ws = $p.WorkingSet64
        if ($ws -gt $peak) { $peak = $ws }
    } catch {}
    if ($sw.Elapsed.TotalSeconds -gt $TimeoutSeconds) { try { $p.Kill() } catch {}; break }
    Start-Sleep -Milliseconds 30
}
$sw.Stop()
$cpu = 0
try { $cpu = $p.TotalProcessorTime.TotalSeconds } catch {}
$json = $null
if (Test-Path $stdout) {
    $text = Get-Content $stdout -Raw
    if ($text) { try { $json = $text | ConvertFrom-Json } catch { $json = $text } }
}
$summary = [ordered]@{
    tag         = $Tag
    exe         = (Split-Path -Leaf $Exe)
    arguments   = $Arguments
    exit        = $p.ExitCode
    wall_s      = [math]::Round($sw.Elapsed.TotalSeconds, 3)
    cpu_s       = [math]::Round($cpu, 3)
    peak_rss_mb = [math]::Round($peak / 1MB, 1)
    result      = $json
}
$summary | ConvertTo-Json -Depth 10 | Set-Content -Path (Join-Path $OutDir "$Tag.summary.json")
Write-Output ("{0}: exit={1} wall={2:N2}s cpu={3:N2}s peak={4:N1}MB" -f $Tag, $p.ExitCode, $sw.Elapsed.TotalSeconds, $cpu, ($peak / 1MB))
if ($json -and $json -isnot [string]) { Write-Output ($json | ConvertTo-Json -Compress -Depth 6) }
