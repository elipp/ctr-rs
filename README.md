# ctr-rs

ctr-rs is an online-focused reimplementation of Crash Team Racing (PS1, 1999), written in Rust.

## What's here

- **Exact, reference-checked physics** - Kart physics/collision follow the original tick for tick. We check them against the C decompilation, [ctr-native](https://github.com/CTR-tools/ctr-native). The weapons, crates and the level's objects follow the original closely.
- **Truly native performance, framerates/aspect ratio** - The original look definitely has its own charm to it, but the reimplementation has no need for [PGXP](https://github.com/libretro/beetle-psx-libretro/tree/master/pgxp) or other (awesome) emulator hacks, since we have full-precision vertices and a real depth buffer.
- **Cross-platform** - The project runs and builds on any system that Rust builds on, and on the browser too!
- **Hassle-free online play** - No patched ROMs, emulator builds or launchers required.
- **Peer-to-peer networking** - Players are directly connected to each other over [QUIC](https://en.wikipedia.org/wiki/QUIC), using [iroh](https://github.com/n0-computer/iroh).
- **No network setup** - No port forwarding, DMZ or dedicated servers. Connections punch through NAT automatically, and fall back to a relay when they can't.
- **30 Hz simulation, hi-FPS rendering** - The game state advances at the original's 30 ticks/s, independent of the rendering frame rate.
- **Custom tracks** - Just drag and drop an `.xdelta` (from e.g. [CTR Custom Tracks](https://ctrcustomtracks.com)) to the client window, and immediately play them with your friends.

## What's not here (not currently planned)

- **Adventure mode**
- **Time trial**
- **Relic race**
- **Battle mode**

## Running it

You provide your own legally obtained disc image of Crash Team Racing. The client asks for it with a file dialog the first time; or point `CTR_BIN` at it, or put it at `assets/ctr-u.bin`.

```
cargo run -p client
```

A build from the [releases](https://github.com/elipp/ctr-rs/releases) asks for the disc image the first time it starts. The macOS build is not signed, so macOS refuses to open it at first: allow it in System Settings → Privacy & Security ("Open Anyway"), or run `xattr -dr com.apple.quarantine ctr-rs.app`. On Linux, the `.AppImage` runs as it is once it is executable (`chmod +x`).

The client has no command line options. `]` and `[` switch the level, `.` and `,` switch the racer, and `Enter` opens the in-game console for `/` commands and chat.

`ctr-query` (`crates/query`) looks up a level's data from the disc, and `ctr-map` (`crates/map`) imports and describes custom tracks.

## Running the tests

```
cargo test --workspace --no-fail-fast
```

Some tests need files that are not in the repo, and without them they skip and pass without checking anything. Their names say which:

- `needs_disc_…`: the disc image (`assets/ctr-u.bin`, or `CTR_BIN`), or the folder prepared from it (`assets/disc`, or `CTR_DISC_DIR`).
- `needs_reference_…`: [ctr-native](https://github.com/CTR-tools/ctr-native) at the commit in [`docs/ctr-native.rev`](docs/ctr-native.rev), checked out at `reference/ctr-native` (git-ignored). Most of them also build the C side with `gcc -m32` (x86-64 only), which on Debian and Ubuntu is the `gcc-multilib` package.
- `needs_disc_needs_reference_…`: both.

Filters match anywhere in the name, so `cargo test --workspace -- --skip needs_disc` skips every test that needs the disc (those that need both as well), and `--skip needs_reference` every test that needs ctr-native.

A new test that needs either must start with its prefix, `needs_disc_` before `needs_reference_` when it needs both, and no other test or module may have either word in its name. CI relies on the names: it skips the `needs_disc` tests, and runs the rest with ctr-native checked out. A test that reads the disc without the prefix skips there and passes without checking anything, and one that reads ctr-native without it still runs, but is not skipped by `--skip needs_reference` where there is no ctr-native.

```
git clone https://github.com/CTR-Tools/ctr-native reference/ctr-native
git -C reference/ctr-native checkout "$(cat docs/ctr-native.rev)"
```

Another commit of ctr-native may not match what the tests expect (they fail to link, or differ by a field), so bump `docs/ctr-native.rev` together with the tests that need it. Run `cargo test --workspace -- --nocapture 2>&1 | grep -i skipping` to see which tests skipped. CI has no disc, so it skips those tests (`--skip needs_disc`); it checks out ctr-native and sets `CTR_REQUIRE_REFERENCE`, which makes a test that finds no reference fail instead of skipping.

## Prior work

This project stands on the work of the CTR community, and absolutely could not exist without it:

- [**ctr-native**](https://github.com/CTR-tools/ctr-native), a native PC port of the game built on the decompilation. It is our first reference for how the game behaves, and the oracle our physics tests compare against.
- [**CTR-ModSDK**](https://github.com/CTR-tools/CTR-ModSDK), and the decompilation, symbols and headers that came out of it. We use them for the layouts of the game's data and for what ctr-native lacks.
- The [**CTR-Tools**](https://github.com/CTR-tools) community, whose years of reverse engineering of the game's formats and code made both of those possible.
- [**PsyCross**](https://github.com/OpenDriver2/PsyCross) and REDRIVER2, whose GTE (the PS1's geometry coprocessor) implementation is ported in `crates/gte`.

ctr-rs is a Rust port that works from these projects. It is not a fork of any of them.

## Notice

ctr-rs is an independent fan project, not affiliated with or endorsed by Sony Interactive Entertainment, Activision Publishing, Naughty Dog or any other rights holder. It ships no game content: no art, models, textures, sounds, music, levels or disc contents. You provide your own legally obtained copy of the game, and everything is read from it at runtime. Custom tracks are not shipped either: you import them yourself from patches you have obtained.

Crash Team Racing was developed by Naughty Dog and published by Sony Computer Entertainment. Crash Bandicoot and Crash Team Racing are trademarks of Activision Publishing, Inc. PlayStation is a trademark of Sony Interactive Entertainment Inc.

Our own code is licensed under the GNU General Public License, version 3, which is the license of ctr-native and CTR-ModSDK, the projects it is ported from. `crates/gte` is also derived from MIT-licensed code, and its notice is in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Also, LLM-based tooling was heavily used in the construction of this project.
