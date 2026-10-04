# Export lifecycle milestone


> Historical milestone record. The subsequent [single Watch export milestone](SINGLE_WATCH_EXPORT.md) replaces full-chart discovery traversal and the SFX prepass with cheap schedule evidence and render-collected audio.
Scope: A/V start-zero correctness, whole-chart export, structured live progress. MV playback subsequently landed as a consumer of this timeline; see [MV export](MV_EXPORT.md). Watch optimization and particle production changes are excluded.

## Data flow and changes

Previously: CLI configuration -> global Watch frame range -> independent SFX event prepass -> SFX WAV -> streamed RGB24 (sequential or bounded pipeline) -> FFmpeg BGM timestamp trim/mix -> H.264/AAC -> existing FFprobe validation.

Now: the same frame scheduler creates `ExportTimeline`; BGM is prepared as exact-length, zero-based stereo PCM before muxing. The same SFX prepass and RGB paths remain. Optional whole-chart discovery runs a configured Watch-only session first. Progress surrounds these phases and counts successful full-frame writes to FFmpeg. The legacy concurrent SFX path still uses its bounded disk spool; that spool is handed to the same pipe writer using bounded I/O, without constructing another RGB frame buffer.

Files/functions:

- `export_timeline.rs`: `ExportTimeline`, `sample_frames`, `bgm_filter`.
- `export_media.rs`: managed-tool `source_duration` and `prepare_bgm`.
- `audio.rs`: clip pre-window SFX to their existing ages, count the complete half-open sample window; expose the existing WAV duration/header helpers within the crate.
- `export_end.rs`: `discover`, `ChartEnd`, cached durations for referenced supported SFX.
- `export_progress.rs`: typed events/sinks, tracker, completed-frame writer, terminal presentation.
- `offline.rs`: read-only EventSession runtime access, one shared constant for the existing million-frame limits, temporal tests. Watch execution behavior is unchanged.
- `video_export.rs`: integrate resolved interval, media preparation, progress, and report evidence.
- `render.rs` / `main.rs`: default-compatible serialized configuration, CLI whole-chart selection, public progress API, CLI presentation.

## One timeline

For requested start `S`, duration `D`, FPS `F`, preserve the existing scheduler:

```text
first index = ceil(S * F)
frame count N = ceil(D * F - existing rounding epsilon)
effective Watch start Se = first index / F
effective duration De = N / F
output time = Watch time - Se
media time = Watch time + bgmOffset
```

The actual exported interval is `[Se, Se + De)`; output zero maps to `Se`. Non-grid requests keep their previous upward frame alignment. Both requested and effective starts are reported. BGM and MV use `ExportTimeline::watch_to_media` / `output_to_media`; no independently accumulated MV clock is needed.

PCM is stereo signed 16-bit at 44,100 Hz, with `ceil(De * 44100)` sample frames (floating-point integer-boundary noise removed). Offset boundaries are rounded to the nearest sample. BGM is resampled, trimmed in sample coordinates, delayed using real zero samples when the requested source begins before zero, padded after source exhaustion, clipped to exactly the required count, and given zero-based timestamps. Even wholly unavailable source intervals produce the required silence. The temporary PCM file length is checked before muxing.

The implementation uses FFmpeg's documented [sample delay](https://ffmpeg.org/ffmpeg-filters.html#adelay), [silence padding](https://ffmpeg.org/ffmpeg-filters.html#apad), and [sample trimming](https://ffmpeg.org/ffmpeg-filters.html#atrim). FFmpeg/FFprobe discovery, distribution pinning, H.264/AAC settings, and existing mux validation tolerances are preserved.

### Established A/V faults

1. The previous negative-source branch shifted `asetpts` forward without inserting PCM silence. This creates a late/short audio stream rather than a zero-based requested interval. Baumkuchen, offset `-1.109`, start `0`, duration `2`, FPS `12`, reproduced this before edits.
2. In SFX mixing, `source_start` was assigned the original event time after checking the window intersection. Consequently a clip born before a positive export start restarted at source sample zero. It now uses the intersection time; minimum-distance grouping continues to use original event times.
3. The SFX output limit floored fractional sample counts although the allocated window used a ceiling. The shared half-open-window count now includes the last in-range sample consistently.

| Export | Video frames | Video start / duration | Audio start / duration | Container start / duration |
|---|---:|---|---|---|
| Baumkuchen before, `[0,2)` | 24 | 0 / 2.000000 | 1.087000 / 0.896000 | 0 / 2.000000 |
| Baumkuchen after, `[0,2)` | 24 | 0 / 2.000000 | 0 / 2.000000 | 0 / 2.000000 |
| Negative-offset Horizon copy before, `[0,1)` | 12 | 0 / 1.000000 | 0.026009 / 0.952018 | 0 / 1.000000 |
| Negative-offset copy after, `[0,1)` | 12 | 0 / 1.000000 | 0 / 1.000000 | 0 / 1.000000 |

