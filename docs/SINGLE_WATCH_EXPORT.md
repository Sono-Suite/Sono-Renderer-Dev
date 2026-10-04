# Single Watch export milestone

## Architecture and scope

Normal explicit-duration exports execute the configured Watch frame schedule once. Normal whole-chart exports first execute preprocessing and cached spawn/despawn schedules for boundary evidence, then execute the playback interval once. Discovery never initializes entities or runs sequential/parallel updates, termination callbacks, particle evaluation, rasterization, UI composition, or MV decoding. It is a separate cheap setup session, not a second playback traversal. Explicit `--validate-determinism` deliberately retains two rendered playback passes for comparison.

The serial Watch producer owns SFX collection in the bounded pipeline. It appends each prepared frame's event report before queue handoff. The sequential path collects the identical reports through frame diagnostics. Reports for the first exported frame include all pre-roll audio operations from Watch zero, preserving pre-window scheduled events and loop state. Neither queue backpressure nor the consumer generates audio operations. Errors discard the collected timeline and staged media.

After video encoding completes, the existing exact-length BGM PCM preparation and existing SFX WAV mixer consume the canonical `ExportTimeline`: media = Watch + bgmOffset, output = Watch - effective start. Final FFmpeg invocation uses `-c:v copy` and AAC audio encoding. MV audio remains ignored. No second runtime, RGB spool, PPM conversion, additional framebuffer copy, video re-encode, or MV timing model is introduced.

## Why the old path was expensive

`export_end::discover` previously called `EventSession::advance_global_frame` once per output-frame index until finite schedules, media, and known SFX tails were exhausted. This executed preprocessing, updateSpawn, activation, sequential and parallel updates, and termination at output FPS. It also generated Draw/particle operations although it never rasterized them. Ordinary SFX exports then created another EventSession for the same schedule before actual rendering. This permitted three full playback traversals.

The preceding controlled AudioPrepass measurements are in `artifacts/audio-prepass/`. At 60 FPS the 120-second event pass evaluated 7,200 Watch frames, 953,516 callbacks, and 754,747,210 expressions in 149.053 seconds with profiling enabled. The 80-90 second interval alone took 43.836 seconds, averaging 460 active entities. Memory setup and commit accounted for 84.527 seconds over the entire pass. All 615 scheduled plays, 89 loop starts, and 89 loop stops were produced in the first interval; there was no subsequent audio emission. BGM preparation and SFX mixing together took less than 0.4 seconds. This milestone removes repeated playback work rather than optimizing those VM operations.

## Cheap whole-chart evidence

The generic supported path proves, conservatively over all callback graph branches:

- No Spawn or audio emitter is reachable outside preprocessing, including schedule callbacks and global updateSpawn.
- updateSpawn is a supported side-effect-free positive affine expression of Runtime Update time. Supported forms are constants, direct time reads, affine Add/Subtract/Multiply, and unconditional compiler return wrappers. Unsupported/stateful/nonlinear clocks remain indeterminate.
- Preprocessing and ordered spawnTime/despawnTime evaluation produce every cached schedule, including entities created during preprocessing.

Finite schedule endpoints are converted from spawn-timeline units to Watch time by the proven affine clock. Input entities with open-ended schedules are indeterminate. Non-input controller endpoints at or above the policy cutoff remain persistent; finite future activation still contributes to the inferred boundary. The cutoff is fixed at 1,000,000 / 60 timeline seconds, independent of requested video FPS. This is an export policy, not a Sonolus semantic constant.

BGM contributes source duration minus bgmOffset. Preprocessing SFX events contribute supported clip durations, scheduled tails, and loop stops. Audible open loops are indeterminate. MV is a timeline consumer and does not replace the existing chart/BGM end policy. Missing or unsupported effect media is handled by the existing duration decoder and error path.

The boundary uses a fixed 60-Hz inference grid, retaining the previous 60-FPS endpoint policy without advancing frames. Schedule completion includes an endpoint lifecycle frame; media tails round upward to the same grid. Output frame quantization happens separately, once, through `FrameRange`. At 12 FPS an inferred 121.95-second boundary still produces 1,464 frames / 122 seconds of output; the inferred boundary itself remains 121.95 at every FPS.

