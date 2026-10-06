# Sono-GCC performance pass

## Follow-up #3: current results (2026-10-06)

The current release build uses a dense node-to-region dispatch table. The
previous `HashMap` lookup added measurable cost on a high-volume Watch workload.
In a targeted pre/post dispatch comparison (three baseline process runs and five
after the change), Bike GCC p50 fell from 11,639 ms to 9,429 ms (19.0%) with
identical graph, callbacks, Draw count, and Draw hash. Interpreter p50 was
effectively unchanged (10,618 ms to 10,573 ms). The dense table uses 4 bytes
per graph node: 853 KB for Larp and 652 KB for the shared Next Sekai graph.

Current paired release-process results use persistent compiled caches, a
one-second Watch run, and UI, particles, SFX, and BGM disabled. Each mode ran in
fresh processes; Dreamer, Baumkuchen, ARMAGEDDON, Larp, and LIMBO had three runs
per mode, while Bike had five. The table reports medians and observed ranges.
All six real fixtures matched callback counts, Draw counts, and raw Draw hashes
between VM and GCC.

| Fixture | VM p50 (range) | GCC p50 (range) | GCC vs VM | GCC regions / scalar cuts |
|---|---:|---:|---:|---:|
| Horizon / Dreamer | 58 ms (55–59) | 56 ms (55–58) | -3.4% | 8,162 / 13,120 |
| Next Sekai / Baumkuchen | 695 ms (694–710) | 643 ms (639–650) | -7.5% | 429,846 / 727,577 |
| Next Sekai / ARMAGEDDON | 2,337 ms (2,321–2,370) | 2,124 ms (2,100–2,141) | -9.1% | 1,721,407 / 2,934,128 |
| Larp 64x | 266 ms (264–269) | 248 ms (242–256) | -6.8% | 134,536 / 235,507 |
| Next Sekai / LIMBO | 13 ms (12–14) | 13 ms (13–13) | 0.0%* | 3,928 / 8,116 |
| Next Sekai / Bike | 10,573 ms (10,404–10,679) | 9,429 ms (9,394–9,959) | -10.8% | 9,619,958 / 15,476,160 |

*LIMBO's times are rounded to milliseconds; this is a coarse tie. These results
are steady-state Watch comparisons, not whole-render results. GCC now wins on
five of these six fixtures, but the overall export result below is still
slower.

### Compilation and cache

Cold compilation timing now separates region code generation from rustc. Larp
compiled in 64.562 s (1.205 s region codegen, 63.353 s rustc); Next Sekai took
42.765 s (0.903 s region codegen, 41.859 s rustc). DLL creation and cache write
were 0.216 s and 0.043 s for Larp, and 0.189 s and 0.035 s for Next Sekai.
Rustc dominates cold compilation.

The persistent Larp cache hit now restores precomputed operation totals from
metadata. Three fresh-process hits each reported a 0 ms operation summary;
lookup was 127–142 ms, comprising 93–103 ms graph identity, 9–11 ms disk
validation, and 24–28 ms metadata reconstruction. DLL loading was another
13–18 ms. Existing cache metadata without
the summary remains readable and computes the summary once. Streaming the
existing cache identity hash removed an 11.5 MB temporary graph copy and kept
the exact cache key, but did not measurably reduce identity time.

Bounded region-codegen workers (1, 2, and 4) and a temporary
`-Ccodegen-units=64` build did not consistently improve total cold compile
time, so neither change was retained. Worker counts produced 66.269, 69.917,
and 66.777 s total compile times; the 64-unit build took 69.995 s. Region
codegen was only about 1.15–1.23 s of the approximately 66–70 s total.

### Whole-chart result and remaining overhead

The current dense-dispatch 20-frame Larp SFX export used 1 fps and 16×16 CPU
rendering, with BGM on and UI/particles off. VM export took 3.084 s and GCC
3.311 s (+7.4%). Pipeline Watch was 600.0 ms vs 630.7 ms; callback evaluation
was 314.91 ms vs 336.24 ms. GCC's profile timers recorded 217 ms in the region
dispatch envelope, 169 ms in VM cuts, and 10 ms in native bodies; these phase
timers overlap and are not additive. GCC and VM matched the whole RGB hash
`5850187ad49282ebd7db747aa99ef1b1d386c46e`, SFX event hash
`065623858bbffa464f9c2da960afbeefd8d4586b` (93 scheduled effects, 3 loop
starts and stops), and encoded MP4 hash
`6B930AF84C08FC925D5B6B8BB3396DA2F9FB6916445B45EE33B305AA793E367E`. The
export profile therefore still shows a small end-to-end GCC regression even
though GCC is faster in the isolated steady-state fixtures.

The dominant cold-compile opportunity is rustc compilation, while current
cache-hit identity hashing remains about 0.1 s. In steady-state Watch, dispatch
was the demonstrated removable overhead; replacing the hash lookup with direct
indexing produced a large Bike gain without changing interpreter time or
observable output. Remaining VM cuts preserve ordered Watch semantics and
side-effects. The phase profile separates the dispatch envelope, VM cuts, and
native bodies, but does not isolate per-helper ABI latency or count allocations
independently. GPU execution time was not collected. GCC stays opt-in: these
measurements do not support making it the default.

