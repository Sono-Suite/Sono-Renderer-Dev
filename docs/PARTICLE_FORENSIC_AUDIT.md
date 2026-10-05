# Particle forensic audit after the Watch VM fixes

The demonstrated particle size discrepancy is corrected. The audit also proved
and corrected `none` easing and delayed-particle lifetime defects. The prior
Pre/Post, Switch, Or, and hold-SFX corrections remain the baseline.

This report distinguishes a verified semantic correction from complete pixel
identity with a recorded client session. The saved recording corroborates the
burst appearance; it does not expose its random seed, exact Watch timestamps,
viewport configuration, or CPU/GPU particle setting. Those limitations remain.

## 1. Canonical repro

The user selected the saved Horizon/Dreamer recording for validation.

| Input | Path under TestingSuite/Horizon | SHA-1 |
|---|---|---|
| Engine | Horizon.zip | a8bc043d1b40869527d9d5fb2694a1b2b826e91c |
| Level | dreamer.json.gz | 64ed48001b0996eba2d6757fef6a7e6181685660 |
| Resources | project(1).scp | 44cdc55465498a9996e185bae1ba8bdff435d5d5 |
| Saved client | real-sonolus-dreamer-reference.mp4 | d50f411f2b2174bb9583a489736484d643130db9 |

SHA-256 values and asset provenance are in
`target/particle-audit/provenance.json`. The selected particle collection is
`coconut-horizon-1`: 43 effects, 15 sprites, 256x256 atlas, interpolation enabled.

Historical Watch 14.9s was rechecked first on the corrected VM. A direct
`run-watch --time 14.9` snapshot initializes historical effects at that time and
shows newly born lane flashes. A production session stepped from zero has zero
active particle draws at 14.9s. Direct initialization is not a continuous-playback
reference. Its evidence is retained as `before-snapshot.*` and
`before-stepped.*`; it was not used to tune the fix.

The useful production frame is Watch **14.45s, frame 867 at 60 FPS**, advanced
through frames 0..867 exactly once, 640x360, CPU, particles enabled, UI disabled,
engine option defaults. The two cyan notes terminate at Watch 14.4s. Their burst
is visible near recording 16.08s. The recording's hit transition is bracketed by
the extracted 15.98s and 16.08s frames; this is event alignment, not a proved
frame-exact global offset. Dreamer's `bgmOffset` is +0.05s, so renderer media time
at this frame is 14.50s. Recording startup time is a separate offset.

```powershell
cargo run --release --example particle_probe -- `
  'TestingSuite/Horizon/Horizon.zip' `
  'TestingSuite/Horizon/dreamer.json.gz' `
  'TestingSuite/Horizon/project(1).scp' `
  target/particle-audit/selected-final.ppm --time 14.45 `
  --trace-config target/particle-audit/selected-config.json `
  --trace-output target/particle-audit/selected-final.jsonl
```

Add `--wgpu` for the backend comparison. The diagnostic runs the production
FrameSession; it does not replay callbacks to collect particle events.

## 2. Before-fix behavior

The continuous-playback frame contained 82 particles. Glows and cyan streaks
were too small. The selected glow's local corners were ±0.25 despite evaluated
`w=h=0.5`; the reference corner calculation uses ±0.5. Its center, rotation,
birth time, progress, color, alpha, resource, and outer transform were unchanged.
The size error survives both pixel backends.

Before RGB SHA-1: `11c3580825a210107fb47a683d42dc5b652c53ea`.
Before image: `target/particle-audit/selected-before.png`.

## 3. Particle pipeline map

