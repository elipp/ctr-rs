//! `ctr-query`: looks things up in the data of the disc image, to answer "what does the game
//! actually say about this?" without a throwaway test. Run `ctr-query help`.
//!
//! The disc is `$CTR_BIN`, or `assets/ctr-u.bin` of the repository. Coordinates are level units
//! (the ones the viewer's probe overlay prints), not world units.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::disc::open_bigfile;
use loader::dram::DramFile;
use loader::lev::{InstDef, Level, QuadBlock, SVec3, quad_flags};
use loader::level::LevelId;
use loader::texture::{FaceMode, face_polygon, resolve_texture};

const HELP: &str = "\
ctr-query: look things up in the disc's data.

usage: ctr-query <command> LEVEL [arguments]

LEVEL is a number (0 is Dingo Canyon, 3 Crash Cove) or a name (`crash-cove`).
Coordinates are level units, as the viewer's probe prints them.

commands:
  level LEVEL                         what the level has: counts, spawns, instance kinds
  quad LEVEL INDEX                    everything about one quad block: flags, vertices, faces
  quad LEVEL --id BLOCK_ID            the same, picked by block id
  near LEVEL X Y Z [--within R]       the quad blocks over (X, Z), nearest to Y first;
                                      R widens the search (default 0)
  find LEVEL [--flag NAME]... [--terrain N] [--pads] [--limit N]
                                      the quad blocks with all the flags (a name as `quad`
                                      prints it, e.g. trigger), terrain type, or that are
                                      turbo pads
  inst LEVEL [--name TEXT] [--at X Y Z --within R]
                                      the placed instances (crates, fruit, hazards)
  pads LEVEL                          each turbo pad's collision block, and the visible blocks
                                      with the same footprint (the ones that show the pad)

environment: CTR_BIN is the disc image.
";

const FLAG_NAMES: [(u16, &str); 13] = [
    (quad_flags::REFLECT_SPLIT_LINE_1, "reflect-1"),
    (quad_flags::LOW_GRAVITY, "low-gravity"),
    (quad_flags::REFLECT_SPLIT_LINE_0, "reflect-0"),
    (quad_flags::NO_COLLISION_RESPONSE, "no-collision-response"),
    (quad_flags::TRIGGER, "trigger"),
    (quad_flags::ENGINE_ECHO, "engine-echo"),
    (quad_flags::KILL_PLANE, "kill-plane"),
    (quad_flags::DOOR, "door"),
    (quad_flags::CAMERA_SEARCH, "camera-search"),
    (quad_flags::GROUND, "ground"),
    (quad_flags::COLLISION_SURFACE, "collision-surface"),
    (quad_flags::NO_CAMERA_RESPAWN_PROBE, "no-respawn-probe"),
    (quad_flags::SKIP_WATER_LIST, "skip-water-list"),
];

fn flag_names(flags: u16) -> Vec<&'static str> {
    FLAG_NAMES
        .iter()
        .filter(|(f, _)| flags & f != 0)
        .map(|(_, n)| *n)
        .collect()
}

/// The arguments after the command, taken out as they are understood.
struct Args(Vec<String>);

impl Args {
    fn positional(&mut self, what: &str) -> Result<String, String> {
        match self.0.iter().position(|a| !a.starts_with("--")) {
            Some(at) => Ok(self.0.remove(at)),
            None => Err(format!("missing {what}")),
        }
    }

    fn number(&mut self, what: &str) -> Result<i32, String> {
        let text = self.positional(what)?;
        text.parse()
            .map_err(|_| format!("{what} is not a number: `{text}`"))
    }

    fn switch(&mut self, name: &str) -> bool {
        let found = self.0.iter().position(|a| a == name);
        found.map(|at| self.0.remove(at)).is_some()
    }

    /// `name VALUE` (any number of times).
    fn options(&mut self, name: &str) -> Result<Vec<String>, String> {
        let mut values = Vec::new();
        while let Some(at) = self.0.iter().position(|a| a == name) {
            self.0.remove(at);
            if at >= self.0.len() {
                return Err(format!("{name} needs a value"));
            }
            values.push(self.0.remove(at));
        }
        Ok(values)
    }

    fn option_number(&mut self, name: &str) -> Result<Option<i32>, String> {
        let mut values = self.options(name)?;
        values
            .pop()
            .map(|v| {
                v.parse()
                    .map_err(|_| format!("{name} is not a number: `{v}`"))
            })
            .transpose()
    }

