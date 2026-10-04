# SDB particle differential

## Result and provenance

**No new Sono-Renderer production particle bug is established. No production particle behavior changed.** This follow-up inspected the actual local `sdb-ref` source, not an assumed specification or a screenshot-only comparison. SDB is third-party implementation evidence; it is not automatically correct Sonolus behavior. The previous [particle handoff](PARTICLE_COMPATIBILITY_HANDOFF.md) and private v1.1.4 `PARTICLE_RENDERING_STATIC_AUDIT.md` remain authoritative for their established client CPU observations. The separate client GPU UV assignment remains unresolved; this task adds no evidence that it differs. No APK/ARM analysis or real-client oracle was performed.

SDB has several observable differences, including ones conflicting with official documentation/client CPU observations. Adopting all its semantics would introduce known incompatibilities. No reference source was copied into production, and no reference implementation is distributed by this report. `REFERENCE_SOURCES.md` records inspection permission and redistribution limits.

Private numerical evidence: `artifacts/sdb-particle/{ledger.json,numeric.json,effects.json,sprite14.rgba,isolated-894.rgb,isolated-gpu-894.rgb}`. `numeric.json` records SHA-256 hashes of the inspected SDB files. **SDB numerical results below are consequences of audited source formulas, not output from executing its native runtime.** Its C++/jsoncpp/OpenGL environment was not built here. Actual Sono CPU and WGPU isolated particle frames were executed.

## Source map

| Source | Particle responsibility |
|---|---|
| `sdb-ref/main.cpp:143-163` | Engine effect name/ID binding; crop each atlas sprite to its own texture, retain sprite array index |
| `sdb-ref/functions/SpawnParticleEffect.h:4-24` | Transform corners, availability check, copy resource, fresh group variables, start/duration/loop, allocate |
| `sdb-ref/functions/MoveParticleEffect.h:4-20` | Existing-instance check and transformed-corner replacement |
| `sdb-ref/functions/DestroyParticleEffect.h:4-8` | Immediate instance erasure |
| `sdb-ref/functions/.Functions.Extra.h:50-52` | Active map and particle counter initialized to zero |
| `sdb-ref/engine/particle.h:83-293` | Parse expressions/groups/properties, sample group variables, evaluate time/geometry |
| `sdb-ref/engine/watch.h:219-225,373-397` | Nonloop expiration, Watch callback order, then particle evaluation/render |
| `sdb-ref/include/png.h:100-117` | Reverse decoded rows during upload; RGBA, linear filtering, no wrap override |
| `sdb-ref/opengl.h:1-78,317-390` | Particle quad submission, inverse-bilinear UV, tint, straight-alpha blend, ordering |
| `src/runtime.rs` | Particle host functions/state; particle raster/sampling and runtime matrix |
| `src/particles.rs` | Resource transform, group copies, timeline/properties/local quad/effect mapping |
| `src/formats.rs` | Resource binding, atlas parsing/validation |
| `src/offline.rs`, `src/gpu_render.rs` | Same evaluated particle snapshot consumed by CPU/WGPU |

## SDB pipeline (before any proposed change)

```text
SCP resource names -> engine effect-ID map, sprite-index cropped textures
raw Spawn corners -> current Runtime Particle Transform -> stored corners
resource copy + group-copy random variables + start/duration/loop -> active map
Watch callbacks, including Move/Destroy -> current active map
effect normalized cycle time -> active group copies/particle entries
property interpolation -> local center + CLOCKWISE half-extent quad
bilinear local [-1,+1] -> stored effect quad
sprite index -> cropped texture -> inverse-bilinear UV -> tinted straight RGBA
source-over RGB -> framebuffer
```

The resource's `transform` expressions are parsed but **never evaluated/applied** anywhere in this SDB particle path. That is distinct from Runtime Particle Transform. Sono applies the resource transform before mapping local geometry; the [official resource-transform specification](https://wiki.sonolus.com/particle-specs/essentials/particle-effect-transform) requires this weighted-expression operation. All **43** effects in the selected Horizon particle resource have identity resource transforms, so this omission does not affect the representative case.

## Host-function comparison

Both consume corners BL/TL/TR/BR: Spawn has **11** scalar operands, Move **9**, Destroy **1**. SDB's grouped C++ XY parameters represent pairs, not different opcode arities. [Official Spawn](https://wiki.sonolus.com/engine-specs/functions/spawn-particle-effect), [Move](https://wiki.sonolus.com/engine-specs/functions/move-particle-effect), [Destroy](https://wiki.sonolus.com/engine-specs/functions/destroy-particle-effect).

