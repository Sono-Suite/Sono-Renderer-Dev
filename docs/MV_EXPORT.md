# MV export on the canonical media timeline

MV is an explicit export input: CLI `--mv <video>`, serialized/library `RenderConfig.mv`. An omitted MV preserves the existing renderer. A supplied missing or undecodable file produces an actionable export error.

Post-render audio mixing and whole-chart discovery now use the [single Watch export architecture](SINGLE_WATCH_EXPORT.md). MV decoding and timestamp selection are unchanged.

## Timing and decoding

Every scheduled frame uses the existing `ExportTimeline::watch_to_media(watch_time)`:

```text
Watch time = global frame index / export FPS
media time = Watch time + level.bgmOffset
```

This is the same source position used for BGM; MV never accumulates a playback clock. Nonzero exports seek to a preceding keyframe, retain source presentation timestamps, and decode sequentially to the requested position. Frame selection is the latest source PTS at or before that media time. Source/container timestamps are measured relative to the container start, preserving a delayed video stream within that container.

One managed FFmpeg process per render session emits RGBA frames and timestamp metadata. The decoder holds a current frame and one lookahead; timestamp buffering and the existing frame pipeline are bounded. Independent determinism passes own independent decoders. Immutable `Arc` snapshots cross the existing pipeline queue. No whole-video pixel or timestamp collection, per-frame process spawning, RGB-to-PPM conversion, or second framebuffer compositor is introduced. The existing GPU texture cache remains bounded and unchanged.

Timestamp parsing uses integer PTS and the filter's rational time base, rather than rounded `pts_time` text. Decode errors/truncated frames propagate; dropping the session kills and waits for its child and joins the stderr reader, including output failures. MV audio, subtitles, and data streams are excluded. BGM plus SFX remain the sole audio inputs.

