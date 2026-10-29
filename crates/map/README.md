# map

Community maps as files of their own (`.ctrmap`), and `ctr-map`, the tool that makes them.

Community tracks are shared as xdelta3 patches against the USA disc image. A patch puts the new
track in the place of one of the game's in `BIGFILE.BIG`, which is no use for a viewer that wants
several maps at once, or for players who all need the same one. `ctr-map import` reads the track
out of the patch once and saves it as a map file; nothing else needs the disc layout or the patch.

## Making a map

You need the USA disc image (`assets/ctr-u.bin`, or the file `CTR_BIN` names) and a patch made
against it (`.xdelta`, not compressed with `-S`):

```sh
cargo run -p map -- import assets/deltas/MarioCircuit1.xdelta
```

This reads the patch through [`xdelta`](../xdelta) without writing the patched disc, finds the
level file it changed in `BIGFILE.BIG`, parses the track as the viewer does, and saves it as
`assets/maps/MarioCircuit1.ctrmap` (about 0.6 to 0.7 MB, compressed with zstd). It takes a few
seconds.

| option | |
|---|---|
| `-o FILE` | where to save it (default `assets/maps/NAME.ctrmap`, the name being the patch's) |
| `--disc BIN` | the disc image (default `CTR_BIN`, or `assets/ctr-u.bin`) |
| `--level N` | which track's slot to read the map from |
| `--entries LEV,VRAM` | the `BIGFILE.BIG` entries that hold the level and its textures, when they are not in a track's slot |

A patch puts its track in the slot of one of the game's tracks, or in several with the same file;
that slot is only where the file was, and the map takes the place of nothing. `--level` is for a
patch that changes the level files of several tracks to different things: the import stops and
lists them with their sizes.

Tools differ in where they put the track. The first patches this was written for put it in the
slot of one of the game's tracks. CTR Editor's patches (the header says `CTREditor` and
`TrackROM`) leave the tracks alone and put it in the entries 221 and 222, which the game uses for
code overlays, and change the overlays themselves to load it. When a patch changes no track's level
file, `import` looks among the entries it changed for one that parses as a level and one after it
that parses as textures, and says which it used. `--entries` names them by hand if it picks wrong.
The overlay code the patch changes is not read.

To see what a map holds:

```sh
cargo run -p map -- info assets/maps/MarioCircuit1.ctrmap
```

It prints the name, the id, the number of quad blocks, vertices, instances and restart points, the
models used by the instances, and the size of the data.

## Using a map

The viewer reads every `*.ctrmap` in its `maps` folder when it starts (`assets/maps` in a debug
build; in a release build `%APPDATA%\ctr-rs\maps` on Windows, `~/.config/ctr-rs/maps` on Linux)
and lists the maps after the game's tracks: `[` and `]` step through the tracks (from the first track, `[` goes to the last map). The
window title shows the map's name. A file that cannot be read is reported on stderr and left out.

A map has the id `Map::id()`, made from its contents, with the top bit set (`is_map_id`). The game's
tracks are small numbers, so the id can stand wherever a track number does, for instance in the
lobby's start message. Two players with the same map have the same id.

## The file

```
"CTRMAP\0\0"  version (u32, little endian)  zstd( bitcode( Map ) )
```

`Map` is the name (the patch's file name, as a patch has no title for it), the parsed `Level`, the
`DramFile` that holds the rest of the level's data (the models and textures that the instances and
the scenery use), and the level's own VRAM images. The texture pages that every track shares
come from the disc, as for the game's tracks.

The version changes with the structs that are saved in it (they derive `serde`); a file of another
version is refused, and has to be imported again from its patch.

In code:

```rust
let map = map::Map::read(path)?;      // or Map::from_bytes(&bytes), e.g. from the network
let id = map.id()?;                   // the track number
let bytes = map.to_bytes()?;          // the file, to save or to send
let hash = map.hash()?;               // 64 bits, to check that a received map is the one asked for
```

## Not done

- Sending a map to a player who joins a race on one they do not have (see `TODO.md`).
- Only the one-player level file and its VRAM are read; the 2 and 4 player and relic files that a
  patch changes are ignored.
- A map has no title of its own.
