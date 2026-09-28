# M0 format and architecture notes

## Scope

M0 handles package inspection, format parsing, resource resolution, and static compatibility analysis. It does not execute WatchData. In particular, listing a function as syntactically present says nothing about VM execution, drawing, particles, or audio support; those are reported as separate capability layers.

## Fixture observations

Reference inputs are supplied in the renderer workspace as ZIP packages:

- `Next Sekai Engine.zip` contains `Next Sekai Engine/engine.json` metadata and `EngineWatchData`, `EngineConfiguration`, and `EngineRom` members.
- The engine's WatchData, configuration, and ROM entries start with gzip magic `1f 8b`. The first two become JSON after decompression; ROM remains a binary resource in M0.
- `Exorcist.zip` contains `Exorcist/item.json` and gzip-compressed `Exorcist/level.data` JSON.
- `ProSeka Faithful 0.8.3.scp` is a ZIP archive containing `sonolus/skins`, `sonolus/backgrounds`, `sonolus/effects`, and `sonolus/particles` item records; resource bytes live under `sonolus/repository/<hash>`. A reader must resolve the archive references, rather than assume resource bytes are named by role.

There was no pre-existing Rust layout to preserve. Fixture archives remain unchanged and integration tests read them in place.

## Specification basis

The [Sonolus engine overview](https://wiki.sonolus.com/engine-specs/overview) describes WatchData as engine behavior represented by archetypes and flattened AST nodes. The current [Engine Watch Data schema](https://wiki.sonolus.com/engine-specs/resources/engine-watch-data) defines skin sprite, effect clip, and particle effect bindings, archetypes, `updateSpawn`, and nodes. The [Watch archetype schema](https://wiki.sonolus.com/engine-specs/resources/engine-watch-data-archetype) documents lifecycle callbacks and imports. [Engine Data Nodes](https://wiki.sonolus.com/engine-specs/resources/engine-data-node) are generic function/value nodes, so the parser retains function names as open strings. [Engine Configuration](https://wiki.sonolus.com/engine-specs/resources/engine-configuration) and [Level Data](https://wiki.sonolus.com/level-specs/resources/level-data) are gzip-compressed JSON. [Engine ROM](https://wiki.sonolus.com/engine-specs/resources/engine-rom) is gzip-compressed binary containing little-endian IEEE-754 single-precision values; M0 retains the decoded bytes and does not interpret them. The supplied SCP archives demonstrate content-addressed repository storage.

Specification versions evolve. The scanner must retain unknown JSON properties and unknown node/function names so a newer or engine-specific construct stays inspectable instead of failing deserialization. A format version is evidence for future runtime requirements, not proof of compatibility.

## Proposed M0 layout

Start with a single Rust package, split into modules while the boundaries stabilize:

```text
src/
  formats/       engine, level, resource-package and gzip/JSON decoding
  watch/         typed WatchData envelope, archetypes, callbacks, generic nodes
  compatibility/ static inventory and per-layer capability report
  cli/           inspect-engine, inspect-level, inspect-resources, compatibility
```

Future crate extraction can follow dependency direction: `formats` is independent; `watch` depends on formats; compatibility depends on both. Runtime/VM, rendering, particles, audio, video, and GUI are intentionally absent from M0.

## M0 data model

- Engine metadata, configuration, ROM bytes, and WatchData are distinct loaded values.
- Watch nodes preserve arbitrary operation/function names, indices, and operands (including operands not understood by this version).
- Level entities preserve archetype identifiers and named/value data without requiring the selected engine to implement them.
- SCP indexing resolves category/name records and resource blobs by repository hash, with hash verification when the package format provides an expected digest.
- Compatibility output includes Sonolus/engine version, archetypes, node operations, memory-block references, resource bindings and required resources, plus level archetype matches. Parse, VM, rendering, particle, and audio status are separate fields; M0 reports runtime capabilities as unimplemented rather than inferring support from parse success.

## Open specification questions to verify against fixtures

1. Current WatchData callbacks are documented (`preprocess`, `spawnTime`, `despawnTime`, `initialize`, `updateSequential`, `updateParallel`, and `terminate`) and reference nodes by index. The scanner retains unfamiliar future fields.
2. Supplied SCP item records use `{item: ...}` JSON and role objects with `hash` and `url`; payloads live at `sonolus/repository/<hash>`. The implementation checks SHA-1 against repository hashes; broader SCP variants still need tests.
3. The supplied level confirms named string archetypes and a list of named `{name, value}` entity data records. Entity data remains generic to preserve extensions.
4. Configuration is schema-described JSON. ROM is documented as packed floats; interpretation belongs in the later runtime memory model, not the M0 loader.

These are parser/schema questions, not reasons for fixture-specific execution behavior. The implementation should preserve unknown fields and make unresolved references visible in diagnostics.
