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

mod support;

use std::path::Path;

use coll::{Collision, search_flags};
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::{BspKind, Level, SVec3, SpawnPoint};
use loader::level::LevelId;
use support::{Rng, build_driver, describe_level, dump, index, run_reference, vec3};

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
    Moved {
        flags: u16,
        wanted: u32,
        ignored: u32,
        door_flags: u32,
        current: SVec3,
        next: SVec3,
        radius: i32,
    },
    Scrub {
        quad: Option<u32>,
        triangle_id: i32,
    },
    /// `bspHitboxesHit`: hitboxes (by file offset) the search skips.
    Mark(Vec<u32>),
}

impl Query {
    fn script(&self) -> String {
        match self {
            Query::Search {
                flags,
                wanted,
                ignored,
                top,
                bottom,
                radius,
            } => {
                format!(
                    "S {flags} {} {} {}{} {radius}",
                    *wanted as i32,
                    *ignored as i32,
                    vec3(*top).trim_start(),
                    vec3(*bottom)
                )
            }
            Query::NormVec {
                dividend,
                bit_shift,
                lod_shift,
                v,
            } => {
                format!(
                    "G {dividend} {bit_shift} {lod_shift}{}{}{}",
                    vec3(v[0]),
                    vec3(v[1]),
                    vec3(v[2])
                )
            }
            Query::Barycentrics(v) => format!("Y{}{}{}", vec3(v[0]), vec3(v[1]), vec3(v[2])),
            Query::Moved {
                flags,
                wanted,
                ignored,
                door_flags,
                current,
                next,
                radius,
            } => {
                format!(
                    "M {flags} {} {} {}{}{} {radius}",
                    *wanted as i32,
                    *ignored as i32,
                    *door_flags as i32,
                    vec3(*current),
                    vec3(*next)
                )
            }
            Query::Scrub { quad, triangle_id } => format!("F {} {triangle_id}", index(*quad)),
            Query::Mark(offsets) => {
                format!(
                    "K {}{}",
                    offsets.len(),
                    offsets.iter().map(|o| format!(" {o}")).collect::<String>()
                )
            }
        }
    }

    fn run(&self, c: &mut Collision, level: &Level) -> String {
        match self {
            Query::Search {
                flags,
                wanted,
                ignored,
                top,
                bottom,
                radius,
            } => {
                let q = &mut c.sps.quad_block_coll;
                (q.search_flags, q.quad_flags_wanted, q.quad_flags_ignored) =
                    (*flags, *wanted, *ignored);
                c.search_quadblocks(level, *top, *bottom, *radius);
                dump(c)
            }
            Query::NormVec {
                dividend,
                bit_shift,
                lod_shift,
                v,
            } => {
                for (bsv, pos) in c.sps.search_vert.iter_mut().zip(v) {
                    bsv.pos = *pos;
                }
                c.sps.collision.tri_normal_vec_bit_shift = *bit_shift;
                c.sps.collision.tri_normal_lod_shift = *lod_shift;
                c.triangle_norm_vec(*dividend, 0, 1, 2);
                dump(c)
            }
            Query::Moved {
                flags,
                wanted,
                ignored,
                door_flags,
                current,
                next,
                radius,
            } => {
                let q = &mut c.sps.quad_block_coll;
                (q.search_flags, q.quad_flags_wanted, q.quad_flags_ignored) =
                    (*flags, *wanted, *ignored);
                c.door_access_flags = *door_flags;
                let r = *radius as i16;
                c.sps.input1.hit_radius = r;
                c.sps.input1.hit_radius_squared = *radius * *radius;
                c.sps.quad_block_coll.hit_radius = r;
                c.sps.quad_block_coll.hit_radius_squared = *radius * *radius;
                if c.prepare_sweep(*current, *next) {
                    c.search_moved(level);
                }
                dump(c)
            }
            Query::Scrub { quad, triangle_id } => {
                c.find_scrub(*quad, *triangle_id);
                dump(c)
            }
            Query::Mark(offsets) => {
                c.sps.num_bsp_hitboxes_hit = offsets.len() as i32;
                for (slot, &offset) in c.sps.bsp_hitboxes_hit.iter_mut().zip(offsets) {
                    *slot = Some(offset);
                }
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
    SVec3 {
        x: r(rng),
        y: r(rng),
        z: r(rng),
    }
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
        let (x, y, z) = (
            pick(rng, a.x, b.x),
            pick(rng, a.y, b.y),
            pick(rng, a.z, b.z),
        );
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
            top: SVec3 {
                x: x.wrapping_add(sx as i16),
                y: y.wrapping_add(up as i16),
                z: z.wrapping_add(sz as i16),
            },
            bottom: SVec3 {
                x,
                y: y.wrapping_sub(down as i16),
                z,
            },
            radius: rng.range(0, 0x100),
        });
    }
    out
}

