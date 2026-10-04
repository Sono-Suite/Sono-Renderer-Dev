# Historical MV presentation regression investigation

## Result

**Closed by the user's configuration correction (2026-10-03): the earlier known-good screenshot used a static stage; the affected case uses a dynamic stage. That comparison was not a valid regression oracle. The issue is now a pre-existing dynamic-stage compatibility discrepancy observed against real Sonolus, not an MV/GUI regression. Do not resume git-history hunting for this visual issue.**

The preserved controlled comparisons acquit MV percentage, GUI, base-clear and RGB24 as introducing changes. Old/current builds reproduce the same dynamic-stage appearance; backend agreement does not establish Sonolus compatibility. `cc08c08` also completed with the current CPU hash listed below. The `ed29c2f` export was stopped when the user closed this investigation; its nonzero exit is cancellation, not a historical renderer failure. No earlier revisions were rendered afterward.

**No last-good/first-bad rendering boundary was established. No production change was made.** This investigation does not assert that the current stage appearance is correct. It establishes that the recovered pre-percentage implementation renders exactly the same tested frames as the post-percentage implementation and current working tree.

The earlier screenshot is retained as historical context, not as a dynamic-stage appearance target. Current CPU/WGPU or GUI/CLI agreement was not used as the historical oracle. No stage-semantic, texture-intent, transform, particle, VM, engine-option or compiler investigation was performed during this historical audit.

## Git and snapshot recovery

Normal history ends at `18baa7d` (2026-10-03 11:05:03 Eastern). It has no MV decoder or percentage feature: `--mv` is an unsupported placeholder. No reachable commit records the MV/percentage milestone, so ordinary `git bisect` cannot bracket that milestone.

`git fsck --no-reflogs --unreachable` found unreachable **trees/blobs**, but no unreachable commits. These preserve complete recoverable source snapshots. They were archived into ignored `artifacts/mv-bisect/` and built offline, without switching branches or modifying the live working tree/index:

| Label | Recovered source tree | Feature state |
|---|---|---|
| HEAD | 18baa7d | before actual MV implementation |
| pre | 324be4e42964e013e2bb5ddfc563bba3d536a973 | initial MV implementation, before percentage |
| pre-single-watch | cf2bda30c7693a26cd9af915188d928a6eca3c4d | single-Watch export, before percentage |
| post | 1bbcf40e689ea3aa8068b0e30aced57b281ad9de | native GUI/percentage, before subsequent opacity audit |
| current | preserved working tree | all current features and opaque-MV presentation |

These are feature-state labels, **not proven good/bad classifications**. Unreachable trees have no commit timestamp or parent; their feature contents and recovered edit history establish relative implementation states, not a fabricated commit chronology.

Two additional pre-percentage root trees (`895ca6...`, sharing the same MV implementation, and the corresponding subtree snapshots) were inventoried. The tested `pre-single-watch` already exercises the single-Watch transition; the additional root differs in retired profiling helper bookkeeping, with no compositor changes.

## Exact source evidence

Across HEAD and all tested historical snapshots, these source blobs are identical:

- `src/gpu_render.rs`: `ecac6c05d70a889d1ee724b57308aa5583b00b2f`
- `src/runtime.rs`: `40998df6e8bbed6f28c69aaa4cfe08cfc7cc45a7`
- `src/formats.rs`: `8573a3fe191dcedf5dfd302003570cd69e24fd74`

Thus the percentage boundary contains no changes to the shared rasterizer/blend helper, WGPU shader/blend state, base clear, framebuffer readback, or resource interpretation in those modules. The current differences in `runtime.rs`/`gpu_render.rs` are the previous audit's `cfg(test)` hooks only.

The pre-percentage MV blob is `39d5d134aa31813356fdd44837f8aef392727ea7`; post-percentage is `f797397d47f2aed327ef1606bc9fcaffe175c24a`. The actual introducing edit was recovered from the project execution history at **2026-10-03 15:32:59 Eastern** (19:32:59 UTC). It adds:

- percentage configuration/state/setter and RGB scaling before MV atlas identity;
- an explicit early return at 100% in the original scaling helper;
- shared resource override paths with default fallbacks, keeping skin loading unchanged;
- session token plumbing and calls to the percentage setter.

It does not edit gameplay blend equations or rasterizer modules. Other changes between the recovered root trees include native GUI and export lifecycle/error/cancellation work. Full source diff and the exact original introducing tool record are preserved; no recovered tool script was blindly executed against the live repository.

## Controlled renders

Same Next RUSH ZIP, confirmed `DISPLAYHOLIC AUDIO.json.gz`, ProSeka Faithful 0.8.3 SCP for skin and particles (older versions use the same SCP fallback), audio and MV paths, default engine options, WGPU, pipeline, 60fps. Input SHA256 hashes and exact commands are recorded. Music/encoding differences are excluded by comparing decoded RGB, rather than requiring historical MP4 container/audio equality.

### Watch 5.0-5.5s, 640x360, 30 frames