| Stage | Production file/function | Implemented? | Audit notes |
|---|---|---|---|
| Compiled graph | watch.rs / WatchData; runtime.rs / WatchVm::eval | Yes | Node IDs, evaluated arguments and callback origins traced |
| Availability | watch_runtime.rs / bind_particle_effect_names; runtime.rs / HasParticleEffect | Yes | Named engine bindings, selected resource membership |
| Spawn/Move/Destroy | runtime.rs / eval_function | Yes | Shared handle table and ordered host events |
| Resource resolution | formats.rs / load_particle_assets | Yes | SCP manifest/hash lookup, gzip JSON, texture decode |
| Definition validation | formats.rs / validate_particle_effect | Yes | Transforms, properties, timeline, sprite bounds, easing |
| Instance snapshot | offline.rs / prepare_global_frame | Yes | Owned snapshot after authoritative callbacks |
| Group/copy expansion | particles.rs / render_instances_traced | Yes | Scheduled resource entries, not per-output-frame emission |
| Random expressions | particles.rs / Rng, add_random_variables | Yes | Eight values shared per group copy, fixed across frames |
| Time/progress | particles.rs / particle_timing | Corrected | Entry-relative birth, lifetime, and repeat interval |
| Property curves | particles.rs / property; runtime.rs / ease_curve | Corrected | `none` was erroneously linear |
| Resource transform | particles.rs / transform_corners | Yes | Eight weighted corner expressions |
| Local geometry | particles.rs / particle_corners, bilinear | Corrected | Removed extra size halving |
| Runtime transform | watch_runtime.rs / block 1003; runtime.rs / gpu_transform_particle_corners | Yes | Particle matrix passed independently of skin matrix |
| Sprite/UV | runtime.rs / sample_particle_sprite; gpu_render.rs / particle DrawOp | Yes | BL/TL/TR/BR mapping, atlas rectangle, interpolation flag |
| Order/blend | particles.rs / order sort; runtime.rs / composite_particle_sprites; gpu_render.rs | Yes | Stable instance/group/copy/entry order, straight alpha |
| Final output | offline.rs / render_prepared_frame; video_export.rs | Yes | CPU or single GPU frame pass, then UI/video output |

The TODO/fallback audit found no missing rate/burst/shape/keyframe machinery
required by this resource format. Its schema expresses those results through
group copies, scheduled entries, weighted expressions, and from/to curves.
Unknown ease names cannot reach the evaluator through validated resources.
Missing bindings/resources and nonfinite evaluated values are suppressed;
diagnostics now report missing bindings, duration/expiration suppression, and
inactive entries. No unconditional tracing was added.

## 4. Selected-particle forensic trace

| Field | Value |
|---|---|
| Effect | ID 14, #NOTE_CIRCULAR_TAP_CYAN |
| Handle | 30 |
| Particle | group 0 / copy 0 / entry 0 |
| Origin | entity 652, TapNote, Terminate |
| Compiled node/root | Spawn node 1271 / callback root 1383 |
| Birth / lifetime | 14.4s / 0.25s |
| Observed Watch time | 14.45s |
| Age / progress | approximately 0.05s / 0.2 |
| Properties | x=0, y=0, w=0.5, h=0.5, r=0, a=0.512 |
| Resource transform | Identity corner expressions |
| Sprite | ID 13; atlas rect x=1, y=165, w=80, h=80 |
| Tint/order | white / (30,0,0,0) |

This glow has no nonzero random coefficients. Recorded random values therefore
cannot explain its size discrepancy. Full raw entries, seeds/randoms, effect
definition, corners, matrix, UVs, order, and blend are in the JSONL traces.

The tracer reuses Watch diagnostics with ORed filters and bounded events/origin
storage. Filters include handle, effect ID, particle tuple, entity, archetype,
callback, node, spawn time, and observation time. Origins are remembered even if
the selected observation window excludes the Spawn event. Unrelated handles do
not consume a selected handle's origin budget. It is disabled by default.

## 5. Host-operation audit

| Operation | Arguments / result | State effect |
|---|---|---|
| HasParticleEffect | effect ID / 0 or 1 | Query selected named resource membership |
| SpawnParticleEffect | effect ID, BL/TL/TR/BR XY, duration, loop / unique handle | Store raw corners, Watch birth time, duration, loop |
| MoveParticleEffect | handle, BL/TL/TR/BR XY / 0 | Replace that handle's corners; existing evaluated particles use them |
| DestroyParticleEffect | handle / 0 | Remove instance immediately before frame particle snapshot |

