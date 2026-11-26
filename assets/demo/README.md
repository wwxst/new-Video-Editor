# Arcline demo media

Self-made demo assets for the Arcline timeline UI. Everything in this folder is
generated locally from FFmpeg's built-in generative filters and Windows SAPI
speech; nothing is downloaded, sampled, or taken from stock/third-party media.

Regenerate (idempotent, overwrites these files):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/demo-media.ps1
# or, if script execution is allowed:
pwsh -File scripts/demo-media.ps1
```

FFmpeg is located through `$env:ARCLINE_FFMPEG_DIR` when set, otherwise through
the pinned LGPL toolchain at `%USERPROFILE%\.local\toolchains\ffmpeg-9.1-lgpl\bin`.
The script fails with an explicit message if `ffmpeg.exe` is missing.

## Summary

| File | Content | Duration | Bytes | SHA-256 |
| --- | --- | --- | --- | --- |
| `main.mp4` | Mandelbrot zoom, H.264 960x540 24 fps, no audio | 12.000000 s | 3,485,784 | `6b40be57b71d1fde6f5e9316a320db976412757c3244cf7c859066ea6982765a` |
| `broll.mp4` | Mandelbrot zoom-out, H.264 960x540 24 fps, no audio | 8.000000 s | 1,220,336 | `dc34c1e2b71009c3f769e5e4ecac24237c510fc25725e01371e5e3d303cba9b3` |
| `poster.png` | Portrait title card, PNG 480x720 | still | 763,612 | `373de5fb9183aaad7b821776907209e5623d29a73958569ea754aa5baab51532` |
| `voice.wav` | SAPI speech, PCM 44100 Hz mono 16-bit | 8.664082 s | 764,218 | `58cc33444609160e620854e666076289033c5c3936dded1e7948e8284020bcc3` |
| `narration.wav` | SAPI speech, PCM 44100 Hz mono 16-bit | 12.518617 s | 1,104,188 | `2964d82c1f05711cf13ace2b8acddfc5ec9474e443dc791c2ef7e0be6ac1b7c7` |

Total: **7,338,138 bytes** (7.0 MiB), well under the 12 MB budget.

## Toolchain

- FFmpeg: `ffmpeg version 9.1-arcline-f414c1c54e`, built LGPL
  (`--disable-gpl --disable-nonfree --disable-network`), from the pinned path
  `%USERPROFILE%\.local\toolchains\ffmpeg-9.1-lgpl\bin`.
- This LGPL build does **not** contain `libx264` (x264 is GPL), so H.264 output
  is produced by the Intel Quick Sync encoder `h264_qsv` (MPEG-4 AVC / H.264,
  High profile, `yuv420p`). The script tries `libx264` first and falls back to
  `h264_qsv`, `h264_nvenc`, `h264_amf`; on this machine it selected `h264_qsv`.
- Speech: Windows SAPI 5.1 through `System.Speech.Synthesis.SpeechSynthesizer`.
  Installed voices: `Microsoft Zira Desktop` (en-US) and
  `Microsoft Huihui Desktop` (zh-CN). `voice.wav` uses Zira; `narration.wav`
  uses Huihui, so the two clips have a different speaker/accent as well as
  different text and pacing. No fallback was needed.
- This build has no `drawtext` filter, so the poster typography is composited
  with `System.Drawing` (GDI+) using `C:/Windows/Fonts/arialbd.ttf`; the
  artwork itself still comes from FFmpeg's `mandelbrot` filter.

## main.mp4

Slow Mandelbrot dive near a filament, 288 frames at 24 fps, with a gentle hue
drift and vignette. The zoom runs from scale 0.12 to 0.002 over the clip, so
the composition and colours change continuously from first to last frame.

Script section: `scripts/demo-media.ps1` step 2. Equivalent command:

```powershell
ffmpeg -hide_banner -loglevel error -nostdin -y -f lavfi `
  -i "mandelbrot=size=960x540:rate=24:start_x=-0.101066:start_y=0.956291:start_scale=0.12:end_scale=0.002:end_pts=288:maxiter=3000,format=yuv420p,hue=h=25*sin(2*PI*t/12):s=1.05,vignette=PI/5" `
  -t 12 -an -c:v h264_qsv -preset medium -b:v 2300k -maxrate 3105k -bufsize 4600k `
  -g 48 -pix_fmt yuv420p -movflags +faststart assets/demo/main.mp4
```

Streams: H.264 (High), 960x540, `yuv420p`, 24/1 fps (constant), 12.000000 s,
no audio stream, 3,485,784 bytes.

## broll.mp4

Mandelbrot zoom **out** from a filament into the whole set (start_scale 0.02 to
end_scale 1.2), with a hue drift in counter-phase to the main clip. 192 frames
at 24 fps - a different location, colour direction and motion direction from
`main.mp4`, so its thumbnails never look like the main track's.

Script section: `scripts/demo-media.ps1` step 3. Equivalent command:

```powershell
ffmpeg -hide_banner -loglevel error -nostdin -y -f lavfi `
  -i "mandelbrot=size=960x540:rate=24:start_x=0.292:start_y=0.0145:start_scale=0.02:end_scale=1.2:end_pts=192:maxiter=2000,format=yuv420p,hue=h=-30*sin(2*PI*t/8):s=1.1,vignette=PI/5" `
  -t 8 -an -c:v h264_qsv -preset medium -b:v 1200k -maxrate 1620k -bufsize 2400k `
  -g 48 -pix_fmt yuv420p -movflags +faststart assets/demo/broll.mp4