This is **inferred**, never an exact Sonolus completion signal. The old discovery also used an inference policy. Engines with runtime Spawn/audio, unsupported clocks, or open-ended input content now report a specific **indeterminate** reason and require `--duration`. There is no silent full-frame discovery fallback and no engine-name or level-name special case. A later callback can depend on persistent memory and previous updates; evaluating only its endpoint or sampling at a lower rate cannot generally establish its future events. Generic exact termination is not claimed.

Baumkuchen evidence: 3,602 entities after preprocessing, 3,564 finite schedules, 38 persistent controllers, 8,231 preprocessing/schedule callbacks, zero discovery playback frames. BGM Watch end is 121.934042 seconds, known SFX tail is 120.269341266 seconds. The preserved inference result is 121.950000 seconds, 7,317 frames at 60 FPS.

## Progress, diagnostics, and failure behavior

Typed normal phases are Preparing -> Rendering -> AudioMixing -> AudioEncoding -> Finalizing -> Complete. Optional determinism preflight uses Validating. Rendering counts successful complete RGB writes and freezes its throughput before audio mixing. Redirected output remains sparse and contains no terminal redraw controls. Discovery reports setup callback count, zero playback frames, and elapsed discovery time.

`ExportReport.watch_traversals` counts actual FrameSession constructions, not a hard-coded expected count. Normal exports assert one; explicit determinism validation asserts two. Legacy event-pass workload fields remain zero. `concurrent_sfx_prepass` configuration and the old CLI prepass switches remain accepted compatibility no-ops; they cannot create an independent Watch session or RGB spool.

Video-only and final staged MP4s use unique RAII temporary files in the destination directory. BGM PCM and SFX WAV also have RAII lifetimes. The encoder child is killed and reaped on error/unwind; pipeline producer/consumer failures and panics cancel the handoff. The final destination is replaced only after successful mux and FFprobe validation. Existing input/output aliases are rejected before work. Ordinary failures clean intermediates and preserve any prior output. Process termination outside normal Rust unwinding can leave temporary files; no corrupt partial video is written to the final destination.

## Files

- `export_end.rs`: conservative graph proof, affine timeline analysis, cached schedule/media inference, and FPS-independent policy.
- `watch_runtime.rs`: boundary-only preprocessing/schedule evidence method; playback callback execution remains unchanged.
- `offline.rs`: discovery access and crate-visible prepared report for producer-side collection.
- `video_export.rs`: render-collected SFX, single normal playback session, staged video encode/post-render audio mux, child cleanup, regression seam.
- `export_progress.rs`: post-render audio phases and render-throughput freeze.
- `main.rs` / `render.rs`: legacy flag descriptions and traversal reporting.

Particle production, MV decoder, WGPU/CPU rasterization, base-clear, RGB24, explicit PPM, evaluator semantics, music/SFX mixer semantics, and Watch memory setup/commit are unchanged.

## Regression coverage

- Real Baumkuchen cheap discovery at 12/30/60 FPS: identical inferred boundary, 38 persistent controllers, zero playback frames.
- Conservative rejection of post-preprocess Spawn/audio and unknown clocks; evaluated scaled timeline and future persistent activation.
- Real 1080p60 particle/SFX workload and unchanged event golden; producer-collected events equal the former event-session reference.
- CPU/WGPU + MV sequential/pipeline RGB and event equality, differing MV source/output FPS and rational PTS coverage.
- End-to-end tiny synthetic engine: scheduled/immediate/per-frame events, pre-roll/nonzero start, positive/negative offsets, explicit/whole-chart + MV, no-SFX, one runtime traversal, optional two-pass determinism, exact BGM sample count, video frame/stream validation, ordered progress phases.
- Final-probe failure preserves prior output and cleans staged media. Existing bounded-pipeline writer failure, worker panic, producer error, and partial-frame progress tests remain.

No visual or audio golden was regenerated. Two synthetic schedule tests now assert the fixed 60-Hz boundary instead of an output-FPS-dependent endpoint; their 10-FPS exported frame coverage remains unchanged.

## Measurements and commands

The runner `artifacts/single-watch/run_exports.py` preserves the exact commands. Engine, skin, chart, BGM, Desktop MV, options (`1=10.8` only), 1920x1080, 60 FPS, default WGPU/pipeline are the user's inputs. Controlled before/after runs use `--profile` and a preserved pre-change release executable. The earlier whole-chart preparation times below are the user's manual observations; the prior whole-chart total wall time was not recorded, so it is not estimated here.

