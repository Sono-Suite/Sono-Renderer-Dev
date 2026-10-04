# SFX decoding through FFmpeg

## Horizon format finding

The repository fixture `TestingSuite/Horizon/project(1).scp` contains Effect
resource **`coconut-horizon-10`**. Its `EffectData` entry `#PERFECT` is clip
filename **`0`** (Watch effect clip ID **1** in `Horizon/EngineWatchData`). The
extensionless payload is 3,095 bytes, starts with
`49 44 33 04 00 00 00 00 00 23 54 53` (`ID3`), SHA-256
`32693c9997a3e8593ed311b6442f8d8e152ad136a12503eec2a6dc80eaa6c5b2`.
The prior error originated in `audio.rs::decode_wav`, which required RIFF/WAVE
and reported `unsupported audio: expected RIFF/WAVE`. FFmpeg recognizes this
payload directly from stdin as MPEG Layer III, 44.1 kHz mono, and decodes it.

## Implementation

`src/audio.rs` now pipes payload bytes to the existing managed/shared FFmpeg
installation. It asks FFmpeg for 44.1 kHz float32 PCM in a WAV stream on stdout;
no temporary audio file or filename extension is used. The small PCM reader
accepts PCM16 and float32 WAV chunks, including FFmpeg's streaming `0xffffffff`
data chunk length. Mono remains mono, preserving the mixer's existing full-level
mono duplication to left/right. Stereo remains stereo. The mixer itself and its
event ordering, timing, clipping, looping, overlap, volume and PCM16 WAV output
are unchanged.

Decoded clips are cached process-wide by SHA-1 of the exact resource bytes.
`clip_duration` used by whole-chart end inference and the later mixer share the
same cache, so each distinct payload is transcoded at most once in the process.
The cache stores canonical PCM for repeated playback. FFmpeg errors include the
resource name and stderr details. The resolver now also handles Cargo's
`target/{debug,release}/deps` executable location by resolving its parent build
profile directory.

Export reports now include playback event requests, named resources used,
new unique payloads decoded during that export, and FFmpeg decode invocations.
The single authoritative Watch traversal remains in place; audio prepass work
is zero unless explicit determinism validation requests its separate pass.

## Validation

Focused command:

```powershell
cargo test --offline audio::tests
```

Eight focused tests pass, including the actual extensionless Horizon MP3,
44.1 kHz/mono decoded format, shared cache reuse, one decode for two overlapping
plays, WAV input, scheduled and loop playback, clipping, resource-specific
malformed-input diagnostics, and prior mixer behavior.

The release whole-chart Baumkuchen control is:
`artifacts/audio-sfx/baumkuchen-sfx-whole-chart.mp4`; its full command and log
are recorded in `artifacts/audio-sfx/baumkuchen-export.log`. Results:

- Watch interval 0..122s, inferred end 121.95s, 1,464 video frames at 12fps.
- BGM offset -1.109s; music source starts at 0 with 1.109s leading silence.
- 1,464 runtime frames in one authoritative Watch traversal; SFX event-only
  traversal 0 frames/callbacks/evaluations.
- SFX stream hash **`be53125a6dd38ad4b59dc7fc15b2ca3ca99c1808`**, identical to
  `artifacts/audio-prepass/20-baseline.log`; counts are 615 scheduled effects,
  89 loop starts and 89 loop stops.
- 704 playback requests, 10 named resources used, **10 unique payloads / 10
  FFmpeg decodes**.
- SFX mixed; 122.0s MP4 has video and audio streams. Decoded audio mean level
  is -9 dBFS (peak 0 dBFS).

Horizon's SFX-enabled 0..20s check is
`artifacts/audio-sfx/horizon-sfx-20s.mp4`, with log
`artifacts/audio-sfx/horizon-export.log`. It has 240 frames at 12fps, duration
20.0s, BGM offset +0.05s with media source starting at +0.05s, 1 Watch traversal,
and 588 scheduled SFX requests mapped to 2 named resources: **2 unique payloads,
2 FFmpeg decodes**. The SFX mixer produced audio. The output audio stream is
present and non-silent.

The standalone `TestingSuite/Horizon/Horizon.zip` engine does not meet the
current whole-chart discovery assumptions: its `updateSpawn` is not a supported
positive affine timeline. The whole-chart command reaches that existing
independent boundary after audio decoding succeeds and reports
`whole-chart end is indeterminate`. Therefore the successful full whole-chart
acceptance run is the Baumkuchen control; Horizon was verified with an explicit
20-second interval. No Watch/end-inference behavior was altered to bypass this.

## Test and build status

Final validation:

- `cargo test --offline`: **190 passed, 0 failed, 3 ignored** (81 library,
  8 CLI, 3 audio integration, 98 integration tests).
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Release executable built successfully at
  `artifacts/audio-sfx/renderer.exe`; it ran both exports above.

No SFX event timing, Watch traversal, whole-chart inference, or mixing behavior
was changed beyond decoding resources and reporting decode metrics.