| Snapshot | No-MV decoded RGB sequence SHA256 | MV decoded RGB sequence SHA256 |
|---|---|---|
| HEAD | b47f950b088a3e2a46bb6dd4622e48f92bb2f0d1832ecb50ec540c1f652e1ffd | unavailable (MV not implemented) |
| pre | b47f950b088a3e2a46bb6dd4622e48f92bb2f0d1832ecb50ec540c1f652e1ffd | 177da8cff1e891b62cdcb302aec9d438b11dd541a08beefde067c4063632f667 |
| pre-single-watch | b47f950b088a3e2a46bb6dd4622e48f92bb2f0d1832ecb50ec540c1f652e1ffd | 177da8cff1e891b62cdcb302aec9d438b11dd541a08beefde067c4063632f667 |
| post (100%) | b47f950b088a3e2a46bb6dd4622e48f92bb2f0d1832ecb50ec540c1f652e1ffd | 177da8cff1e891b62cdcb302aec9d438b11dd541a08beefde067c4063632f667 |
| current (100%) | b47f950b088a3e2a46bb6dd4622e48f92bb2f0d1832ecb50ec540c1f652e1ffd | 177da8cff1e891b62cdcb302aec9d438b11dd541a08beefde067c4063632f667 |

### Watch 3.0-3.1s, 1920x1080, 6 frames

This earlier range has fewer notes and exposes the far stage at native resolution. Pre-percentage and current decoded sequences are also byte-identical:

- no MV: `cbadc9d5a5ac69d9e884157c5815c2ee8f88d696c44d988738c0bbeb4a988e0c`
- MV / current 100%: `26205f619a05483306640630693e7f761fbd52ddda08f9fb210b1c09574a6ba1`

Both comparison ranges use active MV media times. No different-time screenshots were used to classify revisions. All renders completed successfully. No first bad was found, so there is no justified introducing-bug regression test or permanent selective revert.

## Artifacts and preservation

`artifacts/mv-bisect/` contains archived source, isolated historical binaries, offline build logs, MP4s/first-frame PNGs, `historical-comparison.png` (left pre, right current; top no MV, bottom MV), input hashes, commands/RGB hashes, recovered MV snapshot inventory, exact source blob provenance, `full-pre-post.diff`, and `percentage-introduction.txt`.

Historical build setup reused the existing managed FFmpeg binaries locally. An initial missing-tool attempt tried the existing lazy provisioning path and failed under restricted network; tools were then supplied locally. An initial overlapping historical build/render held the shared executable open on Windows; the export was allowed to complete and builds were rerun serially with preserved per-snapshot binaries. Final archived sources were not patched to overcome either setup issue.

Current GUI, MV percentage, single-Watch export, base-clear, RGB24 and all unrelated work remain intact. No golden expectations or production source were changed in this historical investigation. Four historical release builds and all matched renders passed; current full-suite/format/release validation remains the preceding 184-test passing run, since no implementation changed here.

## Further-back scan requested by the user

The user confirmed that the original known-good executable/checkout is unavailable and requested scanning further back. Archived builds were extended into the committed renderer history, using the same Watch 3.0-3.1s, 1920x1080, 60fps DISPLAYHOLIC inputs.

| Revision | Backend | Decoded RGB SHA256 | Comparison |
|---|---|---|---|
| `385628f` | WGPU | `cbadc9d5a5ac69d9e884157c5815c2ee8f88d696c44d988738c0bbeb4a988e0c` | identical to current WGPU |
| `9f83aa4` | WGPU | `cbadc9d5a5ac69d9e884157c5815c2ee8f88d696c44d988738c0bbeb4a988e0c` | identical to current WGPU |
| `22e27f9` | WGPU, explicitly selected | `cbadc9d5a5ac69d9e884157c5815c2ee8f88d696c44d988738c0bbeb4a988e0c` | identical to current WGPU |
| `22e27f9` | CPU, historical default | `5d02639be4f00004de4d09e95d725f5fc53b16d455ba64fc6a9d585e211eae41` | identical to current CPU |
| `6331be7` | CPU, predates WGPU | `5d02639be4f00004de4d09e95d725f5fc53b16d455ba64fc6a9d585e211eae41` | identical to current CPU |

The CPU/WGPU hash difference is not used to classify a historical regression: each backend is compared against its own current output. `22e27f9` introduced WGPU and defaulted to CPU; an additional explicit `--backend wgpu` run prevents that default change from contaminating the comparison. The unchanged WGPU sequence now reaches the first committed WGPU implementation, before base-clear/RGB24 and all recovered MV/GUI work. Older CPU-only source is being checked separately.

Exact commands, hashes, binaries, first-frame PNGs, build/export logs and relevant consecutive source diffs are preserved in `artifacts/mv-bisect/`, including `earlier-comparison.json`, `first-wgpu-comparison.json`, `current-cpu-comparison.json` and `earlier-provenance.json`.

## Unidentified good endpoint

The precise executable/source revision and invocation producing the known-good screenshot have not been identified. The recovered pre-percentage implementations are genuine historical source, but their output in these controlled cases already equals current output. Calling them last good, or labeling the percentage edit first bad, would contradict the measured hashes.

A valid regression boundary still requires a controlled output difference that reproduces the reported appearance. Do not manufacture a compositor fix, bless a golden, or infer correctness merely from the absence of a historical difference.
