# Sono-Renderer CPU Scheduling and Architecture Report

Date: 2026-10-06

## Executive summary

The main confirmed CPU bottleneck is Watch preprocessing. In profiled one-frame
runs it used 87–94% of Watch frame time on the three supplied charts. The
active `UpdateParallel` stage used only 8–12 ms per frame, and the largest
active entity wave was 152 entities. Those charts did not contain a safe,
contiguous callback batch large enough to justify starting workers after
measured scheduling thresholds were applied.

The new execution path is the default and has a persistent, bounded worker
pool for large independent callback batches. It commits outputs in source
order and leaves shared-state callbacks on the ordered path. The explicit
single-threaded opt-out remains available. The scheduler is semantically
validated, but the supplied real charts do not show a CPU parallel speedup:
five-run automatic-vs-single timing differences were within about 0–2.3%,
with overlapping sample ranges.

The measurable throughput gain in this pass came from changing Watch memory
from full-map callback snapshots to shared immutable snapshots and
copy-on-write callback overlays. In comparable single-threaded runs with VM
accounting enabled, Larp Watch time fell from a 206 ms median to 157 ms
(-23.8%), and ARMAGEDDON fell from 2,727 ms to 2,115 ms (-22.4%). These
comparisons use the immediately prior release binary as the baseline; they
are not a comparison of the parallel scheduler against the opt-out.

## Current profile and serialization

Profiled one-frame Watch runs with VM accounting disabled reported:

| Chart | Runtime entities | Active entities | Callbacks | VM evaluations | Preprocess | Frame time | Preprocess share |
|---|---:|---:|---:|---:|---:|---:|---:|
| Larp | 576 | 143 | 1,223 | 2.06M | 127.4 ms | 146.8 ms | 86.8% |
| ARMAGEDDON | 8,030 | 152 | 18,282 | 31.66M | 1,894.6 ms | 2,021.5 ms | 93.7% |
| Baumkuchen | 3,602 | 114 | 8,325 | 10.26M | 788.7 ms | 905.6 ms | 87.1% |

The profiled `UpdateParallel` stage was 7.7–10.9 ms on those runs. Entity
initialization, spawn updates, and report materialization were also small.
This makes the large preprocessing pass the next CPU target; adding workers
to the active update stage alone would have little effect on these charts.

The Watch profile also counted 13.56M function dispatches on ARMAGEDDON and
4.39M on Baumkuchen. Callback VM evaluation dominates the visible Watch
work. Process wall time includes engine, resource, and level startup; those
startup steps were not separately timed by this benchmark. The video-export
profile measures the render pipeline and Watch frames, but does not add a new
GPU timestamp query. CPU/GPU overlap was not changed in this pass.

## Architecture changes

### Callback memory snapshots

Memory maps are shared through `Arc` snapshots. A callback sees an immutable
global snapshot plus its own entity layer; writes use copy-on-write in the
local layer. Temporary-memory block masks hide inherited values while still
allowing the callback to write local loop state. Long overlay chains are
compacted. This removes the previous full global-map copy at each callback
boundary while preserving the VM's visible memory rules.

The current profile's copy-on-write counter recorded 2,268 copied entries
for Larp, 32,730 for ARMAGEDDON, and 15,306 for Baumkuchen. These are current
counts, not a same-chart before/after allocation comparison. Sampled peak
working-set values varied between runs, so this pass does not claim a precise
peak-memory reduction.

### Worker pool and ordered commit

Large safe batches use persistent named workers with bounded per-worker
channels. Workers claim callbacks through a shared atomic cursor and return
private callback results. The runtime sorts results by task index and commits
memory, Draws, events, and other output in source order. Static checks retain
callbacks that touch shared host state, entity arrays, indirect memory
addresses, or ordered particle/loop state on the serial path. If a callback
unexpectedly writes outside its isolated memory overlay, its speculative
result is discarded and the remaining stage is rerun in order. Errors retain
the sequential error location and committed-prefix behavior.

