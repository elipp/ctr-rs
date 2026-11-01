//! `ctr-map`: makes and looks at map files (`.ctrmap`).

use std::path::PathBuf;
use std::process::ExitCode;

use map::Map;

const USAGE: &str = "ctr-map: maps from community patches.

usage:
  ctr-map import PATCH.xdelta [-o OUT.ctrmap] [--level N] [--song-pack N] [--reverb N] [--disc BIN]
      reads the patch against the USA disc image (CTR_BIN, or assets/ctr-u.bin; --disc picks
      one), finds where the patch put its map (a track's slot, or the entries that CTR Editor
      uses; --level N when it changes several different tracks, --entries LEV,VRAM to name
      the BIGFILE entries by hand; --song-pack N takes the music from song pack N of KART.HWL when
      the patch's is not the track's slot's, see `audio`; when the patch changes several
      packs, it lists them and asks, unless the option is given; --reverb N, 0 to 4, sets the
      map's reverb mode; the patch's reverbs are asked for in the same way when it changes
      some, and the track's own is the default), and saves it (default: assets/maps/NAME.ctrmap)
  ctr-map audio PATCH.xdelta [--disc BIN]
      whether the patch changes the game's audio: the song packs of SOUNDS/KART.HWL, and the
      executable's tables of each level's song and reverb
  ctr-map export [-o DIR] [--disc BIN]
      makes a map of each racing track of the game that DIR (default: assets/disc/maps) lacks or
      has an old one of, named __builtin__TRACK.ctrmap; the client does this itself when it starts
  ctr-map info MAP.ctrmap
      what a map file holds";

fn disc() -> PathBuf {
    std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin"))
}

/// When the patch changes several song packs, which one is the track's: the patch does not say
/// (only its author knows), so the packs are listed and a number is asked for. Nothing is asked
/// when no one is there to answer, or when there is not a choice (one pack is taken); an empty answer takes the
/// pack of the track's own slot.
fn ask_song_pack(disc: &std::path::Path, patch: &std::path::Path) -> std::io::Result<Option<u8>> {
    use std::io::{BufRead, IsTerminal, Write};
    let changes = map::audio::changed_packs(disc, &std::fs::read(patch)?)?;
    match changes.as_slice() {
        [] => return Ok(None),
        [only] => {
            println!(
                "the patch changes the music of song pack {} only: using it",
                only.pack
            );
            return Ok(Some(only.pack));
        }
        _ if !std::io::stdin().is_terminal() => return Ok(None),
        _ => {}
    }
    println!(
        "the patch changes the music of {} song packs:",
        changes.len()
    );
    for c in &changes {
        let levels = match c.levels.as_slice() {
            [] => String::new(),
            l => format!(" (the game's level {l:?})"),
        };
        println!(
            "  {:>2}{levels}: {} of {} sequences differ, bpm {:?} -> {:?}, {} samples changed",
            c.pack, c.sequences.0, c.sequences.1, c.was, c.now, c.samples
        );
    }
    loop {
        print!("which is the map's music? a number, or nothing for its track's own pack: ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line)? == 0 {
            return Ok(None);
        }
        match line.trim().parse::<u8>() {
            _ if line.trim().is_empty() => return Ok(None),
            Ok(n) if changes.iter().any(|c| c.pack == n) => return Ok(Some(n)),
            _ => println!("one of the packs listed"),
        }
    }
}

/// When the patch gives levels other reverbs (`data.reverbMode`), which is the map's: one change
/// is taken, several are listed and a number is asked for, which can be any of the game's modes
/// (0 to 4); an empty answer leaves it to the track's own. Nothing is asked of no one.
fn ask_reverb(patch: &std::path::Path, disc: &std::path::Path) -> std::io::Result<Option<u8>> {
    use std::io::{BufRead, IsTerminal, Write};
    let changes = map::audio::changed_reverbs(disc, &std::fs::read(patch)?)?;
    match changes.as_slice() {
        [] => return Ok(None),
        [(level, was, now)] => {
            println!("the patch changes the reverb of level {level} from {was} to {now}: using it");
            return Ok(Some(*now));
        }
        _ if !std::io::stdin().is_terminal() => return Ok(None),
        _ => {}
    }
    println!("the patch changes the reverb of {} levels:", changes.len());
    for (level, was, now) in &changes {
        println!("  level {level:>2}: {was} -> {now}");
    }
    loop {
        print!("which reverb is the map's? a mode 0 to 4, or nothing for its track's own: ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line)? == 0 {
            return Ok(None);
        }
        match line.trim().parse::<u8>() {
            _ if line.trim().is_empty() => return Ok(None),
            Ok(n) if n < 5 => return Ok(Some(n)),
            _ => println!("a mode from 0 to 4"),
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("import") => {
            let mut patch = None;
            let (mut out, mut level, mut disc_path) = (None, None, disc());
            let (mut entries, mut song_pack, mut reverb) = (None, None, None);
            while let Some(arg) = args.next() {
                let mut value = |what: &str| args.next().ok_or(format!("{what} needs a value"));
                match arg.as_str() {
                    "-o" => out = Some(PathBuf::from(value("-o")?)),
                    "--level" => {
                        level = Some(
                            value("--level")?
                                .parse::<u32>()
                                .map_err(|e| format!("--level: {e}"))?,
                        )
                    }
                    "--entries" => {
                        let v = value("--entries")?;
                        let (lev, vram) = v
                            .split_once(',')
                            .ok_or("--entries takes LEV,VRAM (BIGFILE entry numbers)")?;
                        let num =
                            |s: &str| s.parse::<usize>().map_err(|e| format!("--entries: {e}"));
                        entries = Some((num(lev)?, num(vram)?));
                    }
                    "--song-pack" => {
                        song_pack = Some(
                            value("--song-pack")?
                                .parse::<u8>()
                                .map_err(|e| format!("--song-pack: {e}"))?,
                        )
                    }
                    "--reverb" => {
                        reverb = Some(
                            value("--reverb")?
                                .parse::<u8>()
                                .map_err(|e| format!("--reverb: {e}"))?,
                        )
                    }
                    "--disc" => disc_path = PathBuf::from(value("--disc")?),
                    _ if patch.is_none() => patch = Some(PathBuf::from(arg)),
                    _ => return Err(format!("what is {arg}?")),
                }
            }
            let patch = patch.ok_or("import needs a patch")?;
            if reverb.is_none() {
                reverb = ask_reverb(&patch, &disc_path)
                    .map_err(|e| format!("{}: {e}", patch.display()))?;
            }
            if song_pack.is_none() {
                song_pack = ask_song_pack(&disc_path, &patch)
                    .map_err(|e| format!("{}: {e}", patch.display()))?;
            }
            let done = map::import::import(&disc_path, &patch, level, entries, song_pack, reverb)
                .map_err(|e| format!("{}: {e}", patch.display()))?;
            let out = out.unwrap_or_else(|| {
                PathBuf::from("assets/maps").join(format!("{}.ctrmap", done.map.name))
            });
            if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            done.map
                .write(&out)
                .map_err(|e| format!("{}: {e}", out.display()))?;
            println!(
                "{}: read from {}; the patch changes {} entries of BIGFILE.BIG (tracks {:?})",
                done.map.name,
                done.source,
                done.changed.len(),
                done.tracks
            );
            println!("saved {}", out.display());
            Ok(())
        }
        Some("audio") => {
            let (mut patch, mut disc_path) = (None, disc());
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--disc" => {
                        disc_path = PathBuf::from(args.next().ok_or("--disc needs a value")?)
                    }
                    _ if patch.is_none() => patch = Some(PathBuf::from(arg)),
                    _ => return Err(format!("what is {arg}?")),
                }
            }
            let patch = patch.ok_or("audio needs a patch")?;
            let report = map::audio::audit(&disc_path, &patch)
                .map_err(|e| format!("{}: {e}", patch.display()))?;
            print!("{report}");
            Ok(())
        }
        Some("export") => {
            let (mut out, mut disc_path) = (PathBuf::from("assets/disc/maps"), disc());
            while let Some(arg) = args.next() {
                let mut value = |what: &str| args.next().ok_or(format!("{what} needs a value"));
                match arg.as_str() {
                    "-o" => out = PathBuf::from(value("-o")?),
                    "--disc" => disc_path = PathBuf::from(value("--disc")?),
                    _ => return Err(format!("what is {arg}?")),
                }
            }
            let made = map::builtin::export(&disc_path, &out)
                .map_err(|e| format!("{}: {e}", disc_path.display()))?;
            println!("made {made} maps in {}", out.display());
            Ok(())
        }
        Some("info") => {
            let path = PathBuf::from(args.next().ok_or("info needs a file")?);
            let map = Map::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let l = &map.level;
            println!("name: {}", map.name);
            println!("id: {:#x}", map.id().map_err(|e| e.to_string())?);
            println!(
                "level: {} quad blocks, {} vertices, {} instances, {} restart points",
                l.quad_blocks.len(),
                l.vertices.len(),
                l.instances.len(),
                l.restart_points.len()
            );
            let mut names = std::collections::BTreeMap::<&str, usize>::new();
            for i in &l.instances {
                *names.entry(i.model_name.as_str()).or_default() += 1;
            }
            println!("instances: {names:?}");
            println!(
                "data: {} bytes, {} pointers; {} vram images",
                map.dram.data.len(),
                map.dram.pointer_slots.len(),
                map.vram.len()
            );
            match &map.audio {
                Some(a) => println!(
                    "audio: over track {}; song pack {}, reverb {}, {} samples ({} bytes)",
                    a.level,
                    a.song_pack
                        .as_ref()
                        .map_or("the game's".into(), |p| format!(
                            "its own ({} bytes)",
                            p.len()
                        )),
                    a.reverb.map_or("the game's".into(), |r| r.to_string()),
                    a.samples.len(),
                    a.samples.iter().map(|(_, s)| s.len()).sum::<usize>()
                ),
                None => println!("audio: none"),
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
