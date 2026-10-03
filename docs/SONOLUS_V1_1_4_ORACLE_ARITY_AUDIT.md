# Sonolus v1.1.4 oracle fixture arity audit

This audit covers the manually generated Watch oracle set under
`artifacts/real-sonolus-oracles/watch-runtime-blockers`, its generator, the
current minimized fixture source, and the local VM tests that encode unusual
control-flow forms. Official Sonolus function pages are the authority for
signatures; third-party runtimes and renderer behavior do not validate a
client fixture.

## Previous client-run fixture forms

These counts come from the source mapping preserved with the two prior crash
reports. Those historical package bytes were replaced when the generator was
corrected and regenerated below.

| Function / opcode | Historical argument count(s) | Required / allowed count | Valid? |
|---|---:|---:|---|
| `Random` | 0 in original and all reduced Random probes | 2 (`min`, `max`) | **No** |
| `RandomInteger` | 2 | 2 (`min`, `max`) | Yes by arity; original combined package is still invalid because it also has `Random(0)` |
| `While` | 1 and 2 in the combined crash package; 1 in reduced one-body probe | v1.1.4 fixture target used here: 2 | The one-argument form is invalid under that target; the two-argument form is well-shaped |
| `Break` | 0 in the combined and reduced one-body probes | 2 (`count`, `value`) | **No** |

The combined While crash package is invalid regardless of the conflicting
public historical While note below, because it contains `Break()` with zero
arguments. No conclusion that a valid While or Break call crashes survives.

## Current regenerated WatchData function inventory

This per-fixture inventory was produced by reading each generated
`pack-source/engines/*/watchData.json`. Counts separated by `/` are the
distinct argument counts for that function in that fixture. Every generated
invocation passes the signature checks in `validate-oracles.ps1`.

| Fixture | Function(argument count) | Valid? |
|---|---|---|
| `oracle-arctan2` | Add(2), Arctan2(2), DebugLog(1), Execute(5/10), Get(2), Multiply(2), Set(3), Subtract(2) | Yes |
| `oracle-curved-lightweight`, `oracle-curved-standard` | Add(2), DrawCurvedB(14/17), DrawCurvedBT(19), DrawCurvedL(14), DrawCurvedLR(19), DrawCurvedR(14), DrawCurvedT(14), Execute(9/10), Get(2), Multiply(2), Set(3), Subtract(2) | Yes; optional curve depth args are within published signatures |
| `oracle-legacy-while` | Add(2), DebugLog(1), Execute(1/2/3/5/10), Get(2), Less(2), Multiply(2), Set(3), Subtract(2), While(2) | Yes by the two-argument fixture target |
| `oracle-lifecycle-seek` | Add(2), DebugLog(1), Execute(2/10), Get(2), Multiply(2), Set(3), Subtract(2) | Yes |
| `oracle-particle-handles` | Add(2), DebugLog(1), DestroyParticleEffect(1), Equal(2), Execute(2/10/11/13), Get(2), If(3), MoveParticleEffect(9), Multiply(2), Set(3), SpawnParticleEffect(11), Subtract(2) | Yes |
| `oracle-particle-orientation` | SpawnParticleEffect(11) | Yes |
| `oracle-random` | Add(2), DebugLog(1), Execute(10/16/64), Get(2), Multiply(2), Random(2), RandomInteger(2), Set(3), Subtract(2) | Yes by arity; no client rerun |
| `oracle-random-control`, `oracle-while-control` | Add(2), DebugLog(1), DebugPause(0), Execute(3/10), Get(2), Multiply(2), Set(3), Subtract(2) | Yes |
| `oracle-random-next-update` | Add(2), DebugLog(1), DebugPause(0), Execute(2/3/10), Get(2), Multiply(2), Random(2), Set(3), Subtract(2) | Yes |
| `oracle-random-one-float` | Add(2), DebugLog(1), DebugPause(0), Execute(3/10), Get(2), Multiply(2), Random(2), Set(3), Subtract(2) | Yes |
| `oracle-random-one-integer` | Add(2), DebugLog(1), DebugPause(0), Execute(3/10), Get(2), Multiply(2), RandomInteger(2), Set(3), Subtract(2) | Yes |
| `oracle-random-repeated` | Add(2), DebugLog(1), DebugPause(0), Execute(7/10), Get(2), Multiply(2), Random(2), Set(3), Subtract(2) | Yes |
| `oracle-random-two-entities` | Add(2), DebugLog(1), DebugPause(0), Execute(2/3/10), Get(2), Multiply(2), Random(2), Set(3), Subtract(2) | Yes |
| `oracle-replay-streams` | Add(2), DebugLog(1), Execute(2/5/10), Get(2), Multiply(2), Set(3), StreamGetNextKey(2), StreamGetPreviousKey(2), StreamGetValue(2), StreamHas(2), Subtract(2) | Yes |
| `oracle-stack` | Add(2), DebugLog(1), Execute(5/10/25), Get(2), Multiply(2), Set(3), StackEnter(1), StackGet(1), StackGetFrame(1), StackGetFramePointer(0), StackGetPointer(0), StackGrow(1), StackInit(0), StackLeave(0), StackPop(0), StackPush(1), StackSetFrame(2), Subtract(2) | Yes |
| `oracle-stream-set` | Add(2), DebugLog(1), Execute(5/10), Get(2), Multiply(2), Set(3), StreamGetNextKey(2), StreamGetPreviousKey(2), StreamGetValue(2), StreamHas(2), StreamSet(3), Subtract(2) | Yes |
| `oracle-while-one-body`, `oracle-while-two-child-minimal` | Add(2), DebugLog(1), DebugPause(0), Execute(1/2/4/10), Get(2), Multiply(2), Set(3), Subtract(2), While(2) | Yes by arity; the first fixture ID is historical and its current shape has two arguments |