The current measured cutoffs are 192 active stage entities before scheduler
analysis and 128 callbacks in a contiguous safe batch before dispatch. These
are based on the supplied fixtures: the largest active wave was 152 and the
largest safe callback batch in the exploratory run was 107. Earlier
instrumented experiments dispatched 61 Larp callbacks, 107 ARMAGEDDON
callbacks, and 77 Baumkuchen callbacks, but whole-frame timings did not
improve. The measured worker busy/slot-time ratios for those single profiled
runs were 65.5%, 46.9%, and 63.1%; profiling overhead and one-run variance
make those ratios diagnostic rather than throughput results. Raising the
batch cutoff avoids paying that setup/merge cost on the current charts.

No generic task graph was added: current safe work is small, and observable
callback, memory, Draw, and event ordering still requires a serial commit
boundary. The simpler entity-batch scheduler fits the measured workload.

### Opt-out and Sono-GCC

Parallel Watch scheduling is the normal path. `--single-threaded` disables
the scheduler without constructing its worker pool; `--watch-workers COUNT`
caps pool size. The same setting is available through `RenderConfig` and the
GUI. The opt-out disables parallel scheduling and retains ordered callback
execution; the copy-on-write memory representation is shared by both modes.

Sono-GCC remains compatible with callback execution: worker batches retain
the compiled-program handle and callback execution mode. No Watch operation
is moved across its semantic boundary. The regression suite includes the
existing VM/GCC differential coverage.

## Benchmark results

Measurements used the release build on Windows with 12 logical processors
available. The production-like `run-watch` command used a one-second timeline,
no UI/particles/SFX/BGM, and `--no-vm-accounting` to avoid diagnostic
per-operation counters. Each process rendered one Watch frame; frame time
excludes engine/resource loading, while process wall time includes startup.

### Automatic mode versus explicit single-threaded mode

Five runs per mode, medians shown. Worker count was automatic on the default
path. The percentages compare automatic mode with single-threaded mode.

| Chart | Frame median, single | Frame median, auto | Frame delta | Process wall, single | Process wall, auto | Wall delta | CPU ms, single / auto |
|---|---:|---:|---:|---:|---:|---:|---:|
| Larp | 99 ms | 98 ms | -1.0% | 0.884 s | 0.887 s | +0.3% | 797 / 797 |
| ARMAGEDDON | 1.226 s | 1.254 s | +2.3% | 2.016 s | 2.057 s | +2.1% | 1,938 / 1,969 |
| Baumkuchen | 444 ms | 447 ms | +0.7% | 1.247 s | 1.248 s | +0.1% | 1,203 / 1,172 |

The five-run frame ranges overlapped: Larp 94–103 ms single and 92–111 ms
automatic; ARMAGEDDON 1.218–1.301 s and 1.239–1.375 s; Baumkuchen 421–488 ms
and 438–459 ms. The small differences are noise-level on these runs and do
not show a real-chart parallel speedup.

Raw run-watch rows and profile captures are in
`artifacts/sono-renderer-followup5/`, including
`final-single-auto-5x.json`, `watch-worker-benchmarks-no-accounting.json`,
`baumkuchen-worker-benchmarks-no-accounting.json`, and
`watch-worker-profiles-final.json`. The benchmark runner is
`scripts/benchmark_watch_workers.ps1`.

### Worker-count sweep

Three-run median Watch frame times in seconds, with VM accounting disabled:

| Chart | Single | Auto | 2 workers | 4 workers | 8 workers | 12 workers |
|---|---:|---:|---:|---:|---:|---:|
| Larp | 0.097 | 0.092 | 0.095 | 0.095 | 0.096 | 0.091 |
| ARMAGEDDON | 1.210 | 1.225 | 1.223 | 1.221 | 1.226 | 1.261 |
| Baumkuchen | 0.470 | 0.487 | 0.453 | 0.444 | 0.433 | 0.450 |

The final cutoff run recorded zero parallel callbacks, batches, worker slots,
and worker busy time for all three charts. Specifying 2–12 workers therefore
did not create parallel work in these measurements. The small differences in
the table are timing noise, not scaling.

### Previous-build comparison for the memory change

