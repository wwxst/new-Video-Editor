#requires -Version 5.1
<#
.SYNOPSIS
    Generates the self-made demo media set used by the Arcline timeline UI.

.DESCRIPTION
    Produces, from scratch and with no third-party/stock assets:
      assets/demo/main.mp4       H.264, 960x540, 24 fps, 12 s, no audio
      assets/demo/broll.mp4      H.264, 960x540, 24 fps, 8 s, no audio
      assets/demo/poster.png     480x720 portrait PNG title card
      assets/demo/voice.wav      44100 Hz 16-bit mono SAPI speech, >= 6 s
      assets/demo/narration.wav  44100 Hz 16-bit mono SAPI speech, >= 8 s

    Video imagery comes from FFmpeg's built-in generative filters
    (mandelbrot and life), audio from Windows SAPI text-to-speech.
    Poster typography is composited with System.Drawing because this
    pinned LGPL FFmpeg build has no drawtext filter.

    The script is idempotent: it overwrites its outputs on every run.

.PARAMETER OutputDir
    Destination directory. Defaults to <repo>/assets/demo.

.EXAMPLE
    pwsh -File scripts/demo-media.ps1
    $env:ARCLINE_FFMPEG_DIR = 'C:\tools\ffmpeg\bin'; .\scripts\demo-media.ps1
#>
[CmdletBinding()]
param(
    [string]$OutputDir
)

$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# 0. Locate FFmpeg (explicit env var first, then the pinned LGPL toolchain)
# ---------------------------------------------------------------------------
if ([string]::IsNullOrWhiteSpace($env:ARCLINE_FFMPEG_DIR)) {
    $ffmpegDir = Join-Path $env:USERPROFILE '.local\toolchains\ffmpeg-9.1-lgpl\bin'
} else {
    $ffmpegDir = $env:ARCLINE_FFMPEG_DIR
}

$ffmpeg = Join-Path $ffmpegDir 'ffmpeg.exe'
$ffprobe = Join-Path $ffmpegDir 'ffprobe.exe'
if (-not (Test-Path -LiteralPath $ffmpeg -PathType Leaf)) {
    throw ("FFmpeg not found at '$ffmpeg'. Install the pinned LGPL build there " +
           "or set ARCLINE_FFMPEG_DIR to a directory containing ffmpeg.exe/ffprobe.exe.")
}

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
    $OutputDir = Join-Path $repoRoot 'assets\demo'
}
if (-not (Test-Path -LiteralPath $OutputDir)) {
    New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
}
$OutputDir = (Resolve-Path -LiteralPath $OutputDir).Path

# ffmpeg is used only for its own generative sources, so a minimal
# (no-network) build such as the pinned one is sufficient.
$ffBase = @('-hide_banner', '-loglevel', 'error', '-nostdin', '-y')
$filterList = (& $ffmpeg -hide_banner -filters 2>&1 | Out-String)

# ---------------------------------------------------------------------------
# 1. Pick a working H.264 encoder.
#    The pinned LGPL build has no libx264 (x264 is GPL), so on this machine
#    the Intel Quick Sync encoder (h264_qsv) is used instead.  The fallback
#    order keeps the script usable on other machines.
# ---------------------------------------------------------------------------
$encoderList = (& $ffmpeg -hide_banner -encoders 2>&1 | Out-String)

function Test-H264Encoder {
    param([Parameter(Mandatory = $true)][string]$Name)
    $probe = Join-Path $env:TEMP ('arcline-encprobe-' + $Name + '.mp4')
    if (Test-Path -LiteralPath $probe) { Remove-Item -LiteralPath $probe -Force }
    & $ffmpeg -hide_banner -loglevel error -nostdin -y -f lavfi `
        -i 'mandelbrot=size=160x90:rate=24:end_pts=1' -frames:v 2 `
        -c:v $Name -pix_fmt yuv420p $probe 2>$null | Out-Null
    $ok = ($LASTEXITCODE -eq 0) -and (Test-Path -LiteralPath $probe) -and
          ((Get-Item -LiteralPath $probe).Length -gt 0)
    if (Test-Path -LiteralPath $probe) { Remove-Item -LiteralPath $probe -Force }
    return $ok
}

$encoderCandidates = @()
if ($encoderList -match '(?m)^\s*V\S*\s+libx264\s') { $encoderCandidates += 'libx264' }
$encoderCandidates += @('h264_qsv', 'h264_nvenc', 'h264_amf')

