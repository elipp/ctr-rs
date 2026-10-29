# ctr-rs

ctr-rs is and online-focused port of Crash Team Racing (PS1, 1999), written in Rust.

It is heavily based on the [ctr-native](https://github.com/CTR-tools/ctr-native) project; it is used to verify the physics simulation and other mechanics remains as faithful to the original as possible. As we see it, the original CTR's controls are truly S-tier, and basically unmatched to this day.

## What's here

- **Exact physics** - Kart against ground and wall, and kart against kart, follow the original tick for tick. `vehicle/tests/tick.rs` checks them against the C reference, `ctr-native`. The weapons, crates and the level's objects follow the original closely.
- **Truly native performance, framerates/aspect ratio** - While the original wobbly graphics definitely has its own charm to it, the reimplementation has need for [PGXP](https://github.com/libretro/beetle-psx-libretro/tree/master/pgxp) or other workarounds, since we have an actual depth buffer.
- **Cross-platform** - The project runs and builds on any system that Rust builds on.
- **No emulator hacks, ROM patches or launchers** - The current "CTR online" solutions have quite a bit of jank to get running.
- **The modding horizon is unlimited** - Modding ctr-native, while certainly possible, is difficult to get right, and limited in scope due to the recomp-style codebase and the C programming language.
- **30 Hz simulation, hi-FPS rendering** - The game state advances at the original's 30 ticks a second, independent of the frame rate that is drawn.
- **Rollback netcode with no local delay** - Players connect directly to each other with [iroh](https://github.com/n0-computer/iroh), if possible.
- **Custom tracks** - The `ctr-map` turns an xdelta patch of the disc into a `.ctrmap` file that the client can load through the lobby.

## What's not here (not currently planned)
- **Adventure mode**
- **Time trial**
- **Relic race**
- **Battle mode**

## Running it

You provide your own legally obtained disc image of Crash Team Racing. Point `CTR_BIN` at it, or put it at `assets/ctr-u.bin`.

```
cargo run -p client
```

The client has no command line options. `]` and `[` switch the level, `.` and `,` switch the racer, and `/` commands go through the in-game console.

`ctr-query` (`crates/query`) looks up a level's data from the disc, and `ctr-map` (`crates/map`) imports and describes maps.

## Prior work

This project stands on the work of the CTR community, and could not exist without it.

- [**ctr-native**](https://github.com/CTR-tools/ctr-native), a native PC port of the game built on the decompilation. It is our first reference for how the game behaves, and the oracle our physics tests compare against.
- [**CTR-ModSDK**](https://github.com/CTR-tools/CTR-ModSDK), and the decompilation, symbols and headers that came out of it. We use them for the layouts of the game's data and for what ctr-native lacks.
- The [**CTR-Tools**](https://github.com/CTR-tools) community, whose years of reverse engineering of the game's formats and code made both of those possible.
- [**PsyCross**](https://github.com/OpenDriver2/PsyCross) and REDRIVER2, whose GTE (the PS1's geometry coprocessor) implementation is ported in `crates/gte`.

ctr-rs is a Rust port that works from these projects. It is not a fork of any of them.

## Notice

ctr-rs is an independent fan project, not affiliated with or endorsed by Sony Interactive Entertainment, Activision Publishing, Naughty Dog or any other rights holder. It ships no game content: no art, models, textures, sounds, music, levels or disc contents. You provide your own legally obtained copy of the game, and everything is read from it at runtime. Custom maps are not shipped either: you import them yourself from patches you have obtained.

Crash Team Racing was developed by Naughty Dog and published by Sony Computer Entertainment. Crash Bandicoot and Crash Team Racing are trademarks of Activision Publishing, Inc. PlayStation is a trademark of Sony Interactive Entertainment Inc.

Our own code is licensed under the GNU General Public License, version 3, which is the license of ctr-native and CTR-ModSDK, the projects it is ported from. `crates/gte` is also derived from MIT-licensed code, and its notice is in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Also, LLM-based tooling was heavily used in the construction of this project.
