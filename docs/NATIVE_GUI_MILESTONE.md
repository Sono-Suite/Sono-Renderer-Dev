# Native GUI + MV background percentage

## Launch and normal workflow

Build with `cargo build --release`. Double-click `target/release/renderer.exe` or launch it without arguments; `renderer gui` also opens the native window. Existing CLI subcommands keep their console output. On Windows only the GUI detaches its console. Owned media processes run without new console windows and use explicit stdin handles.

1. Project / inputs: use Browse for engine ZIP, level, skin/SCP, optional separate particle SCP, music, and optional MV. Choose output with Save as. Dialog cancellation preserves selections. Separate SFX/background collections and resource-name overrides are available in the resource section; absent overrides use the skin SCP.
2. Engine options: metadata supplies names, toggle/select/slider controls and defaults. English localization strings are displayed when provided. Numeric overrides preserve values outside the advertised widget ranges; arbitrary numeric indices are supported and checked by the backend.
3. Render / video: choose start, Whole Chart or manual duration, dimensions, FPS, CPU/WGPU, pipeline and layers. Whole Chart disables the remembered manual duration. Use selected MV retains the path when disabled. MV background is a presentation brightness slider, not a client-compatibility claim.
4. Render MP4 invokes the shared Rust backend on a worker. Cancel requests cooperative shutdown. Tabs, progress, and logs remain responsive; settings and duplicate Render launches are disabled during an export. Closing requests cancellation before exiting.

## Implementation boundaries

- `src/gui.rs`: native eframe/glow window, rfd dialogs parented to the real window, grouped tabs, options, bounded latest-progress state and 200-line logs. No browser, HTTP server, web page, command-output scraping, or second renderer.
- `src/render.rs`: shared typed `RenderConfig`, `ResourceOverrides`, MV enable/percentage validation and controlled-export entry point.
- `src/export_control.rs`: shared request token, typed Cancelled error, log callback, cancellable owned FFmpeg command polling/reaping. Cancellation is cooperative: an in-flight resource load, VM callback, raster/GPU operation, or media pipe transfer completes before the next safe check. Existing managed-tool provisioning remains synchronous on the worker, with cancellation observed afterward.
- `src/watch_runtime.rs` and `src/offline.rs`: cancellation checks at frames/callback boundaries, including pre-roll and schedule evidence; independent resource source selection. No callback skipping, VM optimization or renderer semantic changes.
- `src/video_export.rs`, `src/export_media.rs`, `src/export_progress.rs`: token propagation, media cancellation, warning/whole-chart logs, complete-frame progress, staged-output safety. Validated output is published last. Normal exports still use one authoritative Watch playback; SFX comes from it; post-render mux copies encoded video.
- `src/export_mv.rs`: brightness of decoded RGB only, before atlas identity and the shared CPU/WGPU background compositor. Following the MV alpha audit, decoded MV presentation alpha is forced to 255; PTS, local/global transforms, particle rendering and gameplay are unchanged. A separate MV enable setting prevents decode entirely when off.
- `src/main.rs`: no-argument GUI launch; CLI `--mv-background`, `--no-mv`, `--particle-resources`, `--particle-name` produce the same configuration.

## Validation

Focused regressions cover finite 0..100 validation/default/serialization, CLI parsing, picker cancellation with spaced Unicode paths, metadata localization, arbitrary option values, duplicate-job prevention, idle/error/cancel transitions, bounded typed progress/log state, running-child cancellation, and staged-file cleanup on Render/AudioEncoding/Finalizing cancellation.

Timestamped lossless MV tests compare 0/40/100 percent selected PTS, alpha, geometry and active intervals. A composition test proves an opaque gameplay sprite stays unchanged over all percentages in CPU and WGPU. The existing real/synthetic pipeline regressions run across percentages and assert unchanged frame count, Watch runtime values, MV timestamps and SFX hashes. Existing 1080p pipeline/sequential, audio-window, particles and golden regressions remain intact; no golden values were updated.

### Desktop evidence

Windows desktop validation uses actual native Open/Save dialogs and keyboard/mouse controls, with the existing Baumkuchen fixture. It does not inject RenderConfig or call a hidden GUI test backend. Test artifacts are under `artifacts/gui` (ignored). The helper is confined to Sono-Renderer-owned windows; initial sandbox desktop access was unavailable, so interactive-desktop tool approval was used.

