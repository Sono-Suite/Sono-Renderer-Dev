# Pre-UI storage checkpoint and cleanup

Date: 2026-10-05

## Checkpoint

- Branch: `main`.
- Commit: `153df7155dddf84e7237c6c24095413eae0efad7` (`small change`).
- Local annotated tag: `pre-ui-known-good-2026-10-05`.
- At the checkpoint, tracked files were clean. The existing untracked
  `dependencies/ffmpeg/` (446,513,664 bytes) and `desktop_10s_profile.txt`
  (1,140 bytes) were retained. The former is the renderer's local managed
  FFmpeg fallback; the latter is a failed profiling invocation record.
- The checkpoint includes the Watch, particle, export, and hold-SFX fixes.
  Cleanup changes in this working tree affect `.gitignore`, this report, and
  `scripts/clean-generated.ps1`; `git diff -- src` is empty.

## Disk audit

The initial recursive scan measured approximately **93.1 GiB (100.0 GB)**.
The largest directories were:

| Directory | Before | Disposition |
|---|---:|---|
| `artifacts/` | 47.58 GiB | Retained forensic evidence and regression media |
| `target/` | 21.58 GiB | Cargo build caches removed; particle/audio evidence kept |
| `TestingSuite/` | 19.02 GiB | Retained canonical fixtures, Sonolus inspection inputs, recordings, and Android SDK |
| `.git/` | 4.36 GiB | Retained; no history rewriting or garbage collection |
| `dependencies/` | 0.42 GiB | Retained managed FFmpeg fallback |

Other material: `sdb-ref/` 0.08 GiB, `assets/` 0.04 GiB, and the root SCP/ZIP
fixtures about 0.04 GiB combined.

The most notable individual files are a 4.05 GiB Git pack, 3.33 GiB Android
system-image files, a 2.5 GiB AVD snapshot, 1.51 GiB system images, and a
600 MiB LLVM installer. Two archived full-workspace copies under
`artifacts/base-clear/` contain further copies of Android/inspection files.
The `artifacts/base-clear/` report describes a reverted WGPU base-clear
experiment; its report, patch, profiles, and JSON results remain useful.

The Git inventory reports three packs totaling 4.12 GiB plus 235.15 MiB loose
objects. Its largest pack is `pack-9221aff996f2e28d42d5adbc67f92ce20c456077.pack`
at 4.05 GiB. The five largest blobs are 661, 443, 399, 352, and 287 MiB and
have no path in the objects reachable from current branches/tags. This indicates
substantial historical/unreferenced object storage. It was left untouched as
requested; no `filter-repo`, BFG, repack, prune, or history rewrite was run.

## Cleanup

Removed:

- `target/debug` (19,262,799,353 bytes).
- Most of `target/release` and the two stale custom Cargo output trees
  `target/base-clear-original` and `target/base-clear-history-9f`.
- Four byte-identical 13,279,803-byte MP4s in `artifacts/base-clear/`; kept
  `baseline.mp4`, SHA-256
  `F5CADAB83005B068CCBF4B63FBAFB47D6E142EBE3865F79966935D5A697CA313`.

The removed build/media files total **23,018,040,520 bytes** (about 21.44 GiB).
The final recursive workspace scan measured **about 76.95 GB (71.67 GiB)**.
New cleanup logs, report files, and four smoke-export files were created during
the audit, so the initial 93.1 GiB and net free-space change are reported
approximately.

The two extracted 19 GiB workspace copies under `artifacts/base-clear/` were
kept. A content-hash comparison stopped on access denied for
`TestingSuite/Sonolus Inspection/1.1.4/ios/tools/llvm/LLVM_23.1.2_X64_wix_en-US.msi`.
Because that left the duplicate audit incomplete, their contents were treated
as unknown rather than deleted. The compressed source archives and experiment
report were also retained.

`target/release/renderer.exe` was active during cleanup (PID 9420), so its
directory was skipped by the cleanup tool. Windows had already removed the
other release outputs; the remaining executable and PDB total about 24 MiB.
The process was left alone. Re-run the cleanup dry-run after it exits to remove
those final files.

## Prevention and verification

`.gitignore` now explicitly ignores `/target/`, the app-managed
`/dependencies/ffmpeg/`, and root `/renders/`, `/diagnostics/`, and `/tmp/`
locations. Existing `TestingSuite/` and `artifacts/` rules remain. The
`scripts/clean-generated.ps1` helper defaults to a dry run and targets only the
Cargo build directories listed in the script. `-Apply` is required to remove
them; it skips an active `target/release/renderer.exe` and checks the custom
Cargo target layout before deletion. Forensic evidence in sibling target
directories is preserved.

After removing the duplicate MP4s, `cargo fmt --check`, `cargo check`,
`cargo build --release`, and `cargo test` passed. Test results: 204 passed,
0 failed, 3 ignored. The test suite breakdown is in
`artifacts/pre-ui-checkpoint/tests-after.txt`. These checks completed before
deleting the generated Cargo build caches; no Rust source changed during cleanup.

Four post-cleanup CPU exports passed and ffprobe found H.264 video plus AAC
audio: Dreamer, Baumkuchen, ARMAGEDDON, and whole-chart Larp. Larp rendered 40
frames through 20 seconds and retained 93 scheduled one-shots, three loop
starts, three loop stops, and one Watch traversal. The full test suite also
passed its WGPU particle, render, and export tests. Logs and media probes are in
`artifacts/pre-ui-checkpoint/`; smoke MP4s are in `target/particle-audit/`.

The known-good tracked renderer at the checkpoint is safe to use as the
starting point for UI-0/presentation-boundary work. The cleanup did not change
runtime semantics or the single authoritative Watch traversal. Remaining large
storage is the retained Android/inspection corpus, the 38.39 GiB archived
workspace copies, the 4.36 GiB Git object store, and forensic/render evidence
whose purpose is documented by existing audits.
