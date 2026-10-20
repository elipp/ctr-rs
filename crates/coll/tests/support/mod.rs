//! Shared by the differential tests of `coll` and `vehicle`: building and
//! running the C drivers, describing a level to them, and the scratchpad dump
//! both sides print.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use coll::Collision;
use loader::lev::{BspChild, BspKind, Level, SVec3};

/// Small deterministic xorshift64* generator, so failures reproduce.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u64) as i32
    }
}

pub fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/ctr-native")
}

/// Builds a C driver against the reference `ctr-native`; `sources` are
/// extra files of the reference to link, relative to it.
pub fn build_driver(driver: &Path, exe_name: &str, sources: &[&str]) -> Option<PathBuf> {
    // Tests of one binary run in parallel and want the same driver: it is built once, so that none
    // is run while another is writing it.
    static BUILT: std::sync::Mutex<Vec<(String, Option<PathBuf>)>> = std::sync::Mutex::new(Vec::new());
    let mut built = BUILT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, exe)) = built.iter().find(|(name, _)| name == exe_name) {
        return exe.clone();
    }
    let exe = build_driver_now(driver, exe_name, sources);
    built.push((exe_name.to_owned(), exe.clone()));
    exe
}

fn build_driver_now(driver: &Path, exe_name: &str, sources: &[&str]) -> Option<PathBuf> {
    let reference = reference_dir();
    if !reference.exists() {
        eprintln!("skipping: {} not found", reference.display());
        return None;
    }
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(exe_name);
    let status = Command::new("gcc")
        .args(["-m32", "-DCTR_NATIVE", "-w", "-O1"])
        .arg("-I")
        .arg(reference.join("include"))
        .arg("-I")
        .arg(reference.join("game"))
        .arg("-I")
        .arg(&reference)
        .arg(driver)
        .args(
            [
                "platform/native_gte_core.c",
                "platform/native_inline_c.c",
                "platform/native_libgte.c",
                "game/MATH/MATH_7_MatrixStubs.c",
            ]
            .iter()
            .chain(sources)
            .map(|f| reference.join(f)),
        )
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
pub fn describe_level(level: &Level) -> String {
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
        write!(s, " {}", q.draw_order_low as i32).unwrap();
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
            BspKind::Leaf { quad_blocks, hitboxes } => {
                write!(s, "1 {} {} {}", quad_blocks.start, quad_blocks.len(), hitboxes.len()).unwrap();
                for h in hitboxes {
                    let (a, b) = (h.bbox.min, h.bbox.max);
                    let flags = h.instance.map_or(0, |i| level.instances[i].flags as i32);
                    write!(
                        s,
                        " {} {} {} {} {} {} {} {}{} {} {} {} {}",
                        h.flag, h.id, a.x, a.y, a.z, b.x, b.y, b.z, vec3(h.center), h.radius, h.offset, flags, h.instance.is_some() as u8
                    )
                    .unwrap();
                }
            }
        }
        s.push('\n');
    }
    s
}

pub fn vec3(v: SVec3) -> String {
    format!(" {} {} {}", v.x, v.y, v.z)
}

pub fn plane(p: coll::Plane) -> String {
    format!("{} {}", vec3(p.normal), p.half_distance)
}

pub fn index<T: Into<i64>>(i: Option<T>) -> i64 {
    i.map_or(-1, Into::into)
}

/// The same text the driver's `pstate` prints.
pub fn dump(c: &Collision) -> String {
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
    o += &vec3(s.candidate.push_out);
    o += &vec3(s.hit.push_out);
    o += &vec3(s.candidate_delta);
    write!(
        o,
        " {} {} {} {} {}",
        s.candidate.triangle_id, s.hit.triangle_id, s.hit.reorder_result, s.hit.normal_axis, s.input1.scrub_depth
    )
    .unwrap();
    let h = s.hit_search_triangle;
    write!(o, " {} {} {}", index(h[0]), index(h[1]), index(h[2])).unwrap();
    write!(o, " {} {} {}", s.did_touch_hitbox, s.num_bsp_hitboxes_hit, s.bsp_hitbox.map_or(-1, |h| h.offset as i64)).unwrap();
    write!(o, " {} {}", s.quad_block_coll.search_flags, s.num_scrub_triangles).unwrap();
    for t in &s.scrub_triangles[..s.num_scrub_triangles as usize] {
        // A hitbox's key is above `i32::MAX` here, as the C driver prints it.
        write!(o, " {} {} {}", index(t.quad.map(|q| q as i32)), t.triangle_id, t.scrub_depth).unwrap();
    }
    for v in &s.search_vert {
        o += &vec3(v.pos);
        write!(o, " {} {}", v.normal_axis, index(v.level_vertex)).unwrap();
        o += &plane(v.plane);
    }
    o
}

pub fn run_reference(exe: &Path, script: &str) -> Vec<String> {
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to run the C reference driver");
    let mut stdin = child.stdin.take().unwrap();
    let script = script.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(script.as_bytes()));
    let out = child.wait_with_output().unwrap();
    // Check the exit status first: a driver that died early also breaks the pipe.
    assert!(out.status.success(), "C reference driver failed: {}", out.status);
    writer.join().unwrap().unwrap();
    String::from_utf8(out.stdout).unwrap().lines().map(str::to_owned).collect()
}

