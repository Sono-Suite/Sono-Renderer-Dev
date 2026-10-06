# Sono-GCC implementation report

## Follow-up #3 performance update (2026-10-06)

The current release measurements replace earlier single-run performance
checkpoints below. Dense node-to-region indexing reduced Bike's GCC Watch p50
from 11.639 s to 9.429 s while VM stayed effectively flat at 10.618 s to
10.573 s. On six real fixtures GCC now wins the isolated one-second Watch run
on five, ties at millisecond resolution on LIMBO, and preserves Draw and
callback outputs. The current whole-chart Larp SFX export remains 7.4% slower
under GCC (3.311 s vs 3.084 s), so GCC remains optional.

Cold compile remains dominated by rustc: 63.353 s of Larp's 64.562 s compile
and 41.859 s of Next Sekai's 42.765 s compile. A persistent Larp cache hit
takes 127–142 ms before separately measured 13–18 ms DLL loading; memoized
operation-summary time is 0 ms on repeated fresh-process hits. A streaming
cache-identity hash preserves the exact key and avoids an 11.5 MB copy, with no
measurable timing improvement. Parallel code generation and 64 codegen units
were measured and not retained because total compile time did not improve
consistently.

The final validation run passed formatting, offline checking, release build,
and the full test suite (107 library tests, 3 ignored, 10 CLI, 3 math, 101
renderer, 9 scheduler, and doc tests). The detailed repeated measurements,
whole-chart hashes, and experiment results are in the
[Follow-up #3 performance report](SONO_GCC_PERFORMANCE_REPORT.md#follow-up-3-current-results-2026-10-06).
Per-helper ABI latency and allocation counts were not separately instrumented;
the report states those measurement limits.

## Coverage status

Sono-GCC accounts for all 161 operation kinds dispatched by the current `WatchVm`, using two deliberately different paths:

- **75 operation kinds have direct native scalar implementations.** They are emitted only inside a statically bounded, safe native region.
- **86 operation kinds are VM scalar cuts.** WatchVm evaluates each cut in ordered graph traversal and passes its `f64` result into a native parent when one is safe to compile.

This is **161/161 represented at the native boundary**, not 161/161 native execution. The 86 cut operations remain implemented and executed by WatchVm. The ABI is currently VM-to-native scalar input; generated code does not call back into the runtime. Graph nodes that do not fit a safe native region execute wholly in WatchVm. The inventory and operation-level observed coverage are in [SONO_GCC_COVERAGE.md](SONO_GCC_COVERAGE.md).

Watch reports now distinguish unique compiled operation nodes, statically eligible nodes, regions actually executed, and dynamic VM-to-native scalar cut calls. A scalar-cut count is an interface-crossing count, not an estimate of time saved. No separate native-to-VM helper overhead exists because that callback direction is not implemented.

## Native and VM behavior

The native implementation retains 32 easing variants along with the existing arithmetic, comparison, interpolation, and scalar math set. All constants preserve exact `f64` bits. Elastic easing stays in WatchVm because an earlier native formula changed a NaN payload.

The 86 VM cuts cover runtime queries, resources, memory, lazy control, loops, drawing, spawn, audio, particles, streams, and the Elastic easing variants. Each operation keeps WatchVm semantics for state, event creation, branch selection, lazy evaluation, errors, and memory access. `If` and related nodes still select branches inside WatchVm. Native scalar parents consume returned values only after ordered cut evaluation.

The source-size safety limit is now 64 MiB. This allows the supplied Larp graph to compile; the previous 8 MiB bound rejected it and correctly selected VM fallback.

## Cache, CLI, and GUI

GUI engine selection loads Watch data and starts precompilation in a bounded background worker when Sono-GCC is selected. Cache lookup and rustc compilation are reported as separate stages. Interpreter mode does not start compilation; stale worker progress is ignored after an engine or mode change. A render started before warmup finishes probes only the in-process exact-graph cache and immediately uses WatchVm on a miss. A later render can adopt the warmed GCC program without changing execution mode. Rendering remains available while the worker is active.

`compile-engine`, `run-watch`, GUI precompilation, and video rendering use the same exact-graph cache. Identity includes serialized graph data, backend and runtime ABI versions, rustc identity, target, and flags. The fresh `compile-engine` check reported a persistent cache hit for Larp after the graph had been warmed by rendering.

GUI state tests cover enabled/disabled warmup, stale engine progress, partial/ready status, worker failure/cancel states, recovery, and the deferred-render path before and after the process cache is warmed. The GUI was not interactively click-automated. The background workflow and shared cache path are covered by code and headless tests, not a measured desktop selection-latency trial.

## Differential and regression checks

The Windows native differential suite passes for every direct-native operation. It compares exact result bits, evaluation and function counts, memory, and display-list state over ordinary, signed-zero, large, infinite, and NaN inputs.

A new mixed-cut differential test compiles an `Add` parent around WatchVm `Set`, `Get`, `If`, `Play`, `Draw`, and `Spawn` cuts. It checks ordered cut-node inputs, lazy inactive-branch behavior, return bits, Watch accounting, mutated memory, audio events, spawn requests, and display-list contents. It is representative of VM cuts and is not an exhaustive per-cut-operation differential suite.

The existing negative-zero `And` regression remains on the interpreter path. Cache identity/compiler/runtime ABI validation, concurrent precompile coordination, GUI worker state, CLI mode selection, and compile fallback tests also pass. The full `cargo test --offline` run passed: 104 library tests (3 ignored), 10 CLI tests, 3 `arctan2` tests, 101 renderer regressions, 9 scheduler tests, and doc tests.

## Earlier real-engine checkpoints

Each fixture ran at Watch time 1 in release mode. VM and Sono-GCC matched the display-list SHA-256, raw Draw SHA-1, callback count, executed Watch operation counts, and VM node-evaluation count.

| Fixture | Graph nodes | Compiled regions / eligible native nodes | Regions executed | VM-to-native cut calls | Callbacks / evaluations | VM / GCC Watch frame | Matching Display SHA-256 |
|---|---:|---:|---:|---:|---:|---:|---|
| Horizon / Dreamer | 2,128 | 424 / 822 | 8,162 | 13,120 | 6,008 / 166,275 | 0.057 / 0.068 s | `959F8C5370160C8CB5C86691EF35E5A61E6A06D0FC3C067F31A7C68D3D1DF275` |
| Baumkuchen | 162,903 | 19,104 / 31,555 | 429,846 | 727,577 | 8,349 / 6,907,301 | 0.709 / 0.838 s | `0EA6A84EB415BE9B6B67D4C4C839EAEAF42D0458B7C4E9D995E9909F10693615` |
| ARMAGEDDON | 162,903 | 19,104 / 31,555 | 1,721,407 | 2,934,128 | 20,308 / 27,253,940 | 2.731 / 3.132 s | `6A0D3F9D9CF300267F16298A9A7B655582E2EC1AAE1940311E5DA23241E3CBC8` |
| Larp | 213,221 | 25,834 / 43,404 | 134,536 | 235,507 | 1,223 / 2,213,821 | 0.279 / 0.347 s | `96E5376F1B19D1B196A94B7EF7D1883F0335642BC15D84EFB8AF0CFA58954DA1` |

Raw Draw SHA-1 also matched per fixture: Dreamer `19ea2e82d0ed3eb2ba70f1ac65e4035d21733357`, Baumkuchen `460aac8e9f63856b504491bbaf95209db385407c`, ARMAGEDDON `4f46ae71567ec1450ffbeeca29e5173d6a5586d5`, and Larp `45d8bf28233741fb579ed84fe9ca1212748b4c86`.

Fresh compilation took 0.832 s for Dreamer, 44.186 s for the shared Baumkuchen/ARMAGEDDON Next Sekai graph, and 72.231 s for Larp. Persistent-cache lookup on the final CLI runs measured 0.079 s for Dreamer, 1.019 s for Baumkuchen, 1.071 s for ARMAGEDDON, and 1.316 s for Larp. ARMAGEDDON reused the same Next Sekai graph cache. DLL load ranged from 0.001 s to 0.015 s. `compile-engine` also reported a persistent cache hit for Larp. These are single measurements.

## Earlier whole-chart checks

Larp ran as a 20-frame, 1 fps, 16x16 CPU whole-chart export in both VM and GCC modes. With BGM, SFX, and particles disabled, both MP4 files had SHA-256 `6B3853FC0942BEBE73B70E1EBC28764F566DD7874C120E6DDAEB3D1E3685648E`.

A second 20-frame pair enabled SFX while keeping BGM and particles disabled. Both modes produced the same SFX event sequence SHA-1 `065623858bbffa464f9c2da960afbeefd8d4586b`: 93 scheduled effects, 3 loop starts, and 3 loop stops. GCC executed 277,804 native regions and crossed 556,810 VM-to-native scalar cuts across this export. The MP4 SHA-256 matched in both modes: `5712E8C59DC1E7111A6F8E43E84D02C42E276BF3B3F8C77D7089C19BB1EC94B9`.

At that earlier single-run checkpoint the SFX-enabled whole-chart command took
3.222 s in VM and 5.047 s in GCC. The repeated Follow-up #3 results above
supersede those timings. No speedup claim is made for the whole-chart export.

## Earlier validation and limits

- Windows native scalar differential tests: pass.
- Four real-engine VM/GCC checkpoints: exact hashes and Watch accounting match.
- Larp whole-chart export with and without SFX: matching 20-frame outputs; SFX ledger matches.
- `compile-engine` persistent cache hit: pass.
- `cargo fmt --check`, `cargo check --offline`, `cargo test --offline`, and `cargo build --release --offline`: pass.
- GUI mode/progress headless tests: pass; interactive click behavior not exercised.
- Non-Windows native DLL support: unavailable; VM fallback remains supported.

Sono-GCC remains opt-in. The coverage figure is 75 direct-native operation kinds plus 86 authoritative VM scalar cuts. It does not mean every runtime operation executes in native code, and it does not replace WatchVm as the semantic oracle.