/// Short sweeps starting inside random quad blocks, interleaved with scrub
/// list updates for the quads that were hit.
fn moved_queries(level: &Level, rng: &mut Rng, trials: u64) -> Vec<Query> {
    let mut out = Vec::new();
    let wanted = [0x3000, 0x1000, 0x2000, 0xffff, 0x0040, 0x1040];
    let ignored = [0, 0, 0, 0x0010, 0x8000];
    for _ in 0..trials {
        let quad = &level.quad_blocks[rng.below(level.quad_blocks.len() as u64) as usize];
        let (a, b) = (quad.bbox.min, quad.bbox.max);
        let pick = |rng: &mut Rng, lo: i16, hi: i16| rng.range(lo as i32, hi as i32) as i16;
        let (x, y, z) = (
            pick(rng, a.x, b.x),
            pick(rng, a.y, b.y),
            pick(rng, a.z, b.z),
        );
        // Mostly slow movements that end near the surface, some fast ones.
        let reach = [0x20, 0x60, 0x200][rng.below(3) as usize];
        let step = |rng: &mut Rng, v: i16| v.wrapping_add(rng.range(-reach, reach) as i16);
        let mut flags = search_flags::HIGH_LOD;
        if rng.below(4) == 0 {
            flags |= search_flags::REUSE_NORMALS;
        }
        out.push(Query::Moved {
            flags,
            wanted: wanted[rng.below(wanted.len() as u64) as usize],
            ignored: ignored[rng.below(ignored.len() as u64) as usize],
            door_flags: [0, 0, 0x7f, 1][rng.below(4) as usize],
            current: SVec3 { x, y, z },
            next: SVec3 {
                x: step(rng, x),
                y: step(rng, y),
                z: step(rng, z),
            },
            radius: if rng.below(3) == 0 {
                rng.range(0, 0x100)
            } else {
                0x19
            },
        });
        if rng.below(3) == 0 {
            // Few distinct (quad, triangle) pairs, so repeats occur, and a
            // reset often enough that the list (15 entries) never overflows.
            let reset = rng.below(10) == 0;
            out.push(Query::Scrub {
                quad: if reset {
                    None
                } else {
                    Some(rng.below(3) as u32)
                },
                triangle_id: rng.range(0, 3),
            });
        }
    }
    out
}

/// Sweeps through and around the hitboxes of the level, with some instances not drawn (so not
/// hit) and, now and then, the flag that makes any hitbox in reach a hit. Quad blocks are mostly
/// ignored, to see the hitboxes alone.
fn hitbox_queries(level: &Level, rng: &mut Rng, trials: u64) -> Vec<Query> {
    let hitboxes: Vec<_> = level
        .bsp
        .iter()
        .filter_map(|n| match &n.kind {
            BspKind::Leaf { hitboxes, .. } => Some(hitboxes),
            _ => None,
        })
        .flatten()
        .collect();
    let mut out = Vec::new();
    if hitboxes.is_empty() {
        return out;
    }
    for _ in 0..trials {
        let h = hitboxes[rng.below(hitboxes.len() as u64) as usize];
        let r = h.radius as i32;
        // From around the sphere, to around it: mostly passing through or grazing it.
        let around =
            |rng: &mut Rng, v: i16| v.wrapping_add(rng.range(-2 * r - 0x30, 2 * r + 0x30) as i16);
        let current = SVec3 {
            x: around(rng, h.center.x),
            y: around(rng, h.center.y),
            z: around(rng, h.center.z),
        };
        let next = if rng.below(8) == 0 {
            current
        } else {
            SVec3 {
                x: around(rng, h.center.x),
                y: around(rng, h.center.y),
                z: around(rng, h.center.z),
            }
        };
        let mut flags = search_flags::HIGH_LOD | search_flags::TEST_INSTANCES;
        if rng.below(10) == 0 {
            flags |= search_flags::FORCE_INSTANCE_HIT;
        }
        // Now and then some are already dealt with, as after an earlier sweep of the step.
        if rng.below(5) == 0 {
            let n = rng.below(4) as usize;
            out.push(Query::Mark(
                (0..n)
                    .map(|_| hitboxes[rng.below(hitboxes.len() as u64) as usize].offset)
                    .collect(),
            ));
            if n > 0 && rng.below(2) == 0 {
                out.push(Query::Mark(vec![h.offset]));
            }
        }
        out.push(Query::Moved {
            flags,
            wanted: [0, 0, 0x3000][rng.below(3) as usize],
            ignored: 0,
            door_flags: 0,
            current,
            next,
            radius: if rng.below(3) == 0 {
                rng.range(0, 0x100)
            } else {
                0x19
            },
        });
    }
    out
}

fn synthetic_queries(rng: &mut Rng, trials: u64) -> Vec<Query> {
    let mut out = Vec::new();
    for _ in 0..trials {
        let bits = [6, 10, 14, 15, 16][rng.below(5) as usize];
        let v = [
            random_vec(rng, bits),
            random_vec(rng, bits),
            random_vec(rng, bits),
        ];
        out.push(Query::NormVec {
            dividend: rng.range(-0x8000, 0x7fff) as i16,
            bit_shift: rng.below(32) as u8,
            lod_shift: [0, 2][rng.below(2) as usize],
            v,
        });
        out.push(Query::Barycentrics([
            random_vec(rng, bits),
            random_vec(rng, bits),
            random_vec(rng, bits),
        ]));
    }
    out
}

