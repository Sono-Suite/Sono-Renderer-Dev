# MV presentation opacity and DISPLAYHOLIC composition audit

## Status and limits

**Closed / superseded:** the user established that the historical known-good screenshot used the static stage, while the affected DISPLAYHOLIC uses the dynamic stage. That comparison was invalid for diagnosing a regression. MV/GUI/history investigation is closed; the remaining issue is dynamic-stage compatibility with real Sonolus. See [DYNAMIC_STAGE_DRAW_AUDIT.md](DYNAMIC_STAGE_DRAW_AUDIT.md). The samples below explain Sono-Renderer's current output, not whether its dynamic-stage interpolation matches Sonolus.

The supplied screenshots show a visual difference, but **no MV-induced stage/gameplay alpha regression was established** in controlled rendering. Do not describe the reported screenshot discrepancy as fixed. No gameplay blend constants, stage transparency, geometry, Watch behavior, particle behavior or GUI panic handling were changed.

A distinct presentation contract violation was corrected: the MV decoder previously preserved source alpha, including alpha-bearing video. The user explicitly requires MV to be an opaque presentation source. `export_mv::scale_background` now dims RGB toward black and forces alpha 255. At 100%, RGB is bit-exact identity. This change does not explain the DISPLAYHOLIC screenshot difference: its decoded video was already opaque, and all four before/after MP4 comparisons were byte-identical.

## Actual rendering chain

1. CPU initializes RGB black (implicit opaque destination), or the selected background base RGB. WGPU initializes `Rgba8Unorm` with `LoadOp::Clear(base_clear_color(...))`, alpha 1. The base-clear optimization remains unchanged.
2. FFmpeg decodes/scales MV to RGBA. The measured DISPLAYHOLIC frame has source alpha 255 throughout.
3. MV presentation: RGB = round(decoded RGB * percentage/100). Source alpha is now always 255, independent of percentage.
4. MV replaces the selected background image during its active media interval. It uses a contain-fit quad over an opaque black base and the existing background mask. Texture upload and target format are linear `Rgba8Unorm`. The background shader returns sampled RGB and source alpha; the mask uses its existing source alpha. No separate MV gameplay pipeline or intermediate alpha-bearing target is introduced.
5. Skin Draw uses straight source RGB and sampled texture alpha times Draw alpha. WGPU uses `ALPHA_BLENDING`: RGB = source RGB * source alpha + destination RGB * (1-source alpha); A = source alpha + destination A * (1-source alpha). Since destination A begins at 1, it remains 1. CPU applies the same RGB source-over equation and stores no destination alpha.
6. Existing particle composition follows gameplay on the same opaque target (CPU RGB or WGPU RGBA). It was observed, not modified or investigated.
7. WGPU readback copies the real RGBA target, then discards alpha when producing RGB24. No unpremultiplication or multiplication by destination alpha occurs. CPU already stores RGB24.

Both background-only and final WGPU readbacks had alpha range **[255,255]** for no MV, 100%, 50%, and 0%, including contain-fit coverage and any untouched pixels.

## Controlled frame and measured stage samples

A frozen prepared DISPLAYHOLIC frame at Watch 5.0s, 640x360/60fps, was rendered with each background and each backend. It retains the original Draw list, particle snapshot, timing and other runtime values. Its Draw-list SHA1 is `105c0b304ed5511ae90bae4a3c36111d7b328fe0` for every case. Canonical MV PTS is 2.9166666666666665s (BGM offset -2.077); percentages do not change selection or geometry.

The actual CPU sampler was reused for source texels. Relevant stage samples, with Draw alpha 1:

| Probe | Pixel | Stage source RGBA | Effective source alpha |
|---|---|---|---|
| far | (320,72) | (32,32,79,19) | 19/255 = 0.074510 |
| middle | (320,180) | (25,30,40,102) | 102/255 = 0.4 |
| near | (320,280) | (25,30,40,102) | 102/255 = 0.4 |

At the middle probe:

| Background | Destination RGB before stage | Stage result RGB |
|---|---|---|
| no MV | (201,134,177) | (131,92,122) |
| MV 100% | (100,11,211) | (70,19,143) |
| MV 50% | (50,6,106) | (40,16,80) |
| MV 0% | (0,0,0) | (10,12,16) |

These are exact rounded source-over results. Far and near probes have later gameplay/particle overlays, recorded separately in the full trace. The skin's `Sekai Lane Background` alpha range is [0,102], `Sekai Stage Border` [0,108], and `Sekai Lane Divider` [0,92]; all use interpolation. The texture contains an alpha gradient and current no-MV rendering samples it too. This does not establish that Sono-Renderer selects the correct texture rows for the engine's requested rendering mode.

CPU/WGPU maximum channel differences across the entire frame: no MV 3, MV100 4, MV50 3, MV0 4. Only 1 channel (MV100) and 3 channels (MV0) exceeded 3 out of 691,200 channels. Representative probe pixels agree, except a 1-byte background interpolation rounding difference in the no-MV middle sample.

## Tests and artifacts

`export_mv` regressions cover transparent/mixed-alpha decoded input with 100/50/0% presentation, exact RGB identity at 100%, opaque background readback, partially transparent gameplay (texture alpha 128, Draw alpha .5), exact CPU source-over results, and WGPU tolerance <=1. No-MV and black-MV cases agree. Existing MV timestamp/geometry, sequential/pipeline, Watch timing, SFX hash and particle tests remain green.

`gpu_render::capture_rgba`, `runtime::skin_sample_at_pixel`, `PreparedFrame` cloning and `mv_composition_tests` are **test-only**. They do not alter production rendering or add production overhead. The real-media audit is ignored in the normal suite and was run explicitly; it requires the user's private DISPLAYHOLIC files:

```
cargo test --offline displayholic_mv_alpha_audit -- --ignored --nocapture
```

It writes `artifacts/mv-alpha/observations.json`, `draws.json`, `stage-blend-summary.json`, raw CPU/WGPU PPMs and `comparison.png`. `SONO_MV_AUDIT_TIME` can select another Watch time. The full-resolution texture diagnostics make the debug audit slower than ordinary release rendering; do not use it for performance conclusions.

CLI and native GUI A/B exports use Watch 5.0-5.5s, 640x360, 60fps, WGPU/pipeline, identical default engine options, the same skin/particle SCP and audio, and no whole-chart discovery. All four GUI/CLI MP4 pairs are **byte-identical**, including audio. See `normalized-config.json`, `gui-cli-comparison.json`, `before-after.json`, `*-active.mp4` (before), `*-opaque.mp4` (after), and `gui-*.mp4` in the artifact directory. At 0%, the active MV background is black and gameplay is composed normally.

The first 0-2s test segment was entirely pre-MV due to the -2.077s BGM offset and therefore tested static fallback rather than active MV. It was replaced by the explicit 5.0-5.5s active segment; do not cite the initial clips as active MV evidence.

Final validation: **184 passed**, 1 private-media audit ignored by default and passed separately, 0 failures; formatting/diff checks and offline release build passed. No golden changes. Pre-existing compiler warnings remain unchanged.

## Historical remaining question (superseded)

The supplied screenshots were taken at similar, not identical Watch times. Their difference has not been attributed to the percentage feature: matched-input no-MV/100/50/0 cases show unchanged gameplay samples/blending and opaque targets. The old known-good MV milestone was not a committed revision; there is no independent pre-percentage binary/output in this audit to claim historical parity against. The 100% RGB identity and unchanged backend equations are independently tested.

Do not compensate by forcing the translucent stage skin opaque. A further fix requires a reproducing identical-time/configuration pair or a concrete discrepancy upstream of the measured Draw/resource inputs. That investigation was not broadened into Watch semantics, stage geometry, transforms, particles, compiler, or ARM work.

## Historical follow-up

The later [MV_PRESENTATION_HISTORY_AUDIT.md](MV_PRESENTATION_HISTORY_AUDIT.md) recovered pre-percentage source from unreachable git trees and built it independently. This supersedes the earlier availability limitation above; controlled historical/current RGB sequences matched at both tested resolutions/ranges. No last-good/first-bad endpoint was established.