Argument order, four-corner order, return values and unique-handle contract agree
with [Spawn](https://wiki.sonolus.com/engine-specs/functions/spawn-particle-effect),
[Move](https://wiki.sonolus.com/engine-specs/functions/move-particle-effect), and
[Destroy](https://wiki.sonolus.com/engine-specs/functions/destroy-particle-effect).
Handles remain monotonic from zero. Invalid Move/Destroy handles remain nonfatal.
The established negative missing-effect sentinel returns 0 without allocating;
this compatibility behavior was preserved, not reinterpreted as a universal
client rule for invalid inputs. An unavailable positive resource produces no
particle draw. No ID-reservation experiment was repeated.

SDB supports the three host lifecycles but differs in several details. Available
sonorust-poc particle operations are TODOs and cannot establish client parity;
its Play block ID 1004 must not be substituted for Watch block 1003.

Selected Spawn arguments, exactly as evaluated:

```text
14,
-1.4596820153068535, -1.1652456079472344,
-0.6545564339302465, -0.34683722402541983,
 0.16385194999156824,-1.151962805402027,
-0.6412736313850388, -1.9703711893238416,
0.25, 0
returned handle = 30
```

`spawn-graph.json` identifies the dependencies. `graph-traced.jsonl` captures
entity 652's Terminate callback at 14.4s, including arithmetic and memory reads.
The slice uses Get, Sin, Cos, Multiply, Add, and Subtract. For example node 1243
computes `-0.4092041919609074 - 1.0504778233459462`, yielding the supplied BL X.
The complete four-corner rotation/translation arithmetic is consistent. No new
VM primitive defect was demonstrated on this operand path.

## 6. Resource/parser audit

Raw ParticleData is retained in `particle-raw.json`; each traced EffectEvaluation
includes its parsed definition. All used structure/fields match. Three numeric
coefficients differ at ordinary f64 parsing/serialization precision (maximum
about 2.22e-16); none changes the selected constant glow or establishes the size
error. No fields are missing from this effect.

The [official resource schema](https://wiki.sonolus.com/particle-specs/resources/particle-data-effect)
has effect transform, groups/count, entries, sprite/color/start/duration, and
x/y/w/h/r/a from/to/ease. There is no separate particles-per-second, implicit
animated sprite index, independent emitter geometry, or blend-mode field in
this schema. All 43 Horizon effects have zero nonzero random transform terms;
none has an entry whose start+duration exceeds 1. The delayed-tail regression is
a generic control, not a claim that Horizon uses that case.

The relevant entry uses constant x/y/w/h/r and outCubic alpha. Sprite 13's atlas
rectangle and decoded texture are the same in both backends. Binding ID 14 is
resolved by effect name, not by assuming it equals the resource-array index.

## 7. Time/lifetime/randomness audit

**Time:** simulation reads Watch time. Media time is Watch time+bgmOffset; BGM
offset does not advance a particle's age. Effects born in a callback are evaluated
after that callback at age zero in the same frame. Destroy excludes the instance
from that frame's snapshot.

**Lifetime correction:** birth is effect birth+entry.start*effect duration;
lifetime is entry.duration*effect duration. A delayed entry can remain alive
after the original effect interval. Loops repeat relative to the entry's birth,
using effect duration as repeat interval. The old whole-effect phase calculation
clipped a tail crossing the interval. Nonloop particles expire at their own end;
loop particles include exact progress 1 before becoming inactive or repeating.
Before first birth they do not render.

The v1.1.4 CPU reference has independent entry StartTime/Duration/LoopDuration
state, removes expired nonloop entries, and uses entry-relative time for progress.
The static dump omitted floating comparisons; actual ELF-aware inspection
resolved the comparisons and the remainder call. This is the evidence for the
focused correction, not an inference from SDB boundary behavior.

**Randomness:** the renderer's deterministic generator is seeded by effect
handle and group/copy identity. It reconstructs eight values in [0,1), shares
them across entries in that group copy, and keeps them fixed across frames and
loop cycles. Transform and group streams are separate. SDB uses a different
random source; the client also does not expose an equivalent seed in the saved
recording. Exact random-particle image identity is consequently not claimed.
The selected glow avoids this limitation because all random coefficients are 0.

## 8. Transform audit

The proven change is local size. Positive rotation remains counterclockwise about
x/y; coordinates remain Y-up. Local points map from [-1,1] to [0,1], then through
BL/TL/TR/BR bilinear interpolation. No coordinate flip, corner swap, matrix
reversal, or fixture offset was introduced.

The selected resource transform is identity. At 14.45s, block 1003 contains:

```text
[ .8085, 0,      0, 0        ]
[ 0,     .8085, 0, .5166315 ]
[ 0,     0,      1, 0        ]
[ 0,     0,      0, 1        ]
```

For this matrix, world `x'=.8085*x`, `y'=.8085*y+.5166315`. The matrix is obtained
after frame callbacks and supplied to both backends independently of the skin
matrix. Screen X is divided by aspect 640/360; raster Y uses `1-y'`. Pixel-center
sampling remains unchanged.

[Watch Runtime Particle Transform](https://wiki.sonolus.com/engine-specs/watch-blocks/runtime-particle-transform)
specifies block 1003, 16 identity-initialized entries and callback access. It does
not explicitly settle every sampling/Move/random-transform edge case. General
3D/projective matrix semantics and client GPU details were not proved by this
2D affine fixture. No speculative sampling change was made.

## 9. Geometry/UV audit

The selected local corner table makes the first error independent of the camera:

| Corner | Before local | Corrected local | Corrected effect space | Corrected pixel XY |
|---|---|---|---|---|
| BL | (-.25,-.25) | (-.5,-.5) | (-1.053798524,-1.161924907) | (166.640701,256.101262) |
| TL | (-.25,+.25) | (-.5,+.5) | (-.651235733,-.752720715) | (225.225664,196.549776) |
| TR | (+.25,+.25) | (+.5,+.5) | (-.242031541,-1.155283506) | (284.777150,255.134739) |
| BR | (+.25,-.25) | (+.5,-.5) | (-.644594332,-1.564487698) | (226.192187,314.686225) |

Before pixel corners were (196.174813,255.859631),
(225.467295,226.083888), (255.243038,255.376369),
(225.950556,285.152112). The center is unchanged; each displacement from center
is doubled. This is a scale error, not translation, winding or UV orientation.

Local UVs are BL(0,0), TL(0,1), TR(1,1), BR(1,0). Source decoded atlas UVs use a
top origin: BL(1/256,245/256), TL(1/256,165/256), TR(81/256,165/256),
BR(81/256,245/256). Sampling honors the interpolation flag, texel-center offset,
and sprite-rectangle clamping. No sprite name exists in this format's sprite
table; index 13 and the rectangle identify it.

## 10. CPU/WGPU comparison

Both backends received 82 particles and identical selected-particle pre-raster
traces (excluding observation sequence numbers). State, geometry, UVs and order
agree. AMD Radeon integrated Vulkan was the WGPU adapter.

| Image | RGB SHA-1 |
|---|---|
| Before CPU | 11c3580825a210107fb47a683d42dc5b652c53ea |
| Corrected CPU | 4709d7c97171ba8b3d37143a5c8aab8097cac28a |
| Corrected WGPU | 70f26325710e7846fd974bfcbd2c5c32ae1e97c4 |

CPU/WGPU full-frame difference: maximum channel error 1, mean channel error
0.0047858796, 3,308 differing channels out of 691,200. This includes skin and
background rounding. Before/after CPU: 58,433 changed channels, maximum 217,
mean 2.2162876157. The canonical size correction changes actual pixels.

Both paths use inverse-bilinear UV recovery and straight-alpha source-over.
Tint multiplies source RGB; effective alpha is texture alpha*clamped property
alpha. Particles are drawn after gameplay sprites, before UI, in stable resource
submission order. The saved random burst cannot prove every non-parallelogram
client shader interpolation or blend detail; none was changed without evidence.

## 11. First divergence

```text
FIRST PARTICLE DIVERGENCE
fixture = Horizon / Dreamer / coconut-horizon-1
effect = 14 / #NOTE_CIRCULAR_TAP_CYAN
handle = 30
particle = (group 0, copy 0, entry 0)
time/frame = Watch 14.45 / 867 at 60 FPS
stage = local particle geometry
operation/function = src/particles.rs / particle_corners
input = x=y=r=0, w=h=.5; unchanged effect quad and matrix
old renderer result = local corners (+/- .25, +/- .25)
correct/reference result = local corners (+/- .5, +/- .5)
evidence = v1.1.4 CPU corner helper plus caller/property dataflow,
           verified with ELF-aware inspection; SDB independently agrees on size;
           selected nonrandom production trace and actual before/after pixels
```

The old private audit misidentified the helper's 0.5 coordinate normalization as
half-size scaling. Checking the caller confirms the evaluated dimensions are
passed without halving. The four multipliers are ±1. The Cpp2IL human-readable
Disassembly subsection used an incorrect file-offset mapping; actual ELF-aware
disassembly agrees with its relevant ISIL arithmetic and resolves missing
floating instructions. Recovered reference code remains private and was not
copied into the product or this report.

## 12. Root cause

Correct compiled arguments -> correct resource/property values -> extra `*0.5`
in local corner construction -> half-length particle axes -> quarter-area glow
and shorter/thinner streaks -> visibly undersized cyan burst in both backends.
Changing the outer transform could not correct this local semantic error.

Adjacent roots: `none` shared the linear branch, animating instead of holding;
whole-effect wrapping/clipping discarded delayed entry tails and loop endpoints.
Neither adjacent issue explains this constant-sized Horizon glow; both were
proved independently and corrected generically.

## 13. Fixes

| File/function | Old behavior | New behavior | Generic evidence |
|---|---|---|---|
| particles.rs / particle_corners | Apply half w/h before effect-coordinate normalization | Apply signed w/h extents, then normalize coordinates | Reference caller+helper, independent size control, production trace |
| particles.rs / property | `none` interpolates linearly | Hold from; select to only at progress exactly 1 | v1.1.4 easing behavior, including resolved floating comparison |
| particles.rs / particle_timing and instance expiry | Wrap whole effect before entry delay; clip at effect duration; exclude all entry endpoints | Track entry birth/lifetime; repeat from birth; retain delayed tails and loop endpoint | Reference entry state and boundary behavior; independent controls |
| watch_diagnostics.rs; runtime.rs; offline.rs; main.rs; particle_probe.rs | No particle pipeline trace | Bounded opt-in host and simulation trace | Observes the existing traversal and evaluator |

No production geometry correction depends on engine name, effect name, sprite
ID, fixture, resolution, or screenshot coordinates. The existing queue regression
also needed its controlled SlowSink delay increased from 20ms to 200ms: debug
Watch evaluation could be slower than the old sink, producing no backpressure.
This is a test-control adjustment; the queue/render implementation is unchanged.

## 14. Regression tests

New tests:

- particle_size_spans_the_effect_quad_at_unit_size
- particle_size_rotation_and_pivot_are_applied_before_effect_mapping
- none_easing_holds_from_until_the_exact_endpoint
- delayed_particle_lifetime_can_cross_the_effect_interval
- looping_particle_includes_its_property_endpoint_but_nonlooping_particle_expires
- particle_trace_retains_filtered_spawn_origin_and_selected_particle_identity
- particle_host_move_destroy_and_missing_id_preserve_handle_identity

The existing active-particle integration test additionally checks that tracing
does not change evaluated draws and that a looping entry repeats at its interval.
Existing GPU blend, atlas sampling, snapshot ownership, backend parity, FIFO,
single-Watch, SFX and export tests remain in the suite.

## 15. Canonical after-fix result

Before: `selected-before.png` and `selected-before.jsonl`.
After: `selected-after.png`, final current-code `selected-final.png/ppm` and
`selected-final.jsonl`. WGPU: `selected-final-wgpu.ppm/jsonl`.
Reference: `reference-16.08.png`, from the user-selected saved Sonolus recording.
All diagnostic files are under `target/particle-audit`.

The selected nonrandom glow now matches the reference size calculation at every
corner. Its x/y/r/alpha/progress, Spawn operands, effect identity, runtime matrix,
and particle count are preserved. The final current-code CPU hash is unchanged
from the size-corrected image: the additional generic easing/lifetime corrections
do not affect this Horizon frame. The saved client burst has the larger glows and
longer cyan streaks corroborating the corrected appearance. No random-seed- or
viewport-independent full-frame pixel-equality claim is made.

## 16. Production regression matrix

| Fixture/resources | Run | Result |
|---|---|---|
| Horizon / Dreamer / project(1).scp | 14.45s, 60-FPS stepped CPU/WGPU | 82 draws; identical traced state; max backend channel delta 1 |
| Horizon / Dreamer / project(1).scp | CPU video segment 14.4..15.4s | Export/mux PASS |
| Next RUSH / Baumkuchen / Faithful 0.8.3 | CPU video segment 10..11s | Export/mux PASS |
| Next RUSH / ARMAGEDDON / Faithful 0.8.3 | CPU video segment 10..11s | Export/mux PASS |
| Larp / supplied Next RUSH / Faithful 0.8.4 | Whole chart, option 24=1 | 40 frames, inferred end 19.516667s; export/mux PASS |
| Generic invalid-ID/Move/Destroy control | Unit lifecycle test | Nonfatal missing ID; stable identity; destroy/removal; no reuse |
| Generic delayed and looping entries | Numerical regressions | Delayed tails and endpoint policy PASS |

Cheap video smokes use 160x90 at 2 FPS with particles, BGM/SFX, and no UI.
They demonstrate stability and integration, not fine timing/visual parity.
The canonical 60-FPS frame and unit controls provide the finer checks.
The Larp whole-chart run retains 93 scheduled one-shots, three loop starts,
three loop stops, one Watch traversal, and zero event-prepass frames/callbacks.
No prior hold-SFX semantic correction was changed.

All four video exports were rerun after all three corrections. FFprobe confirms
H.264 video and AAC audio in each output: 1.000s for the three segment controls,
20.000s for the 2-FPS whole-chart export (40 complete frames). The latter includes
the final frame interval beyond the inferred 19.516667s chart end. Media probe
results are saved as `media-*.json`; final frame metrics are in `verification.json`.

## 17. Build/test state

Final current-code command results are recorded in `target/particle-audit`.

```text
cargo fmt --check = PASS (fmt-final.txt)
cargo check = PASS (check-final.txt)
cargo build --release = PASS (build-release-final.txt)
cargo build --release --examples = PASS (build-probe-complete.txt)
cargo test = PASS (tests-verified.txt)
204 passed, 0 failed, 3 ignored
```

Breakdown: library 86 passed/3 ignored, CLI 8, arctan2 integration 3,
m0 integration 98, Watch scheduler 9, doc tests 0. This is seven new tests above
the 197-pass baseline; the three ignored tests remain ignored. Existing compiler
warnings remain; these commands succeeded with exit code 0.

An initial full debug run hit the old 20ms SlowSink timing assertion, reproduced
in isolation. Increasing that test control produced a complete 202-pass run
before the two lifetime regressions were added. Starting another Windows test
link while the prior m0 executable was still running also produced LNK1104;
the final suite is run after that executable exits. Neither is reported as a
particle-rendering failure or concealed as a passing run.

## 18. Architecture

```text
authoritative Watch traversals = 1
particle prepass = NO
engine-specific workaround = NO
Project SEKAI-specific workaround = NO
fixture-specific visual correction = NO
synthesized particle host operations = NO
SFX prepass = NO
```

Diagnostics observe the authoritative callbacks and the production particle
snapshot. Separate CPU/WGPU diagnostic sessions are comparison runs, not a
second traversal inside a production export. Particle evaluation remains once
per prepared output frame; random state is not advanced by tracing.

## 19. Remaining known particle discrepancies

The demonstrated size/easing/delayed-lifetime defects are corrected. Remaining
client-conformance questions are not silently declared solved:

- Exact client random seed reconstruction and resulting random-particle positions.
- Full client GPU shader/interior-UV behavior on non-parallelogram particles.
- Random resource-transform sampling across Move, and generalized matrix cases
  beyond this affine fixture; Horizon has no random transform terms.
- Precise recorded Watch-time alignment and viewport/options/GPU-setting provenance.
- Client behavior for arbitrary invalid IDs/handles beyond established compatibility.

Existing minimal orientation/handle oracles were inspected. A fresh local client
run was attempted with the installed Android SDK/AVD. ADB remained offline;
the launched QEMU processes had all VM threads suspended and no boot progress,
and acceleration check reports no installed Android hypervisor driver. The
task-owned stalled emulator was stopped. No client oracle result is invented.
The user chose the saved Horizon/Dreamer recording instead. Resolving those
remaining live-client questions needs an executing client/device or additional
authoritative shader/runtime evidence; the saved random recording cannot provide
the missing internal state.
