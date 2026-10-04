# Particle compatibility handoff

Status: investigation stopped on 2026-10-03. No production particle-rendering defect established; no production change justified.

The current private evidence handoff is `TestingSuite/Sonolus Inspection/1.1.4/findings/PARTICLE_RENDERING_STATIC_AUDIT.md` (ignored, local interoperability research). It records input hashes, tool provenance, evidence locations, observations, and confidence. Recovered client implementation artifacts must remain private and must not be copied, translated, or ported into renderer code.

Sonolus v1.1.4 APK CPU particle behavior agrees with Sono-Renderer on U/V orientation, full width/height semantics, center pivot, positive CCW rotation, and local-coordinate/effect-quad interpolation. Earlier investigations established no discrepancy in effect quads, Spawn operands, local centers, outer transforms, stepped particle age/progress, CPU/WGPU coordinates, or resource lookup. ID-zero and SDB transform-sampling experiments did not explain the visible discrepancy.

The client's separate GPU particle shader remains undecoded, leaving its precise corner-to-UV assignment unresolved. No evidence currently establishes a GPU/CPU behavioral difference. Do not infer a renderer defect from that uncertainty.

The validated particle-orientation fixture remains available at `artifacts/real-sonolus-oracles/watch-runtime-blockers/particle-orientation/README.md`, with its prediction at `generated/sono-renderer-prediction.png` beneath that directory. Do not run or request a real-client particle oracle now. It is a fallback if future evidence makes the GPU distinction important.

The arity audit is complete for this investigation: historical `Random()` was malformed and supplies no valid semantic evidence; the historical While package was independently malformed through zero-argument `Break()`. The version-specific While signature remains separate and does not justify changing current compatibility behavior. The orientation fixture's relevant `SpawnParticleEffect` call uses the official eleven arguments.

Resume only on new evidence, using the static audit and existing corpus. Current work moves to export timing, whole-chart rendering, and structured progress; particle investigation and Watch optimization are out of scope.

## Requested SDB follow-up (2026-10-03)

The subsequent user-requested source differential is complete in [SDB_PARTICLE_DIFFERENTIAL.md](SDB_PARTICLE_DIFFERENTIAL.md). SDB differs on half-extents/clockwise rotation, host transform sampling, handle origin, ignored resource transforms, repeat/linear texture filtering, none easing, exact timeline endpoints, and nonnegative unavailable-resource bookkeeping. These differences do not establish a new Sonolus-compatible production fix. The full-size/CCW/UV client CPU evidence above remains intact. A private real-fixture diagnostic was added; no production behavior, goldens, or existing particle tests changed. No new client oracle was requested. Resume from the differential report if new evidence arrives; do not repeat the shelved hypotheses based on SDB disagreement alone.
