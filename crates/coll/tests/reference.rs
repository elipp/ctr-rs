//! Differential test: run collision searches on real tracks (and random
//! triangles) through this crate and through the reference `COLL.c`, and
//! require identical scratchpad state after every query.
//!
//! The C side is built with `gcc -m32` from `reference/ctr-native` and fed a
//! text description of the level; the test is skipped (with a note) if the
//! reference or the disc image is unavailable. `COLL_REFERENCE_TRIALS`
//! overrides the number of queries per track.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use coll::{Collision, search_flags};
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::{BspChild, BspKind, Level, SVec3, SpawnPoint};
use loader::level::LevelId;

/// Small deterministic xorshift64* generator, so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u64) as i32
    }
}

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/ctr-native")
}

fn build_reference_driver() -> Option<PathBuf> {
    let reference = reference_dir();
    if !reference.exists() {
        eprintln!("skipping: {} not found", reference.display());
        return None;
    }
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join("coll_reference");
    let status = Command::new("gcc")
        .args(["-m32", "-DCTR_NATIVE", "-w", "-O1"])
        .arg("-I")
        .arg(reference.join("include"))
        .arg("-I")
        .arg(reference.join("game"))
        .arg("-I")
        .arg(&reference)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/reference.c"))
        .args([
            "game/COLL.c",
            "platform/native_gte_core.c",
            "platform/native_inline_c.c",
            "platform/native_libgte.c",
            "game/MATH/MATH_7_MatrixStubs.c",
        ].map(|f| reference.join(f)))
        // regionsEXE.h defines `sdata` (identically) in every translation unit;
        // the rest of the game that COLL.c calls into is never reached here.
        .args(["-Wl,-z,muldefs", "-Wl,--unresolved-symbols=ignore-all", "-static"])
        .arg("-o")
        .arg(&exe)
        .status()
        .expect("failed to run gcc");
    assert!(status.success(), "could not build the C reference driver (needs gcc with -m32)");
    Some(exe)
}

/// The driver's description of a level (`V`, `Q` and `B` blocks).
fn describe_level(level: &Level) -> String {
    let mut s = String::new();
    writeln!(s, "V {}", level.vertices.len()).unwrap();
    for v in &level.vertices {
        writeln!(s, "{} {} {} {}", v.pos.x, v.pos.y, v.pos.z, v.flags).unwrap();
    }
    writeln!(s, "Q {}", level.quad_blocks.len()).unwrap();
    for q in &level.quad_blocks {
        for i in q.vertex_index {
            write!(s, "{i} ").unwrap();
        }
        let (a, b) = (q.bbox.min, q.bbox.max);
        write!(s, "{} {} {} {} {} {} {} {} {}", q.flags, a.x, a.y, a.z, b.x, b.y, b.z, q.terrain_type, q.tri_normal_vec_bit_shift).unwrap();
        for d in q.tri_normal_vec_dividend {
            write!(s, " {d}").unwrap();
        }
        s.push('\n');
    }
    writeln!(s, "B {}", level.bsp.len()).unwrap();
    for n in &level.bsp {
        let (a, b) = (n.bbox.min, n.bbox.max);
        write!(s, "{} {} {} {} {} {} {} {} ", n.flag, n.id, a.x, a.y, a.z, b.x, b.y, b.z).unwrap();
        match &n.kind {
            BspKind::Branch { children, .. } => {
                s.push_str("0");
                for c in children {
                    let raw = match c {
                        BspChild::None => 0xffffu16,
                        BspChild::Branch(i) => *i,
                        BspChild::Leaf(i) => 0x4000 | i,
                    };
                    write!(s, " {raw}").unwrap();
                }
            }
            BspKind::Leaf { quad_blocks, .. } => {
                write!(s, "1 {} {}", quad_blocks.start, quad_blocks.len()).unwrap();
            }
        }
        s.push('\n');
    }
    s
}

fn vec3(v: SVec3) -> String {
    format!(" {} {} {}", v.x, v.y, v.z)
}

fn plane(p: coll::Plane) -> String {
    format!("{} {}", vec3(p.normal), p.half_distance)
}

fn index<T: Into<i64>>(i: Option<T>) -> i64 {
    i.map_or(-1, Into::into)
}

