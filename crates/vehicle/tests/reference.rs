//! Differential test: run `COLL_MOVED_PlayerSearch` on real tracks through this
//! crate and through the reference C code, from randomised driver states, and
//! require identical driver, effects and scratchpad afterwards.
//!
//! The C side is built with `gcc -m32` from `reference/ctr-native`
//! (`tests/reference/reference.c`); the test is skipped (with a note) if the
//! reference or the disc image is unavailable. `VEHICLE_REFERENCE_TRIALS`
//! overrides the number of searches per track.

#[path = "../../loader/tests/common/mod.rs"]
mod common;
#[path = "../../coll/tests/support/mod.rs"]
mod support;

use std::fmt::Write as _;
use std::path::Path;

use coll::Collision;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::{Level, SVec3};
use loader::level::LevelId;
use math::Vec3;
use support::{Rng, build_driver, describe_level, dump, run_reference};
use vehicle::scrub::SCRUBS;
use vehicle::search::moved_player_search;
use vehicle::{Driver, Effect, MatrixArray, PhysState};

fn quad(q: Option<u32>) -> i64 {
    q.map_or(-1, i64::from)
}

/// The driver's fields in the order of the C driver's `fields` table.
fn fields(d: &Driver) -> Vec<i64> {
    let v3 = |v: Vec3| [v.x as i64, v.y as i64, v.z as i64];
    let s3 = |v: SVec3| [v.x as i64, v.y as i64, v.z as i64];
    let mut o: Vec<i64> = vec![d.driver_id as i64, d.kart_state as i64, d.simp_turn_state as i64];
    o.extend(v3(d.pos_curr));
    o.extend(v3(d.velocity));
    o.extend(v3(d.origin_to_center));
    o.extend([d.collision_flags as i64, d.step_flag_set as i32 as i64, d.actions_flag_set as i32 as i64, d.actions_flag_set_prev_frame as i32 as i64]);
    o.extend([quad(d.curr_block_touching), quad(d.under_driver), quad(d.last_valid)]);
    for v in [d.normal_vec_up, d.axis_angle1_normal_vec, d.sps_hit_pos, d.sps_normal_vec, d.pos_wall_coll] {
        o.extend(s3(v));
    }
    o.extend([d.v_shift_count, d.speed, d.speed_approx, d.base_speed, d.fire_speed, d.jump_height_curr].map(i64::from));
    o.extend([d.x_speed, d.y_speed, d.z_speed].map(i64::from));
    o.extend([
        d.speedometer_needle_value,
        d.reserves,
        d.turbo_outside_timer,
        d.turbo_meter_room_left,
        d.wall_rub_timer,
        d.wall_rub_speed_limit,
        d.angle,
        d.rot_curr_w,
        d.turn_angle_curr,
        d.turn_angle_lerp_vel,
        d.turn_wobble_angle,
        d.turn_wobble_velocity,
        d.turn_wobble_timer,
        d.amp_turn_state,
        d.num_frames_spent_steering,
        d.rotation_spin_rate,
        d.veh_fire_audio_cooldown,
    ]
    .map(i64::from));
    o.push(d.screen_offset_y as i64);
    o.extend([d.distance_from_ground, d.reserved_0x40e, d.jump_squish_stretch, d.jump_squish_stretch2].map(i64::from));
    o.extend([d.anim_index as i64, d.anim_frame as i64, d.inst_scale_x as i64, d.inst_scale_y as i64]);
    o.push(match d.matrix_array {
        MatrixArray::None => 0,
        MatrixArray::WheelieStart => 1,
        MatrixArray::WheelieHold => 2,
        MatrixArray::WheelieRecover => 3,
        MatrixArray::CrashFall => 4,
        MatrixArray::Jump(n) => n as i64,
    });
    o.push(d.matrix_index as i64);
    o.push((d.phys_state == PhysState::SlamWall) as i64);
    o
}

