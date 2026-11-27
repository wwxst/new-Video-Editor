# Clean and incremental build measurement for the two POC binaries.
# Each package gets its own target directory so dependency compilation is not shared.

param(
    [string]$Workspace = (Resolve-Path "..\ui-validation").Path,
    [string]$OutDir = "."
)

$ErrorActionPreference = "Continue"
$results = @()

function Get-TreeSize($path) {
    if (-not (Test-Path $path)) { return 0 }
    (Get-ChildItem $path -Recurse -File -ErrorAction SilentlyContinue |
        Measure-Object -Property Length -Sum).Sum
}

function Measure-Build($Package, $Label, $TargetDir) {
    $log = Join-Path $OutDir "$Label.build.log"
    $target = Join-Path $Workspace $TargetDir
    if (Test-Path $target) { Remove-Item $target -Recurse -Force }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    Push-Location $Workspace
    $env:CARGO_TARGET_DIR = $target
    & cargo build --release -p $Package 2>&1 | Tee-Object -FilePath $log
    $code = $LASTEXITCODE
    $sw.Stop()
    $env:CARGO_TARGET_DIR = $null
    Pop-Location
    $logLines = (Get-Content $log | Measure-Object -Line).Lines
    $logBytes = (Get-Item $log).Length
    $bin = Join-Path $target "release\$Package.exe"
    [ordered]@{
        label = $Label
        package = $Package
        clean_seconds = [math]::Round($sw.Elapsed.TotalSeconds, 2)
        exit = $code
        log_lines = $logLines
        log_bytes = $logBytes
        target_bytes = Get-TreeSize $target
        binary_bytes = if (Test-Path $bin) { (Get-Item $bin).Length } else { 0 }
    }
}

function Measure-Incremental($Package, $Label, $TargetDir) {
    $target = Join-Path $Workspace $TargetDir
    $src = Join-Path $Workspace "crates\$Package\src\main.rs"
    (Get-Item $src).LastWriteTime = Get-Date
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    Push-Location $Workspace
    $env:CARGO_TARGET_DIR = $target
    $null = & cargo build --release -p $Package 2>&1
    $code = $LASTEXITCODE
    $sw.Stop()
    $env:CARGO_TARGET_DIR = $null
    Pop-Location
    [ordered]@{
        label = "$Label-incremental-rust"
        seconds = [math]::Round($sw.Elapsed.TotalSeconds, 2)
        exit = $code
    }
}

foreach ($cfg in @(
    @{ Package = "egui-poc"; Label = "egui"; Target = "target\measure-egui" },
    @{ Package = "slint-poc"; Label = "slint"; Target = "target\measure-slint" }
)) {
    $r = Measure-Build $cfg.Package $cfg.Label $cfg.Target
    $results += $r
    $results += Measure-Incremental $cfg.Package $cfg.Label $cfg.Target
    if ($cfg.Package -eq "slint-poc") {
        $ui = Join-Path $Workspace "crates\slint-poc\ui\app.slint"
        (Get-Item $ui).LastWriteTime = Get-Date
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        Push-Location $Workspace
        $env:CARGO_TARGET_DIR = (Join-Path $Workspace $cfg.Target)
        $null = & cargo build --release -p $cfg.Package 2>&1
        $sw.Stop()
        $env:CARGO_TARGET_DIR = $null
        Pop-Location
        $results += [ordered]@{
            label = "slint-incremental-slint"
            seconds = [math]::Round($sw.Elapsed.TotalSeconds, 2)
        }
    }
}

$results | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $OutDir "build-matrix.json")
$results | ConvertTo-Json -Depth 6
