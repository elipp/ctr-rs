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
use loader::lev::{BspKind, InstDef, Level, QuadBlock, SVec3, quad_flags};
use loader::level::LevelId;
use loader::texture::{FaceMode, face_polygon, resolve_texture};

const HELP: &str = "\
ctr-query: look things up in the disc's data.

usage: ctr-query <command> LEVEL [arguments]

LEVEL is a number (0 is Dingo Canyon, 3 Crash Cove) or a name (`crash-cove`).
Coordinates are level units, as the viewer's probe prints them.

commands:
  level LEVEL                         what the level has: counts, spawns, instance kinds
  sky LEVEL                           the clear colors and the sky dome's vertex colors by height
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
  crates [LEVEL] [--half N] [--height N] [--min N]
                                      the crates (all levels without LEVEL) that are inside the
                                      drawn ground: a surface cuts the crate between its base and
                                      HEIGHT above (default 96), anywhere in a square of half side
                                      HALF (default 40) around it, by at least MIN (default 4) at its center
  pads LEVEL                          each turbo pad's collision block, and the visible blocks
                                      with the same footprint (the ones that show the pad)
  icons [FIRST [LAST]]                which BIGFILE entries carry icons with global indices
                                      FIRST..=LAST (default 32..=55, the driver portraits),
                                      with each icon's name and texture layout
  models [ENTRY]                      the models of a pack (default the racer pack): id, name,
                                      and per LOD the faces, animations and frames
  portraits | weapons                 decodes the HUD pictures the way the viewer does and
                                      counts the texels of each

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

