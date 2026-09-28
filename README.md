# Sono Renderer

An engine agnostic Sonolus Watch-mode inspector. Current scope is M0: package parsing and static compatibility analysis. There is no gameplay execution or rendering yet.

## Build and inspect

```text
cargo run -- inspect-engine <engine-directory>
cargo run -- inspect-level <level-directory-or-level.data>
cargo run -- inspect-resources <collection.scp>
cargo run -- compatibility <engine-directory> <collection.scp> <level-directory-or-level.data>
```

Engine directories may use the server fixture names (`engine.json`, `EngineWatchData`, `EngineConfiguration`, `EngineRom`) or conventional lower-case equivalents. ZIP packages are read directly. Gzip is detected by its magic header. Level directories accept `level.data`, `data`, or `LevelData`; ZIP level packages are also supported. SCP resource hashes are checked against repository entries.

See [M0 format and architecture notes](docs/M0-design.md) for fixture observations, schema references, module boundaries, and unresolved format questions.
