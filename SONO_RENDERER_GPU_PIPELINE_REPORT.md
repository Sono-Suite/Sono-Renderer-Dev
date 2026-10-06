# Sono-Renderer GPU Pipeline Investigation

## Result

The useful changes are aligned draw-parameter batching and an optional, bounded two-frame GPU render pool. The first reduces CPU command setup; the second overlaps command submission, GPU work, readback, and CPU unpacking. On the tested 4K workload, two render workers improved median export time by about 7%. At 1080p, extra workers did not help, so the default remains one worker. The pool can be selected for an export with `SONO_WGPU_IN_FLIGHT=2`.

The pipeline is still synchronous at each worker's readback boundary: it requests `map_async`, then waits with `Device::poll(wait_indefinitely())`. The render pool overlaps that wait with other frames, but does not remove it. Profiling shows why a broader rewrite is not currently justified: 4K benefits from two workers, while Watch preparation becomes the limiter and three or four workers spend more memory for little or no end-to-end gain.

## Workload and method

Measurements used the repository's `larp 64x` fixture, Next RUSH engine and resources, WGPU on **AMD Radeon(TM) Graphics / Vulkan / IntegratedGpu**, Sono-GCC, audio/SFX enabled, and FFmpeg's normal export settings. The 4K case rendered 24 frames (2 seconds at 12 FPS); the 1080p case rendered 60 frames (5 seconds at 12 FPS). Each 4K worker depth was repeated three times; each 1080p depth was repeated twice. The benchmark records renderer-reported export time, process wall time, and sampled process working set (50 ms sampling interval).

GPU timestamp queries are not enabled by this backend, so `GPU wait/map` is wall time from submission through mapping completion, not a measurement of GPU execution alone. CPU/GPU utilization percentages are therefore not reported. Stage totals in the profiler overlap and must not be added together.

## Optimizations and A/B results

### Draw parameter batching

The renderer now packs the frame's aligned `Params` records into one uniform buffer and uses dynamic offsets. Each resident atlas gets one bind group per frame instead of allocating a uniform buffer and bind group for every draw. The pipeline and atlas textures remain persistent across frames.

| Resolution | Encode/submit before | After | Change |
|---|---:|---:|---:|
| 640×360 | 1.769 ms/frame | 0.394 ms/frame | −78% |
| 1920×1080 | 2.35 ms/frame | 0.799 ms/frame | −66% |
| 3840×2160 | 1.58 ms/frame | 0.681 ms/frame | −57% |

The WGPU RGB and SFX sequence hashes were unchanged in the before/after checks. End-to-end improvement is smaller because command setup was only one part of export time.

### Bounded frames in flight

`SONO_WGPU_IN_FLIGHT` selects 1–4 persistent WGPU render workers; invalid values fall back to 1. CPU rendering always uses one worker. Prepared Watch frames remain in a capacity-2 queue. Worker input queues rendezvous, completed results are bounded by worker count, and a frame-indexed reorder map writes to FFmpeg strictly in order.

| Workload | Workers | Median export | Median render phase | Sampled peak working set |
|---|---:|---:|---:|---:|
| 4K, 24 frames | 1 | 5.821 s | 1.690 s | 684 MB |
| 4K, 24 frames | 2 | **5.398 s** | 1.333 s | 946 MB |
| 4K, 24 frames | 3 | 5.442 s | 1.307 s | 1,097 MB |
| 4K, 24 frames | 4 | 5.384 s | 1.312 s | 1,121 MB |
| 1080p, 60 frames | 1 | 5.220 s | 2.104 s* | 666 MB |
| 1080p, 60 frames | 2 | 5.212 s | 2.119 s* | 672 MB |
| 1080p, 60 frames | 3 | 5.270 s | 2.118 s* | 678 MB |

`*` The 1080p render phase is from the first run at each depth; the table's export time is the median of two runs. The 8 ms difference between one and two workers is within run variation.

At 4K, two workers reduced median export time by 7.3% and median process wall time from 6.46 s to 6.04 s. Three workers made the render phase marginally shorter but made export 44 ms slower than two workers. Four workers were only 14 ms faster than two in renderer-reported export time, while their median process wall time was 59 ms slower and working set was another 175 MB higher. Two is the practical ceiling on this machine. At 1080p, one worker remains the sensible setting.