These VM-accounting-enabled Watch frame medians compare the immediately prior
release binary, which used full-map callback cloning, with the current
copy-on-write implementation in single-threaded mode:

| Chart | Prior release | Current release, single-threaded | Change |
|---|---:|---:|---:|
| Larp | 206 ms | 157 ms | -23.8% |
| ARMAGEDDON | 2.727 s | 2.115 s | -22.4% |

Baumkuchen measured 968 ms on the prior release and 977 ms in one current
profiled run, which is not enough evidence for a change. Historical and
current values above came from three repeated Watch runs except for that
Baumkuchen sample. These are one-frame Watch timings, not sustained video
throughput results.

### Full CPU video export check

The Larp fixture rendered 12 frames at 640x360 and 12 fps through the CPU
backend, with audio enabled and determinism preflight enabled, in both
automatic and single-threaded modes. Both runs completed and mixed SFX. All
12 raw Draw hashes and all 12 RGB hashes matched in order; the resulting MP4
files also had identical SHA-256 hashes. Automatic mode recorded zero worker
callbacks and zero batches on this export. The pipeline reported a queue
capacity of 2 and one render worker; the automatic run recorded 304 ms in
Watch and 450 ms in CPU rendering, with 194 ms first-frame latency. Pipeline
stage totals overlap across frames and should not be summed. The matching MP4
SHA-256 was
`83297254AC625DEB247EA6EDE2E6F3D1B49C87D7E106B1A53A0E583337F1914F`.
This validates output compatibility; it is not a multi-minute steady-state
throughput benchmark. The per-run logs are in the same artifacts directory.

The run-watch A/B results also matched callback counts, Draw counts, and raw
Draw SHA-1 across all worker modes:

| Chart | Callbacks | Draws | Raw Draw SHA-1 |
|---|---:|---:|---|
| Larp | 1,223 | 111 | `af6be453f7bddb73d89f06beebee2d05ad06003e` |
| ARMAGEDDON | 18,282 | 75 | `2dc4f2cdcedf0f9d61586a2f9341ca4b14ae2580` |
| Baumkuchen | 8,325 | 59 | `24988258a2f6d90432acf9de5e0a1c73ceec70c3` |

## Validation

- `cargo build --release --offline` succeeded for the final code.
- `cargo fmt --check` and `git diff --check` passed.
- Full `cargo test --offline` passed: 114 library tests, 11 CLI tests, 3
  arctan tests, 101 M0 integration tests, and 9 Watch scheduler tests; 3
  library tests were ignored.
- Scheduler tests compare parallel output with ordered execution, exercise a
  256-callback independent batch, verify shared host-array writes stay
  ordered, check dynamic shared-memory fallback, preserve initialization
  error prefixes, and verify the single-threaded opt-out stays lazy.
- The CPU video export passed determinism preflight in both scheduler modes;
  Draw/RGB sequences and MP4 SHA-256 matched.
- A dedicated thread sanitizer or race detector was not run. Concurrency
  coverage here is differential and integration testing rather than a
  sanitizer proof.

## Remaining bottlenecks and limits

1. Preprocess remains the dominant serial stage. The real charts' safe
   contiguous batches are below the measured dispatch threshold, while
   indirect/shared host-state operations must retain source order. The next
   useful optimization target is preprocessing callback cost or a larger
   genuinely independent workload, not a larger worker count.
2. This pass did not measure long-duration video steady-state throughput or
   a reliable before/after peak-memory delta. Current memory samples are
   coarse process working-set samples and include startup.
3. Engine, resource, and level preparation are included in process wall time
   but are not separately instrumented here. GPU submission/readback timing
   remains outside this CPU scheduling pass.
4. Real chart workloads did not activate the worker pool after the measured
   cutoff was set. The 256-callback synthetic fixture verifies actual worker
   execution and deterministic merge, but does not establish a real-chart
   throughput gain.

The scheduler now avoids dispatching the small safe ranges found in these
fixtures while keeping parallel execution available for larger independent
stages. The measured result is a faster shared-memory path and a validated,
conservative scheduler, with no unsupported parallel speedup claim.