Inputs: Next RUSH engine ZIP; ProSeka Faithful 0.8.3 SCP selected for skin and particles; Baumkuchen x Retry Now JSON gzip, supplied MP3 and desktop MV. Option 1=10.8, start 0, duration 20, 1920x1080, 60 FPS, WGPU, pipeline, particles/SFX/BGM enabled.

- Native dialogs populated all inputs; a cancelled Engine picker retained the path. Missing inputs were reported visibly.
- 100 percent MV export completed 1200 frames in 22.4 seconds; H.264 1920x1080 at 60/1 plus AAC, 20.000000 seconds. The initial Save automation accepted its default desktop `render.mp4`; the resulting file was copied to `artifacts/gui/mv-100.mp4`. Later automation correctly targets the modern Save filename control and selects artifact paths.
- 0 percent export completed; `artifacts/gui/mv-0.mp4`. `zero-live-progress.png` records 1006/1200, 83 percent, 20.3 elapsed seconds, 53.0 FPS, ETA 3.7 seconds.
- Whole Chart disabled duration, inferred 121.950000 seconds / 7317 frames and showed 723/7317 with FPS/ETA. Cancel returned idle by the next screenshot at 749/7317; `cancelled.mp4` was absent and all `sono-*` staged files were removed. Existing output preservation is independently covered by backend tests.
- Native launch testing found and corrected stale inherited stdin after FreeConsole (OS error 50) and dialog ownership. Those failures are GUI launch/resource-process issues; no MV timing semantics were altered.

### Final results

- Final-build native no-MV export: Use selected MV unchecked, path retained; `artifacts/gui/no-mv-verified.mp4`, 1200 frames / 20.000000 seconds, H.264 1920x1080 60/1 + AAC; completed in 18.7 seconds. `no-mv-complete.png` proves the intended output path, and `final-option-speed.png` shows option 1=10.8. Actual keyboard input in the OS Save filename field removed a modern DirectUI automation ambiguity. The earlier no-MV run accepted the default desktop `Charts/mv-100.mp4`; it was copied to `artifacts/gui/no-mv.mp4` and has the same output bytes.
- Final-build picker cancellation retained Engine selection (`final-picker-cancel.png`).
- All 0%, 100%, and both no-MV exports contain exactly 863 identical AAC packet payloads with identical PTS/DTS/durations. SHA-256 of the canonical ordered audio packet evidence: `9d35995ffac7349c9ee82ba5734062ce8ebd23b2fcdbae58248097590c64d94b`. Probe/packet evidence: `artifacts/gui/media-validation.json`.
- Extracted Watch/output 15.0-second comparison frames: `frame-0.png` and `frame-100.png`. Gameplay remains visible at zero; the MV is black. This establishes the requested presentation control, not Sonolus client percentage semantics.
- CLI validation rendered two frames at 64x36/10 FPS, start 0 / duration 0.2, with `--mv-background 40 --no-mv --mv 'intentionally absent disabled MV.mp4'`: successful MP4 (`cli-no-mv.mp4`), proving disabled MV is not opened. Existing CLI console/progress output remains functional.
- Final complete suite: **181 passed, 0 failed** (72 library, 8 CLI, 3 arctan2, 98 m0; 0 doctests). Log: `artifacts/gui/full-suite.txt`. PowerShell's merged native-stderr redirection records warnings as NativeCommandError and the tool wrapper reports status 1; every Cargo test result, including doctests, reports success with zero failures. An earlier run was invalidated by editing dependencies during its doctest phase; the final suite was run without concurrent source changes.
- `cargo fmt --check`, `git diff --check`, `cargo build --release --offline` passed. Only pre-existing deprecated atomic/retired profiling helper warnings remain.
- No golden expectations changed. No production particle, MV timing, GPU, base-clear, RGB24, SFX scheduling or VM operation implementation was changed. Watch only receives cooperative request checks at safe boundaries; all existing runtime regression tests pass.

Desktop validation is complete. No real Sonolus oracle, particle investigation, performance work, or compiler work is required to resume this milestone.

