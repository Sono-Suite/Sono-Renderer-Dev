# Skin render-mode compatibility

## Status and evidence limits

Engine-selected skin modes are now parsed and honored. Next RUSH explicitly
selects Lightweight, automatically, for both CPU and WGPU and for sequential
and pipeline consumers. Standard retains its previous inverse-bilinear path.

**Specification-established:** [Engine Watch Data](https://wiki.sonolus.com/engine-specs/resources/engine-watch-data)
defines the optional `skin.renderMode` as `default`, `standard`, or `lightweight`.
Absent/default uses the user's setting; an explicit engine mode overrides it.
Sono-Renderer's existing fallback is Standard; there is no new CLI override.
Unknown strings and non-string explicit values fail parsing with field context.
The typed preference resolves against a typed default/user mode once at session
construction and is carried in immutable frame rendering resources.

**Not specification-established:** the exact Lightweight interpolation formula.
The [general graphics specification](https://wiki.sonolus.com/engine-specs/essentials/graphics)
describes BL/TL/TR/BR ordering and bilinear skin Draw interpolation, but gives
no mode-specific Lightweight exception or shader formula. This is a material
gap, not proof of projective interpolation.

The implemented Lightweight projective convention is an **explicit compatibility
assumption supported by local rendering references**, not verified Sonolus behavior:

| Evidence | Actual observation | Limit |
|---|---|---|
| `sdb-ref/opengl.h`, `inverse_bilinear`, active fragment shader | Inverse bilinear; flips texture Y for top-origin image storage | No skin render-mode selection; does not implement a distinct Lightweight path |
| `nxsk-preview-ref/src/preview/gl.ts`, `perspectiveWeights`, vertex/fragment shaders | Projective quad mapping through homogeneous UV weights, BL/TL/TR + BL/TR/BR | Editor preview, not an authoritative implementation of Sonolus Lightweight |
| Local sonorust resource model | Default/Standard/Lightweight representation | No independent mode-specific pixel implementation |
| Previously recorded client metadata in `DYNAMIC_STAGE_DRAW_AUDIT.md` | Separate Standard matrix/UV and Lightweight UVQ structures | Final shader undecoded; structure supports a hypothesis, not behavioral proof |

References were read as mathematical/behavioral evidence. No client implementation
was ported. No new APK/native/ARM investigation or client run was performed.
The projective assumption remains subject to correction if a future observation
contradicts it. Its better-looking stage is not used as proof of the convention.

## Implemented mapping

After the existing runtime and sprite transforms, corners are BL/TL/TR/BR.
Bottom-origin local UVs are `(0,0),(0,1),(1,1),(1,0)`, respectively. Atlas-left
remains BL/TL; atlas-top remains TL/TR. Neither UV orientation nor vertex
geometry changes with the mode.

Let the diagonals meet at `BL + t*(TR-BL) = TL + s*(BR-TL)`. Homogeneous
weights in BL/TL/TR/BR order are:

```text
q = [1/(1-t), 1/(1-s), 1/t, 1/s]
UV(P) = sum(lambda_i * q_i * UV_i) / sum(lambda_i * q_i)
```

Weights are normalized by their maximum (a common factor cancels). Barycentric
`lambda` is evaluated first in BL/TL/TR, then BL/TR/BR. This is projective,
not screen-affine or inverse-bilinear. On parallelograms all weights agree and
the projective and bilinear mappings agree. CPU and WGPU both evaluate this
mapping at pixel centers with f32 coverage/interpolation precision.

Clipping preserves the original quad/homography: cropping to the viewport does
not replace the corners or renormalize UVs. Coverage uses triangle barycentrics
with the existing small numerical tolerance, then clamps UV for sampling.
Parallel, invalid, or non-interior diagonal intersections use unit weights
(affine triangles); degenerate triangles do not draw. **These fallback and edge
rules are implementation assumptions, not client-verified corner cases.**

Sampling remains the existing sprite-region-clamped nearest/bilinear filtering
controlled by resource interpolation. Atlas coordinates are
`x + u*w - .5`, `y + (1-v)*h - .5`. Draw alpha remains uniform across the
quad, multiplied by sampled texture alpha; skin tint, byte-space source-over
blending, ordering, sprite transforms and resource data are unchanged. This pass
does not introduce vertex color/alpha or linear-light blending.

Only skin Draw commands receive this mode. Backgrounds, masks and particles
retain their existing paths. Standard CPU and shader math is unchanged.

## Files and focused coverage

- `src/skin_render_mode.rs`: typed modes, precedence, independent projective math;
  tests for absent/default/explicit modes, malformed values, Next RUSH override,
  corner UVs, parallelogram behavior, clipping and degenerate fallback.
- `src/watch.rs`: validates the optional JSON field while retaining existing
  sprite binding JSON; exposes typed preference.
- `src/offline.rs`: resolves once and carries mode into either pixel consumer.
- `src/runtime.rs`: Standard wrapper preserved; mode-aware CPU skin renderer.
- `src/gpu_render.rs`: mode-aware skin commands/uniforms and matching WGSL math.
- `src/lib.rs`: module export.
- `src/dynamic_stage_tests.rs`: asymmetric 1x256 alpha-ramp trapezoid (near/far
  width ratio 16), independent analytic centerline predictions, byte-identical
  legacy/explicit Standard output, distinct Lightweight output and CPU/WGPU
  tolerance. Private DISPLAYHOLIC probe freezes one prepared frame and compares
  both modes/backends; also verifies formerly divergent thin-quad coverage.

No golden expectations were changed. Diagnostic image hashes change when the
engine explicitly selects the previously ignored Lightweight mode; those hashes
are observations, not revised acceptance goldens. Existing MV/GUI/audio/particle,
Watch scheduling, base-clear and RGB24 work is preserved.

## DISPLAYHOLIC result

Exact private fixture: Next RUSH engine, DISPLAYHOLIC AUDIO level, ProSeka
Faithful 0.8.3 skin, default options, Watch **5.0s**, stepped at 60fps, 640x360.
Diagnostics confirm requested `lightweight` and effective `lightweight`.
The before and after use the **same frozen prepared frame**, changing only skin
mode. The actual Draw/resource provenance remains in
`artifacts/dynamic-stage/raw-stage-draws.json` and `state.json`.

Stage entity 63, UpdateParallel node 16454, Draw node 11704, sprite 7
(Sekai Lane Background), alpha 1. No alpha/texture compensation:

| Pixel | Standard texture RGBA | Lightweight texture RGBA |
|---|---|---|
| Far (320,72) | (32,32,79,19) | (25,30,40,102) |
| Middle (320,180) | (25,30,40,102) | (25,30,40,102) |
| Near (320,280) | (25,30,40,102) | (25,30,40,102) |

The stage-only images show the stronger stage/edges extending toward the
horizon; the sampled near appearance is unchanged. This removes the measured
excessive horizon fade **under the implemented projective assumption**, in the
direction described by the user. Exact agreement with real Sonolus is not
established without an aligned reference. Other skin Draws from this explicitly
Lightweight engine also receive the mode, as required; this is not a stage hack.

Frozen-frame CPU/WGPU maximum channel difference: **3** for both modes.
Channels differing by more than 2: Standard 35, Lightweight 34. Thin-quad
diagnostics have no differing target pixels. Synthetic Lightweight tolerance
is at most 2. Full raw RGB hashes (`mode-render-comparison.json`):

| Mode | CPU SHA1 | WGPU SHA1 |
|---|---|---|
| Standard | 04b4ca106182dbf17896854a320c8b9ec20238bf | 8d2f506f97c1b519a601be2ff8f2968cac2fb5d3 |
| Lightweight | 23e6b4a276d1bd6ee09c80e8fa14ab28a24ac2f3 | 723bd2654a612bd00ba0090ac8055dbc02e4f24d |

Artifacts: `artifacts/skin-render-mode/stage-only-Standard.png`,
`stage-only-Lightweight.png`, `displayholic-standard.png`,
`displayholic-lightweight.png`; underlying lossless PPMs and diagnostics are in
`artifacts/dynamic-stage`. The probe fixture/predictions remain intact and
**UNOBSERVED**. Do not treat its predicted pixels as Sonolus facts.

## Reproduction and validation

```powershell
cargo test --offline skin_render_mode
cargo test --offline tapered_draw_gradient
cargo test --release --offline --lib displayholic_dynamic_stage_draw_trace -- --ignored --nocapture
cargo test --offline
cargo fmt --all -- --check
cargo rustc --release --offline --bin renderer -- -o artifacts/skin-render-mode/renderer.exe
./artifacts/skin-render-mode/renderer.exe render-video "TestingSuite/Next RUSH/engine/Next RUSH.zip" "TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp" "C:/Users/Admin/Desktop/Charts/In Progress/ABM - DISPLAYHOLIC/DISPLAYHOLIC AUDIO.json.gz" "C:/Users/Admin/Desktop/Charts/In Progress/ABM - DISPLAYHOLIC/DISPLAYHOLIC AUDIO.mp3" artifacts/skin-render-mode/displayholic-lightweight.mp4 --start-time 5 --duration 0.1 --fps 60 --width 640 --height 360 --no-bgm --no-sfx
```

The separate release executable avoids overwriting the executable currently
running the user's GUI. The GUI was not stopped. Use the artifact executable
for this build; the already-running application cannot acquire new code.

Validation completed:

- `cargo test --offline`: **188 passed, 0 failed, 3 ignored** (79 library,
  8 CLI, 3 audio integration, 98 remaining integration tests). Includes
  sequential/pipeline equivalence, WGPU pipeline determinism, MV composition,
  export audio offsets and failure cleanup, and existing particle coverage.
- Explicit private DISPLAYHOLIC release diagnostic: **1 passed**; frozen RGB
  hashes and tolerance above confirmed after the final coverage correction.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Release binary build: passed; existing deprecated/dead-code warnings remain.
- Actual CLI WGPU export: **6 frames**, Watch **5.0–5.1s**, 640x360/60fps,
  completed with a 0.100000s H.264 video. RGB sequence SHA1:
  `94f5df9cde34dc976535015f687850bc741d0997`.

The standalone artifact executable initially could not find its own FFmpeg
installation and attempted a network download. Existing validated workspace
FFmpeg/FFprobe executables were then hard-linked into its normal managed
`dependencies/ffmpeg/8.1` directory, and the export succeeded. No FFmpeg discovery
or export code was changed. Those artifact-local dependencies are present for
the reproduction command above.