$h264Encoder = $null
foreach ($candidate in $encoderCandidates) {
    if ($encoderList -notmatch ('(?m)^\s*V\S*\s+' + [regex]::Escape($candidate) + '\s')) { continue }
    if (Test-H264Encoder -Name $candidate) { $h264Encoder = $candidate; break }
}
if (-not $h264Encoder) {
    throw ('No usable H.264 encoder in this FFmpeg build. Tried: ' + ($encoderCandidates -join ', ') +
           '. Build FFmpeg with libx264 (GPL) or supply a working hardware H.264 encoder.')
}
Write-Host ("Using H.264 encoder: {0}" -f $h264Encoder)

# Encodes one clip from a lavfi filter graph.  Bitrate-capped so the file
# stays within the size budget no matter which encoder won.
function Invoke-VideoEncode {
    param(
        [Parameter(Mandatory = $true)][string]$FilterGraph,
        [Parameter(Mandatory = $true)][string]$OutFile,
        [Parameter(Mandatory = $true)][int]$Seconds,
        [Parameter(Mandatory = $true)][int]$BitrateK
    )
    $ffArgs = $ffBase + @('-f', 'lavfi', '-i', $FilterGraph, '-t', "$Seconds", '-an')
    if ($h264Encoder -eq 'libx264') {
        $ffArgs += @('-c:v', 'libx264', '-preset', 'medium', '-crf', '23',
                     '-maxrate', ("{0}k" -f [int]($BitrateK * 1.35)),
                     '-bufsize', ("{0}k" -f [int]($BitrateK * 2)),
                     '-g', '48', '-pix_fmt', 'yuv420p')
    } else {
        $ffArgs += @('-c:v', $h264Encoder, '-preset', 'medium',
                     '-b:v', "${BitrateK}k",
                     '-maxrate', ("{0}k" -f [int]($BitrateK * 1.35)),
                     '-bufsize', ("{0}k" -f [int]($BitrateK * 2)),
                     '-g', '48', '-pix_fmt', 'yuv420p')
    }
    $ffArgs += @('-movflags', '+faststart', $OutFile)
    & $ffmpeg @ffArgs
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $OutFile)) {
        throw ("FFmpeg failed while writing '$OutFile' (exit $LASTEXITCODE).")
    }
}

# ---------------------------------------------------------------------------
# 2. Video 1 - main timeline clip: slow Mandelbrot dive with drifting hue.
#    start_scale 0.12 -> end_scale 0.002 over 288 frames (12 s at 24 fps)
#    keeps the frame full of detail from first to last second while the
#    composition clearly changes.
# ---------------------------------------------------------------------------
$mainGraph = 'mandelbrot=size=960x540:rate=24:start_x=-0.101066:start_y=0.956291:start_scale=0.12:end_scale=0.002:end_pts=288:maxiter=3000,format=yuv420p,hue=h=25*sin(2*PI*t/12):s=1.05,vignette=PI/5'
Invoke-VideoEncode -FilterGraph $mainGraph -OutFile (Join-Path $OutputDir 'main.mp4') -Seconds 12 -BitrateK 2300
Write-Host 'Wrote main.mp4'

# ---------------------------------------------------------------------------
# 3. Video 2 - B-roll: Mandelbrot zoom OUT from a filament into the whole set,
#    with a counter-phase hue drift. 8 s at 24 fps, a different character and
#    motion direction from the main clip.
# ---------------------------------------------------------------------------
$brollGraph = 'mandelbrot=size=960x540:rate=24:start_x=0.292:start_y=0.0145:start_scale=0.02:end_scale=1.2:end_pts=192:maxiter=2000,format=yuv420p,hue=h=-30*sin(2*PI*t/8):s=1.1,vignette=PI/5'
Invoke-VideoEncode -FilterGraph $brollGraph -OutFile (Join-Path $OutputDir 'broll.mp4') -Seconds 8 -BitrateK 1200
Write-Host 'Wrote broll.mp4'

# ---------------------------------------------------------------------------
# 4. Poster - portrait 480x720 Mandelbrot still plus typography.
#    drawtext is not compiled into the pinned LGPL build, so the text is
#    composited with System.Drawing (GDI+) using arialbd.ttf when present.
# ---------------------------------------------------------------------------
$posterPath = Join-Path $OutputDir 'poster.png'
$posterBase = Join-Path $env:TEMP 'arcline-poster-base.png'
$posterGraph = 'mandelbrot=size=480x720:rate=1:start_x=-0.101066:start_y=0.956291:start_scale=0.06:end_scale=0.06:end_pts=1:maxiter=3000,vignette=PI/4'
& $ffmpeg @ffBase -f lavfi -i $posterGraph -frames:v 1 -compression_level 9 $posterBase
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $posterBase)) {
    throw 'FFmpeg failed while rendering the poster base image.'
}