Baumkuchen BGM source duration: 120.825042 s. The two-second interval requires media `[-1.109, 0.891)`, 88,200 stereo sample frames, with 48,907 leading silent sample frames and 39,293 source sample frames. Video is neither shifted nor shortened. The SFX window remains Watch `[0,2)` and output `[0,2)`, with 615 scheduled plays and 89 loop starts/stops collected. AAC packet padding is not used to weaken validation.

## Whole-chart end policy

CLI: `--whole-chart`, optionally `--start-time S`. Reject explicit `--duration` with it. Whole-chart defaults to zero; bounded default behavior remains unchanged. Library/serialized `whole_chart: true` selects inferred duration instead of the legacy duration field.

The configured discovery session uses the same ROM, defaults, options, aspect ratio, resource-name availability, and background as rendering. It advances from frame zero and observes the **actual** `updateSpawn` timeline, including spawned entities and termination callbacks. Finite schedule endpoints are compared to that spawning timeline, not assumed to be Watch timestamps. This follows the documented [Watch lifetime system](https://wiki.sonolus.com/engine-specs/watch-lifecycle/lifetime-system) and [timeline explanation](https://wiki.sonolus.com/sonolus.js-guide/watch/02).

Policy:

1. Retain all finite input-entity schedules and ordinary finite non-input schedules, including later content after BGM ends.
2. Treat non-input entities whose endpoint is at/above the reported policy threshold `1,000,000 / FPS`, including infinity, as persistent. Still wait for their finite activation times, even when spawned by preprocessing or other callbacks.
3. Wait for finite schedules to finish, the BGM Watch bound `source duration - bgmOffset`, and observed supported SFX clip/loop-stop bounds. Decode durations lazily for referenced clips; unavailable payloads have the mixer's existing skip behavior and unsupported present payloads remain errors.
4. Include the frame executing final termination/activation callbacks. Round audio bounds upward through the same frame grid; append no arbitrary tail.

The threshold is an **export inference policy in spawning-timeline units**, derived from the pre-existing frame safety limit, not a Sonolus semantic constant or proof that a lifetime lies outside the Watch-time domain. `hasInput` identifies playable entities in the [official archetype specification](https://wiki.sonolus.com/engine-specs/resources/engine-watch-data-archetype); it does not establish an exact chart finish. No engine names or engine sentinel constants are used in the implementation.

Why inference is needed: Next RUSH/Baumkuchen supplies static controller intervals `[-100000000,+100000000]` and a last spawned non-input final display with an effectively unbounded endpoint. Horizon has analogous persistent controller endpoints of `999999`. Waiting for those endpoints is not a usable song-export boundary. Their source expressions and runtime schedules were inspected locally; no engine or VM behavior was changed.

Every discovered result is explicitly **inferred**, never labeled an exact Sonolus completion signal. Generic callbacks in persistent controllers could create later content that these bounds do not predict, and unusual spawning-time scales could make the threshold classification inappropriate. Use explicit `--duration` when that assumption does not fit an engine. This policy is not proof of future quiescence.

Indeterminate/error cases include no finite schedule/BGM anchor, nonfinite/backward spawning time, open-ended input entities, audible loops with no observed stop, exceeding the existing million-frame safety bound, unsupported referenced SFX, and starts at/beyond the inferred end. The renderer reports the condition and requests an explicit duration rather than silently truncating inputs at the music duration.

### Whole-chart checks

| Fixture | SFX | Frames | Scheduled Watch interval | FFprobe video / audio duration |
|---|---|---:|---|---|
| Horizon/Dreamer | disabled | 1498 | `[0,124.833333)` | 124.833008 / 124.832993 |
| Next RUSH/Baumkuchen | enabled | 1464 | `[0,122.000000)` | 122.000000 / 122.000000 |

Both have stream start zero and pass the unchanged FFprobe gate. Horizon's tiny reported duration difference is container time-base rounding within the existing tolerance, with all 1498 frames present. Horizon SFX-enabled whole-chart preparation encounters an existing unsupported non-WAV clip; it is not silently omitted. Bounded Horizon exports with the existing SFX configuration pass.

## Structured progress

Public APIs: `RenderConfig::render_video_with_progress`, `video_export::export_config_with_progress`, and `export_with_progress`. Existing no-sink APIs remain available. A `ProgressSink` is a `Send` callback receiving serializable `ExportProgress` values; a GUI can forward them through a channel without parsing CLI output. No GUI was implemented.

Phases: `preparing`, `audio-prepass`, `rendering`, `finalizing`, `complete`. Fields: completed frames, optional total frames/percentage, elapsed seconds, optional FPS/ETA. Whole-chart totals are unknown during discovery and become the scheduler's exact total afterward. Discovery and sequential SFX passes emit throttled elapsed-time pulses without incrementing exported frame counts.

Completion counts increase only after successful full RGB-frame handoff to FFmpeg, on the sequential or pipeline consumer path. Partial/failed writes cannot invent complete frames. The legacy concurrent spool is counted only during FFmpeg handoff. Rendering reaches 100% before mux finalization and probing; `complete` is emitted only after count equality and successful validation.

Throughput/ETA omit the first frame's startup/pre-roll latency. They remain unavailable until at least four completed frames and one second of post-first-frame sampling. The cumulative rate is then used for remaining-frame ETA and frozen at finalization. ETA estimates rendering/handoff only; it does not promise a mux/probe finish time.

Example structured event:

```json
{"phase":"rendering","completed_frames":240,"total_frames":1464,"percentage":16.39344262295082,"elapsed_seconds":65.0,"frames_per_second":40.0,"eta_seconds":30.6}
```

Interactive stderr uses one ASCII line capped/padded to 78 columns, at most four refreshes per second; phase transitions and completion end the line. Redirected stderr has no redraw/ANSI escapes and emits phase changes, completion, and at most one periodic line every five seconds. Profiling output remains independently available.

## Validation and reproducibility

Focused coverage includes exact managed-FFmpeg PCM clipping/padding, zero/positive starts and both offset signs, wholly unavailable source intervals, SFX pre-window age and near-start events, fractional sample counts, no SFX, partial/failed progress writes, ETA math without wall-clock assertions, actual scaled spawning timelines, supported finite lifetime bounds, persistent future activation, SFX tails, indeterminate input lifetimes, and CLI conflicts.

The full suite also exercises CPU/WGPU tolerance, sequential/pipeline FIFO and determinism, concurrent SFX equivalence, existing renderer hashes, and particle/runtime behavior. No existing visual golden hashes were updated. FFmpeg-decoded video frame MD5 sequences before/after are identical for Horizon start zero, the negative-offset copy, and Baumkuchen start zero. Baumkuchen sequential/pipeline `[2,3)` RGB sequence SHA-1 matches: `3db77e428a906b4e085a738dd77ae7a1ac0c69bf`.

Local artifacts: `artifacts/export-milestone/validation.json`, `final-full-suite.log`, baseline/after `.log`, `.mp4`, and `.framemd5` files. The bounded diagnostic runner is `validate_exports.py`; final report values should be taken from those artifacts. Development warnings about path canonicalization and deprecated `fetch_update` predate this milestone.

Final checks on 2026-10-03: `cargo test` passed **164 tests** (56 library, 7 CLI, 3 arctan2 integration, 98 main integration; 0 doctests), with no failures. Release build, formatting check, and `git diff --check` passed. The final concurrent-SFX Horizon export has the same RGB sequence as its sequential/pipeline counterparts. A Windows PTY check displayed the bounded progress line successfully; redirected logs contain no ANSI or carriage-return redraw controls. Base-clear, direct RGB24, explicit PPM APIs, profiling, particle production code, and Watch VM/runtime behavior remain preserved.

Real reproducer command (PowerShell; paths quoted):

```powershell
cargo run --release -- render-video 'TestingSuite/Next RUSH/engine/Next RUSH.zip' 'TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp' 'TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz' 'TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/ANIMATION_VIDIO_Baumkuchen_and_Credits_X_Retry_Now_Sub_English_and_jangan_NAKISO_amaamala_1789700599.mp3' artifacts/export-milestone/after-baum-start0.mp4 --start-time 0 --duration 2 --fps 12 --width 160 --height 90 --backend wgpu --level-option 1=10.8 --level-option 22=1 --no-particles --validate-determinism
```

For the whole-chart Baumkuchen command, replace `--start-time 0 --duration 2` with `--whole-chart` and omit `--validate-determinism` for an ordinary export. To check positive-start equivalence, use `--start-time 2 --duration 1`, then repeat with `--sequential-frames`. CPU comparison uses `--backend cpu` and a bounded 0.25-second interval.

Horizon whole-chart command:

```powershell
cargo run --release -- render-video TestingSuite/Horizon/Horizon.zip 'TestingSuite/Horizon/project(1).scp' TestingSuite/Horizon/dreamer.json.gz TestingSuite/Horizon/dreamer.mp3 artifacts/export-milestone/whole-horizon.mp4 --whole-chart --fps 12 --width 160 --height 90 --backend wgpu --no-sfx
```

Probe any output using the reported managed FFprobe path:

```powershell
dependencies/ffmpeg/8.1/ffprobe.exe -v error -show_entries 'format=start_time,duration:stream=codec_type,start_time,duration,nb_frames,sample_rate' -of json artifacts/export-milestone/whole-baum.mp4
```