/// The same text the driver's `pstate` prints.
fn dump(c: &Collision) -> String {
    let s = &c.sps;
    let mut o = String::new();
    write!(o, "{} {} {} {}", s.num_triangles_tested, s.did_touch_quadblock, s.collision.step_flags, s.hit_fraction).unwrap();
    o += &vec3(s.quad_block_coll.pos);
    o += &vec3(s.quad_block_coll.hit_pos);
    o += &vec3(s.hit.hit_pos);
    o += &plane(s.hit.plane);
    write!(o, " {}", index(s.hit.quad)).unwrap();
    o += &vec3(s.candidate.hit_pos);
    write!(o, " {}", s.candidate.normal_axis).unwrap();
    o += &plane(s.candidate.plane);
    write!(o, " {}", index(s.candidate.quad)).unwrap();
    write!(o, " {} {}", s.hit_barycentrics.v1, s.hit_barycentrics.v2).unwrap();
    let t = s.hit_level_triangle;
    write!(o, " {} {} {}", index(t[0]), index(t[1]), index(t[2])).unwrap();
    let packed = (s.quad_second_tri_index_a as u32 * 65536 + s.quad_second_tri_index_b as u32) as i32;
    write!(
        o,
        " {} {} {} {}",
        s.collision.tri_normal_vec_dividend, s.collision.tri_normal_vec_bit_shift, s.collision.tri_normal_lod_shift, packed
    )
    .unwrap();
    for v in &s.search_vert {
        o += &vec3(v.pos);
        write!(o, " {} {}", v.normal_axis, index(v.level_vertex)).unwrap();
        o += &plane(v.plane);
    }
    o
}

/// One script line for the driver and a way to get the same answer from Rust.
enum Query {
    Search {
        flags: u16,
        wanted: u32,
        ignored: u32,
        top: SVec3,
        bottom: SVec3,
        radius: i32,
    },
    NormVec {
        dividend: i16,
        bit_shift: u8,
        lod_shift: u8,
        v: [SVec3; 3],
    },
    Barycentrics([SVec3; 3]),
}

impl Query {
    fn script(&self) -> String {
        match self {
            Query::Search { flags, wanted, ignored, top, bottom, radius } => {
                format!("S {flags} {} {} {}{} {radius}", *wanted as i32, *ignored as i32, vec3(*top).trim_start(), vec3(*bottom))
            }
            Query::NormVec { dividend, bit_shift, lod_shift, v } => {
                format!("G {dividend} {bit_shift} {lod_shift}{}{}{}", vec3(v[0]), vec3(v[1]), vec3(v[2]))
            }
            Query::Barycentrics(v) => format!("Y{}{}{}", vec3(v[0]), vec3(v[1]), vec3(v[2])),
        }
    }

    fn run(&self, c: &mut Collision, level: &Level) -> String {
        match self {
            Query::Search { flags, wanted, ignored, top, bottom, radius } => {
                let q = &mut c.sps.quad_block_coll;
                (q.search_flags, q.quad_flags_wanted, q.quad_flags_ignored) = (*flags, *wanted, *ignored);
                c.search_quadblocks(level, *top, *bottom, *radius);
                dump(c)
            }
            Query::NormVec { dividend, bit_shift, lod_shift, v } => {
                for (bsv, pos) in c.sps.search_vert.iter_mut().zip(v) {
                    bsv.pos = *pos;
                }
                c.sps.collision.tri_normal_vec_bit_shift = *bit_shift;
                c.sps.collision.tri_normal_lod_shift = *lod_shift;
                c.triangle_norm_vec(*dividend, 0, 1, 2);
                dump(c)
            }
            Query::Barycentrics(v) => {
                let out = coll::triangle_barycentrics(&mut c.gte, v[0], v[1], v[2]);
                vec3(out)
            }
        }
    }
}


fn random_vec(rng: &mut Rng, bits: u32) -> SVec3 {
    let r = |rng: &mut Rng| {
        let b = rng.below(bits as u64 + 1) as u32;
        let v = (rng.next() >> 40) as i32 & ((1 << b) - 1);
        (if rng.below(2) == 0 { -v } else { v }) as i16
    };
    SVec3 { x: r(rng), y: r(rng), z: r(rng) }
}

