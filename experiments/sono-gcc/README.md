# Sono-GCC feasibility spike

This standalone experiment takes selected Watch node roots plus explicit cut
nodes, emits Rust expressions, compiles them to a native Windows DLL with
`rustc`, and compares the results with `WatchVm`. The cut nodes stand for
interpreter/helper values supplied by the caller. The current workload cuts
three local `Get` nodes into memory values.

The compiler is deliberately limited to pure scalar operations. It is not
connected to `WatchRuntime` and is not selected by the renderer. Unsupported
operations and cyclic graphs fail compilation. Read the
[feasibility report](../../artifacts/sono-gcc-feasibility/REPORT.md) for the
measured scope and limitations.

Run from the repository root on Windows x64:

```powershell
$env:CARGO_TARGET_DIR = 'artifacts/sono-gcc-feasibility/target'
cargo run --release --offline --manifest-path experiments/sono-gcc/Cargo.toml
```

The generated Rust source and DLL are written under
`artifacts/sono-gcc-feasibility/native/`. Set `SONO_GCC_ITERATIONS` to change
the per-sample benchmark length.