| Measurement | Before | After |
|---|---:|---:|
| Whole-chart discovery | ~186 s (user observation) | ~0.53 s (isolated release discovery) |
| Whole-chart Rendering phase begins | ~356 s (user observation) | ~1 s (rounded progress log) |
| Expensive whole-chart Watch traversals | 3 | 1 |
| Whole-chart total export wall time | not recorded | 229.23 s |
| 20-second control total wall time | 31.67 s | 24.76 s |
| 20-second control Rendering begins | ~8 s | <1 s |
| Whole-chart inferred boundary | 121.95 s | 121.95 s |
| Whole-chart submitted frames | 7,317 | 7,317 |

The new whole-chart pipeline's first-frame latency after session initialization is 541.9 ms. Startup (832.6 ms), encoder launch, session initialization (943.7 ms), and this handoff imply approximately 2.3 seconds until first submitted RGB, excluding small wrapper overhead. The progress line marks entry into Rendering, not actual first-frame completion. The old manual run did not record exact first-frame handoff time.

The new actual stream runs 7,317 runtime frames, 954,258 callbacks, and 755,388,441 evaluations. SFX event-only frames/callbacks/evaluations are all zero. Cheap discovery still executes 8,231 setup/schedule callbacks; that work is explicitly separate from the single playback traversal. The 20-second control runs 1,200 runtime frames, 65,918 callbacks, and 70,046,431 evaluations once; the baseline ran this workload again for SFX.

Final isolated release discovery: 12 FPS **0.539 s**, 30 FPS **0.529 s**, 60 FPS **0.527 s**. Discovery at all three rates resolves the same 121.95-second boundary with zero playback frames and the same callback/population counts. `artifacts/single-watch/discovery.json` records the final measurements. Discovery cost is independent of output frame count.

### Media equivalence and FFprobe

Control RGB sequence SHA-1 is unchanged: `16f248719e61c5384b0919620be62204e1b0e06b`. Whole-chart RGB sequence SHA-1 is `4041f44008c855075876e912cb57db004bd884ff`. SFX event SHA-1 remains `be53125a6dd38ad4b59dc7fc15b2ca3ca99c1808` with 0 immediate operations, 615 scheduled plays, 89 loop starts, and 89 loop stops.

The user's existing whole-chart output `artifacts/e2e/baumkuchen_test_gpu_6.mp4` provides a complete prior-media comparison. Its encoded H.264 and AAC payload hashes match the new whole-chart export exactly. The 20-second before/after payload hashes also match. Whole-chart packet PTS, DTS, and duration sequences are identical for all 7,317 video packets and 5,253 AAC packets. These comparisons establish unchanged visible frames, audio, and their timing, including MV synchronization; they do not infer new Sonolus compatibility claims.

| Whole-chart stream | Codec / format | Start | Duration | Frames/packets |
|---|---|---:|---:|---:|
| Video | H.264 / yuv420p / 1920x1080 / 60 FPS | 0 | 121.950000 s | 7,317 |
| Audio | AAC | 0 | 121.950000 s | 5,253 |

The 20-second control has 1,200 video frames; both streams start at zero and last exactly 20.000000 seconds. `whole-after.json`, `control-before.json`, `control-after.json`, `packet-hashes.json`, and `packet-timestamps.json` retain FFprobe, command, timing, and equivalence evidence. New whole-chart media is `artifacts/single-watch/whole-after.mp4`. No staged `sono-*` files remain after successful exports.

### Validation

Final `cargo test`: **171 passed**, 0 failed (63 library, 7 CLI, 3 arctan2 integration, 98 main integration; no doctests). Focused single-Watch export and release whole-chart tests also pass. `cargo build --release`, `cargo fmt --all -- --check`, and `git diff --check` pass. Existing deprecated-atomic and retired-prepass helper warnings are not VM changes.

A final defensive check aligns discovery's post-preprocess Runtime Update/scaled-time/timescale inputs with `frame(0)`, tested against a deliberately distinct host time-map anchor. Final release discovery still produces the same real-fixture boundary/evidence. Playback rendering code and media output are unaffected by this boundary-only initialization.

Reproduce the control and whole-chart run:

```powershell
python artifacts/single-watch/run_exports.py control-before control-after whole-after
cargo test --release whole_chart -- --nocapture
cargo test
cargo build --release
cargo fmt --all -- --check
git diff --check
```

To run only the current whole-chart export, use `python artifacts/single-watch/run_exports.py whole-after`. The script adds `--profile` and writes to the diagnostic artifact directory; it does not overwrite the user's existing baseline output.