| Operation | SDB | Sono-Renderer |
|---|---|---|
| Spawn | Missing map entry returns 0; otherwise sample Runtime Particle Transform, copy resource and group variables, allocate `++counter`, store current time/duration/loop, return ID | Negative sentinel returns 0 without allocation. Otherwise allocate monotonic ID from 0, store RAW corners/time/duration/loop and emit event. Evaluation later skips unavailable bindings/assets |
| Move | Nonexistent ID returns 0. Replace transformed corners only; retain start/duration/loop/random variables | Nonexistent ID safely changes no live state, but emits diagnostic event. Replace raw corners only; retain start/duration/loop/deterministic variables |
| Destroy | Nonexistent ID returns 0; erase present instance immediately, no surviving particle population | Returns 0; remove present instance immediately and emit event, no surviving particle population |

**Availability qualification:** the user-described NOP behavior is exact for negative missing IDs and for visual evaluation of unavailable resources. It is not a general host-level availability check: current Spawn can allocate an invisible instance for a nonnegative unbound resource ID. SDB rejects that ID before allocation. The existing negative-sentinel regression passes. No real affected output or stronger Sonolus evidence establishes that the nonnegative bookkeeping difference explains this fixture, so no validation/host change was made.

SDB starts valid handles at 1; Sono starts at 0. The official Spawn requirement is uniqueness, without a specified numeric origin. Preserve the previous ID-0 A/B conclusion: 7 versus 8 draws produced identical RGB. This is not new evidence justifying a handle fix.

Move updates **all existing and future evaluations** through the shared effect quad in both implementations. Neither maintains detached particle world positions or resets age on Move. SDB re-samples the runtime matrix on Move; Sono uses the current render matrix. No additional emission is created by Move.

## Runtime Particle Transform

Both use Watch memory block **1003**, matching XY equations for its 16-slot layout:

```text
x' = m0*x + m1*y + m2 + m3
y' = m4*x + m5*y + m6 + m7
```

SDB samples at Spawn/Move and stores transformed effect corners. Sono evaluates resource transform/local geometry/effect mapping from raw corners, then applies the current matrix once to the final particle quad, shared by CPU/WGPU. It does not apply Runtime Skin Transform to particles. Existing effects retain the prior sample in SDB until moved; in Sono a later runtime-matrix change affects the next render.

For an affine XY matrix and identity resource transform, `T(bilinear(Q,u,v)) = bilinear(T(Q),u,v)` because the four weights sum to one, including extrapolation. Hence placement order is equivalent when the sampled matrix is the same. **Timing is the actual difference**, not a missing extra matrix. The previous controlled SDB sampling experiment failed to affect the visible discrepancy and was reverted; this inspection finds the same evidence, not a new reason to resurrect it.

## Geometry and effect mapping

For signed corners `s = (-1,-1),(-1,+1),(+1,+1),(+1,-1)` and center C=(x,y):

```text
Sono: L = C + R(+r) * (sx*w/2, sy*h/2)
SDB:  L = C + R(-r) * (sx*w,   sy*h)
```