$arialBold = 'C:\Windows\Fonts\arialbd.ttf'
if (Test-Path -LiteralPath $arialBold) {
    Add-Type -AssemblyName System.Drawing
    $bmp = New-Object System.Drawing.Bitmap 480, 720, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $base = [System.Drawing.Image]::FromFile($posterBase)
    $g.DrawImage($base, 0, 0, 480, 720)

    # Bottom scrim so the title stays legible over the fractal.
    $scrimRect = New-Object System.Drawing.Rectangle 0, 360, 480, 360
    $transparent = [System.Drawing.Color]::FromArgb(0, 0, 0, 0)
    $dark = [System.Drawing.Color]::FromArgb(246, 0, 0, 0)
    $scrim = New-Object System.Drawing.Drawing2D.LinearGradientBrush `
        $scrimRect, $transparent, $dark, ([System.Drawing.Drawing2D.LinearGradientMode]::Vertical)
    $g.FillRectangle($scrim, $scrimRect)

    $white = [System.Drawing.Color]::FromArgb(255, 255, 255, 255)
    $muted = [System.Drawing.Color]::FromArgb(255, 170, 178, 188)
    $accent = [System.Drawing.Color]::FromArgb(255, 255, 166, 64)

    function Draw-SpacedText {
        param($Graphics, [string]$Text, $Font, $Color, [double]$X, [double]$Y, [double]$Spacing)
        $cursor = $X
        foreach ($ch in $Text.ToCharArray()) {
            $s = [string]$ch
            $Graphics.DrawString($s, $Font, (New-Object System.Drawing.SolidBrush $Color), $cursor, $Y)
            $cursor += $Graphics.MeasureString($s, $Font).Width + $Spacing
        }
    }

    $fontLabel = New-Object System.Drawing.Font 'Arial', 11, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)
    $fontTitle = New-Object System.Drawing.Font 'Arial', 62, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)
    $fontSub = New-Object System.Drawing.Font 'Arial', 15, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
    $fontFoot = New-Object System.Drawing.Font 'Arial', 10, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)

    Draw-SpacedText -Graphics $g -Text 'ARCLINE / DEMO ASSET' -Font $fontLabel -Color $muted -X 36 -Y 38 -Spacing 2.2
    $g.DrawString('ARCLINE', $fontTitle, (New-Object System.Drawing.SolidBrush $white), 32, 536)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush $accent), 36, 626, 64, 3)
    Draw-SpacedText -Graphics $g -Text 'TIMELINE DEMO' -Font $fontSub -Color $white -X 36 -Y 642 -Spacing 3.0
    $g.DrawString('FFMPEG 9.1 / WINDOWS SAPI / LOCALLY GENERATED', $fontFoot, (New-Object System.Drawing.SolidBrush $muted), 36, 676)

    $bmp.Save($posterPath, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $base.Dispose(); $bmp.Dispose()
    Write-Host 'Wrote poster.png (with title overlay)'
} else {
    Move-Item -LiteralPath $posterBase -Destination $posterPath -Force
    Write-Host 'Wrote poster.png (no arialbd.ttf found, text overlay skipped)'
}
if (Test-Path -LiteralPath $posterBase) { Remove-Item -LiteralPath $posterBase -Force }

# ---------------------------------------------------------------------------
# 5. Voice-over - Windows SAPI text to speech, 44100 Hz 16-bit mono.
#    Fallback: flite (if compiled in), then a synthetic speech-like signal.
# ---------------------------------------------------------------------------
$voiceText = 'Welcome to Arcline. This is the demo timeline. Video, images and audio are decoded from real files.'
$narrationText = 'Every cut tells a story. Drag a clip, trim the edges, and watch the playhead move. Sound and picture stay in sync, frame by frame.'

function Get-SapiVoices {
    Add-Type -AssemblyName System.Speech
    $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
    $voices = @($synth.GetInstalledVoices() | Where-Object { $_.Enabled } | ForEach-Object { $_.VoiceInfo })
    $synth.Dispose()
    return $voices
}

function New-SapiWav {
    param([string]$Path, [string]$Text, [string]$VoiceName, [int]$Rate)
    Add-Type -AssemblyName System.Speech
    $fmtArgs = @(44100,
        [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,
        [System.Speech.AudioFormat.AudioChannel]::Mono)
    $fmt = New-Object -TypeName System.Speech.AudioFormat.SpeechAudioFormatInfo -ArgumentList $fmtArgs
    $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
    if (-not [string]::IsNullOrWhiteSpace($VoiceName)) { $synth.SelectVoice($VoiceName) }
    $synth.Rate = $Rate
    $synth.SetOutputToWaveFile($Path, $fmt)
    try {
        $synth.Speak($Text)
    } finally {
        $synth.Dispose()
    }
}

$sapiVoices = @()
try {
    $sapiVoices = Get-SapiVoices
} catch {
    Write-Warning ('Windows SAPI unavailable: ' + $_.Exception.Message)
}

if ($sapiVoices.Count -gt 0) {
    $english = @($sapiVoices | Where-Object { $_.Culture.Name -like 'en-*' })
    $primary = $null
    if ($english.Count -gt 0) { $primary = $english[0] }
    else { $primary = $sapiVoices[0] }
    $secondary = $sapiVoices | Where-Object { $_.Name -ne $primary.Name } | Select-Object -First 1
    if ($null -eq $secondary) { $secondary = $primary }

    New-SapiWav -Path (Join-Path $OutputDir 'voice.wav') -Text $voiceText -VoiceName $primary.Name -Rate 0
    if ($secondary.Name -eq $primary.Name) {
        # No second voice installed: keep the same voice but slow it down so
        # the two clips still differ in pacing and dynamics.
        New-SapiWav -Path (Join-Path $OutputDir 'narration.wav') -Text $narrationText -VoiceName $primary.Name -Rate -2
    } else {
        New-SapiWav -Path (Join-Path $OutputDir 'narration.wav') -Text $narrationText -VoiceName $secondary.Name -Rate 0
    }
    Write-Host ("Wrote voice.wav ({0}) and narration.wav ({1})" -f $primary.Name, $secondary.Name)
} elseif ($filterList -match '(?m)^\s*\.\.\s+flite\s') {
    $voiceTxt = Join-Path $env:TEMP 'arcline-voice.txt'
    $narrTxt = Join-Path $env:TEMP 'arcline-narration.txt'
    Set-Content -LiteralPath $voiceTxt -Value $voiceText -Encoding ASCII
    Set-Content -LiteralPath $narrTxt -Value $narrationText -Encoding ASCII
    & $ffmpeg @ffBase -f lavfi -i ('flite=textfile=' + $voiceTxt + ':voice=slt') -ar 44100 -ac 1 -c:a pcm_s16le (Join-Path $OutputDir 'voice.wav')
    & $ffmpeg @ffBase -f lavfi -i ('flite=textfile=' + $narrTxt + ':voice=kal') -ar 44100 -ac 1 -c:a pcm_s16le (Join-Path $OutputDir 'narration.wav')
    Remove-Item -LiteralPath $voiceTxt, $narrTxt -Force
    Write-Warning 'SAPI unavailable; used the FFmpeg flite filter instead.'
} else {
    # Last resort: speech-like modulated tone with loud/quiet passages.
    $voiceGraph = 'aevalsrc=0.16*sin(2*PI*150*t)*(0.55+0.45*sin(2*PI*4.5*t))*if(lt(mod(t\,1.5)\,1.0)\,1\,0):s=44100:d=7'
    $narrGraph = 'aevalsrc=0.13*sin(2*PI*135*t)*(0.5+0.5*sin(2*PI*3.5*t))*if(lt(mod(t\,2.1)\,1.4)\,1\,0):s=44100:d=9'
    & $ffmpeg @ffBase -f lavfi -i $voiceGraph -ar 44100 -ac 1 -c:a pcm_s16le (Join-Path $OutputDir 'voice.wav')
    & $ffmpeg @ffBase -f lavfi -i $narrGraph -ar 44100 -ac 1 -c:a pcm_s16le (Join-Path $OutputDir 'narration.wav')
    Write-Warning 'No SAPI voice and no flite filter; synthesised a speech-like signal instead.'
}

# ---------------------------------------------------------------------------
# 6. Summary and sanity checks.
# ---------------------------------------------------------------------------
Write-Host ''
Write-Host ('Demo media written to: ' + $OutputDir)
$totalBytes = 0
foreach ($name in 'main.mp4', 'broll.mp4', 'poster.png', 'voice.wav', 'narration.wav') {
    $path = Join-Path $OutputDir $name
    if (-not (Test-Path -LiteralPath $path)) { throw "Expected output missing: $path" }
    $item = Get-Item -LiteralPath $path
    $totalBytes += $item.Length
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    $duration = ''
    if ($name -ne 'poster.png') {
        $duration = (& $ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 $path)
    }
    Write-Host ('  {0,-14} {1,10} bytes {2,8}  {3}' -f $name, $item.Length, $duration, $hash)
}

$limits = @{ 'main.mp4' = 4MB; 'broll.mp4' = 3MB; 'poster.png' = 1.5MB }
foreach ($name in $limits.Keys) {
    $len = (Get-Item -LiteralPath (Join-Path $OutputDir $name)).Length
    if ($len -gt $limits[$name]) { Write-Warning ("$name is $len bytes, over the $($limits[$name]) byte target.") }
}
Write-Host ('  total          {0,10} bytes (limit 12 MB)' -f $totalBytes)
if ($totalBytes -gt 12MB) { throw 'assets/demo exceeds the 12 MB budget.' }
Write-Host 'Done.'
