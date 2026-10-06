# Sono Renderer

Please scroll down to find the AI disclosure section.

This is a Sonolus renderer, with compatibility suited for Next Sekai, Next Rush, and Horizon charts.

This project works by simulating the Watch mode function on Sonolus. By simulating the Watch mode, this project is able to generate flawless recordings of MVs using any given engine zip file, SCP file, etc.

If you wish to get a .scp and engine zip, please reference Sono-Server, as Sono-Server has prepackaged .scp files and engine folders! Just zip an engine folder, and you're good :P

This project has a basic GUI, but also supports command line inputs:

```text
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --start-time 0 --duration 10 --fps 60 --width 1920 --height 1080
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --whole-chart
```

## Watch execution modes

Sono VM / interpreter remains the default. `render-video` and `run-watch` accept
`--vm interpreter` or `--vm sono-gcc` (also `--vm=...`). The existing GUI has
the same setting under **Video / Watch execution**.

```text
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --vm interpreter
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --vm sono-gcc
cargo run --release -- run-watch <engine.zip> <level.json.gz> --vm sono-gcc
```

## Watch CPU scheduling

Parallel Watch scheduling is the default. Independent callback batches use a
persistent bounded worker pool and merge their results in source order. Small
batches and callbacks that touch shared state stay on the ordered path. Use
`--watch-workers` to cap worker count, or `--single-threaded` to disable the
scheduler and avoid creating workers:

```text
cargo run --release -- run-watch <engine.zip> <level.json.gz> --watch-workers 4
cargo run --release -- run-watch <engine.zip> <level.json.gz> --single-threaded
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --single-threaded
```

The GUI has the same scheduler checkbox. `run-watch` also accepts
`--no-vm-accounting` for production-like CPU measurements without per-operation
diagnostic counters; normal runs keep those counters enabled.

Sono-GCC is an opt-in hybrid path. It compiles 75 kinds of eligible scalar
operations directly, including arithmetic, math, interpolation, and 32 easing
variants. The other 86 Watch operation kinds can cross an ordered scalar cut:
WatchVm evaluates the operation with authoritative semantics, then a native
parent can consume its exact `f64` result. The cut does not move the operation
into native code or call back from the DLL into WatchVm. Graphs without a safe
native parent, or beyond static region limits, continue entirely in WatchVm.
Elastic easing remains interpreted because native code changed a NaN payload
in differential testing. Watch tracing also forces interpreter execution for
traced callbacks.

Native DLL generation currently requires Windows and an available Rust
compiler (`rustc`). Compile or load failures fall back to the interpreter;
unsupported graph paths always remain there. Selecting an engine while
Sono-GCC is enabled starts background graph loading and precompilation. The GUI
reports cache lookup, compilation, partial coverage, and fallback status;
rendering remains available while compilation runs. GUI precompilation,
`compile-engine <engine.zip>`, and rendering share the exact-graph cache across
processes when the cache directory is writable. Cache identity includes the
graph, backend/runtime ABI, compiler identity, target, and build flags.

```text
cargo run --release -- compile-engine <engine.zip>
```

The current operation inventory and per-operation implementation/test status
are in [the coverage matrix](SONO_GCC_COVERAGE.md). The implementation report
records real-engine parity, cache timings, scalar-cut counts, and remaining limits.
The [performance report](SONO_GCC_PERFORMANCE_REPORT.md) contains repeatable
real-chart benchmarks, cache-hit breakdowns, and the current export bottleneck.

In the latest repeated release benchmarks, GCC reduced isolated Watch time on
five of six real fixtures and tied at millisecond resolution on LIMBO. The
20-frame Larp SFX export remained 7.4% slower end to end under GCC, so Sono-GCC
is intentionally not the default. See the [implementation report](SONO_GCC_IMPLEMENTATION_REPORT.md)
for the tested operation subset and parity matrix, and the [performance report](SONO_GCC_PERFORMANCE_REPORT.md)
for current timings and benchmark details.

no documentation lol have fun good luck with cli :sob:

legal jargan lol:

I do not claim to have written any of the code in this project, as I do not personally know the programming language Rust. This project is AI assisted "to the absolute extreme", and if you don't like that, it's fine. However, please make the distinction that I am not going to turn my YouTube content flow and actual creative works into AI slop machine. AI content slop is not something that I endorse in any way, shape or form. Please understand my intention with this project is not to claim that I wrote the code in this project, but to provide a tool that helps automate the chart production process much much quicker.

Acknowledgements:

Zihad - Sonolus Renderer Code provided as reference

LittleYang0531 - More rendering code provided as reference

qwewqa / Hyeon - Next Sekai Preview implementation for development reference

Burrito (Sonolus) - literally the entire reason this exists, W

Being mentioned here does not necessarily mean that they have endorsed this project.
Credit is given to acknowledge the people who made related works that contribute to its testing.

All developers whose code was provided to AI as a development reference were made explicitly aware and gave permission for my use case of their code. Again, this permission does not imply endorsement of Sono-Renderer.

This program was built and tested against Sonolus v1.1.4.