Stateful and effectful operations still cross into ordered WatchVm cuts to
preserve memory, branching, rendering, and event semantics. Further pure-scalar
coverage or lower per-region setup/accounting cost remain implementation
opportunities, but the current aggregate dispatch timer does not isolate those
components enough to claim another change would pay for itself. This pass
therefore retains the directly measured dense-index gain and rejects the tested
compile-parallelism changes; it does not claim that every remaining GCC cost is
semantically unavoidable.

Full validation after the changes: `cargo fmt --check`, `cargo check --offline`,
`cargo build --release --offline`, and `cargo test --offline` passed. The final
test run passed 107 library tests (3 ignored), 10 CLI tests, 3 math tests, 101
renderer tests, 9 scheduler tests, and doc tests. Six real fixtures and the
whole-chart output passed the parity checks described above. Raw current
measurements are under `artifacts/sono-gcc-performance/followup3/`.

## Previous pass results (historical baseline)

### Previous pass summary

That pass profiled real chart execution, removed repeated hot-path work, made
persistent cache hits restore precomputed region metadata, and added a repeatable
benchmark harness. Exact Watch parity held across the tested fixtures. The
changes improved the measured GCC/VM gap on the large charts. At that checkpoint
GCC was still slower than the interpreter on every measured Watch workload and
whole-chart export. Those results were superseded by the Follow-up #3 results
above; GCC remains opt-in.

The main remaining GCC costs are ordered Watch VM cuts and per-region dispatch.
The whole-chart profile measured 249 ms in the GCC region-dispatch envelope,
including 194 ms evaluating Watch VM cuts and 10 ms executing native regions.
Those phase timers overlap and are instrumentation-enabled measurements; do not
sum them. The export profile shows initialization and render orchestration are
large fixed costs, while CPU frame compositing and FFmpeg handoff are small. GPU
execution time was not collected.

### Previous pass method

The four main fixtures were measured five times per mode in separate release
processes, sequentially, using `run-watch --time 1` with UI, particles, SFX, and
BGM disabled. The table reports the median Watch time and observed range. The
interpreter and GCC runs were paired by fixture, and the benchmark harness also
records process time, cache lookup, DLL load, callbacks, Draws, region and cut
counts, heap-input fallbacks, and Draw hashes.

The small and high-volume fixtures were measured the same way, also for five
runs. The 20-frame Larp whole-chart SFX export used three sequential VM/GCC
pairs at 1 fps and 16x16 on the CPU backend, with profiling enabled. Detailed
GCC phase timing is opt-in; its extra clock reads can add overhead. Phase totals
can overlap.

### Previous pass Watch workload results

| Fixture | VM Watch p50 (range) | GCC Watch p50 (range) | GCC overhead vs VM | GCC regions / scalar cuts at 1 s |
|---|---:|---:|---:|---:|
| Horizon / Dreamer | 56 ms (55–56) | 58 ms (56–61) | 3.6% | 8,162 / 13,120 |
| Next Sekai / Baumkuchen | 711 ms (694–778) | 741 ms (733–751) | 4.2% | 429,846 / 727,577 |
| Next Sekai / ARMAGEDDON | 2,336 ms (2,301–2,347) | 2,524 ms (2,497–2,595) | 8.0% | 1,721,407 / 2,934,128 |
| Larp 64x | 274 ms (266–288) | 291 ms (285–294) | 6.2% | 134,536 / 235,507 |
| Next Sekai / LIMBO (small) | 13 ms (12–14) | 14 ms (13–16) | 7.7% (1 ms) | 3,928 / 8,116 |
| Next Sekai / Bike (high volume) | 10,798 ms (10,456–10,859) | 11,527 ms (11,458–11,800) | 6.8% | 9,619,958 / 15,476,160 |

Watch times are rounded to milliseconds, so the percentage on the small chart is
especially coarse. Bike is a high-volume stress fixture; the available suite did
not provide an isolated arithmetic-only benchmark.

The first baseline pass had GCC medians of 73 ms (Dreamer), 913 ms
(Baumkuchen), 3,186 ms (ARMAGEDDON), and 319 ms (Larp). The latest measurements
are lower, but interpreter measurements also shifted between passes; these
before/after values show direction and are not a controlled causal estimate.
The current paired mode comparisons above are the more reliable result: GCC
still loses by 3.6–8.0% on the four main fixtures.

### Previous pass hot-path and cache changes

- Region cut inputs now use an eight-value stack buffer. Larger regions retain
  a heap fallback. Heap-input fallback counts were 0 for Dreamer, 3,088 for
  Baumkuchen, 6,794 for ARMAGEDDON, 812 for Larp, 64 for LIMBO, and 13,307 for
  Bike. This avoids a heap buffer for more than 99% of regions on the four main
  charts and the Bike stress fixture.