fn effect_line(e: &Effect) -> String {
    match *e {
        Effect::Sound { id, echo } => format!(" S {id} {}", echo as u8),
        Effect::Voiceline { id, driver_id, priority } => format!(" V {id} {driver_id} {priority}"),
        Effect::JogCon { driver_id, pattern, strength } => format!(" J {driver_id} {pattern} {strength}"),
        Effect::ShockFreq { driver_id, a, b } => format!(" F {driver_id} {a} {b}"),
        Effect::ShockForce { driver_id, a, b } => format!(" G {driver_id} {a} {b}"),
        // Camera state is not part of the comparison.
        Effect::CameraFireSpeedZoom { .. } => String::new(),
    }
}

fn join(values: &[i64]) -> String {
    values.iter().map(|v| format!(" {v}")).collect()
}

/// One search: the world around the driver, and the driver.
struct Case {
    elapsed_ms: i32,
    light_matrix: [i16; 9],
    door_flags: u32,
    driver: Driver,
}

impl Case {
    fn script(&self) -> String {
        let m = self.light_matrix.map(|v| v.to_string()).join(" ");
        format!("P {} {m} {} {}", self.elapsed_ms, self.door_flags as i32, join(&fields(&self.driver)))
    }

    /// The same line the C driver prints.
    fn run(&self, c: &mut Collision, level: &Level) -> String {
        let mut d = self.driver.clone();
        c.door_access_flags = self.door_flags;

        let mut effects = Vec::new();
        moved_player_search(&mut d, c, level, &self.light_matrix, self.elapsed_ms, &mut effects);

        let mut o = join(&fields(&d));
        o.push_str(" |");
        for e in &effects {
            o.push_str(&effect_line(e));
        }
        write!(o, " | {}", dump(c)).unwrap();
        o
    }
}

fn scrub_script() -> String {
    let mut s = String::from("K");
    for scrub in &SCRUBS {
        write!(s, " {} {} {}", scrub.flags, scrub.speed_limit, scrub.impact_angle).unwrap();
    }
    s
}

/// The quarter-turn table `data.trigApprox`: entry `i` is `(sin, cos)` of
/// angle `i`, which `math::sin_cos` reads back from the same table.
fn trig_script() -> String {
    let mut s = String::from("T");
    for angle in 0..0x400 {
        let (sin, cos) = math::sin_cos(angle);
        write!(s, " {sin} {cos}").unwrap();
    }
    s
}