The pool moves the bottleneck: on the 4K single-worker run, Watch preparation took about 1.0 s, the producer was blocked for 465 ms, and the render consumer waited for prepared work for 269 ms. With two workers, producer blocking fell to zero and consumer waiting rose to about 491 ms. The consumer is now more often waiting for Watch, rather than forcing Watch to wait behind a completed render/readback.

Every repeated 4K and 1080p output had the same ordered per-frame RGB SHA-1 sequence at worker depths 1, 2, 3, and 4. The 4K sequence hash was `59b1b9e401af5eae2080203259606336d52e04a2a`. The SFX event hash stayed `065623858bbffa464f9c2da960afbeefd8d4586b` (93 scheduled effects, 3 loop starts, 3 loop stops). The renderer's FFprobe validation passed for all benchmark outputs.

## Pipeline costs and decisions

On the 4K single-worker run, the median GPU wait/map was 26.2 ms/frame and readback unpack was 13.9 ms/frame. The median encode/submit cost was 0.62 ms/frame. Readback was 33,177,600 bytes per frame. Creating the render target and readback buffer took about 0.13 ms and 0.02 ms per frame respectively. A per-worker resource pool was not added: those setup costs are negligible beside readback and unpack, while a pool would retain the large target and staging allocations.

FFmpeg handoff took 72 ms across the 24-frame single-worker export (about 3 ms/frame); ordered output writes are not the dominant rendering cost. The existing bounded worker/result flow lets rendering continue while prior frames are handed to FFmpeg, and adding another encoder queue would add buffering without removing the measured Watch or readback costs. Backpressure remains bounded if the encoder slows.

A prototype that moved per-frame diagnostics and SHA-1 work onto render workers did not show a stable improvement. Summed per-frame diagnostics/hash time remained about 0.29–0.31 s for 24 frames, and the apparent small differences between worker depths were within run variation. It was removed. The RGB digest is part of the renderer's emitted frame diagnostics, so skipping it would change observable output.

The row-by-row RGBA-to-RGB readback conversion remains a meaningful CPU cost. The export also has to produce an exact RGB SHA-1 for each frame, so merely passing the mapped RGBA buffer through to FFmpeg would not remove all RGB traversal while preserving the current diagnostic contract. No output-format rewrite was adopted without evidence that it beats this path end to end.

## Final architecture

- Default: one render worker, preserving current memory use and avoiding regressions at 1080p.
- Optional high-resolution mode: `SONO_WGPU_IN_FLIGHT=2`, with the same ordered output and a fixed upper bound on frames in flight.
- GPU device, pipeline, and atlas textures stay shared/cached. Per-frame render targets, readback buffers, and the aligned draw-parameter buffer are still created per frame.
- Each worker still waits for its own mapped readback. A separate nonblocking readback-completion stage was not added because the measured worker pool already supplies the useful overlap; deeper pools mostly wait on Watch and increase memory.

## Validation and reproduction

- `cargo fmt --check` and `git diff --check` passed.
- `cargo test --offline`: 230 passed, 3 ignored. The bounded FIFO integration test also passed again after the final worker-pool code was restored.
- `cargo build --release --offline` passed using an isolated target directory.
- A 24-frame `--validate-determinism` WGPU export at three workers matched the sequential preflight and completed with the same RGB and SFX hashes.
- Repeated real fixture exports validated frame counts, ordered RGB hashes, SFX hashes, and FFprobe output.

Build into the isolated benchmark target and run the A/B harness with:

```powershell
$env:CARGO_TARGET_DIR = 'target\gpu-followup4-build'
cargo build --release --offline
.\scripts\benchmark_gpu_inflight.ps1 -Renderer 'target\gpu-followup4-build\release\renderer.exe'
```

The script accepts `-Workers`, `-Runs`, `-Width`, `-Height`, `-Duration`, and `-Fps`; it writes MP4s, logs, and a CSV summary under `artifacts\sono-renderer-follow-up4`.
