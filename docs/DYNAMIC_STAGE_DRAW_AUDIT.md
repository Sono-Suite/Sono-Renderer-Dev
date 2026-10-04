# DISPLAYHOLIC dynamic-stage Draw compatibility audit

**Superseded status:** mode selection and reference-supported Lightweight
interpolation are now implemented; see [SKIN_RENDER_MODE_COMPATIBILITY.md](SKIN_RENDER_MODE_COMPATIBILITY.md).
The following audit records the pre-implementation findings. Its generated
real-client probe remains **UNOBSERVED**, preserved as fallback infrastructure;
no client run is requested for this pass.

## Status

The static/dynamic screenshot mismatch closes the historical MV/GUI regression investigation. Its evidence remains in [MV_PRESENTATION_HISTORY_AUDIT.md](MV_PRESENTATION_HISTORY_AUDIT.md). The user reports that the same dynamic stage is substantially stronger near the horizon in real Sonolus. This discrepancy is **not fixed**.

**Concrete compatibility omission found:** the supplied Next RUSH `EngineWatchData.skin.renderMode` is **`lightweight`**. Sono-Renderer preserves `skin` as JSON but never reads its `renderMode`; it always uses its inverse-bilinear skin rasterizer. This is independent of selecting CPU versus WGPU. According to the [official Engine Watch Data specification](https://wiki.sonolus.com/engine-specs/resources/engine-watch-data), an explicit engine mode overrides the user's settings. The user's Standard setting therefore does not establish that this engine renders in Standard mode.

The precise Lightweight interpolation formula has not yet been established sufficiently to implement it. Projective mapping removes much of the measured horizon fade, but that visual result alone does not justify changing production behavior. No MV/GUI, Draw, texture, blend, particle, Watch, or transform production behavior was changed. No goldens changed.

## Exact Draw and resource

Inputs: Next RUSH ZIP, `DISPLAYHOLIC AUDIO.json.gz`, `ProSeka Faithful 0.8.3.scp`, default options, Watch **5.0s**, stepped at **60fps**, **640x360**. This matches the previously captured frame, not a different-time static-stage comparison.

Stage entity **63**, archetype **Stage**, **UpdateParallel** callback node **16454**, Draw node **11704**, sprite ID **7**, **Sekai Lane Background**:

```text
Draw(
  7,
  -6.400767000229221, -5.946176102241358,       // BL
  -0.0005860851772165332, 1.0018899801675654,  // TL
   0.15396394898246604, 0.994391307081079,     // TR
   5.851433580174554, -6.540645377140246,       // BR
  16, 1, -0.09, 1071, 0                     // z1, a, z2, z3, z4
)
```

All 14 operands above are captured at host Draw execution, including argument-node IDs and evaluated values. `raw-stage-draws.json` contains all **37 Stage commands**, including repeated node **12145** for five lane dividers. Repeated node IDs are distinguished by their actual quad, rather than treating every loop iteration as one Draw.

Selected skin manifest is **ProSekaFaithful**. Texture content hash: **0d892827884d76b91e430824e2e998ff980ec605**, dimensions **4096x4096**. Lane background rectangle: **x=2075, y=262, w=1, h=120**. Interpolation is enabled. Sprite transform and runtime transform are identity for this frame. Screen vertices:

```text
BL (-832.1380600412599, 1250.3116984034443)
TL (319.89450466810104, -0.3401964301617655)
TR (347.71351081684384, 1.0095647254057738)
BR (1373.2580444314199, 1357.3161678852443)
```

The quad extends substantially below the framebuffer. UV is calculated against the complete quad; clipping the raster bounds does not renormalize UV against the visible part.

Draw has a scalar alpha, not four vertex colors. Effective vertex tint for this command is uniformly **(1,1,1,1)**. `z1..z4` are the existing ordering tuple, not four vertex alpha values. [Official Draw signature/order](https://wiki.sonolus.com/engine-specs/functions/draw) supports BL/TL/TR/BR and scalar `a`.

Top-origin atlas UV corners:

```text
BL (2075/4096, 382/4096)
TL (2075/4096, 262/4096)
TR (2076/4096, 262/4096)
BR (2076/4096, 382/4096)
```

Texel-index coordinates are UV*atlasSize minus 0.5. Do not confuse these with normalized UV edge coordinates. The measured neighboring atlas columns 2074/2075/2076 contain identical texels at every representative sampled row; per-sprite clamping versus atlas filtering does not change those samples.

## Numerical source of the fade

With BL=B, BR-B=E, TL-B=F, TR-BR-TL+BL=G, the current forward mapping is:

```text
P(u,v) = B + u*E + v*F + u*v*G
textureX = 2075 + u - 0.5
textureY = 262 + (1-v)*120 - 0.5
```

An independent polynomial elimination of u checks the production Newton inverse, reconstructs the pixel center, and reproduces the actual CPU sampler exactly. All RGBA channels receive the same four texture-filter weights. Draw alpha remains 1.

| Probe pixel | Bilinear (u,v) | Atlas texel-index Y | Sample RGBA | Effective alpha | Destination RGB -> result after this Draw |
|---|---|---:|---|---:|---|
| far (320,72) | (0.434440212,0.944272366) | 268.187316081 | (32,32,79,19) | 0.074510 | (51,23,93) -> (50,24,92) |
| middle (320,180) | (0.486171944,0.861611786) | 278.106585703 | (25,30,40,102) | 0.4 | (201,134,177) -> (131,92,122) |
| near (320,280) | (0.500582374,0.785074212) | 287.291094611 | (25,30,40,102) | 0.4 | (110,53,144) -> (76,44,102) |

Far filtering combines row 268 `(28,28,85,18)` with row 269 `(51,51,51,25)`: vertical weights **0.812683919 / 0.187316081**. X samples are duplicates because the resource is one texel wide and adjacent atlas padding agrees. This gives `(32,32,79,19)` after rounding. Source-over is `round(sourceRGB*a + destinationRGB*(1-a))`.

Thus **the current Draw's weak far opacity originates in the chosen texture row/alpha**, not vertex-alpha interpolation or a later framebuffer alpha multiplication. This locates current behavior; it does not establish that this is the row Sonolus should choose in Lightweight mode.

At each border/divider's own screen-space midline with bilinear v=.92/.82/.73:

| Sprite / nodes | Far sampled alpha | Middle alpha | Near alpha | Projective far alpha hypothesis |
|---|---:|---:|---:|---:|
| Stage Border, 11754 / 11845 | 22 | 54 | 54 | 54 |
| Lane Divider, 12145 (five quads) | 39 | 92 | 92 | 92 |

The far lane-background probe receives a later `Sekai Guide Green` draw; the near probe receives judgment graphics/divider/slot draws. The middle probe receives only the lane background at that pixel. Complete ordering and intermediate blend results are preserved in `blend-trace.json`. The fade is already present in the isolated stage source sample, before any of these overlays.

## Evidence for the remaining interpolation question

| Source | Observation | Limit |
|---|---|---|
| Official graphics specification | Skin Draws use bilinear mapping; background uses perspective mapping. | General graphics page does not explain the Lightweight alternative. [Graphics](https://wiki.sonolus.com/engine-specs/essentials/graphics) |
| Official EngineWatchData | Explicit `skin.renderMode` overrides user settings. | Establishes that Next RUSH requests Lightweight, not its full pixel algorithm. |
| Local SDB, `opengl.h:46-71` | Its active skin fragment shader solves inverse bilinear UV and flips texture Y. | Third-party evidence; not proof of Sonolus Lightweight fidelity. Commented ImageMagick perspective code in `command.h` is not its active GL renderer. |
| Local sonorust | Main reference supplies model/IR definitions, not a usable skin raster specification. POC submits BL/TL/TR/BR textured triangles with ordinary Bevy material UVs. | POC `normal_uv.wgsl` is not sufficient to claim a working independent mode-compatible runtime. |
| Local nxsk preview | `src/preview/gl.ts` uses homogeneous projective UV weights/division. | Editor preview implementation, not authoritative Sonolus runtime; explains a competing formula only. |
| Existing real-client curved-mode oracle | Both mode fixtures rendered. | `curved-draw/RESULT.md` explicitly does not establish interpolation or mode parity. |
| Existing v1.1.4 APK managed inventory | Separate `RenderStandardSystem` / `RenderLightweightSystem`; Standard vertex fields include UV bounds and a matrix, Lightweight includes UVQ and q-weighted UV construction. Material catalog has separate SkinStandardMaterial/SkinLightweightMaterial. | Relevant stored metadata/ISIL only; missing/unimplemented decompiler instructions and undecoded final shader prevent claiming an exact q formula or final sampling semantics. No fresh ARM/Burst analysis or code port performed. |

**Projective numerical hypothesis:** the exact same far pixel would have v=**0.176098941**, rather than .944272366, and sample RGBA **(25,30,40,102)**. Middle/near projective v are .072817033/.044046748. This is a large, measurable distinction. It is not justified to replace all Standard bilinear Draws with this formula.

## Controlled test and one targeted observation

`src/dynamic_stage_tests.rs` adds:

- `tapered_draw_gradient_follows_documented_bilinear_coordinates`: non-square trapezoid, near/far width ratio 16, one-pixel-wide 256-row white alpha ramp. Checks five analytic y->v->alpha values, exact CPU source-over and WGPU tolerance <=1. No shared inverse function is used to compute expected values.
- Ignored `displayholic_dynamic_stage_draw_trace`: advances every runtime frame, enables expensive Draw tracing only for the target frame, writes operands/resource pixels/actual sampler values. No production behavior changes.

**One stationary forced-Lightweight fixture** is generated under `artifacts/dynamic-stage/probe/`. It uses the exact recorded dynamic-stage quad, a 1x256 white linear alpha ramp, an opaque black underlay, scalar Draw alpha 1, identity transforms, one entity, no animation, particles, audio events, randomness or MV. Two Draw calls each have exactly **11 arguments**; one Execute has **2**, valid under the [official variadic Execute signature](https://wiki.sonolus.com/engine-specs/functions/execute). Its engine explicitly forces Lightweight, so the user's global Standard setting cannot contaminate the experiment. `probe/arity.json` records every function invocation.

Sono-Renderer renders this package successfully; six frames have the same raw RGB SHA1 **00861e6f2e14af4eafa1baadfc28c60e31d578e5** and Draw SHA1 **409692b5d8887f01f52694888bd27004f9112bb1**. The client package has **not** been executed in Sonolus. `probe/README.md` gives the one-observation instructions and distinguishing predictions. No broad client oracle run is requested.

## Validation / preservation

Focused analytic CPU/WGPU and actual private-fixture tests: **2 passed**. Full `cargo test --offline`: **185 passed, 2 ignored, 0 failures**; the private dynamic-stage trace passed separately. Formatting and diff checks passed. Existing sequential/pipeline, MV, base-clear/RGB24, Watch and SFX tests remain passing. No expectations updated.

The only source additions are the test module and its cfg(test) registration in `offline.rs`. MV/GUI and all production rendering code remain untouched. Known first probe-build failures used unavailable image/PNG crates; the probe now writes raw RGBA with standard I/O and converts it in the private Python artifact script. A release all-target test build encountered the already-running GUI executable lock; the focused `--lib` run avoided touching that executable and passed. No user-owned GUI process was stopped.

Resume at **Lightweight final UVQ sampling**, not geometry, transforms, particles, history, or general Watch semantics. A production correction requires either reliable final-shader evidence or the one forced-Lightweight ramp observation. Respect the Standard bilinear path when adding verified Lightweight behavior.