/// Segments through the middle of random quad blocks, plus random noise.
fn queries(level: &Level, rng: &mut Rng, trials: u64) -> Vec<Query> {
    let mut out = Vec::new();
    let wanted = [0xffff, 0x1000, 0x2000, 0x3000, 0x0040, 0x1040];
    let ignored = [0, 0, 0x10, 0x0200, 0x8000];
    for _ in 0..trials {
        let quad = &level.quad_blocks[rng.below(level.quad_blocks.len() as u64) as usize];
        let (a, b) = (quad.bbox.min, quad.bbox.max);
        let pick = |rng: &mut Rng, lo: i16, hi: i16| rng.range(lo as i32, hi as i32) as i16;
        let (x, y, z) = (pick(rng, a.x, b.x), pick(rng, a.y, b.y), pick(rng, a.z, b.z));
        let (up, down) = (rng.range(0, 0x400), rng.range(0, 0x400));
        let (sx, sz) = (rng.range(-0x180, 0x180), rng.range(-0x180, 0x180));
        let mut flags = search_flags::HIGH_LOD;
        if rng.below(4) == 0 {
            flags |= search_flags::REUSE_NORMALS;
        }
        out.push(Query::Search {
            flags,
            wanted: wanted[rng.below(wanted.len() as u64) as usize],
            ignored: ignored[rng.below(ignored.len() as u64) as usize],
            top: SVec3 { x: x.wrapping_add(sx as i16), y: y.wrapping_add(up as i16), z: z.wrapping_add(sz as i16) },
            bottom: SVec3 { x, y: y.wrapping_sub(down as i16), z },
            radius: rng.range(0, 0x100),
        });
    }
    out
}

fn synthetic_queries(rng: &mut Rng, trials: u64) -> Vec<Query> {
    let mut out = Vec::new();
    for _ in 0..trials {
        let bits = [6, 10, 14, 15, 16][rng.below(5) as usize];
        let v = [random_vec(rng, bits), random_vec(rng, bits), random_vec(rng, bits)];
        out.push(Query::NormVec {
            dividend: rng.range(-0x8000, 0x7fff) as i16,
            bit_shift: rng.below(32) as u8,
            lod_shift: [0, 2][rng.below(2) as usize],
            v,
        });
        out.push(Query::Barycentrics([random_vec(rng, bits), random_vec(rng, bits), random_vec(rng, bits)]));
    }
    out
}

fn run_reference(exe: &Path, script: &str) -> Vec<String> {
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to run the C reference driver");
    let mut stdin = child.stdin.take().unwrap();
    let script = script.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(script.as_bytes()));
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    assert!(out.status.success(), "C reference driver failed: {}", out.status);
    String::from_utf8(out.stdout).unwrap().lines().map(str::to_owned).collect()
}

fn compare(what: &str, level: &Level, exe: &Path, queries: &[Query]) {
    let mut script = describe_level(level);
    for q in queries {
        script.push_str(&q.script());
        script.push('\n');
    }
    let reference = run_reference(exe, &script);
    assert_eq!(reference.len(), queries.len(), "{what}: driver printed {} lines for {} queries", reference.len(), queries.len());

    let mut c = Collision::default();
    let (mut failures, mut hits) = (0, 0);
    for (i, (q, want)) in queries.iter().zip(&reference).enumerate() {
        let got = q.run(&mut c, level);
        if matches!(q, Query::Search { .. }) && c.sps.did_touch_quadblock > 0 {
            hits += 1;
        }
        if &got != want {
            failures += 1;
            if failures <= 3 {
                eprintln!("{what}: MISMATCH on query {i}: `{}`\n  rust: {got}\n  ref:  {want}", q.script());
            }
        }
    }
    eprintln!("{what}: {} queries, {hits} with a hit", queries.len());
    assert_eq!(failures, 0, "{what}: {failures} of {} queries differ from the reference", queries.len());
}

fn trials() -> u64 {
    std::env::var("COLL_REFERENCE_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(3000)
}

#[test]
fn matches_reference_on_random_triangles() {
    let Some(exe) = build_reference_driver() else { return };
    let empty = Level {
        quad_blocks: Vec::new(),
        vertices: Vec::new(),
        bsp: Vec::new(),
        driver_spawns: [SpawnPoint { pos: SVec3::default(), rot: SVec3::default() }; 8],
        restart_points: Vec::new(),
        config_flags: 0,
        jump_vertical_speed_cap: 0,
    };
    let mut rng = Rng(0x1234_5678_9abc_def1);
    compare("synthetic", &empty, &exe, &synthetic_queries(&mut rng, trials()));
}

#[test]
fn matches_reference_on_tracks() {
    let Some(exe) = build_reference_driver() else { return };
    let Some(mut b) = common::open_bigfile() else { return };
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let idx = BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev);
        let file = b.big.read(&mut b.disc, idx.get()).unwrap();
        let level = Level::parse(&DramFile::parse(&file).unwrap()).unwrap();
        compare(&format!("level {id}"), &level, &exe, &queries(&level, &mut rng, trials()));
    }
}