- Native function accounting now uses compact operation IDs and fixed counters;
  touched operation names are flushed once per Watch execution. This removes
  repeated per-region map scans and string cloning.
- Persistent cache schema 3 stores validated, root-ordered region metadata. A
  Larp cache hit previously spent about 1.226 s rebuilding that metadata; the
  schema 3 hit restores it in about 25 ms. Total persistent cache lookup fell
  from about 1.346 s to 139 ms (about 90% shorter). The current Larp split is
  about 100 ms graph identity, 10 ms disk validation, 25 ms metadata restore,
  and 13 ms DLL load. These are separate stage timings; DLL load is excluded
  from the reported cache-lookup duration.
- The schema change caused one cache rebuild. That single cold compile measured
  about 0.8 s for Dreamer, 42 s for the shared Next Sekai graph, and 65 s for
  Larp. Warm and cold figures are not interchangeable.

For the Larp graph, 134,536 native regions executed in one Watch second, with
235,507 ordered scalar cuts. The most frequent runtime-side operations included
`Get` (339,672), `Set` (142,426), `Execute` (108,246), and `If` (81,233). These
operations stay in WatchVm so memory, lazy control flow, and side effects keep
the authoritative semantics.

### Previous pass whole-chart profile and parity

The three profiled SFX-enabled Larp exports measured:

| Mode | Total export p50 (range) | First-frame latency p50 | Callback evaluator p50 | Pipeline Watch p50 |
|---|---:|---:|---:|---:|
| Interpreter | 2.918 s (2.915-2.920) | 257 ms | 0.322 s | 607 ms |
| Sono-GCC | 3.394 s (3.384-3.436) | 296 ms | 0.384 s | 666 ms |

GCC was 16.3% slower end to end. Each export produced 277,804 native regions
and 556,810 scalar cuts. Across the 20 frames, the opt-in GCC timers recorded
249 ms in region dispatch, 194 ms evaluating VM cuts, and 10 ms running native
regions. Region dispatch encloses the other two measurements. The current
process-cache hit still spent about 100 ms rebuilding graph identity.

For that export, CPU render/composite took about 2 ms total, FFmpeg handoff about
1 ms, FFmpeg finalization about 25 ms, and FFprobe validation about 57 ms.
Initialization/startup was about 1.19 s in interpreter mode and 1.44 s in GCC
mode; the render-orchestration residual was about 0.61 s and 0.67 s
respectively. These profile stages overlap and should not be added. GPU time was
not measured, and the low-resolution CPU run does not support a GPU bottleneck
claim. The first-frame figures include preprocess and come from warm-cache
exports; the GUI's background-compile fallback path was not timed interactively.

VM and GCC matched display-list JSON SHA-256, raw Draw SHA-1, callback counts,
and VM evaluation counts for Dreamer, Baumkuchen, ARMAGEDDON, Larp, LIMBO, and
Bike. The SFX-enabled whole-chart runs also matched the SFX event SHA-1
`065623858bbffa464f9c2da960afbeefd8d4586b` (93 scheduled effects, 3 loop
starts, 3 loop stops) and MP4 SHA-256
`5712E8C59DC1E7111A6F8E43E84D02C42E276BF3B3F8C77D7089C19BB1EC94B9`.

### What limited export performance in the previous pass

That measured export was limited by initialization, Watch execution, and
render orchestration. GCC does not reduce total export time on these fixtures;
its VM cuts and region-dispatch cost outweigh the native scalar work it saves.
CPU frame rendering and FFmpeg handoff are small in this low-resolution profile.
GPU timing and interactive GUI first-render latency were not measured. The
large-chart GUI precompile and cold-compile paths were not benchmarked across
worker counts.

### Reproduction and historical artifacts

Run the release benchmark harness with a fixture's engine, level, and resources:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\benchmark_sono_gcc.ps1 `
  -Name larp `
  -Engine 'TestingSuite\Next Sekai Engine\levels\larp 64x\Next RUSH.zip' `
  -Level 'TestingSuite\Next Sekai Engine\levels\larp 64x\larp 64x.json.gz' `
  -Resources 'TestingSuite\Next Sekai Engine\levels\larp 64x\ProSeka Faithful 0.8.4.scp' `
  -Runs 5 -Time 1
```

Raw logs and CSVs from this pass are in the ignored
`artifacts/sono-gcc-performance/` directory. The benchmark harness is
[`scripts/benchmark_sono_gcc.ps1`](scripts/benchmark_sono_gcc.ps1).

### Previous pass validation

- `cargo fmt --check`: passed.
- `cargo check --offline`: passed.
- `cargo test --offline`: 106 library tests passed (3 ignored), 10 CLI tests,
  3 math integration tests, 101 renderer tests, 9 scheduler tests, and doc tests
  passed.
- `cargo build --release --offline`: passed.
- Six real-engine VM/GCC display-list and Draw hash comparisons: matched.
- Larp whole-chart SFX event and MP4 hashes: matched.

The build reports existing warnings for a deprecated atomic method and unused
profiling/progress helpers.