    /// `name X Y Z`
    fn point(&mut self, name: &str) -> Result<Option<[i32; 3]>, String> {
        let Some(at) = self.0.iter().position(|a| a == name) else {
            return Ok(None);
        };
        if at + 3 >= self.0.len() {
            return Err(format!("{name} needs three numbers"));
        }
        let values: Vec<String> = self.0.drain(at..=at + 3).skip(1).collect();
        let n = |i: usize| {
            values[i]
                .parse::<i32>()
                .map_err(|_| format!("{name}: not a number: `{}`", values[i]))
        };
        Ok(Some([n(0)?, n(1)?, n(2)?]))
    }

    fn finish(self) -> Result<(), String> {
        match self.0.first() {
            None => Ok(()),
            Some(extra) => Err(format!("unexpected `{extra}` (see `ctr-query help`)")),
        }
    }
}

fn level_id(text: &str) -> Result<LevelId, String> {
    if let Ok(n) = text.parse() {
        return Ok(LevelId(n));
    }
    let normal = |s: &str| s.to_lowercase().replace(['-', ' ', '.'], "_");
    let names = [
        ("dingo_canyon", LevelId::DINGO_CANYON),
        ("dragon_mines", LevelId::DRAGON_MINES),
        ("blizzard_bluff", LevelId::BLIZZARD_BLUFF),
        ("crash_cove", LevelId::CRASH_COVE),
        ("tiger_temple", LevelId::TIGER_TEMPLE),
        ("papu_pyramid", LevelId::PAPU_PYRAMID),
        ("roo_tubes", LevelId::ROO_TUBES),
        ("hot_air_skyway", LevelId::HOT_AIR_SKYWAY),
        ("sewer_speedway", LevelId::SEWER_SPEEDWAY),
        ("mystery_caves", LevelId::MYSTERY_CAVES),
        ("cortex_castle", LevelId::CORTEX_CASTLE),
        ("n_gin_labs", LevelId::N_GIN_LABS),
        ("polar_pass", LevelId::POLAR_PASS),
        ("oxide_station", LevelId::OXIDE_STATION),
        ("coco_park", LevelId::COCO_PARK),
        ("tiny_arena", LevelId::TINY_ARENA),
        ("slide_coliseum", LevelId::SLIDE_COLISEUM),
        ("turbo_track", LevelId::TURBO_TRACK),
    ];
    let wanted = normal(text);
    names
        .iter()
        .find(|(n, _)| *n == wanted)
        .map(|(_, id)| *id)
        .ok_or_else(|| format!("no level `{text}`: a number or a name like crash-cove"))
}

struct Loaded {
    id: LevelId,
    dram: DramFile,
    level: Level,
}

fn load(id: LevelId) -> Result<Loaded, String> {
    let path = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin")
        });
    let (mut disc, big) = open_bigfile(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let index = BigIndex::level(id, Lod::OnePlayer, LevFile::Lev);
    let file = big
        .read(&mut disc, index.get())
        .map_err(|e| format!("reading level {}: {e}", id.0))?;
    let dram = DramFile::parse(&file).map_err(|e| format!("level {}: {e}", id.0))?;
    let level = Level::parse(&dram).map_err(|e| format!("level {}: {e}", id.0))?;
    Ok(Loaded { id, dram, level })
}

fn p(v: SVec3) -> String {
    format!("({}, {}, {})", v.x, v.y, v.z)
}

/// One line about a quad block.
fn quad_line(index: usize, q: &QuadBlock) -> String {
    let mut tags = Vec::new();
    if render::mesh::is_turbo_pad(q) {
        tags.push("TURBO-PAD");
    }
    if render::mesh::is_collision_wall(q) {
        tags.push("hidden");
    }
    format!(
        "#{index} id {} bbox {}..{} terrain {} flags [{}] {}",
        q.block_id,
        p(q.bbox.min),
        p(q.bbox.max),
        q.terrain_type,
        flag_names(q.flags).join(" "),
        tags.join(" ")
    )
}

