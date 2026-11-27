# Startup and idle measurement.
#
# Starts the UI, waits for its first-frame marker, then samples CPU and working set for a fixed
# idle window. The UI is not animating during that window (both POCs stop requesting redraws).

param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Tag,
    [string]$Arguments = "",
    [int]$IdleSeconds = 6,
    [int]$SampleSeconds = 4,
    [string]$OutDir = "."
)

$ErrorActionPreference = "Stop"
$marker = Join-Path $OutDir "$Tag.startup.json"
$result = Join-Path $OutDir "$Tag.result.json"
$stdout = Join-Path $OutDir "$Tag.stdout.txt"
$stderr = Join-Path $OutDir "$Tag.stderr.txt"
Remove-Item $marker, $result -ErrorAction SilentlyContinue

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$p = Start-Process -FilePath $Exe -ArgumentList $Arguments -PassThru -NoNewWindow `
    -RedirectStandardOutput $stdout -RedirectStandardError $stderr
$peak = 0
$deadline = (Get-Date).AddSeconds(20)
while (-not (Test-Path $marker) -and (Get-Date) -lt $deadline) {
    try {
        $p.Refresh()
        if ($p.WorkingSet64 -gt $peak) { $peak = $p.WorkingSet64 }
    } catch {}
    Start-Sleep -Milliseconds 20
}
$startupExternal = $sw.Elapsed.TotalMilliseconds
$startupJson = if (Test-Path $marker) { Get-Content $marker -Raw | ConvertFrom-Json } else { $null }
$cpuAtStart = 0.0
$wsAtStart = 0
try { $p.Refresh(); $cpuAtStart = $p.TotalProcessorTime.TotalSeconds; $wsAtStart = $p.WorkingSet64 } catch {}

# Let the first frame settle, then sample pure idle.
Start-Sleep -Milliseconds 400
try { $p.Refresh(); $cpuAtStart = $p.TotalProcessorTime.TotalSeconds; $wsAtStart = $p.WorkingSet64 } catch {}
$sampleStart = Get-Date
$samples = @()
while (((Get-Date) - $sampleStart).TotalSeconds -lt $SampleSeconds -and -not $p.HasExited) {
    try {
        $p.Refresh()
        if ($p.WorkingSet64 -gt $peak) { $peak = $p.WorkingSet64 }
        $samples += $p.WorkingSet64
    } catch {}
    Start-Sleep -Milliseconds 100
}
$cpuAtEnd = $cpuAtStart
try { $p.Refresh(); $cpuAtEnd = $p.TotalProcessorTime.TotalSeconds } catch {}
$elapsedIdle = ((Get-Date) - $sampleStart).TotalSeconds
$idleCpu = $cpuAtEnd - $cpuAtStart
$avgWs = if ($samples.Count -gt 0) { ($samples | Measure-Object -Average).Average } else { $wsAtStart }

try { if (-not $p.HasExited) { $p.WaitForExit(8000) | Out-Null } } catch {}
if (-not $p.HasExited) { try { $p.Kill() } catch {} }
$sw.Stop()

$appResult = if (Test-Path $result) { Get-Content $result -Raw | ConvertFrom-Json } else { $null }
$summary = [ordered]@{
    tag = $Tag
    exe = (Split-Path -Leaf $Exe)
    startup_external_ms = [math]::Round($startupExternal, 1)
    startup_internal_ms = if ($startupJson) { [math]::Round($startupJson.first_frame_ms, 1) } else { $null }
    idle_s = [math]::Round($elapsedIdle, 2)
    idle_cpu_s = [math]::Round($idleCpu, 4)
    idle_cpu_pct_of_one_core = if ($elapsedIdle -gt 0) { [math]::Round(100.0 * $idleCpu / $elapsedIdle, 2) } else { $null }
    idle_ws_mb = [math]::Round($avgWs / 1MB, 1)
    peak_rss_mb = [math]::Round($peak / 1MB, 1)
    exit = $p.ExitCode
    app = $appResult
}
$summary | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $OutDir "$Tag.summary.json")
$summary | ConvertTo-Json -Depth 8