fn sky_summary(level: &Level) {
    for (name, c) in ["top", "bottom", "gradient"].iter().zip(level.clear_colors) {
        match c {
            Some(c) => println!("clear color {name}: {c:?}"),
            None => println!("clear color {name}: off"),
        }
    }
    let Some(sky) = &level.skybox else {
        println!("no skybox");
        return;
    };
    // The rim of the dome's holes: edges that belong to a single triangle.
    let mut edges: BTreeMap<(u16, u16), u32> = BTreeMap::new();
    for t in sky.segments.iter().flatten() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut rim: Vec<u16> = edges
        .iter()
        .filter(|(_, n)| **n == 1)
        .flat_map(|(e, _)| [e.0, e.1])
        .collect();
    rim.sort_by_key(|&i| std::cmp::Reverse(sky.vertices[i as usize].pos.y));
    rim.dedup();
    println!("{} rim vertices, highest first (pos, color):", rim.len());
    rim.sort_by_key(|&i| (std::cmp::Reverse(sky.vertices[i as usize].pos.y), i));
    rim.dedup();
    for &i in &rim {
        let v = sky.vertices[i as usize];
        println!("  {} {:?}", p(v.pos), v.color);
    }
    let ys: Vec<i32> = sky.vertices.iter().map(|v| v.pos.y as i32).collect();
    let rs = sky
        .vertices
        .iter()
        .map(|v| ((v.pos.x as f32).hypot(v.pos.z as f32)) as i32);
    println!(
        "y {}..{}, horizontal radius up to {}",
        ys.iter().min().unwrap(),
        ys.iter().max().unwrap(),
        rs.max().unwrap()
    );
    let mut vertices = sky.vertices.clone();
    vertices.sort_by_key(|v| std::cmp::Reverse(v.pos.y));
    println!("{} sky vertices, highest first (y, color):", vertices.len());
    let mut last = None;
    for v in &vertices {
        if last != Some((v.pos.y, v.color)) {
            println!("  {} {:?}", v.pos.y, v.color);
            last = Some((v.pos.y, v.color));
        }
    }
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
    let rain = &level.rain_buffer;
    let weathered = level
        .quad_blocks
        .iter()
        .filter(|q| q.weather_intensity != 0)
        .count();
    println!(
        "weather: {rain:x?}; {weathered} quad blocks have intensity (max {})",
        level
            .quad_blocks
            .iter()
            .map(|q| q.weather_intensity)
            .max()
            .unwrap_or(0)
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
    println!(
        "paths: {} position paths of {:?} points, {} recorded paths of {:?} points",
        level.position_paths.len(),
        level
            .position_paths
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        level.paths.len(),
        level.paths.iter().map(Vec::len).collect::<Vec<_>>(),
    );
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

/// The drawn triangles of the level in the xz plane, as three corners each.
fn drawn_triangles(l: &Loaded) -> Vec<[[i32; 3]; 3]> {
    let level = &l.level;
    let mut out = Vec::new();
    for q in &level.quad_blocks {
        if render::mesh::is_collision_wall(q) {
            continue;
        }
        let is_quad = q.vertex_index[2] != q.vertex_index[3];
        for face in 0..if is_quad { 4 } else { 3 } {
            if q.texture_mid[face] == 0 {
                continue;
            }
            let mode = FaceMode::of(q, face);
            let Some(slots) = face_polygon(face, mode).ok() else {
                continue;
            };
            let at = |s: usize| {
                let v = level.vertices[q.vertex_index[s] as usize].pos;
                [v.x as i32, v.y as i32, v.z as i32]
            };
            let red = slots
                .iter()
                .any(|&s| level.vertices[q.vertex_index[s] as usize].color_hi[..3] == [255, 0, 0]);
            if red {
                continue;
            }
            out.push([at(slots[0]), at(slots[1]), at(slots[2])]);
            out.push([at(slots[1]), at(slots[2]), at(slots[3])]);
        }
    }
    out
}

/// The height of the triangle over (x, z), if it is over it.
fn height_at(t: &[[i32; 3]; 3], x: i32, z: i32) -> Option<f64> {
    let [a, b, c] = t.map(|v| [v[0] as f64, v[1] as f64, v[2] as f64]);
    let (x, z) = (x as f64, z as f64);
    let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if d.abs() < 1e-9 {
        return None;
    }
    let u = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
    let v = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
    let w = 1.0 - u - v;
    let e = -1e-6;
    (u >= e && v >= e && w >= e).then(|| u * a[1] + v * b[1] + w * c[1])
}

fn sunk_crates(l: &Loaded, half: i32, height: i32, min: i32) {
    let triangles = drawn_triangles(l);
    let mut total = 0;
    let mut sunk = Vec::new();
    for (i, d) in l.level.instances.iter().enumerate() {
        if !d.model_name.starts_with("crate") {
            continue;
        }
        total += 1;
        let (x, y, z) = (d.pos.x as i32, d.pos.y as i32, d.pos.z as i32);
        // How far the highest surface in the crate's span over each sample is above its base,
        // and how far the ground below the center is under it.
        let (mut worst, mut center, mut below) = (i32::MIN, None, None::<i32>);
        for (dx, dz) in [
            (0, 0),
            (-half, -half),
            (half, -half),
            (-half, half),
            (half, half),
            (0, -half),
            (0, half),
            (-half, 0),
            (half, 0),
        ] {
            let mut top = None::<i32>;
            for t in &triangles {
                if let Some(h) = height_at(t, x + dx, z + dz) {
                    let up = h.round() as i32 - y;
                    if up > 0 && up < height {
                        top = Some(top.map_or(up, |t: i32| t.max(up)));
                    }
                    if (dx, dz) == (0, 0) && up <= 0 {
                        below = Some(below.map_or(-up, |b: i32| b.min(-up)));
                    }
                }
            }
            if let Some(up) = top {
                worst = worst.max(up);
                if (dx, dz) == (0, 0) {
                    center = Some(up);
                }
            }
        }
        if center.is_some_and(|c| c >= min) {
            sunk.push((i, d, worst, center, below));
        }
    }
    println!(
        "level {}: {} crates, {} inside the ground",
        l.id.0,
        total,
        sunk.len()
    );
    for (i, d, worst, center, below) in sunk {
        println!(
            "  #{i} {:?} pos {} cut by up to {worst} (at the center {}; ground below the center {})",
            d.model_name,
            p(d.pos),
            center.map_or("none".into(), |c| c.to_string()),
            below.map_or("none".into(), |b| format!("{b} down")),
        );
    }
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
        for line in hitbox_lines(&l.level, i) {
            println!("      {line}");
        }
    }
}

/// `BSP_HITBOX_CLASS_TOUCH`: a hitbox of this class is only recorded, with no collision response.
const CLASS_TOUCH: u8 = 4;

/// The hitboxes of an instance: one line each, with the number of BSP leaves that list it.
fn hitbox_lines(level: &Level, instance: usize) -> Vec<String> {
    let mut found: Vec<(&loader::lev::BspHitbox, usize)> = Vec::new();
    for node in &level.bsp {
        let BspKind::Leaf { hitboxes, .. } = &node.kind else {
            continue;
        };
        for h in hitboxes.iter().filter(|h| h.instance == Some(instance)) {
            match found.iter_mut().find(|(f, _)| f.offset == h.offset) {
                Some((_, leaves)) => *leaves += 1,
                None => found.push((h, 1)),
            }
        }
    }
    found
        .into_iter()
        .map(|(h, leaves)| {
            format!(
                "hitbox flag {:#04x} class {}{} id {} center {} radius {} in {leaves} leaves",
                h.flag & 0xff,
                h.class(),
                if h.class() == CLASS_TOUCH {
                    " (touch: recorded, no bump)"
                } else {
                    ""
                },
                h.id,
                p(h.center),
                h.radius
            )
        })
        .collect()
}

fn icon_scan(range: std::ops::RangeInclusive<i32>) -> Result<(), String> {
    let path = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin")
        });
    let (mut disc, big) = open_bigfile(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    for index in 0..big.entries.len() {
        let Ok(file) = big.read(&mut disc, index) else {
            continue;
        };
        let Ok(dram) = DramFile::parse(&file) else {
            continue;
        };
        let Ok(icons) = loader::icons::parse_icons(&dram) else {
            continue;
        };
        for i in icons.iter().filter(|i| range.contains(&i.global_index)) {
            println!(
                "entry {index}: {} {:?} {:?}",
                i.global_index, i.name, i.layout
            );
        }
    }
    Ok(())
}