/// Drivers placed in random quad blocks, moving at varied speeds in random
/// directions, with a mix of the states `ScrubImpact` looks at.
fn cases(level: &Level, rng: &mut Rng, trials: u64) -> Vec<Case> {
    let mut out = Vec::new();
    let identity = [0x1000, 0, 0, 0, 0x1000, 0, 0, 0, 0x1000];
    for _ in 0..trials {
        let quad = &level.quad_blocks[rng.below(level.quad_blocks.len() as u64) as usize];
        let (a, b) = (quad.bbox.min, quad.bbox.max);
        let pick = |rng: &mut Rng, lo: i16, hi: i16| rng.range(lo as i32, hi as i32);
        // Sphere centre in the scratchpad's units, a little above the quad.
        let centre = [pick(rng, a.x, b.x), pick(rng, a.y, b.y) + rng.range(0, 0x30), pick(rng, a.z, b.z)];
        let origin_to_center = Vec3 { x: 0, y: rng.range(0, 0x30), z: 0 };
        let shift = [0, 1, 3, 5][rng.below(4) as usize];
        let speed = |rng: &mut Rng| rng.range(-0x1000, 0x1000) << shift;
        let flags = |rng: &mut Rng, bits: &[u32]| bits.iter().filter(|_| rng.below(2) == 0).fold(0, |a, b| a | b);

        let mut d = Driver {
            driver_id: rng.below(8) as u8,
            kart_state: [0, 0, 1, 2, 5][rng.below(5) as usize],
            simp_turn_state: rng.range(-2, 2) as i8,
            origin_to_center,
            pos_curr: Vec3 {
                x: (centre[0] - origin_to_center.x) << 8 | rng.range(0, 255),
                y: (centre[1] - origin_to_center.y) << 8 | rng.range(0, 255),
                z: (centre[2] - origin_to_center.z) << 8 | rng.range(0, 255),
            },
            velocity: Vec3 { x: speed(rng), y: speed(rng), z: speed(rng) },
            actions_flag_set: flags(rng, &[0x1, 0x80, 0x2000, 0x10000]),
            actions_flag_set_prev_frame: flags(rng, &[0x1]),
            under_driver: [None, None, Some(rng.below(level.quad_blocks.len() as u64) as u32)][rng.below(3) as usize],
            v_shift_count: [0, 0, 1, 7][rng.below(4) as usize],
            speed_approx: rng.range(-0x500, 0x1200) as i16,
            jump_height_curr: rng.range(-0x400, 0x400) as i16,
            fire_speed: [0, 0, 0, 5][rng.below(4) as usize],
            reserves: rng.range(0, 0x100) as i16,
            turbo_outside_timer: rng.range(0, 0x100) as i16,
            wall_rub_timer: [0, 0, 0x40][rng.below(3) as usize],
            wall_rub_speed_limit: rng.range(0, 0x4000) as i16,
            angle: rng.range(-0x800, 0x7ff) as i16,
            rot_curr_w: rng.range(-0x800, 0x7ff) as i16,
            turn_angle_curr: rng.range(-0x100, 0x100) as i16,
            ..Driver::default()
        };
        // Scribble over fields the search only writes, so stale values show.
        let mut junk = || rng.range(-9, 9);
        (d.x_speed, d.y_speed, d.z_speed) = (junk(), junk(), junk());
        d.anim_index = rng.below(5) as u8;
        d.anim_frame = rng.range(0, 9) as i16;
        (d.inst_scale_x, d.inst_scale_y) = (rng.range(0, 0x1000) as i16, rng.range(0, 0x1000) as i16);
        d.matrix_index = rng.below(4) as u8;
        for field in [
            &mut d.speed,
            &mut d.base_speed,
            &mut d.speedometer_needle_value,
            &mut d.turbo_meter_room_left,
            &mut d.rotation_spin_rate,
            &mut d.turn_angle_lerp_vel,
            &mut d.turn_wobble_angle,
            &mut d.turn_wobble_velocity,
            &mut d.turn_wobble_timer,
            &mut d.amp_turn_state,
            &mut d.num_frames_spent_steering,
            &mut d.veh_fire_audio_cooldown,
            &mut d.reserved_0x40e,
            &mut d.jump_squish_stretch,
            &mut d.jump_squish_stretch2,
        ] {
            *field = rng.range(-9, 9) as i16;
        }
        d.screen_offset_y = rng.range(-9, 9) as i8;
        d.distance_from_ground = rng.range(-9, 9) as i16;
        d.collision_flags = flags(rng, &[0x1, 0x2, 0x4, 0x8]) as u16;

        let light_matrix = if rng.below(2) == 0 {
            identity
        } else {
            // Rotation-like: entries up to one unit.
            std::array::from_fn(|_| rng.range(-0x1000, 0x1000) as i16)
        };
        out.push(Case {
            elapsed_ms: [32, 32, 33, 16][rng.below(4) as usize],
            light_matrix,
            door_flags: [0, 0, 0x7f][rng.below(3) as usize],
            driver: d,
        });
    }
    out
}

