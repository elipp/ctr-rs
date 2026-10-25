//! `ctr-map`: makes and looks at map files (`.ctrmap`).

use std::path::PathBuf;
use std::process::ExitCode;

use map::Map;

const USAGE: &str = "ctr-map: maps from community patches.

usage:
  ctr-map import PATCH.xdelta [-o OUT.ctrmap] [--level N] [--disc BIN]
      reads the patch against the USA disc image (CTR_BIN, or assets/ctr-u.bin; --disc picks
      one), finds the track the patch put its map in (--level N when it changes several
      different ones), and saves it (default: assets/maps/NAME.ctrmap)
  ctr-map info MAP.ctrmap
      what a map file holds";

fn disc() -> PathBuf {
    std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin"))
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("import") => {
            let mut patch = None;
            let (mut out, mut level, mut disc_path) = (None, None, disc());
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
                    "--disc" => disc_path = PathBuf::from(value("--disc")?),
                    _ if patch.is_none() => patch = Some(PathBuf::from(arg)),
                    _ => return Err(format!("what is {arg}?")),
                }
            }
            let patch = patch.ok_or("import needs a patch")?;
            let done = map::import::import(&disc_path, &patch, level)
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
                "{}: read from the slot of track {}; the patch changes {} entries of BIGFILE.BIG (tracks {:?})",
                done.map.name,
                done.from_track,
                done.changed.len(),
                done.tracks
            );
            println!("saved {}", out.display());
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