/// Decodes the HUD pictures the way the viewer does, to see that every cell has something in it.
fn hud_pictures(which: &str) -> Result<(), String> {
    use render::{portraits, weapons};
    let path = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin")
        });
    let (mut disc, big) = open_bigfile(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut vram = loader::vram::Vram::default();
    let shared = big
        .read(&mut disc, BigIndex::SHARED_MPK_VRM.get())
        .map_err(|e| e.to_string())?;
    for img in loader::vram::parse_vram_file(&shared).map_err(|e| e.to_string())? {
        vram.upload(&img).map_err(|e| e.to_string())?;
    }
    let pack = big
        .read(&mut disc, BigIndex::ARCADE_1P_PACK.get())
        .map_err(|e| e.to_string())?;
    let icons = loader::icons::parse_icons(&DramFile::parse(&pack).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let (atlas, ids, width, height) = match which {
        "portraits" => (
            portraits::portrait_atlas(&icons, &vram),
            &portraits::ICON_IDS[..],
            portraits::WIDTH,
            portraits::HEIGHT,
        ),
        _ => (
            weapons::weapon_atlas(&icons, &vram),
            &weapons::ICON_IDS[..],
            weapons::WIDTH,
            weapons::HEIGHT,
        ),
    };
    if which == "weapons" {
        let shine = weapons::shine_image(&icons, &vram);
        let visible = shine.rgba.chunks(4).filter(|p| p[3] != 0).count();
        println!(
            "shine: {visible}/{} texels, brightest alpha {}",
            shine.width * shine.height,
            shine.rgba.chunks(4).map(|p| p[3]).max().unwrap_or(0)
        );
    }
    for (n, id) in ids.iter().enumerate() {
        let cell = &atlas.rgba[n * width * height * 4..][..width * height * 4];
        let visible = cell.chunks(4).filter(|p| p[3] != 0).count();
        println!("cell {n} (icon {id}): {visible}/{} texels", width * height);
    }
    Ok(())
}

fn pack_models(entry: Option<usize>) -> Result<(), String> {
    let path = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin")
        });
    let (mut disc, big) = open_bigfile(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let entry = entry.unwrap_or_else(|| BigIndex::ARCADE_1P_PACK.nth(0).get());
    let file = big.read(&mut disc, entry).map_err(|e| e.to_string())?;
    let pack = DramFile::parse(&file).map_err(|e| e.to_string())?;
    for m in loader::model::parse_pack_models(&pack).map_err(|e| e.to_string())? {
        println!("{:#04x} {:?}", m.id, m.name);
        for (i, h) in m.headers.iter().enumerate() {
            let frames: Vec<String> = h
                .animations
                .iter()
                .map(|a| format!("{}:{}", a.name, a.num_frames))
                .collect();
            println!(
                "     lod {i} {:?} flags {:#x} commands {} animations [{}]",
                h.name,
                h.flags,
                h.commands.len(),
                frames.join(", ")
            );
            // The extent of the first frame, in model units and scaled by the header's.
            let frame = h
                .static_frame
                .as_ref()
                .or(h.animations.first().and_then(|a| a.frames.first()));
            if let Some(f) = frame.filter(|f| !f.vertices.is_empty()) {
                let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
                for v in &f.vertices {
                    for a in 0..3 {
                        lo[a] = lo[a].min(v[a]);
                        hi[a] = hi[a].max(v[a]);
                    }
                }
                let scaled: Vec<String> = (0..3)
                    .map(|a| {
                        let k = h.scale[a] as f32 / 4096.0;
                        format!("{:.0}..{:.0}", lo[a] as f32 * k, hi[a] as f32 * k)
                    })
                    .collect();
                println!("          scale {:?} extent {}", h.scale, scaled.join(" "));
            }
        }
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        print!("{HELP}");
        return Ok(());
    }
    let command = args.remove(0);
    if command == "icons" {
        let first = args.first().map_or(Ok(32), |a| a.parse::<i32>());
        let last = args.get(1).map_or(Ok(55), |a| a.parse::<i32>());
        let (Ok(first), Ok(last)) = (first, last) else {
            return Err("icons takes numbers".into());
        };
        return icon_scan(first..=last);
    }
    if command == "models" {
        return pack_models(args.first().and_then(|a| a.parse().ok()));
    }
    if matches!(command.as_str(), "portraits" | "weapons") {
        return hud_pictures(&command);
    }
    if command == "crates" {
        let mut args = Args(args);
        let half = args.option_number("--half")?.unwrap_or(40);
        let height = args.option_number("--height")?.unwrap_or(96);
        let min = args.option_number("--min")?.unwrap_or(4);
        let levels: Vec<LevelId> = match args.0.first().filter(|a| !a.starts_with("--")) {
            Some(level) => vec![level_id(level)?],
            None => (0..=17).map(LevelId).collect(),
        };
        for id in levels {
            match load(id) {
                Ok(l) => sunk_crates(&l, half, height, min),
                Err(e) => println!("level {}: {e}", id.0),
            }
        }
        return Ok(());
    }
    let mut args = Args(args);
    let level = level_id(&args.positional("LEVEL")?)?;
    let l = load(level)?;
    match command.as_str() {
        "level" => {
            args.finish()?;
            level_summary(&l);
        }
        "sky" => {
            args.finish()?;
            sky_summary(&l.level);
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
