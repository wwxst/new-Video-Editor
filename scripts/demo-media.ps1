# Download and prepare the real-world demo media used by the sample timeline.
#
# Network is required. The script downloads the Commons derivatives listed in assets/demo/README.md,
# trims the selected sections and re-encodes them into assets/demo. It does not generate test
# patterns, Mandelbrot renders or synthetic speech.
#
#   pwsh -NoProfile -File scripts/demo-media.ps1
#   pwsh -NoProfile -File scripts/demo-media.ps1 -Stage D:\tmp\arcline-media -Out assets\demo
#
# FFmpeg is located through $env:ARCLINE_FFMPEG_DIR when set, otherwise through the pinned LGPL
# toolchain at %USERPROFILE%\.local\toolchains\ffmpeg-9.1-lgpl\bin, otherwise from PATH.

[CmdletBinding()]
param(
    [string]$Stage = (Join-Path $env:TEMP 'arcline-demo-media'),
    [string]$Out = (Join-Path (Split-Path -Parent $PSScriptRoot) 'assets\demo')
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Stage, $Out | Out-Null

function Find-Ffmpeg {
    $candidates = @()
    if ($env:ARCLINE_FFMPEG_DIR) { $candidates += (Join-Path $env:ARCLINE_FFMPEG_DIR 'bin\ffmpeg.exe') }
    $candidates += (Join-Path $env:USERPROFILE '.local\toolchains\ffmpeg-9.1-lgpl\bin\ffmpeg.exe')
    $candidates += (Join-Path $env:USERPROFILE '.local\toolchains\ffmpeg-8.1-lgpl\bin\ffmpeg.exe')
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    $command = Get-Command ffmpeg -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    throw 'ffmpeg.exe not found: set ARCLINE_FFMPEG_DIR or install FFmpeg on PATH.'
}

$ffmpeg = Find-Ffmpeg
$encoders = (& $ffmpeg -hide_banner -encoders 2>$null) -join "`n"
if ($encoders -match '\blibx264\b') {
    $videoArgs = @('-c:v', 'libx264', '-preset', 'medium', '-crf', '23', '-pix_fmt', 'yuv420p')
    $encoderName = 'libx264'
} elseif ($encoders -match '\bh264_qsv\b') {
    $videoArgs = @('-c:v', 'h264_qsv', '-preset', 'medium', '-b:v', '900k', '-maxrate', '1200k', '-bufsize', '1800k', '-pix_fmt', 'yuv420p')
    $encoderName = 'h264_qsv'
} else {
    throw 'No H.264 encoder found (need libx264 or h264_qsv).'
}
Write-Host "FFmpeg: $ffmpeg"
Write-Host "H.264 encoder: $encoderName"

$downloads = @(
    @{ Name = 'people_market_street.webm'; Url = 'https://upload.wikimedia.org/wikipedia/commons/transcoded/6/64/2012-07-18_Market_Street_-_San_Francisco.webm/2012-07-18_Market_Street_-_San_Francisco.webm.480p.vp9.webm' },
    @{ Name = 'train_mishawum.webm'; Url = 'https://upload.wikimedia.org/wikipedia/commons/transcoded/9/99/Outbound_train_passing_through_Mishawum_station.webm/Outbound_train_passing_through_Mishawum_station.webm.480p.vp9.webm' },
    @{ Name = 'tamale_market.webm'; Url = 'https://upload.wikimedia.org/wikipedia/commons/transcoded/9/9f/Tamale_Market_%28No_Audio%29.webm/Tamale_Market_%28No_Audio%29.webm.480p.vp9.webm' },
    @{ Name = 'ocean_waves.webm'; Url = 'https://upload.wikimedia.org/wikipedia/commons/transcoded/9/9b/Ocean_waves_at_L%C3%A6kjavik_beach%2C_Iceland.webm/Ocean_waves_at_L%C3%A6kjavik_beach%2C_Iceland.webm.480p.vp9.webm' },
    @{ Name = 'birds_ferry.webm'; Url = 'https://upload.wikimedia.org/wikipedia/commons/transcoded/0/00/Birds_flying_behind_a_ferry.webm/Birds_flying_behind_a_ferry.webm.480p.vp9.webm' },
    @{ Name = 'mountain_lake.jpg'; Url = 'https://thumb.wikimedia.org/wikipedia/commons/thumb/0/03/Sunrise_at_Maligne_lake.jpg/1280px-Sunrise_at_Maligne_lake.jpg' },
    @{ Name = 'studio_portrait.jpg'; Url = 'https://upload.wikimedia.org/wikipedia/commons/8/8a/Acquanetta_Studio_Portrait_Photograph_1944_Universal_Pictures_Jungle_Woman_by_Ray_Jones.jpg' },
    @{ Name = 'piano_theme.wav'; Url = 'https://upload.wikimedia.org/wikipedia/commons/5/57/Relaxing-piano-music-351844.wav' },
    @{ Name = 'librivox_pooh_00.mp3'; Url = 'https://upload.wikimedia.org/wikipedia/commons/f/f0/LibriVox_-_Winnie-the-Pooh_%281926%29_00.mp3' }
)

foreach ($item in $downloads) {
    $target = Join-Path $Stage $item.Name
    if (Test-Path -LiteralPath $target) {
        Write-Host "cached  $($item.Name)"
        continue
    }
    Write-Host "download $($item.Name)"
    & curl.exe -sL --fail --max-time 600 -o $target $item.Url
    if ($LASTEXITCODE -ne 0) { throw "download failed: $($item.Name)" }
}

# Five real video clips: people, landscape, moving objects.
$videoJobs = @(
    @{ Out = 'city_street.mp4';   Src = 'people_market_street.webm'; Start = 0;  Seconds = 5.5; Audio = $true },
    @{ Out = 'train_station.mp4'; Src = 'train_mishawum.webm';       Start = 17; Seconds = 5.5; Audio = $true },
    @{ Out = 'market_crowd.mp4';  Src = 'tamale_market.webm';        Start = 0;  Seconds = 4.5; Audio = $false },
    @{ Out = 'ocean_cliffs.mp4';  Src = 'ocean_waves.webm';          Start = 0;  Seconds = 5.5; Audio = $false },
    @{ Out = 'seagulls.mp4';      Src = 'birds_ferry.webm';          Start = 9;  Seconds = 5.5; Audio = $false }
)

foreach ($job in $videoJobs) {
    $source = Join-Path $Stage $job.Src
    $target = Join-Path $Out $job.Out
    $audioArgs = if ($job.Audio) { @('-c:a', 'aac', '-b:a', '96k', '-ac', '1', '-ar', '44100') } else { @('-an') }
    Write-Host "encode  $($job.Out)"
    $ffmpegArgs = @('-hide_banner', '-loglevel', 'error', '-y', '-ss', $job.Start, '-t', $job.Seconds, '-i', $source, '-vf', 'scale=640:360,fps=24') +
        $videoArgs + $audioArgs + @('-g', '48', '-movflags', '+faststart', $target)
    & $ffmpeg @ffmpegArgs
    if ($LASTEXITCODE -ne 0) { throw "encode failed: $($job.Out)" }
}

# Background music: first 20 s, faded, mono 44.1 kHz PCM.
Write-Host 'encode  piano_theme.wav'
& $ffmpeg -hide_banner -loglevel error -y -ss 0 -t 20 -i (Join-Path $Stage 'piano_theme.wav') `
    -af 'afade=t=in:st=0:d=1,afade=t=out:st=18.5:d=1.5,volume=0.85' `
    -ac 1 -ar 44100 -c:a pcm_s16le (Join-Path $Out 'piano_theme.wav')
if ($LASTEXITCODE -ne 0) { throw 'encode failed: piano_theme.wav' }

# Human voice narration: a LibriVox reading, 30-48 s, speech-band filtered and limited.
Write-Host 'encode  narration.wav'
& $ffmpeg -hide_banner -loglevel error -y -ss 30 -t 18 -i (Join-Path $Stage 'librivox_pooh_00.mp3') `
    -af 'highpass=f=100,volume=3.0,alimiter=limit=0.9,afade=t=in:st=0:d=0.4,afade=t=out:st=17.2:d=0.8' `
    -ac 1 -ar 44100 -c:a pcm_s16le (Join-Path $Out 'narration.wav')
if ($LASTEXITCODE -ne 0) { throw 'encode failed: narration.wav' }

# The still decoder handles PNG reliably; keep the real photograph pixels and convert the container.
Write-Host 'encode  mountain_lake.png'
& $ffmpeg -hide_banner -loglevel error -y -i (Join-Path $Stage 'mountain_lake.jpg') -frames:v 1 (Join-Path $Out 'mountain_lake.png')
if ($LASTEXITCODE -ne 0) { throw 'encode failed: mountain_lake.png' }
Write-Host 'encode  studio_portrait.png'
& $ffmpeg -hide_banner -loglevel error -y -i (Join-Path $Stage 'studio_portrait.jpg') -frames:v 1 (Join-Path $Out 'studio_portrait.png')
if ($LASTEXITCODE -ne 0) { throw 'encode failed: studio_portrait.png' }

Write-Host ''
Write-Host 'Wrote:'
Get-ChildItem -LiteralPath $Out -File | Where-Object { $_.Name -ne 'README.md' } |
    Sort-Object Name |
    ForEach-Object {
        $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLower()
        '{0,-22} {1,9} bytes  {2}' -f $_.Name, $_.Length, $hash
    }
