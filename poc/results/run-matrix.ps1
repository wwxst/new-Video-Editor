# UI measurement matrix: egui and Slint draw the same scene; every run is measured from outside.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$egui = (Resolve-Path "..\ui-validation\target\release\egui-poc.exe").Path
$slint = (Resolve-Path "..\ui-validation\target\release\slint-poc.exe").Path

function Measure-Run {
    param($Exe, $Label, $Clips, $Frames, $Video, $Interact, $Tag)
    $out = "$Tag.json"
    $shot = "$Tag.ppm"
    $a = @("--clips", "$Clips", "--frames", "$Frames", "--video-mode", $Video,
        "--interact", $Interact, "--zoom", "60", "--out", $out)
    if ($Label -eq "slint") { $a += @("--shot", $shot) }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $Exe -ArgumentList $a -PassThru -WorkingDirectory $PSScriptRoot
    $peak = 0
    while (-not $p.HasExited) {
        try {
            $p.Refresh()
            if ($p.WorkingSet64 -gt $peak) { $peak = $p.WorkingSet64 }
        } catch {}
        Start-Sleep -Milliseconds 40
    }
    $sw.Stop()
    $cpu = $p.TotalProcessorTime.TotalSeconds
    $code = $p.ExitCode
    $j = $null
    if (Test-Path $out) { $j = Get-Content $out -Raw | ConvertFrom-Json }
    [ordered]@{
        tag = $Tag; label = $Label; clips = $Clips; video = $Video; interact = $Interact
        exit = $code; wall_s = [math]::Round($sw.Elapsed.TotalSeconds, 3)
        cpu_s = [math]::Round($cpu, 3); peak_rss_mb = [math]::Round($peak / 1MB, 1)
        result = $j
    } | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $PSScriptRoot "$Tag.summary.json")
    if ($j) {
        "{0}: {1} clips {2} {3} -> p50={4}us p95={5}us max={6}us wall={7:N2}s cpu={8:N2}s peak={9:N1}MB" -f `
            $Tag, $Clips, $Video, $Interact, $j.steady_us_p50, $j.steady_us_p95, $j.steady_us_max, `
            $sw.Elapsed.TotalSeconds, $cpu, ($peak / 1MB)
    }
    else { "{0}: no result (exit {1})" -f $Tag, $code }
}

$configs = @(
    @{Exe=$egui; Label="egui"; Clips=100;  Video="none";     Interact="scroll"; Tag="egui-100-none-scroll"},
    @{Exe=$egui; Label="egui"; Clips=1000; Video="none";     Interact="scroll"; Tag="egui-1000-none-scroll"},
    @{Exe=$egui; Label="egui"; Clips=4000; Video="none";     Interact="scroll"; Tag="egui-4000-none-scroll"},
    @{Exe=$egui; Label="egui"; Clips=1000; Video="readback"; Interact="scroll"; Tag="egui-1000-readback"},
    @{Exe=$egui; Label="egui"; Clips=1000; Video="native";   Interact="scroll"; Tag="egui-1000-native"},
    @{Exe=$egui; Label="egui"; Clips=1000; Video="none";     Interact="zoom";   Tag="egui-1000-zoom"},
    @{Exe=$slint; Label="slint"; Clips=100;  Video="none";     Interact="scroll"; Tag="slint-100-none-scroll"},
    @{Exe=$slint; Label="slint"; Clips=1000; Video="none";     Interact="scroll"; Tag="slint-1000-none-scroll"},
    @{Exe=$slint; Label="slint"; Clips=4000; Video="none";     Interact="scroll"; Tag="slint-4000-none-scroll"},
    @{Exe=$slint; Label="slint"; Clips=1000; Video="readback"; Interact="scroll"; Tag="slint-1000-readback"},
    @{Exe=$slint; Label="slint"; Clips=1000; Video="native";   Interact="scroll"; Tag="slint-1000-native"},
    @{Exe=$slint; Label="slint"; Clips=1000; Video="none";     Interact="zoom";   Tag="slint-1000-zoom"}
)
foreach ($c in $configs) {
    Measure-Run -Exe $c.Exe -Label $c.Label -Clips $c.Clips -Frames 600 -Video $c.Video -Interact $c.Interact -Tag $c.Tag
}