There are no `DoWhile`, `Switch`, `SwitchInteger`, `SwitchWithDefault`,
`SwitchIntegerWithDefault`, `Lerp`, `Remap`, `Unlerp`, or related interpolation
calls in the saved generated oracle WatchData. They therefore have no real
client observations in that corpus. Focused `tests/m0.rs` VM examples are
separate, local-only coverage:

| Function | Test argument count(s) | Required / allowed count | Valid? |
|---|---:|---:|---|
| `DoWhile` | 2 | 2 (`body`, `test`) | Yes |
| `Break` | 2 | 2 (`count`, `value`) | Yes |
| `While` | 2 and 1 | v1.1.4 fixture target: 2 | 2-argument test valid; one-argument test is local legacy compatibility only |
| `SwitchInteger` | 4 | discriminant plus branch list | Yes |
| `SwitchIntegerWithDefault` | 4 | discriminant, branches, default | Yes |
| `SwitchWithDefault` | 6 and 8 | discriminant, test/consequent pairs, default | Yes |
| `Lerp` | 3 | 3 (`a`, `b`, `x`) | Yes |
| `Unlerp`, `UnlerpClamped` | 3 | 3 | Yes |
| `Remap` | 5 | 5 | Yes |

The saved valid `If` nodes are three-argument. These unit tests establish only
Sono-Renderer behavior; none is a real-client observation.

`SpawnParticleEffect` is exactly 11 arguments: effect ID, eight corner
coordinates, duration, and loop flag. `MoveParticleEffect` is exactly 9:
instance ID and eight corner coordinates. `DestroyParticleEffect` is exactly
one instance ID. The particle-handle fixture uses these counts correctly.

## Crash conclusions corrected

| Fixture observation | Fixture validity | Conclusion that survives |
|---|---|---|
| Historical `oracle-random` terminated during selection/loading | Invalid: contained repeated `Random()` nodes (0 arguments). Its `RandomInteger(0,3)` calls were individually well-shaped but shared an invalid package. | The package terminated. Withdraw any inference about valid Random/RandomInteger range, sequence, seed, callback continuity, or crashes. |
| Historical `oracle-random-one-float`, `oracle-random-repeated`, `oracle-random-two-entities`, `oracle-random-next-update` outputs | Invalid: each used zero-argument `Random()`. None was run. | No client observation. Current regenerated versions use `Random(0,1)` and remain unrun. |
| `oracle-random-one-integer` | Valid arity (`RandomInteger(0,3)`); not run. | No observation yet. |
| Historical `oracle-legacy-while` terminated during selection/loading | Invalid: contained `While(body)` (1 argument) and `Break()` (0 arguments), in addition to a separate two-argument While. | The package terminated. Withdraw any inference that valid While or Break forms crash. |
| Historical `oracle-while-one-body` reduced probe | Invalid: one-argument While and zero-argument Break. Not run. | No client observation. Its current regenerated version has a two-argument While and is unrun. |
| Current `oracle-while-two-child-minimal` | Two-argument While; not run. | No observation yet. |