Both rotate about C before effect mapping. SDB therefore has **double extents and positive clockwise rotation**. Sono uses full dimensions and positive CCW rotation, matching the saved Sonolus v1.1.4 CPU audit; the [official particle effect documentation](https://wiki.sonolus.com/particle-specs/resources/particle-data-effect) also specifies CCW radians. Size, rotation, or asymmetric-texture orientation cannot be corrected by blindly adopting SDB geometry.

Both use exactly the same local/effect mapping:

```text
u=(Lx+1)/2; v=(Ly+1)/2
P=(1-u)(1-v)*BL + (1-u)v*TL + uv*TR + u(1-v)*BR
```

SDB nests left/right edge lerps; Sono sums weights. Algebraically identical, no X/Y swap, Y inversion, corner reassociation, coordinate clamp, or origin difference. Extrema reach the corresponding effect corners; (0,0) reaches their arithmetic mean. Out-of-range local vertices extrapolate in both. Aspect conversion divides world X by viewport aspect, not local particle X. SDB additionally scales to its original viewport; Sono uses the requested framebuffer dimensions. These are presentation coordinate conventions, not different particle position domains.

## Texture/UV correspondence

| Geometric corner | Source sprite texel side, both implementations |
|---|---|
| BL | atlas-left / bottom |
| TL | atlas-left / top |
| TR | atlas-right / top |
| BR | atlas-right / bottom |

Sono keeps the atlas, uses `texX=rect.x+U*w-.5`, `texY=rect.y+(1-V)*h-.5`, and clamps samples to that rectangle. SDB crops the rectangle, uploads rows in reverse order, computes its inverse UV with a top-left origin, then flips V in the fragment shader. Those two SDB Y conversions combined assign the same source top/bottom as Sono. An isolated `uv.y=1-uv.y` inspection would give the wrong conclusion. The two-triangle vertex mesh does not rotate U or change sprite index; the fragment shader recovers inverse-bilinear coordinates.

SDB always sets GL_LINEAR, ignores the resource `interpolation` flag, and does not change GL's default GL_REPEAT wrap. Sono honors the flag and clamps CPU/WGPU to sprite bounds. With sprite 14's endpoint alphas **252 / 1**, at U=0 SDB repeat filtering averages endpoints to **126.5**, whereas Sono clamp gives **252**. This is a concrete SDB difference affecting the texture edge. It does not establish that Sonolus repeats a sprite; no authoritative evidence justifies adopting it. Interior samples agree apart from texture quantization. Sprite index is constant per resource entry in both: no animated atlas-frame field or frame-selection machinery exists in this schema/path. Animation means property timelines/active entries, not implicit sprite cycling.

## Timing/emission/boundaries

Both expand `group.count` copies of every entry, sharing eight fixed random variables within each copy. SDB samples them on Spawn with C `rand`; Sono reconstructs deterministic per-instance/group/copy values. Exact RNG parity was not attempted. SDB property expressions admit corner coefficients internally, but official particle-property inputs and Sono validation allow only constant/random terms; invalid-resource extensions are not evidence against valid Sono resources.

Both compute normalized effect cycle `f = fract((time-start)/effectDuration)`, then entry progress `(f-entry.start)/entry.duration`. Particle birth = effect start + entry.start*effect duration, lifetime = entry.duration*effect duration. Loop repeats the scheduled resource timeline; neither is a particles-per-output-frame rate emitter. Group count does not generate an unbounded independent-age population. [Official resource timeline fields](https://wiki.sonolus.com/particle-specs/resources/particle-data-effect) support this model.

Differences:

- SDB admits entry end equality (`f <= start+duration`), Sono excludes equality. SDB expiration excludes only `time > effectEnd`; Sono excludes `time >= effectEnd`. At an exact nonloop effect end, SDB wraps normalized time to 0 and can redraw the starting entry once. This is a boundary discrepancy, not a proved Sonolus requirement. Zero/negative duration is guarded by Sono; SDB divides without such a guard.
- SDB `none` easing yields zero interpolation factor (hold `from`); Sono currently treats it as linear. Controlled from=0,to=1,progress=.5 gives **0 versus .5**. The current official page lists `none` but does not state its exact step/hold rule; no fresh client evidence resolves it. There are **zero animated `none` properties in the 43 selected Horizon effects**, so it cannot explain these diamonds. Leave production unchanged rather than infer semantics from the label alone.
- SDB clears expired effects before lifecycle/update callbacks, then evaluates particle draws after callbacks. New effects therefore render at age 0 in that callback's frame; Destroy prevents rendering in that frame. Sono snapshots evaluated particles after callbacks too. No integration delta advances particle age independently of Watch time in either.

## One real particle: birth to pixel

Inputs: `TestingSuite/Horizon/Horizon.zip`, `dreamer.json.gz`, `project(1).scp`, default resource/engine options, 640x360, time 14.9. Effect binding **0 -> #LANE_LINEAR**, instance **1**, group/copy/entry **0/0/0**, sprite **14**, nonloop duration **.25s**. Resource: one group copy, one entry, start=0,duration=1, x=y=0,w=h=1,r=pi/2, white tint; alpha from 1 to 0 with outCubic. No random coefficient contributes to this entry.

**Sampling provenance:** this is the historical single-snapshot initialization at 14.9, followed by two adjacent frames; it is not a stepped-from-zero gameplay comparison. A separate stepped pass had 33 stored instances and zero evaluated draws in this interval, recorded in `stepped-ledger.json`. Thus the original seven age-zero diamonds must not be described as a universal steady-state particle population. No lifecycle change is proposed from this distinction.

Raw engine/stored BL/TL/TR/BR:

```text
BL (1.059521740516, -1.023935881778)
TL (0.211904348103, -0.204787176356)
TR (0.231677478301, -0.052996264812)
BR (1.158387391506, -0.264981324060)
```

Runtime matrix XY: scale **1.2** on both axes, Y translation **1.08**. Resource transform identity. Center before runtime transform **(.665372739607,-.386675161751)**, after **(.798447287528,.615989805898)**. For SDB comparison, supply that same captured matrix to its formula; this does not pretend to replay historical host matrix writes in native SDB.

| Corner | Sono final XY | Sono framebuffer XY | SDB formula framebuffer XY |
|---|---|---|---|
| BL | (1.088324,.643469) | (515.898,64.176) | (365.771,29.834) |
| TL | (1.040869,.279171) | (507.356,129.749) | (370.042,-2.953) |
| TR | (.520434,.679585) | (413.678,57.675) | (570.212,42.836) |
| BR | (.544162,.861734) | (417.949,24.888) | (548.857,206.770) |

Both map local center to the same effect-space point. Different local extents/rotation produce different quads and different physical gradient edges. The larger/oppositely rotated SDB output is real mathematical evidence of a difference, **not evidence that it matches Sonolus**.

Actual sprite 14: atlas **256x256**, rectangle **(165,201,80,1)**, interpolation=true. At Sono framebuffer pixel **(482,80)**: particle UV **(.303412661,.507765139)** -> atlas texel coordinates **(188.773012843,200.992234861)** -> rounded RGBA **(220,178,239,178)** -> white tint and Draw alpha1 -> effective alpha **178/255** -> actual isolated black pixel **(154,124,167)**. The recorded result is actual CPU raster, byte-identical WGPU at this frame.

Blend equation for both RGB paths, with straight source color S, texture/property alpha A and destination D: **S*A + D*(1-A)**. For the same interior sample over D=(30,60,90), Sono rounds to **(163,142,194)**; SDB's unquantized linear texture sample predicts floating **(162.706,142.157,193.821)**, which rounds to the same RGB. This separates blend algebra from the different geometry/edge wrap, rather than claiming the same screen pixel samples the same texel in SDB's larger quad.

SDB GL_BLEND also applies its factors to alpha: `Aout=As²+Ad*(1-As)`. Sono WGPU uses `Aout=As+Ad*(1-As)` and CPU stores RGB only. With an initially opaque destination this differs in framebuffer alpha, **not this pass's RGB source-over result**. No premultiplication/unpremultiplication or retained detached particle layer was found in this path.

| Watch time | Spawn / Destroy / Move calls | Stored live instances / draws | Selected age | Progress | Alpha |
|---|---|---|---:|---:|---:|
| 14.900000 | 8 / 8 / 0 | 7 / 7 | 0 | 0 | 1 |
| 14.916667 | 0 / 0 / 7 | 7 / 7 | .016667 | .066667 | .813037 |
| 14.933333 | 0 / 0 / 7 | 7 / 7 | .033333 | .133333 | .650963 |

SDB's same selected-instance timeline has the same progress and alpha at these times (away from end boundaries). Its different allocation can leave eight instances under Destroy(0); preserve the prior byte-identical-frame conclusion. This task did not rerun or adopt that experiment. Both draw in instance/group/copy/entry order above gameplay; SDB increments artificial depth per submitted draw with LEQUAL/depth writes, Sono uses painter order. No independent particle Z property changes that sequence.

## Controlled checks, validation and changes

- Added ignored `offline::sdb_particle_tests::horizon_sdb_particle_value_ledger`, which records the representative instance/events/resource and isolates actual CPU/WGPU raster. It evaluates real production APIs; it does not implement SDB in production or assert SDB semantics as correct. Three frame maximum channel differences: **0,1,1**, tolerance <=2.
- `artifacts/sdb-particle/analyze.py` checks extrema/center on a non-axis-aligned quad and computes formula consequences: width1 identity span 1 vs2, positive pi/2 width axis +Y vs-Y, none midpoint .5 vs0, repeat/clamp endpoint alpha126.5 vs252. These are minimal mathematical differential controls, not client observations or additional oracle packages.
- Existing focused resource/Spawn/Move/Destroy/sentinel tests: **10 passed**. Focused library particle/snapshot/CPU-WGPU tests: **3 passed**, private probe ignored by default and **passed separately**.
- Full `cargo test --offline`: **185 passed, 3 ignored, 0 failed**, including existing sequential/pipeline and unrelated rendering checks. The private particle probe passed separately. `cargo fmt --all -- --check` and `git diff --check` passed. `cargo build --release --offline --lib` passed; a new standalone GUI/CLI executable build was not required for this test/documentation-only change. Pre-existing warnings remain. PowerShell's stderr redirection reported `NativeCommandError` for Cargo's canonicalization warning, yielding shell status 1 for the captured full-suite command; all Rust test binaries and doc-tests explicitly report success in `full-tests.txt`. No golden updates. No production fix or regression test blessing SDB behavior is justified.

Reproduction:

```powershell
cargo test --release --offline --lib horizon_sdb_particle_value_ledger -- --ignored --nocapture
python artifacts/sdb-particle/analyze.py
cargo test --release --offline --lib particle
cargo test --offline --test m0 particle
```

The only source changes for this task are the test module and cfg(test) registration in `offline.rs`. All previous base-clear/RGB24/profiling/particle/runtime work is preserved. No dynamic-stage, general Draw, audio, MV, GUI, export, compiler or unrelated VM investigation or production change was performed.

**Decision:** keep Sono's established full-size/CCW/center/bilinear/UV behavior. Record the new SDB transform omission, repeat filtering, none easing, boundary and nonnegative missing-resource differences with their actual relevance/uncertainty. None satisfies all three required gates (real discrepancy, affected output, stronger evidence favoring SDB) for changing production. Do not request another client particle oracle on this evidence.