fn level_summary(l: &Loaded) {
    let level = &l.level;
    println!(
        "level {}: {} quad blocks, {} vertices, {} instances",
        l.id.0,
        level.quad_blocks.len(),
        level.vertices.len(),
        level.instances.len()
    );
    println!(
        "config flags {:#x}, jump vertical speed cap {}",
        level.config_flags, level.jump_vertical_speed_cap
    );
    println!(
        "{} restart points, {} BSP nodes",
        level.restart_points.len(),
        level.bsp.len()
    );
    let pads = level
        .quad_blocks
        .iter()
        .filter(|q| render::mesh::is_turbo_pad(q))
        .count();
    let hidden = level
        .quad_blocks
        .iter()
        .filter(|q| render::mesh::is_collision_wall(q))
        .count();
    println!("{pads} turbo pad blocks, {hidden} hidden collision blocks");
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for i in &level.instances {
        *kinds.entry(i.model_name.as_str()).or_default() += 1;
    }
    println!("instances by model: {kinds:?}");
    for (n, s) in level.driver_spawns.iter().enumerate() {
        println!("spawn {n}: pos {} rot {}", p(s.pos), p(s.rot));
    }
    let _ = &l.dram;
}

fn quad_detail(l: &Loaded, index: usize) -> Result<(), String> {
    let level = &l.level;
    let q = level.quad_blocks.get(index).ok_or_else(|| {
        format!(
            "level {} has {} quad blocks",
            l.id.0,
            level.quad_blocks.len()
        )
    })?;
    println!("{}", quad_line(index, q));
    println!(
        "flags {:#06x}, checkpoint {}, draw_order_low {:#010x} high {:#010x} (face orders {:?}, low {:?})",
        q.flags,
        q.checkpoint_index,
        q.draw_order_low,
        q.draw_order_high,
        q.draw_order_high.to_le_bytes().map(|b| b as i8),
        q.draw_order_low.to_le_bytes()[0] as i8
    );
    println!(
        "texture_mid {:x?}, texture_low {:#x}, pvs {:#x}",
        q.texture_mid, q.texture_low, q.pvs
    );
    println!("the 3x3 patch (slot: position, colour_hi):");
    for (slot, &v) in q.vertex_index.iter().enumerate() {
        let v = &level.vertices[v as usize];
        println!("  {slot}: {} {:?}", p(v.pos), v.color_hi);
    }
    let is_quad = q.vertex_index[2] != q.vertex_index[3];
    for face in 0..if is_quad { 4 } else { 3 } {
        let ptr = q.texture_mid[face];
        let group = resolve_texture(&l.dram, ptr).ok().flatten();
        let mode = if group.is_some() {
            FaceMode::of(q, face)
        } else {
            FaceMode {
                rotation: 0,
                mode: 0,
            }
        };
        let slots = face_polygon(face, mode).ok();
        let red = slots.is_some_and(|s| {
            s.iter()
                .any(|&s| level.vertices[q.vertex_index[s] as usize].color_hi[..3] == [255, 0, 0])
        });
        let texture = match group {
            Some(g) => format!(
                "tpage {:#06x} clut {:#06x} uv {:?}",
                g.best().tpage,
                g.best().clut,
                g.best().uv
            ),
            None => "no texture".to_string(),
        };
        let fate = if red {
            "SKIPPED by the red-marker rule"
        } else if slots.is_none() {
            "no polygon"
        } else {
            "drawn"
        };
        println!("face {face}: ptr {ptr:#x} {texture}, mode {mode:?}, slots {slots:?}: {fate}");
    }
    Ok(())
}

fn near(l: &Loaded, at: [i32; 3], within: i32) {
    let mut hits: Vec<(i32, usize)> = Vec::new();
    for (i, q) in l.level.quad_blocks.iter().enumerate() {
        let (lo, hi) = (q.bbox.min, q.bbox.max);
        let inside = |v: i32, lo: i16, hi: i16| lo as i32 - within <= v && v <= hi as i32 + within;
        if inside(at[0], lo.x, hi.x) && inside(at[2], lo.z, hi.z) {
            // How far Y is from the block's own range.
            let gap = (lo.y as i32 - at[1]).max(at[1] - hi.y as i32).max(0);
            hits.push((gap, i));
        }
    }
    hits.sort();
    println!(
        "{} quad blocks over x {} z {} (within {within}), by distance in y:",
        hits.len(),
        at[0],
        at[2]
    );
    for (gap, i) in hits {
        println!("  y-off {gap:5}  {}", quad_line(i, &l.level.quad_blocks[i]));
    }
}

fn find(l: &Loaded, flags: u16, terrain: Option<i32>, pads: bool, limit: usize) {
    let hits: Vec<usize> = l
        .level
        .quad_blocks
        .iter()
        .enumerate()
        .filter(|(_, q)| q.flags & flags == flags)
        .filter(|(_, q)| terrain.is_none_or(|t| q.terrain_type as i32 == t))
        .filter(|(_, q)| !pads || render::mesh::is_turbo_pad(q))
        .map(|(i, _)| i)
        .collect();
    println!(
        "{} quad blocks match{}",
        hits.len(),
        if hits.len() > limit {
            format!(" (first {limit})")
        } else {
            String::new()
        }
    );
    for &i in hits.iter().take(limit) {
        println!("  {}", quad_line(i, &l.level.quad_blocks[i]));
    }
}