/// Returns how many moved queries touched an instance hitbox.
fn compare(what: &str, level: &Level, exe: &Path, queries: &[Query]) -> u32 {
    let mut script = describe_level(level);
    for q in queries {
        script.push_str(&q.script());
        script.push('\n');
    }
    let reference = run_reference(exe, &script);
    assert_eq!(
        reference.len(),
        queries.len(),
        "{what}: driver printed {} lines for {} queries",
        reference.len(),
        queries.len()
    );

    let mut c = Collision::default();
    let (mut failures, mut hits, mut touched) = (0, 0, 0);
    // How often each `reorder_result` (-1..=6) was the final hit of a moved query.
    let mut clips = [0u32; 8];
    for (i, (q, want)) in queries.iter().zip(&reference).enumerate() {
        let got = q.run(&mut c, level);
        if matches!(q, Query::Search { .. } | Query::Moved { .. }) && c.sps.did_touch_quadblock > 0
        {
            hits += 1;
        }
        if matches!(q, Query::Moved { .. }) && c.sps.did_touch_quadblock > 0 {
            clips[(c.sps.hit.reorder_result + 1) as usize] += 1;
        }
        if matches!(q, Query::Moved { .. }) && c.sps.did_touch_hitbox > 0 {
            touched += 1;
        }
        if &got != want {
            failures += 1;
            if failures <= 3 {
                eprintln!(
                    "{what}: MISMATCH on query {i}: `{}`\n  rust: {got}\n  ref:  {want}",
                    q.script()
                );
            }
        }
    }
    eprintln!(
        "{what}: {} queries, {hits} with a hit, moved hits by clip result (-1..6): {clips:?}",
        queries.len()
    );
    assert_eq!(
        failures,
        0,
        "{what}: {failures} of {} queries differ from the reference",
        queries.len()
    );
    touched
}

fn trials() -> u64 {
    std::env::var("COLL_REFERENCE_TRIALS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000)
}

#[test]
fn matches_reference_on_random_triangles() {
    let Some(exe) = build_driver(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/reference.c"),
        "coll_reference",
        &["game/COLL.c", "game/COLL/COLL_InstanceQuery.c"],
    ) else {
        return;
    };
    let empty = Level {
        instances: Vec::new(),
        models: Vec::new(),
        spawn_delays: Vec::new(),
        position_paths: Vec::new(),
        paths: Vec::new(),
        fly_in: None,
        skybox: None,
        clear_colors: [None; 3],
        quad_blocks: Vec::new(),
        vertices: Vec::new(),
        bsp: Vec::new(),
        driver_spawns: [SpawnPoint {
            pos: SVec3::default(),
            rot: SVec3::default(),
        }; 8],
        restart_points: Vec::new(),
        water: Default::default(),
        scenery: Default::default(),
        config_flags: 0,
        jump_vertical_speed_cap: 0,
        split_lines: [0; 2],
        rain_buffer: Default::default(),
    };
    let mut rng = Rng(0x1234_5678_9abc_def1);
    compare(
        "synthetic",
        &empty,
        &exe,
        &synthetic_queries(&mut rng, trials()),
    );
}

#[test]
fn matches_reference_on_tracks() {
    let Some(exe) = build_driver(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/reference.c"),
        "coll_reference",
        &["game/COLL.c", "game/COLL/COLL_InstanceQuery.c"],
    ) else {
        return;
    };
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let idx = BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev);
        let file = b.big.read(&mut b.disc, idx.get()).unwrap();
        let level = Level::parse(&DramFile::parse(&file).unwrap()).unwrap();
        compare(
            &format!("level {id}"),
            &level,
            &exe,
            &queries(&level, &mut rng, trials()),
        );
        compare(
            &format!("level {id} moved"),
            &level,
            &exe,
            &moved_queries(&level, &mut rng, trials()),
        );

        // The same level with some instances not drawn.
        let mut hidden = level.clone();
        for inst in &mut hidden.instances {
            if rng.below(4) == 0 {
                inst.flags &= !coll::instance::DRAW_COLLISION_MASK;
            }
        }
        // And with hitboxes the real ones are not: cylinders (no Y axis), and a Y range.
        for node in &mut hidden.bsp {
            if let BspKind::Leaf { hitboxes, .. } = &mut node.kind {
                for h in hitboxes {
                    match rng.below(4) {
                        0 => h.flag &= !0x40,
                        1 => {
                            h.flag |= 0x20;
                            h.id = -(rng.range(0, 2 * h.radius as i32) as i16);
                        }
                        _ => {}
                    }
                }
            }
        }
        let queries = hitbox_queries(&hidden, &mut rng, trials());
        let touched = compare(&format!("level {id} hitboxes"), &hidden, &exe, &queries);
        if touched > 0 {
            eprintln!("level {id}: {touched} hitbox queries touched a hitbox");
        }
    }
}
