# DISPLAYHOLIC GUI export panic investigation

## Status

The reported production panic has **not been reproduced and its root cause is not established**. Do not mark it fixed. No backend/renderer workaround was applied.

The supplied `DISPLAYHOLIC AUDIO.json.gz` (confirmed by the user, not the separate `DISPLAYAHOLIC.json.gz`) resolves to 20.200000s. Native GUI exports completed with MV, WGPU/pipeline, BGM/SFX and particles enabled, with ProSeka Faithful 0.8.3 selected as both skin and particle SCP. Settings were GUI defaults: Watch start 0, whole chart, 1920x1080, 60fps, MV background 100%, no explicit level-option overrides or determinism preflight.

Two launch environments were exercised: inherited tool-session handles and an explicit Windows `CREATE_NEW_CONSOLE` launch, followed by the existing GUI `FreeConsole()`. Both succeeded. Therefore the post-discovery stderr write is not an established cause; it was not changed. No evidence implicated MV, same-file resource overrides, duration propagation, WGPU initialization, FFmpeg staging or paths containing spaces in these runs.

## Changes limited to GUI diagnostics

`src/gui.rs`:

- `catch_export_panic` installs a once-only hook with thread-local scope. It captures the actual string payload (or labels a non-string payload), source location and `Backtrace::capture()` **at the panic origin**, before unwinding. Other threads delegate to the previous hook. Successful exports have no altered backend behavior.
- Worker errors carry the last progress phase and engine/level/output paths. The full error chain is retained in Export log. The summary shows the root error's first line rather than obscuring it behind context.
- Export log has Copy log, including multiline backtraces. Set `RUST_BACKTRACE=1` before launching the executable to enable captured traces (Windows release symbol availability may limit details).
- `poll` logs an error once; it previously appended both `{error:#}` and the identical summary. Cancellation and successful summaries remain visible, and failed jobs return to idle.
- Focused tests cover caught panic payload/origin/recovery, non-string panics, and single error logging/cancellation/duplicate-job prevention. The origin assertion uses `file!()` to support Windows path separators.

No backend, CLI, particle, VM, MV, whole-chart, GPU, profiling, RGB24 or base-clear behavior was modified in this investigation. No golden values changed.

## Reproduction evidence

Validation artifacts are under ignored `artifacts/gui/displayholic/`:

- `fresh-console-success.png`: completed native GUI, 1212/1212 frames, 20.20s, approximately 42.5s elapsed.
- `cancelled.png`: next export cancelled at Rendering 0/1212 in 0.6s; GUI idle, existing completed output preserved.
- `render.mp4`: successful native GUI export.
- `cli.mp4`, `cli-command.json`, `cli.txt`: equivalent CLI configuration; exit 0.
- `media-validation.json`: FFprobe reports H.264 1920x1080, 1212 frames, video/audio/container duration 20.200000s for GUI and CLI outputs.
- `focused-tests.txt`, `full-suite.txt`, `release-build.txt`: validation transcripts. Final full suite: 182 passed (73 library, 8 CLI, 3 arctan2 integration, 98 m0; 0 doctests), 0 failures. Formatting/diff checks and offline release build passed.
- `final-release-status.png`, `final-release.mp4`, `final-gui-log.txt`: final release also completed the same native GUI workflow (42.9s). Copy log was exercised through the native GUI and its exported text verified.
- `backend-error.png`, `backend-error-log.txt`: intentionally missing MV produced a descriptive backend error after discovery, returned to idle, retained the completed output and logged the error exactly once. The real MV selection was then restored.

The fresh-console run used the actual native Open/Save pickers and Render MP4 button, not a backend-only shortcut. The resource/level/media paths contain spaces. Output is in the workspace; the user's existing chart-directory output was not replaced.

## Next evidence required

If the failure recurs in the rebuilt release executable, open Export log and use Copy log. Preserve the panic payload, origin/backtrace, phase, selected engine options, resolution/FPS, validation settings and launch method. A backend fix must follow that concrete failure; do not infer one from a successful reproduction. The older generic log has no recoverable panic payload.