```

Streams: H.264 (High), 960x540, `yuv420p`, 24/1 fps, 8.000000 s, no audio,
1,220,336 bytes.

## poster.png

Portrait 2:3 title card. The base frame is a static Mandelbrot render at
480x720; title, rule, subtitle and footer are drawn on top with GDI+.

Script section: `scripts/demo-media.ps1` step 4. Base image command:

```powershell
ffmpeg -hide_banner -loglevel error -nostdin -y -f lavfi `
  -i "mandelbrot=size=480x720:rate=1:start_x=-0.101066:start_y=0.956291:start_scale=0.06:end_scale=0.06:end_pts=1:maxiter=3000,vignette=PI/4" `
  -frames:v 1 -compression_level 9 %TEMP%\arcline-poster-base.png
```

Text overlay: `System.Drawing` 480x720 32-bpp bitmap, vertical black scrim
(alpha 0 at y=360 to 246 at y=720), "ARCLINE" in Arial Bold 62 px, an orange
rule, "TIMELINE DEMO", and the footer "FFMPEG 9.1 / WINDOWS SAPI / LOCALLY
GENERATED", plus the top label "ARCLINE / DEMO ASSET".

Image: PNG, 480x720 (`rgba`), 763,612 bytes. Decodes cleanly with
`ffmpeg -i poster.png -frames:v 1`.

## voice.wav

Windows SAPI, Zira, rate 0, 44.1 kHz 16-bit mono WAV. Text:

> Welcome to Arcline. This is the demo timeline. Video, images and audio are decoded from real files.

Script section: `scripts/demo-media.ps1` step 5. Equivalent PowerShell:

```powershell
Add-Type -AssemblyName System.Speech
$fmt = New-Object -TypeName System.Speech.AudioFormat.SpeechAudioFormatInfo -ArgumentList @(
  44100,
  [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,
  [System.Speech.AudioFormat.AudioChannel]::Mono)
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$synth.SelectVoice('Microsoft Zira Desktop')
$synth.Rate = 0
$synth.SetOutputToWaveFile('assets/demo/voice.wav', $fmt)
$synth.Speak('Welcome to Arcline. This is the demo timeline. Video, images and audio are decoded from real files.')
$synth.Dispose()
```

Audio: `pcm_s16le`, 44100 Hz, mono, 16 bits/sample, 8.664082 s, 764,218 bytes.
`volumedetect`: mean -20.7 dB, max -2.5 dB. Sentence pauses give clearly
separated loud and quiet passages.

## narration.wav

Second SAPI clip with the other installed voice (Huihui) and different text
and pacing. Text:

> Every cut tells a story. Drag a clip, trim the edges, and watch the playhead move. Sound and picture stay in sync, frame by frame.

Script section: `scripts/demo-media.ps1` step 5 (same code path, secondary
voice selected automatically, rate 0).

Audio: `pcm_s16le`, 44100 Hz, mono, 16 bits/sample, 12.518617 s, 1,104,188
bytes. `volumedetect`: mean -29.5 dB, max -7.8 dB - a quieter, more dynamic
read than `voice.wav`, with pauses between sentences.

## Verification performed

ffprobe (all files decode, parameters as described above):

- `main.mp4`: h264 / High, 960x540, yuv420p, 24/1, 12.000000 s, no audio.
- `broll.mp4`: h264 / High, 960x540, yuv420p, 24/1, 8.000000 s, no audio.
- `poster.png`: png, 480x720, rgba, decodes with `-frames:v 1`.
- `voice.wav`: pcm_s16le, 44100 Hz, mono, 16-bit, 8.664082 s.
- `narration.wav`: pcm_s16le, 44100 Hz, mono, 16-bit, 12.518617 s.

Timestamp-dependent content - one frame decoded at t=0.5, middle and near-end,
all three SHA-256 distinct:

| Clip | t=0.5 | middle | near-end |
| --- | --- | --- | --- |
| `main.mp4` | t=0.5 `fdc8eb1f2321fbda4954df9489d5e4ca2f5c8e9111903ebef1a661be9946a303` | t=6.0 `b157ed99f2715859925b2128f2ffc377e38f224d1ab5e1bfb363c2e997b39610` | t=11.5 `2e827f9280d273a25bba4277642ee426ceb0e2474645405be06b82c96a02ca87` |
| `broll.mp4` | t=0.5 `4c477de7dbbfc43ca5e15f4451bab68e9684b493d76a9ca0796358e990f84c65` | t=4.0 `9b40e0a9d2853e947ded53aa9bef7ce9bf02de58f30ac8300b463c39de2a95d3` | t=7.5 `563273104ae1ade111c1fb4c3c6451d7307c6374bed1563212909a3617adfa8c` |

Audio levels - `ffmpeg -i <file> -af volumedetect -f null -`:

- `voice.wav`: mean_volume -20.7 dB, max_volume -2.5 dB.
- `narration.wav`: mean_volume -29.5 dB, max_volume -7.8 dB.

Both WAVs decode and are non-silent. Two consecutive runs of the generator
produced byte-identical files (same sizes and SHA-256), confirming the script
is reproducible and idempotent on this machine.

## Provenance and licensing

- Video content is computed by FFmpeg's own `mandelbrot` filter (two different dives).
  No test patterns, screen captures, stock footage, textures, or third-party
  media were used.
- The poster artwork is the same `mandelbrot` generator; its lettering is
  rendered locally with Windows GDI+ and the system font `arialbd.ttf`.
- Speech is synthesised locally by Windows SAPI from the text quoted above.
  No recorded human voice or external audio sample is included.
- No network access was used at any point (the pinned FFmpeg build is even
  compiled with `--disable-network`).
- These files are wholly generated content, contain no third-party or stock
  media, and are safe to commit to the repository.

The SHA-256 values above identify the exact files generated on this machine
(2026-11-26, Windows 11). Re-running the script on another machine or with a
different H.264 encoder may produce different but equivalent bytes; sizes and
stream parameters should stay within the documented budgets.
