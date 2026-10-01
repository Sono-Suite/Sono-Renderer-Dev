# Sono Renderer

Please scroll down to find the AI disclosure section.

An engine agnostic Sonolus Watch-mode renderer in development. The repository provides package parsing, static compatibility analysis, direct LevelData loading, project input checks, FFmpeg resolution, and partial Watch VM and entity lifecycle execution. A diagnostic PPM rasterizer can visualize abstract sprite rectangles; Sonolus resource rendering, gameplay video rendering, and video export are not implemented. The runtime and lifecycle cover only a subset of Sonolus semantics.

## Build and inspect

```text
cargo run -- inspect-engine <engine-directory>
cargo run -- inspect-level <level-directory-or-level.data>
cargo run -- inspect-resources <collection.scp>
cargo run -- compatibility <engine-directory> <collection.scp> <level-directory-or-level.data>
cargo run -- validate-project <engine.zip> <resources.scp> <level.json.gz> <music-file> [--cover <image>] [--mv <video>]
cargo run -- ffmpeg-info
```

The normal project input model takes the engine ZIP, resource package,
`level.json.gz`, and music as separate inputs. Cover and custom MV are optional.
`validate-project` parses the engine, SCP index, and level data and verifies the
required and supplied optional files are accessible. It does not decode or
validate the media content. Audio decoding and gameplay rendering are not
implemented.

`ffmpeg-info` resolves and validates FFmpeg and FFprobe. The application prefers
`../addons/ffmpeg.exe` and `../addons/ffprobe.exe`, located relative to the
application root (the repository root for Cargo development builds). Valid
shared binaries are used in place. If either one is missing or invalid, the
application reuses a valid managed copy or downloads the pinned Windows FFmpeg
8.1 distribution, verifies its SHA-256, extracts it into
`dependencies/ffmpeg/8.1`, and validates both executables. This command prints
the selected source and executable paths. It does not use or modify system
PATH. Linux uses native executable names and can reuse an existing managed
installation, but automatic Linux download/extraction is not implemented yet.
Normal inspection commands do not provision FFmpeg.

Engine directories may use the server fixture names (`engine.json`, `EngineWatchData`, `EngineConfiguration`, `EngineRom`) or conventional lower-case equivalents. ZIP packages are read directly. Gzip is detected by its magic header. Standalone JSON or gzip-compressed LevelData is supported directly; legacy level directories and ZIP packages remain available to the inspector. SCP resource hashes are checked against repository entries.

See [M0 format and architecture notes](docs/M0-design.md) for fixture observations, schema references, module boundaries, and unresolved format questions.

legal jargan lol:

I do not claim to have written any of the code in this project, as I do not personally know the programming language Rust. This project is AI assisted "to the absolute extreme", and if you don't like that, it's fine. However, please make the distinction that I am not going to turn my YouTube content flow and actual creative works into AI slop machine. AI content slop is not something that I endorse in any way, shape or form. Please understand my intention with this project is not to claim that I wrote the code in this project, but to provide a tool that helps automate the chart production process much much quicker.

Acknowledgements:

Zihad - Sonolus Renderer Code provided as reference

LittleYang0531 - More rendering code provided as reference

qwewqa / Hyeon - Next Sekai Preview implementation for development reference

Burrito (Sonolus) - literally the entire reason this exists, W

Being mentioned here does not necessarily mean that they have endorsed this project.
Credit is given to acknowledge the people who made related works that contribute to its testing.

All developers whose code was provided to AI as a development reference were made explicitly aware and gave permission for my use case of their code. Again, this permission does not imply endorsement of Sono-Renderer.

This program was built and tested against Sonolus v1.1.4.