fn trials() -> u64 {
    std::env::var("VEHICLE_REFERENCE_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(2000)
}

#[test]
fn matches_reference_on_tracks() {
    let driver_c = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/reference.c");
    let Some(exe) = build_driver(
        &driver_c,
        "vehicle_reference",
        &["game/COLL.c", "game/Vehicle/VehCalc.c", "game/Vehicle/VehAfterColl.c", "game/Vehicle/VehPhysProc.c"],
    ) else {
        return;
    };
    let Some(mut b) = common::open_bigfile() else { return };
    let mut rng = Rng(0x51f1_5a4d_0f3c_77b1);
    // (searches that hit, slammed, swept more than once, projected along a wall)
    let mut totals = [0u32; 4];

    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let idx = BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev);
        let file = b.big.read(&mut b.disc, idx.get()).unwrap();
        let level = Level::parse(&DramFile::parse(&file).unwrap()).unwrap();
        let cases = cases(&level, &mut rng, trials());

        let mut script = describe_level(&level);
        script.push_str(&scrub_script());
        script.push('\n');
        script.push_str(&trig_script());
        script.push('\n');
        for c in &cases {
            script.push_str(&c.script());
            script.push('\n');
        }
        let reference = run_reference(&exe, &script);
        assert_eq!(reference.len(), cases.len(), "level {id}: driver printed {} lines for {} cases", reference.len(), cases.len());

        let mut collision = Collision::default();
        let (mut failures, mut hits, mut slams) = (0, 0, 0);
        let (mut multi, mut projected) = (0, 0);
        for (i, (case, want)) in cases.iter().zip(&reference).enumerate() {
            let got = case.run(&mut collision, &level);
            hits += (collision.sps.did_touch_quadblock > 0) as u32;
            slams += got.contains(" S 6 ") as u32;
            multi += (collision.sps.num_scrub_triangles > 1) as u32;
            projected += (collision.sps.quad_block_coll.search_flags & coll::search_flags::WALL_PROJECTION_DONE != 0) as u32;
            if &got != want.trim_end() && got.trim_start() != want.trim_start() {
                failures += 1;
                if failures <= 3 {
                    eprintln!("level {id}: MISMATCH on case {i}: `{}`\n  rust: {got}\n  ref:  {want}", case.script());
                }
            }
        }
        eprintln!("level {id}: {} cases, {hits} with a hit, {slams} slams, {multi} with several scrubs, {projected} wall projections", cases.len());
        totals = [totals[0] + hits, totals[1] + slams, totals[2] + multi, totals[3] + projected];
        assert_eq!(failures, 0, "level {id}: {failures} of {} cases differ from the reference", cases.len());
    }
    eprintln!("total (hit, slam, multi, projected): {totals:?}");
    assert!(totals.iter().all(|&n| n > 0), "the cases do not cover hits, slams, repeated scrubs and wall projection: {totals:?}");
}

/// `SCRUBS` against the `MetaDataScrub` initialiser in the reference source.
#[test]
fn scrub_table_matches_reference_source() {
    let source = support::reference_dir().join("game/zGlobal_DATA.c");
    let Ok(text) = std::fs::read_to_string(&source) else {
        eprintln!("skipping: {} not found", source.display());
        return;
    };
    let table = text.split(".MetaDataScrub =").nth(1).and_then(|t| t.split(".emSet_DirtLR").next()).expect("MetaDataScrub in zGlobal_DATA.c");
    let number = |s: &str| match s.trim().strip_prefix("0x").or(s.trim().strip_prefix("0X")) {
        Some(hex) => i32::from_str_radix(hex, 16).unwrap(),
        None => s.trim().parse().unwrap(),
    };
    let entries: Vec<(i32, i32, i32)> = table
        .lines()
        .filter_map(|l| l.trim().strip_prefix('{')?.split('}').next().map(str::to_owned))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (f.len() == 4).then(|| (number(f[1]), number(f[2]), number(f[3])))
        })
        .collect();
    let ours: Vec<(i32, i32, i32)> = SCRUBS.iter().map(|s| (s.flags as i32, s.speed_limit, s.impact_angle)).collect();
    assert_eq!(entries, ours);
}
