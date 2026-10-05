# Watch hold-SFX forensic audit

Canonical archive: Next RUSH **public version 2.14.2**, engine format **13**.
Engine SHA-1 `0e53198c71b2739def8ce6aeee16d0fb4ae22e01`; level SHA-1
`3ef55933866663dfa52f3725eb1003c3a8bb0cb8`. Both are pinned in the integration
regression. Assets are the untouched files in
`TestingSuite/Next Sekai Engine/levels/larp 64x`, with `ProSeka Faithful 0.8.4.scp`.
Option 24 is enabled, as in the canonical autoplay reproduction.

**Endpoint A reached:** a generic VM correction restores three normal hold loops,
and their signal is verified in decoded audio from the exported MP4.

## 1. Canonical real-Sonolus behavior

Direct playback of this exact fixture in real Sonolus was **not observed in this
run**. The local Android SDK was inspected; `adb devices -l` returned no devices.
The existing x64 emulator startup log reports a missing hypervisor driver. No
real-client DebugLog or audio result is claimed here.

Intended behavior is established from the exact compiled program and the public
contracts: [IncrementPost](https://wiki.sonolus.com/engine-specs/functions/increment-post)
returns the updated value; [IncrementPre](https://wiki.sonolus.com/engine-specs/functions/increment-pre)
returns the original value. Local `sdb-ref/functions/IncrementPost.h` and
`IncrementPre.h` independently implement that distinction. Applying it makes
the compiled scheduler itself emit the intervals below. This is reference and
execution evidence, not a substitute claim of real-device observation.

## 2. Canonical Sono-Renderer behavior before fixes

The actual preprocessing capture contained 93 `PlayScheduled` requests and no
loop requests. Every request is identified, including provenance, in
[the complete before timeline](HOLD_SFX_EVENTS_BEFORE.csv).

All the following are **PlayScheduled**, node **49283**, callback **Preprocess**,
JumpLoop slot **10837**, minimum distance **0.0167**, with no loop handle:

| Time | Effect ID | Effect name | Entity | Archetype |
|---:|---:|---|---:|---|
| 1.714285714 | 9 | Sekai Critical Tap | 87 | CriticalTapNote |
| 1.714285714 | 1 | #PERFECT | 174 | NormalHeadTapNote |
| 2.142857143 | 1 | #PERFECT | 175 | NormalTailReleaseNote |
| 2.142857143 | 1 | #PERFECT | 178 | NormalHeadTapNote |
| 2.357142857 | 1 | #PERFECT | 88 | NormalTapNote |
| 2.571428571 | 13 | Sekai Trace | 179 | NormalTailTraceNote |
| 2.571428571 | 1 | #PERFECT | 182 | NormalHeadTapNote |
| 2.678571429 | 13 | Sekai Trace | 89 | NormalTraceNote |
| 2.785714286 | 13 | Sekai Trace | 90 | NormalTraceNote |
| 3.000000000 | 9 | Sekai Critical Tap | 91 | CriticalTapNote |
| 3.000000000 | 1 | #PERFECT | 183 | NormalTailReleaseNote |

Thus the previously unidentified events at 1.7142857 and 2.1428571 were tap/head/
release one-shots. They did not contain a differently represented hold loop.
These APIs have **distance**, not a volume argument; no volume was inferred.

## 3. Compiled scheduler reconstruction

Connector preprocess root **120739** calls JumpLoop **120738**, with **3455**
branches. The two input chains are `176 -> 180 -> 184 -> 0`, linked through
Entity Data Array block 4101, offsets 15 and 16 in 32-value rows. Their associated
head and tail note times are 1.714285714/2.142857143,
2.142857143/2.571428571, and 2.571428571/3.0. Normal kind is **1**.

The program counts each list and performs bottom-up linked merges with run
widths 1, 2, 4. Slots 53/64 delimit runs of the first list; slots 115/126 do so
for the second list. Increment return values feed the width comparisons.
Merge comparisons preserve a stable ordering for equal event times.

The event sweep then combines the two sorted cursors. Relevant temporary cells
in block **10000** are reused across phases:

| Cell | Meaning during the event sweep |
|---:|---|
| 1 | Remaining tail/deactivation cursor |
| 14 | Remaining head/activation cursor |
| 3 | Next boundary, the minimum available head/tail time |
| 4 | Current segment start |
| 2 | Latest consumed deactivation boundary, initially +100000000 |
| 16 | Latest consumed activation boundary, initially -100000000 |
| 17 | Representative most recently activated connector, initially 0 |
| 5 | Connector kind, then selected effect ID |

There is no numeric active-hold count in this sweep. Its activity test is the
boundary relation `temp[16] >= temp[2]`, plus a nonzero representative connector.
At a tie the program consumes matching activations (slots 249–252), then matching
deactivations (254–257), before updating the segment start. Equal times therefore
produce no zero-length audio request. The three adjacent holds each produce a
separate positive-length segment.

Sentinels are identified by producers and consumers, not by magnitude alone:
node **111998** initializes temp[4]/temp[16] to -100000000, temp[2] to +100000000,
and temp[17] to zero. Slots 172/174 also assign +100000000 when a boundary cursor
has no candidate; slot 173 takes a `Min` with the available tail time. Slots
173/175/262 consume the activation/deactivation watermark comparison. Before
any activation, -100000000 >= +100000000 is false. Slots 250–257 replace the
watermarks with real consumed event times. The sentinels never become hold
start/stop times in the corrected capture.

Normal kind 1 follows **195 -> 198**, which selects effect **7 (#HOLD)**.
The critical mapping follows **197**, selecting **11 (Sekai Critical Hold)**.
The replay/input chain is consulted for suppression boundaries (BeatToTime,
GetShifted, Copy, and paired SwitchIntegerWithDefault cursors). Interior allowed
segments call slot 234; the final allowed segment calls slot 247. Start handles
are saved and passed immediately to the matching scheduled stop. The later
321/334 path performs analogous trailing-segment scheduling. Empty cursors and
an inactive watermark terminate at 335. The outer dispatcher eventually reaches
its final branch 3454.

### Every compiled loop call

No immediate `PlayLooped` or `StopLooped` nodes occur in this Watch graph.
All calls below are scheduled; the complete parent/dependency query is retained
in `target/hold-audit/loop-graph.json`.

| Start node | Stop node(s) | Callback root / JumpLoop slots | Effect/time/handle expressions |
|---:|---|---|---|
| 8871 | 8873 | Initialization 9055 / 1480 | temp[3] (7 or 11), temp[0] -> temp[1]; returned handle saved in temp[0] |
| 8955 | 8873 | Initialization 9055 / 1507 | temp[2] (7 or 11), temp[0] -> temp[1]; handle temp[0] |
| 8988 | 8873 | Initialization 9055 / 1513,1518 | literal 7 (#HOLD), temp[0] -> temp[1]; handle temp[0] |
| 9035 | 8873 | Initialization 9055 / 1524,1529 | literal 11 (Sekai Critical Hold), temp[0] -> temp[1]; handle temp[0] |
| 112183 | 112185,112212 | Connector 120739 / 234,247 | temp[5] (7 or 11), temp[4] -> temp[21] or temp[3]; handle temp[18] or temp[0] |
| 112399 | 112401,112428 | Connector 120739 / 321,334 | temp[1] (7 or 11), temp[4] -> temp[12] or temp[2]; handle temp[10] or temp[0] |

Initialization slots 1450/1451 gate its replay path on Runtime Environment[4]
and Option[24]. It reads streams 6000035–6000038 or normal streams
2000018/2000019 and critical streams 4000018/4000019. Effect-selection nodes
8868/8870 and 8952/8954 resolve the dynamic IDs to 11/7. These are normal/critical
hold replay schedules, not an unidentified third loop effect. The canonical
autoplay capture emits its loops through Connector slot 247 only.

## 4. VM semantic audit

The full static dependency closure of Connector scheduler slots 41–334 contains
the following function surface. PASS means the valid arguments used by this
compiled slice agree with the documented contract and the traced state changes;
it does not claim universal real-client conformance for invalid inputs.

| Primitive/subsystem | Status | Evidence |
|---|---|---|
| Get, Set | PASS | Reads/writes match block/address arithmetic; 4101 rows retain cross-entity links; scratch stores return their value |
| GetShifted | PASS | `Get(id,x+y*s)`; official contract, SDB implementation, existing addressing tests |
| Copy | PASS | Snapshot source before writes, including overlap; existing range tests. SDB's byte-count memmove is defective and was not copied |
| SetMultiply | PASS | Stored/returned product; official contract and SDB |
| IncrementPost | FIXED | Updated counter must be returned; official contract, SDB, new all-address-form tests |
| Execute | PASS | Ordered children, last result; route and side-effect trace |
| If | PASS | Only selected consequent executes; evaluated-argument nulls expose skipped children |
| SwitchWithDefault | PASS | Ordered test/consequent pairs; existing connector-shaped and fractional tests |
| SwitchIntegerWithDefault | PASS | Equality to integer branch labels, final default; existing fractional/laziness tests |
| Add, Multiply | PASS | Ordered numeric operands; address arithmetic and merge widths agree |
| Min | PASS | Chooses minimum available event time; sentinel-to-real-time transitions captured |
| Equal | PASS | Exact equal boundary times consume both cursor groups |
| Less, LessOr | PASS | Strict/inclusive ordering; incorrect counter operand, not comparison implementation, caused bad merge routing |
| Greater, GreaterOr | PASS | Positive entity IDs and watermark ties agree with supplied values |
| BeatToTime | PASS | BPM-backed conversion on suppression-chain events; canonical hold times preserved |
| PlayLoopedScheduled, StopLoopedScheduled | PASS | IDs, returned handle identity and clock contract; final media verification |
| JumpLoop | PASS for this path | Starts at 0, follows returned integral slot, final branch result, out-of-range zero, persistent scratch between iterations, nested execution and side effects tested |
| Temporary block 10000 | PASS for initialized scratch | Official initial values are unpredictable. Renderer resets at callback entry, retains writes throughout that callback. Compiled scheduler initializes relevant state before use; no required cross-callback retention demonstrated |
| Entity references/4101/4102/4001 | PASS for this path | Resolved IDs/32-value row addressing, preprocess writes, shared suppression cursor updates and linked lists captured |
| Callback order/lifecycle | PASS for fixture | Initialization -3, notes 0, Connector 1; scheduling once in preprocess; subsequent frame adds no duplicate loops |

JumpLoop's dedicated audit includes zero branches, negative/large nonexistent
branches, last-branch termination, nested loops, memory persistence, evaluation
order and side effects. Existing Break tests exercise unwind propagation; this
scheduler slice contains no Break. **Fractional branch results remain outside
the established oracle coverage:** SDB truncates, renderer rejects, and the
public contract does not specify conversion. No speculative patch was made.

Temporary Memory is documented as operational memory during callbacks, with
unpredictable initial contents. It is neither an evaluation-local reset nor
persistent entity state. Renderer chooses zero for untouched locations; this
is an allowed initial value, not a guaranteed value engines should depend on.
The private previous native analysis supports a per-thread storage partition
but does not establish client clearing/reuse timing. No lifetime fix is justified.

The six audio host operations have correct argument order and returns for valid
requests: Play(id,distance)/PlayScheduled(id,time,distance) return zero;
PlayLooped(id)/PlayLoopedScheduled(id,startTime) return unique handles;
StopLooped(handle)/StopLoopedScheduled(handle,endTime) return zero. Scheduled
timestamps remain in the Watch/BGM clock, with media offset applied downstream.
All canonical requests meet the 0.5-second lead-time requirement. Distance is
not volume. Unknown/missing resource payloads are diagnostic warnings in the
mixer; exact real-client invalid-ID behavior remains unverified. SDB's scheduled
loop hosts are stubs and cannot serve as an audio oracle.

## 5. First divergence

There are two useful boundaries; distinguishing them avoids overstating a later
symptom as the first difference.

```text
First different scalar result in the captured Connector 176 callback:
callback = Preprocess; root = 120739; JumpLoop = 120738
slot = 6; node = 9356; operation = IncrementPost
inputs = block 10000, address 0, original value 0
old result = 0; reference/corrected result = 1
The surrounding Execute discards this return; memory becomes 1 in both runs.

FIRST DIFFERENT CONTROL-FLOW ROUTE in the relevant scheduler:
entity = 176; callback = Preprocess
slot = 53; node = 8121; operation = IncrementPost
inputs = block 10000, address 15, original count 0, merge width 1
old result = 0; reference/corrected result = 1
Less node 111737: old 0 < 1 true; corrected 1 < 1 false
next slot = old 54; corrected 57
```

The same defect subsequently hits the deactivation sort at **slot 115**, node
**2707**, block 10000 address 4. Less node **111876** incorrectly chooses another
item when the width-one run is already full. This is the destructive divergence
that loses the tail list.

## 6. Complete causal chain

```text
Pre/Post return names interpreted as conventional C-style operators
  -> IncrementPost returns original count instead of updated count
  -> width-one merge runs consume two connectors
  -> deactivation width-two pass takes 176->180->184 entirely as left run
  -> right-run pointer is 0; slot 122 routes to unmatched-run append 162
  -> node 111984 writes append link using new-tail 0 (row-zero address 16)
  -> new-head temp[16] remains 0
  -> node 111985 replaces list-head temp[1] with 0
  -> slot 168 sees 0 > 0 false and routes 262
  -> slot 262 sees -100000000 >= +100000000 false and routes 335
  -> no slot 234/247 or 321/334, no loop host request, no #HOLD audio
```

With corrected return values, both sorts retain the chain. Activation 176
advances temp[14] to 180 and sets temp[16]/temp[2] to 1.714285714 and temp[17]
to 176. At the next boundary, kind 1 maps to effect 7; slot 246 sees
2.142857143 > 1.714285714 and reaches 247. It starts handle 0 and stops that
same handle at 2.142857143. At the tie, 180 becomes the representative while
176 is removed from the tail cursor. The same transitions emit handles 1 and 2
for 180 and 184. All six host requests originate from **entity 176**, which
schedules the linked chain. Later Connector callbacks do not schedule it again.

Actual and expected input IDs/kinds/times agree before sorting. The old run
first differs in returned counters, then routing, then list-head state; it never
reaches effect selection. The corrected run reaches each stage with the values
above. This excludes a comparison, callback-order, cross-entity-write, resource
lookup or downstream mixer defect as the canonical cause.

## 7. Fixes

1. **runtime.rs / increment-decrement dispatch:** Pre returns old, Post returns
   new, for direct, shifted and pointed address forms. Previously reversed for
   all twelve variants. Only IncrementPost is required to restore this fixture;
   the same established naming contract justifies correcting the family.
2. **runtime.rs / Switch:** replace indexed dispatch with ordered explicit
   test/consequent matching, returning zero on no match. The official
   [Switch contract](https://wiki.sonolus.com/engine-specs/functions/switch) and
   SDB agree. Fractional explicit tests now work, skipped branches stay lazy.
   This is an adjacent defect, not the canonical cause.
3. **runtime.rs / Or:** return the first nonzero value and stop evaluating later
   arguments. Previously evaluated everything and returned a Boolean. The
   [official Or contract](https://wiki.sonolus.com/engine-specs/functions/or)
   establishes value semantics; conditional evaluation agrees with the official
   control-flow optimization rules. SDB only accepts 1 and is not authoritative
   for general nonzero values. This is also adjacent to the canonical cause.

Old hardcoded RHS node/archetype logging and callback replay were removed.
The replacement diagnostics are generic, opt-in and bounded. No fixture or
resource was altered. No private/native implementation was copied into runtime.

## 8. Regression tests

New `tests/watch_scheduler.rs`:

- `sonolus_pre_post_updates_return_the_named_state_for_all_address_forms`
- `post_increment_run_counter_does_not_consume_an_extra_merge_item`
- `canonical_next_rush_normal_holds_schedule_three_matched_intervals`
- `switch_matches_test_values_and_evaluates_only_selected_consequent`
- `or_returns_first_nonzero_value_and_short_circuits_side_effects`
- `jump_loop_routes_nested_branches_preserves_memory_and_stops_at_last`
- `execution_trace_is_bounded_filters_writes_and_resolves_stops`

The canonical integration test verifies pinned archive hashes, 93 one-shots,
three starts/stops, effect 7, exact interval times and matched handles, and no
duplicate scheduling on a later frame. It skips explicitly when local assets
are unavailable. All seven tests passed with the supplied fixture.

Existing tests for pointed/shifted Pre/Post now assert the documented returned
state. The Baumkuchen workload test was re-pinned after the correction: 10890
callbacks, 12870999 evaluations, 615 one-shots, 90 starts and 90 stops. Its old
89-loop pin described the defective VM. Independent sequential/pipelined RGB
and event comparisons are preserved; the new event hash is
`2d8c261fbba9a28a77842a05665720e655d73f0f`.

## 9. Canonical after-fix result

| Metric | Before | After |
|---|---:|---:|
| PlayScheduled | 93 | 93 |
| PlayLoopedScheduled | 0 | 3 |
| StopLoopedScheduled | 0 | 3 |

| Effect ID/name | Start | Stop | Handle | Origin entity | Start/stop node | Slot |
|---|---:|---:|---:|---:|---|---:|
| 7 / #HOLD | 1.7142857142857142 | 2.142857142857143 | 0 | 176 | 112183 / 112212 | 247 |
| 7 / #HOLD | 2.142857142857143 | 2.5714285714285716 | 1 | 176 | 112183 / 112212 | 247 |
| 7 / #HOLD | 2.5714285714285716 | 3.0 | 2 | 176 | 112183 / 112212 | 247 |

All originate in Connector.Preprocess. The full 99-request semantic timeline is
[HOLD_SFX_EVENTS_AFTER.csv](HOLD_SFX_EVENTS_AFTER.csv). Time-zero raw Draw hash
remained `69f51340b927d4518e5ca1de96584a92627f20cc` across the causal correction.

Before trace: 27917 records, zero drops. Corrected causal trace: 30384 records,
zero drops. Final tracer including constants: 51803 records, zero drops.
Complete entity 176/180/184 preprocess routes, with sequence, slot, child and
next slot, are in `target/hold-audit/{before,after-increment,final}-routes.csv`;
their JSONL traces retain evaluated arguments and every relevant memory operation.

## 10. Final media verification

`target/hold-audit/larp-hold-fixed.mp4`: 1.5–3.2 seconds, 51 H.264 frames at
30 FPS, 640x360, AAC stereo 44.1 kHz. BGM offset is -1.454 seconds; output-time
hold intervals are 0.214285714–0.642857143, 0.642857143–1.071428571 and
1.071428571–1.5 seconds.

`watch_trace_audio` reconstructs SFX from the **captured actual host requests**,
without evaluating Watch. Two diagnostic comparison mixes used the same BGM
window and encoder settings; one retains all captured requests, the other omits
loop requests. These are validation controls, not production event synthesis.

Decoded final MP4 vs encoded captured-event mix: **149940 PCM channel samples
compared, every sample identical, RMSE 0**. Against the control without holds:
RMSE **1568.384 PCM16 units**. This proves the final encoded media contains the
requested hold payload, not merely a nonempty host-event list. Earlier direct
PCM residual correlations per hold were 0.733/0.951/0.851 with gains
0.943/0.979/0.951; identical encoded comparison eliminates AAC loss as a doubt.
Evidence: `encoded-audio-evidence.json`, `final-audio-evidence.json`, and WAV/M4A
controls in `target/hold-audit`.

Canonical critical-hold count is zero: this fixture contains normal holds.
The graph retains critical mapping to effect 11. A separate complete audio-host
trace of ARMAGEDDON identifies **87 #HOLD starts and 17 Sekai Critical Hold
starts**, with 104 matching scheduled stops and zero dropped records. This
exercises both mappings; no direct client critical-audio comparison is claimed.

## 11. Production regression matrix

| Fixture / resources | Check | One-shots | Starts/stops | Result |
|---|---|---:|---:|---|
| Horizon / Dreamer / project(1).scp | Watch at 10s; 10–11s export | 588 | 0/0 | PASS |
| Next RUSH / Baumkuchen / Faithful 0.8.3 | Watch at 10s; 10–11s export; pipeline RGB/event tests | 615 | 90/90 | PASS after re-pinning old workload |
| Next Sekai / ARMAGEDDON / Faithful 0.8.3 | Watch at 10s; 10–11s export | 1480 | 104/104 | PASS |
| Canonical Larp / Faithful 0.8.4 | 1.5–3.2s high-resolution segment and audio identity check | 93 | 3/3 | PASS |
| Canonical Larp / Faithful 0.8.4 | Whole-chart export, inferred end 19.516667s, 40 frames to 20s | 93 | 3/3 | PASS |

Cheap matrix exports use 160x90 at 2 FPS; they are lifecycle/render/mux smoke
checks, not every-frame visual equivalence proofs. All report one Watch playback
traversal and zero SFX-prepass workload. FFprobe verifies H.264/AAC streams and
durations. Particles were enabled. Existing particle snapshot, dynamic-stage,
CPU/WGPU and sequential/pipeline comparison tests provide the finer checks.
Horizon at 10s activates RotateEvent/ShiftEvent/ZoomEvent and spawns LaneEffect
and particles. ARMAGEDDON executes particle hosts and hundreds of draw operations.

## 12. Test/build state

Validation logs are in `target/hold-audit`: `fmt-check.txt`, `check.txt`,
`release-build.txt`, `release-examples.txt`, `scheduler-tests.txt` and
`full-tests-final.txt`. The initial full suite exposed the stale Baumkuchen
workload pin described in section 8; it was not hidden or attributed to timing.
The historically flaky `bounded_pipeline_matches_sequential_hashes_and_preserves_fifo`
passed in both full runs.

```text
cargo fmt --check = PASS
cargo check = PASS
cargo check --examples = PASS
cargo build --release = PASS
cargo build --release --examples = PASS
cargo test = PASS: 197 passed, 3 ignored, 0 failed
known unrelated pipeline timing test = PASS
```

The passing groups contain 81 library, 8 binary, 3 arctan2, 98 m0, and 7
scheduler/diagnostic tests. The full suite took about 274 seconds. The targeted
scheduler tests were also rerun after the final tracer capacity guard.

## 13. Architecture

```text
authoritative Watch traversals per production export = 1
SFX prepass added = NO
engine-specific runtime workaround = NO
Project SEKAI-specific runtime workaround = NO
manual hold inference = NO
synthetic production events = NO
```

Diagnostics attach to that same traversal. The comparison PCM example reads
saved events and does no Watch execution. Existing test-only comparison sessions
are validation oracles and do not enter the production export path.

### Reusing the tools

`run-watch --watch-trace-output trace.jsonl --watch-trace-config filters.json`
records evaluation sequence, entity/archetype/callback/root, node, compiled child
IDs, evaluated arguments/results, nested loop slots, routes, memory reads and
committed/rejected writes, and audio requests with resolved effect names. A
skipped lazy argument is null. Numeric values use strings to preserve nonfinite
values. Constants are included. Default capacity is 10000; maximum is 2000000.
Capacity exhaustion is explicitly reported by the final dropped count.

```json
{"capacity":1000000,"filters":[
  {"entities":[176,180,184],"callbacks":["Preprocess"]},
  {"kinds":["audio"]}
]}
```

Filters are ORed; nonempty fields inside each filter are ANDed. Available fields
also include archetypes, nodes, operations, slots, blocks, addresses
(`[[10000,4]]`), kinds, time_min and time_max. Time filters select callback clock,
not the future timestamp of a scheduled event. Kinds include evaluation, route,
memory_read, memory_write and audio. The old saved baseline uses
memory_write_attempt; final traces include committed/error status.

Compiled graph queries never run the VM:

```powershell
cargo run --example watch_graph -- $engine --node 112183 --consumers --depth 5 --limit 500
cargo run --example watch_graph -- $engine --root 120738 --slot 53 --depth 6 --limit 500
cargo run --example watch_graph -- $engine --operation PlayLoopedScheduled --operation StopLoopedScheduled
cargo run --example watch_graph -- $engine --block 10000 --root 120739 --limit 100
```

Other flags are --literal, --output, --dump. Output includes parent indices,
callback roots and containing JumpLoop slots; slicing is bounded by depth and
node limit, with truncation signaled. --root scopes operation/literal/block
queries; with --slot it must name a JumpLoop. Argument validation rejects invalid
indices. --block recognizes statically literal Get/Set block operands; computed
addresses remain visible as expression dependencies rather than guessed values.

The audio reconstruction example takes engine, resources, effect collection,
trace, output, --start, --duration and optional --omit-loops. On Windows, launch
a built example from `target/debug` rather than its `examples` subdirectory so
the existing FFmpeg application-root discovery finds shared addons.

## 14. Remaining known issues

- Direct exact-fixture playback and DebugLog in real Sonolus remain unobserved
  here; no connected Android client was available.
- Fractional JumpLoop routing differs from SDB's truncation and needs a real
  client oracle before changing it. The scheduler uses integral routes.
- Temporary-memory native clearing/reuse timing is not established, but initial
  contents are explicitly unpredictable and this initialized path is correct.
- Real-client invalid effect-ID and invalid loop-handle edge behavior is not
  independently established. Valid canonical audio calls are verified end to end.
- Broader unsupported function names fail explicitly. Existing known private
  GPU/particle oracle gaps remain outside this hold fix; no speculative semantic
  changes were made for them.
- Build emits existing deprecation/dead-code warnings. Local fixture tests depend
  on supplied, untracked TestingSuite assets.