The official public release history says While changed to a single-body form
in Sonolus 0.7.0, which conflicts with the v1.1.4 two-argument contract being
used for this fixture audit. This is recorded as a versioned-spec discrepancy,
not as evidence that the malformed crash probe established either form.
Before interpreting a future While oracle, validate its serialized node against
the exact v1.1.4 signature source. The Random and Break arity defects alone
already invalidate the combined crash fixture.

## Generator and renderer behavior

The generator now emits `Random(0,1)` and two-argument While calls. The
ignored generated fixture packages were regenerated with the corrected source;
the historical client crash notes still describe the earlier malformed
versions and distinguish the current unrun packages.

Sono-Renderer's `WatchVm` accepts one- and two-argument While nodes. That
means it accepts a one-argument shape outside the v1.1.4 contract used here;
this is local legacy compatibility, not Sonolus evidence. The unit test is named
`watch_vm_local_legacy_single_body_while_repeats_until_break` to make that
scope explicit. `Break`, `DoWhile`, and particle Spawn/Move/Destroy explicitly
check their required counts. `Get` and `Set` read only their required leading
arguments and do not explicitly reject extra children, so the VM can accept
over-arity forms locally; this audit does not change that runtime policy.
`Random` and `RandomInteger` have no Watch VM implementation, so the runtime
rejects the operation as unsupported rather than accepting a malformed or
well-formed call. No VM validation change was made.

## Particle-orientation fixture arity gate

The generated minimal orientation oracle uses a single `SpawnParticleEffect`
call with exactly 11 arguments and no Random, loop, Break, switch, or
interpolation opcodes. Its arguments are `1`, then four BL/TL/TR/BR coordinate
pairs, then duration `10`, then loop flag `0`. This call shape matches the
official function page. It contains one constant particle with nonzero `x/y`,
non-square `w/h`, and nonzero rotation. Its texture has four distinct opaque
quadrants. No real-client result is claimed here.

## Official references

- [Random](https://wiki.sonolus.com/engine-specs/functions/random) — `min`, `max`.
- [Break](https://wiki.sonolus.com/engine-specs/functions/break) — `count`, `value`.
- [DoWhile](https://wiki.sonolus.com/engine-specs/functions/do-while) — `body`, `test`.
- [SpawnParticleEffect](https://wiki.sonolus.com/engine-specs/functions/spawn-particle-effect) — 11 arguments, including BL/TL/TR/BR corners.
- [MoveParticleEffect](https://wiki.sonolus.com/engine-specs/functions/move-particle-effect) — 9 arguments.
- [DestroyParticleEffect](https://wiki.sonolus.com/engine-specs/functions/destroy-particle-effect) — 1 argument.
- [DrawCurvedB](https://wiki.sonolus.com/engine-specs/functions/draw-curved-b) and [DrawCurvedBT](https://wiki.sonolus.com/engine-specs/functions/draw-curved-bt) — required geometry/curve values and optional depth values.
- [Add](https://wiki.sonolus.com/engine-specs/functions/add) — variadic ordered values.
- [Lerp](https://wiki.sonolus.com/engine-specs/functions/lerp) — 3 arguments.
- [Remap](https://wiki.sonolus.com/engine-specs/functions/remap) — 5 arguments.
- [Switch](https://wiki.sonolus.com/engine-specs/functions/switch) and [SwitchWithDefault](https://wiki.sonolus.com/engine-specs/functions/switch-with-default) — discriminant, branch pairs, optional default branch.
- [Sonolus 0.7.0 release notes](https://wiki.sonolus.com/release-notes/versions/0.7.0) — While single-body change, noted above as conflicting version evidence.
