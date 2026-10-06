# Sono Renderer

Please scroll down to find the AI disclosure section.

This is a Sonolus renderer, with compatibility suited for Next Sekai, Next Rush, and Horizon charts.

This project works by simulating the Watch mode function on Sonolus. By simulating the Watch mode, this project is able to generate flawless recordings of MVs using any given engine zip file, SCP file, etc.

If you wish to get a .scp and engine zip, please reference Sono-Server, as Sono-Server has prepackaged .scp files and engine folders! Just zip an engine folder, and you're good :P

This project has a basic GUI, but also supports command line inputs:

```text
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --start-time 0 --duration 10 --fps 60 --width 1920 --height 1080
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --whole-chart
```

no documentation lol have fun good luck with cli :sob:

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