FFmpeg behavior is documented in its [seek/timestamp options](https://ffmpeg.org/ffmpeg.html) and [showinfo filter](https://ffmpeg.org/ffmpeg-filters.html#showinfo).

## Explicit presentation policy

The existing project input model already defines a custom MV path. The inspected [official LevelItem schema](https://wiki.sonolus.com/custom-server-specs/misc/level-item) and [SCP level resource layout](https://github.com/Sonolus/sonolus-pack/blob/main/README.md) do not establish an MV discovery field or an authoritative custom-video presentation convention. Accordingly this implementation does not invent a Sonolus package field or claim a native-client MV compatibility result.

For this **export presentation feature**:

- Select the first video stream from the explicit file; no directory/name guessing or automatic SCP MV discovery.
- While available, replace the selected background image with the MV, behind skin/stage, particles, and UI. Reuse the existing background sampling/composition path in both CPU and WGPU.
- Fit the displayed video aspect inside the viewport, with black letterboxing; account for sample aspect and ordinary 90-degree display rotations. Arbitrary display rotations are rejected explicitly.
- Preserve the selected background mask/configuration. Existing unsupported nonzero background blur remains unsupported.
- Use a fixed viewport fit for the custom MV; do not change Watch Runtime Background state or engine inputs. The selected background retains its existing runtime quad when restored.
- Before media time zero, before a delayed first video frame, and after the source ends, restore the selected background. No looping or frozen indefinite tail.
- Prefer video-stream duration to container duration. When only container duration is available, reaching decoder EOF bounds the final frame using its reported duration (nominal source frame interval if the packet duration is unavailable). Files without any finite duration are rejected. Unusual files missing final-frame timing have this nominal-interval limitation.
- MV duration does not define or extend the inferred chart end.

These choices are explicit exporter policy; exact native Sonolus custom-video layering, scaling, and boundary behavior have not been established. No particle investigation, APK analysis, or real-client oracle was reopened.

## Integration

- `export_mv.rs`: managed probe, sequential decoder, source timestamp selection and child cleanup.
- `offline.rs`: optional decoder on `FrameSession`, owned MV snapshot on `PreparedFrame`, existing CPU/WGPU background input selection; `RenderedFrame.mv_media_pts` records the selected source timestamp.
- `render.rs`: default-compatible optional input, preview and video configuration.
- `video_export.rs`: configure each render session from the resolved canonical timeline; report input path and per-frame source PTS.
- `main.rs`: enable the existing `--mv` flag and report its policy.

Whole-chart discovery still uses the established Watch/audio end policy. MV probing/decoder startup belongs to Preparing; successful full RGB24 writes count toward Rendering; Finalizing/Complete retain the existing mux/probe gates. The typed progress callback and terminal renderer are unchanged. SFX event-only passes do not decode MV.

## Focused regression coverage

`mv_samples_canonical_media_time_with_offset_preroll_and_fps_mismatch` uses a lossless, one-second, 32×16 video: four frames at PTS 0/250/500/750 ms, each a distinct color with its PTS burned into its pixels. It checks zero/nonzero and non-grid starts, positive/negative offsets, wholly unavailable intervals, source FPS above/below export FPS, source exhaustion, and aspect fit.

`mv_selects_by_presentation_timestamp_not_frame_index` checks variable source PTS 0/125/500/875 ms and exact 30 FPS rational boundaries in a lossless MOV. It also rejects backward requests.

`mv_pipeline_matches_sequential_and_cpu_wgpu_share_composition` checks actual Watch composition with a negative media offset, exact selected PTS, byte-identical sequential/pipeline output in each backend, CPU/WGPU maximum channel difference ≤2, background restoration before/after the MV, and pipeline cancellation with a closed output.

## Validation artifacts and reproduction

Local artifacts live in `artifacts/mv-milestone/`: `timestamped.mkv`, `timestamped-audio.mkv` (with deliberately loud unrelated audio), `validation.json`, source probes, export logs/MP4s, and `full-suite.log`. `validate_mv.py` recreates the tiny source and runs the recorded commands; it accepts case names to rerun a bounded subset.

```powershell
python artifacts/mv-milestone/validate_mv.py tiny-start0 tiny-positive-start real-start0 real-positive
python artifacts/mv-milestone/validate_mv.py whole-real whole-tiny
```

The real MV is the already-existing Baumkuchen video in the same level directory as its BGM. Exact whole-chart command:

```powershell
cargo run --release -- render-video 'TestingSuite/Next RUSH/engine/Next RUSH.zip' 'TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp' 'TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz' 'TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/ANIMATION_VIDIO_Baumkuchen_and_Credits_X_Retry_Now_Sub_English_and_jangan_NAKISO_amaamala_1789700599.mp3' artifacts/mv-milestone/whole-real.mp4 --mv 'TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/ANIMATION_VIDIO_Baumkuchen_and_Credits_X_Retry_Now_Sub_English_and_jangan_NAKISO_amaamala_1789534736.mp4' --whole-chart --fps 12 --width 160 --height 90 --level-option 1=10.8 --level-option 22=1 --no-particles
```

For bounded visual checks use `[0,3)` (`--start-time 0 --duration 3`) or `[2,3)` (`--start-time 2 --duration 1 --validate-determinism`) instead of `--whole-chart`. The latter consumes MV/BGM media `[0.891,1.891)`; the former consumes `[-1.109,1.891)` and visibly restores the static background during the initial unavailable-media interval.

Full-chart validation retains the inferred `[0,122)` interval: 1464 frames at 12 FPS, both stream starts zero and both stream durations 122 seconds. A one-second MV produces the same whole-chart frame count/end as the real MV. Progress reaches 1464/1464, then Finalizing and Complete. Adding a loud MV audio track leaves the decoded output audio byte-identical to the MV-without-audio and no-MV exports.

Measured tiny-fixture positions (Baumkuchen offset `-1.109`):

| Watch time | Canonical media time | Selected source PTS / image |
|---:|---:|---|
| 0.000000 | -1.109000 | no MV; existing background |
| 1.166667 | 0.057667 | 0.000000 / red `000` |
| 1.416667 | 0.307667 | 0.250000 / green `250` |
| 1.666667 | 0.557667 | 0.500000 / blue `500` |
| 1.916667 | 0.807667 | 0.750000 / yellow `750` |
| 2.166667 | 1.057667 | no MV; existing background |

All selected real-video timestamps were independently checked against FFprobe's source-frame PTS. The nominally 30 FPS file actually stores alternating millisecond positions such as `0.033` / `0.067`, so `frame_index / nominal_FPS` would not be an exact source-PTS oracle. The `[2,3)` export correctly begins with source PTS `0.867` at media position `0.891`.

Final checks on 2026-10-03: **167 tests passed** (59 library, 7 CLI, 3 arctan2 integration, 98 main integration; zero failures), release build, formatting check, and diff whitespace check passed. All ten real/synthetic exports passed the unchanged FFprobe gate. Tiny sequential/pipeline RGB sequence SHA-1 is `e7a122389607a9f620397d687bc61f264dd345bf`. `checks.json` records the source-PTS comparisons, ignored-audio check, restored-background checks, and unchanged prior no-MV frame hashes. `tiny-start0-contact.png` and `real-positive-contact.png` show the existing stage drawn over the selected MV frames. Pre-existing canonicalization/deprecated-atomic warnings remain.

No existing golden expectations are changed. Base-clear, direct RGB24, explicit PPM APIs, Watch behavior, particle production code, SFX, and profiling remain preserved.

## Background percentage

`RenderConfig.mv_background` / `--mv-background` / the desktop slider accept finite percentages from 0 to 100, default 100. The decoder scales RGB toward black and forces presentation alpha to 255, retaining presentation timestamps, before the existing background compositor draws gameplay over it. Zero makes the active MV opaque black; 100 preserves decoded RGB exactly. Decoded video alpha is never used as MV presentation opacity. The selected background mask still applies. Outside the source interval, the existing static background fallback is unchanged. This is an export presentation control, not an established Sonolus client semantic.

`RenderConfig.mv_enabled` (default true), the GUI's Use selected MV toggle, and CLI `--no-mv` control activation. Disabling it retains the selected configuration path but performs no MV decode. No second clock is introduced; Watch, BGM, SFX, inference, and output-frame counts are independent of these presentation settings.

The DISPLAYHOLIC compositing investigation and its remaining uncertainty are recorded in [MV_ALPHA_COMPOSITING_AUDIT.md](MV_ALPHA_COMPOSITING_AUDIT.md).