/// The corners of a block's footprint, which is how a turbo pad's collision block and the block
/// that shows it are matched.
fn footprint(q: &QuadBlock) -> [i16; 4] {
    [q.bbox.min.x, q.bbox.max.x, q.bbox.min.z, q.bbox.max.z]
}

fn pads(l: &Loaded) {
    let blocks = &l.level.quad_blocks;
    for (i, pad) in blocks
        .iter()
        .enumerate()
        .filter(|(_, q)| render::mesh::is_turbo_pad(q))
    {
        println!("{}", quad_line(i, pad));
        let same: Vec<usize> = blocks
            .iter()
            .enumerate()
            .filter(|(j, q)| {
                *j != i && !render::mesh::is_collision_wall(q) && footprint(q) == footprint(pad)
            })
            .map(|(j, _)| j)
            .collect();
        if same.is_empty() {
            println!("    no visible block has this footprint");
        }
        for j in same {
            println!("    shown by {}", quad_line(j, &blocks[j]));
        }
    }
}

fn inst_line(i: usize, d: &InstDef) -> String {
    format!(
        "#{i} {:?} model {:?} id {} pos {} rot {} scale {} flags {:#x}",
        d.name,
        d.model_name,
        d.model_id,
        p(d.pos),
        p(d.rot),
        p(d.scale),
        d.flags
    )
}

fn instances(l: &Loaded, name: Option<&str>, at: Option<[i32; 3]>, within: i32) {
    let hits: Vec<usize> = l
        .level
        .instances
        .iter()
        .enumerate()
        .filter(|(_, d)| name.is_none_or(|n| d.name.contains(n) || d.model_name.contains(n)))
        .filter(|(_, d)| {
            at.is_none_or(|a| {
                let d2 = |x: i16, v: i32| (x as i64 - v as i64).pow(2);
                d2(d.pos.x, a[0]) + d2(d.pos.y, a[1]) + d2(d.pos.z, a[2]) <= (within as i64).pow(2)
            })
        })
        .map(|(i, _)| i)
        .collect();
    println!("{} instances", hits.len());
    for i in hits {
        println!("  {}", inst_line(i, &l.level.instances[i]));
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        print!("{HELP}");
        return Ok(());
    }
    let command = args.remove(0);
    let mut args = Args(args);
    let level = level_id(&args.positional("LEVEL")?)?;
    let l = load(level)?;
    match command.as_str() {
        "level" => {
            args.finish()?;
            level_summary(&l);
        }
        "quad" => {
            let by_id = args.option_number("--id")?;
            let index = match by_id {
                Some(id) => l
                    .level
                    .quad_blocks
                    .iter()
                    .position(|q| q.block_id as i32 == id)
                    .ok_or_else(|| format!("no quad block has id {id}"))?,
                None => args.number("INDEX")? as usize,
            };
            args.finish()?;
            quad_detail(&l, index)?;
        }
        "near" => {
            let within = args.option_number("--within")?.unwrap_or(0);
            let at = [args.number("X")?, args.number("Y")?, args.number("Z")?];
            args.finish()?;
            near(&l, at, within);
        }
        "find" => {
            let mut flags = 0;
            for name in args.options("--flag")? {
                let (bit, _) = FLAG_NAMES.iter().find(|(_, n)| *n == name).ok_or_else(|| {
                    format!(
                        "no flag `{name}`: one of {}",
                        FLAG_NAMES
                            .iter()
                            .map(|(_, n)| *n)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
                flags |= bit;
            }
            let terrain = args.option_number("--terrain")?;
            let limit = args.option_number("--limit")?.unwrap_or(50) as usize;
            let pads = args.switch("--pads");
            args.finish()?;
            find(&l, flags, terrain, pads, limit);
        }
        "pads" => {
            args.finish()?;
            pads(&l);
        }
        "inst" => {
            let name = args.options("--name")?.pop();
            let within = args.option_number("--within")?.unwrap_or(0);
            let at = args.point("--at")?;
            args.finish()?;
            instances(&l, name.as_deref(), at, within);
        }
        other => return Err(format!("unknown command `{other}` (see `ctr-query help`)")),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("ctr-query: {message}");
            ExitCode::from(2)
        }
    }
}
