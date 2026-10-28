//! Differential test of the whole driver tick: drives karts of every
//! character over the real tracks with random inputs, through this crate and
//! through the reference C code (`tests/reference/tick.c`: the real
//! `VehPhysProc_Driving_PhysLinear`, `VehPhysGeneral_PhysAngular`,
//! `VehPhysForce_OnApplyForces`, `COLL_MOVED_PlayerSearch`,
//! `COLL_FIXED_PlayerSearch` and `VehPhysGeneral_JumpAndFriction` run in
//! `MainFrame`'s order), and requires the same driver after every tick.
//!
//! The C side is built with `gcc -m32` from `reference/ctr-native`; the test is
//! skipped (with a note) if the reference or the disc image is unavailable.
//! `VEHICLE_TICK_RUNS` and `VEHICLE_TICK_TICKS` override the number of runs per
//! track and ticks per run.
//!
//! The C side does not run the audio and particle stages; the kart is alone, so its
//! `CollideDrivers` only has the turbo pads and the pushback to do.

#[path = "../../loader/tests/common/mod.rs"]
mod common;
#[path = "../../coll/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;

use std::fmt::Write as _;
use std::path::Path;

use coll::Collision;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::{Level, SVec3};
use loader::level::LevelId;
use support::{Rng, build_driver, describe_level, run_reference};
use vehicle::scrub::SCRUBS;
use vehicle::sim::{spawn, tick};
use vehicle::stats::{CHARACTER_ENGINES, META_PHYS};
use vehicle::terrain::TERRAINS;
use vehicle::{Driver, Effect, Env, Input, MatrixArray, PhysState, buttons};

fn quad(q: Option<u32>) -> i64 {
    q.map_or(-1, i64::from)
}

/// The driver's fields in the order of the C driver's `fields` table, with the
/// C expressions that name them.
fn fields(d: &Driver) -> Vec<(&'static str, i64)> {
    let mut f: Vec<(&'static str, i64)> = Vec::new();
    macro_rules! push {
        ($name:expr, $value:expr) => {
            f.push(($name, $value as i64))
        };
    }
    macro_rules! v3 {
        ($name:literal, $v:expr) => {
            push!(concat!($name, ".x"), $v.x);
            push!(concat!($name, ".y"), $v.y);
            push!(concat!($name, ".z"), $v.z);
        };
    }
    macro_rules! matrix {
        ($name:literal, $m:expr) => {
            push!(concat!($name, ".m[0][0]"), $m.m[0][0]);
            push!(concat!($name, ".m[0][1]"), $m.m[0][1]);
            push!(concat!($name, ".m[0][2]"), $m.m[0][2]);
            push!(concat!($name, ".m[1][0]"), $m.m[1][0]);
            push!(concat!($name, ".m[1][1]"), $m.m[1][1]);
            push!(concat!($name, ".m[1][2]"), $m.m[1][2]);
            push!(concat!($name, ".m[2][0]"), $m.m[2][0]);
            push!(concat!($name, ".m[2][1]"), $m.m[2][1]);
            push!(concat!($name, ".m[2][2]"), $m.m[2][2]);
        };
    }

    push!("d.driverID", d.driver_id);
    push!("d.kartState", d.kart_state);
    push!("d.simpTurnState", d.simp_turn_state);
    push!(
        "phys_state",
        match d.phys_state {
            PhysState::Driving => 0,
            PhysState::SlamWall => 1,
            PhysState::FreezeVShift => 2,
            PhysState::PowerSlide => 3,
            PhysState::SpinFirst => 4,
            PhysState::SpinLast => 5,
            PhysState::SpinStop => 6,
            // The C driver stubs the mask grab out (`Env::respawns` is off).
            PhysState::MaskGrab | PhysState::RevEngine =>
                unreachable!("the oracle's drivers do not respawn"),
            PhysState::Tumble => unreachable!("the oracle's drivers are not blasted"),
        }
    );
    push!("init_pending", d.init_pending);
    v3!("d.posCurr", d.pos_curr);
    v3!("d.posPrev", d.pos_prev);
    v3!("d.velocity", d.velocity);
    v3!("d.originToCenter", d.origin_to_center);
    push!("d.collisionFlags", d.collision_flags as i16);
    push!("d.stepFlagSet", d.step_flag_set as i32);
    push!("d.actionsFlagSet", d.actions_flag_set as i32);
    push!(
        "d.actionsFlagSetPrevFrame",
        d.actions_flag_set_prev_frame as i32
    );
    push!("d.currBlockTouching", quad(d.curr_block_touching));
    push!("d.underDriver", quad(d.under_driver));
    push!("d.lastValid", quad(d.last_valid));
    v3!("d.normalVecUP", d.normal_vec_up);
    v3!("d.AxisAngle1_normalVec", d.axis_angle1_normal_vec);
    v3!("d.AxisAngle2_normalVec", d.axis_angle2_normal_vec);
    v3!("d.AxisAngle3_normalVec", d.axis_angle3_normal_vec);
    v3!("d.AxisAngle4_normalVec", d.axis_angle4_normal_vec);
    v3!("d.spsHitPos", d.sps_hit_pos);
    v3!("d.spsNormalVec", d.sps_normal_vec);
    v3!("d.posWallColl", d.pos_wall_coll);
    push!("d.vShiftCount", d.v_shift_count);
    push!("d.speed", d.speed);
    push!("d.speedApprox", d.speed_approx);
    push!("d.baseSpeed", d.base_speed);
    push!("d.fireSpeed", d.fire_speed);
    push!("d.fireSpeedCap", d.fire_speed_cap);
    push!("d.jumpHeightCurr", d.jump_height_curr);
    push!("d.jumpHeightPrev", d.jump_height_prev);
    push!("d.xSpeed", d.x_speed);
    push!("d.ySpeed", d.y_speed);
    push!("d.zSpeed", d.z_speed);
    push!("d.speedometerNeedleValue", d.speedometer_needle_value);
    push!("d.reserves", d.reserves);
    push!("d.turbo_outsideTimer", d.turbo_outside_timer);
    push!("d.turbo_MeterRoomLeft", d.turbo_meter_room_left);
    push!("d.wallRubTimer", d.wall_rub_timer);
    push!("d.wallRubSpeedLimit", d.wall_rub_speed_limit);
    push!("d.angle", d.angle);
    push!("d.rotCurr.w", d.rot_curr_w);
    push!("d.turnAngleCurr", d.turn_angle_curr);
    push!("d.turnAnglePrev", d.turn_angle_prev);
    push!("d.turnAngleLerpVel", d.turn_angle_lerp_vel);
    push!("d.turnAngleLerpTarget", d.turn_angle_lerp_target);
    push!("d.turnWobbleAngle", d.turn_wobble_angle);
    push!("d.turnWobbleVelocity", d.turn_wobble_velocity);
    push!("d.turnWobbleTimer", d.turn_wobble_timer);
    push!("d.ampTurnState", d.amp_turn_state);
    push!("d.numFramesSpentSteering", d.num_frames_spent_steering);
    push!("d.rotationSpinRate", d.rotation_spin_rate);
    push!("d.VehFire_AudioCooldown", d.veh_fire_audio_cooldown);
    push!("d.Screen_OffsetY", d.screen_offset_y);
    push!("d.distanceFromGround", d.distance_from_ground);
    push!("d.reserved_0x40e", d.reserved_0x40e);
    push!("d.jumpSquishStretch", d.jump_squish_stretch);
    push!("d.jumpSquishStretch2", d.jump_squish_stretch2);
    push!("inst.animIndex", d.anim_index);
    push!("inst.animFrame", d.anim_frame);
    push!("inst.scale.x", d.inst_scale_x);
    push!("inst.scale.y", d.inst_scale_y);
    push!(
        "d.matrixArray",
        match d.matrix_array {
            MatrixArray::None => 0,
            MatrixArray::WheelieStart => 1,
            MatrixArray::WheelieHold => 2,
            MatrixArray::WheelieRecover => 3,
            MatrixArray::CrashFall => 4,
            MatrixArray::SquishRecover => 5,
            MatrixArray::Blasted => 6,
            MatrixArray::Jump(n) => n,
        }
    );
    push!("d.matrixIndex", d.matrix_index);
    push!("d.numWumpas", d.num_wumpas);
    push!("d.accelConst", d.accel_const);
    push!("d.turnConst", d.turn_const);
    push!("d.turboConst", d.turbo_const);
    push!("d.clockReceive", d.clock_receive);
    push!("d.hazardTimer", d.hazard_timer);
    push!("d.rainCloudEffect", d.rain_cloud_effect);
    push!("d.burnTimer", d.burn_timer);
    push!("d.squishTimer", d.squish_timer);
    push!("d.superEngineTimer", d.super_engine_timer);
    push!("d.accelTapWindowTimer", d.accel_tap_window_timer);
    push!("d.accelTapCount", d.accel_tap_count);
    push!("d.vShiftStartGuardTimer", d.v_shift_start_guard_timer);
    push!("d.vShiftWindowTimer", d.v_shift_window_timer);
    push!("d.driverRank", d.driver_rank);
    push!("d.jump_TenBuffer", d.jump_ten_buffer);
    push!("d.jump_CooldownMS", d.jump_cooldown_ms);
    push!("d.jump_CoyoteTimerMS", d.jump_coyote_timer_ms);
    push!("d.jump_ForcedMS", d.jump_forced_ms);
    push!("d.jump_InitialVelY", d.jump_initial_vel_y);
    push!("d.jump_HighJumpTimerMS", d.jump_high_jump_timer_ms);
    push!("d.jump_LandingBoost", d.jump_landing_boost);
    push!("d.numberOfJumps", d.number_of_jumps);
    push!("d.jumpMeter", d.jump_meter);
    push!("d.jumpMeterTimer", d.jump_meter_timer);
    push!("d.forcedJumpType", d.forced_jump_type);
    push!("d.buttonUsedToStartDrift", d.button_used_to_start_drift);
    push!("d.rotCurr.x", d.rot_curr_x);
    push!("d.rotCurr.y", d.rot_curr_y);
    push!("d.rotCurr.z", d.rot_curr_z);
    push!("d.rotPrev.x", d.rot_prev[0]);
    push!("d.rotPrev.y", d.rot_prev[1]);
    push!("d.rotPrev.z", d.rot_prev[2]);
    push!("d.rotPrev.w", d.rot_prev[3]);
    matrix!("d.matrixMovingDir", d.matrix_moving_dir);
    matrix!("d.matrixFacingDir", d.matrix_facing_dir);
    push!("terrainMeta1", d.terrain_meta1);
    push!("terrainMeta2", d.terrain_meta2);
    push!("d.currentTerrain", d.current_terrain);
    push!("d.terrainFrictionTimer", d.terrain_friction_timer);
    push!("d.terrainScaledBaseSpeed", d.terrain_scaled_base_speed);
    push!("d.normalVecID", d.normal_vec_id);
    push!("d.axisRotationX", d.axis_rotation_x);
    push!("d.axisRotationY", d.axis_rotation_y);
    push!("d.forwardDir", d.forward_dir);
    v3!("d.forwardAccelVector", d.forward_accel_vector);
    push!("d.forwardAccelImpulse", d.forward_accel_impulse);
    v3!("d.accel", d.accel);
    push!("d.wheelRotation", d.wheel_rotation);
    push!(
        "d.boolFirstFrameSinceRevEngine",
        d.bool_first_frame_since_rev_engine
    );
    push!("d.quadBlockHeight", d.quad_block_height);
    push!(
        "d.distanceToFinish_checkpoint",
        d.distance_to_finish_checkpoint
    );
    push!("d.timeUntilDriftSpinout", d.time_until_drift_spinout);
    push!("d.previousFrameMultDrift", d.previous_frame_mult_drift);
    push!("d.multDrift", d.mult_drift);
    push!("d.NoInputTimer", d.no_input_timer);
    push!("d.failedBoostExhaustTimer", d.failed_boost_exhaust_timer);
    push!(
        "d.KartStates.Drifting.numFramesDrifting",
        d.num_frames_drifting
    );
    push!(
        "d.KartStates.Drifting.driftBoostTimeMS",
        d.drift_boost_time_ms
    );
    push!(
        "d.KartStates.Drifting.driftTotalTimeMS",
        d.drift_total_time_ms
    );
    push!(
        "d.KartStates.Drifting.numBoostsAttempted",
        d.num_boosts_attempted
    );
    push!(
        "d.KartStates.Drifting.numBoostsSuccess",
        d.num_boosts_success
    );
    f
}

fn effect_line(e: &Effect, d: &Driver) -> String {
    match *e {
        Effect::Sound { id, echo } => format!(" S {id} {}", echo as u8),
        Effect::Voiceline {
            id,
            driver_id: _,
            priority,
        } => format!(" V {id} {} {priority}", d.character),
        Effect::JogCon {
            driver_id,
            pattern,
            strength,
        } => format!(" J {driver_id} {pattern} {strength}"),
        Effect::ShockFreq { driver_id, a, b } => format!(" F {driver_id} {a} {b}"),
        Effect::ShockForce { driver_id, a, b } => format!(" G {driver_id} {a} {b}"),
        Effect::DriverCrashing { echo, volume } => format!(" C {} {volume}", echo as u8),
        // Camera state is not part of the comparison.
        Effect::CameraFireSpeedZoom { .. } => String::new(),
        // The C oracle has no turbo fire, which plays the boost.
        Effect::FireAudio { .. } => String::new(),
        // Nor does it run the emitter or the turbo fire, which keep the loops going.
        Effect::Recycle { .. } => String::new(),
    }
}

/// The level parts the coll driver's blocks leave out.
fn extras_script(level: &Level) -> String {
    let mut s = String::from("E");
    for q in &level.quad_blocks {
        write!(s, " {} {}", q.mul_norm_vec_y, q.checkpoint_index).unwrap();
    }
    write!(
        s,
        " {} {} {}",
        level.jump_vertical_speed_cap,
        level.config_flags as i32,
        level.restart_points.len()
    )
    .unwrap();
    for p in &level.restart_points {
        write!(s, " {} {}", p.dist_to_finish, p.next_forward).unwrap();
    }
    s
}

fn tables_script() -> String {
    let mut s = String::from("K");
    for scrub in &SCRUBS {
        write!(
            s,
            " {} {} {}",
            scrub.flags, scrub.speed_limit, scrub.impact_angle
        )
        .unwrap();
    }
    s.push_str("\nT");
    for angle in 0..0x400 {
        let (sin, cos) = math::sin_cos(angle);
        write!(s, " {sin} {cos}").unwrap();
    }
    s.push_str("\nR");
    for t in &TERRAINS {
        write!(
            s,
            " {} {} {} {} {} {} {} {} {}",
            t.flags,
            t.speed_multiplier,
            t.slow_until_speed,
            t.counter_steer_ratio,
            t.turn_lean_scale,
            t.ground_friction_scale,
            t.turn_angle_scale,
            t.turn_response_scale,
            t.sound
        )
        .unwrap();
    }
    s.push('\n');
    s
}

struct Frame {
    env: Env,
    input: Input,
}

impl Frame {
    fn script(&self) -> String {
        format!(
            "X {} {} {} {} {} {} {} {} {}",
            self.env.elapsed_time_ms,
            self.env.elapsed_event_time,
            self.env.timer,
            self.env.end_of_race as u8,
            self.input.buttons_held,
            self.input.buttons_tapped,
            self.input.stick_lx,
            self.input.stick_ly,
            self.input.stick_ry
        )
    }
}

/// A run: a kart of a character placed on a track, and the inputs it is driven with.
struct Run {
    character: usize,
    driver: Driver,
    frames: Vec<Frame>,
}

/// Driving habits: how the sticks and buttons change over a run.
fn frames(rng: &mut Rng, ticks: usize) -> Vec<Frame> {
    let mut out = Vec::new();
    let mut held = 0u32;
    let (mut hop_ticks, mut hop_button) = (0, 0u32);
    let (mut lx, mut ly, mut ry) = (0x80u8, 0x80u8, 0x80u8);
    for t in 0..ticks {
        let prev = held;
        // Re-roll the pedals and the stick every few ticks.
        if rng.below(12) == 0 {
            held &= !(buttons::CROSS | buttons::SQUARE);
            match rng.below(10) {
                0..=6 => held |= buttons::CROSS,
                7 => held |= buttons::SQUARE,
                8 => held |= buttons::CROSS | buttons::SQUARE,
                _ => {}
            }
            ry = if rng.below(8) == 0 {
                rng.range(0, 255) as u8
            } else {
                0x80
            };
        }
        if rng.below(8) == 0 {
            lx = match rng.below(5) {
                0 => 0x80,
                1 => 0,
                2 => 0xff,
                _ => rng.range(0, 255) as u8,
            };
        }
        if rng.below(30) == 0 {
            ly = [0x80, 0x80, 0, 0xff, 0xf0][rng.below(5) as usize];
        }
        // Hops: held for a while, so that a landing can start a drift; the
        // other button is tapped now and then to cash in the turbo meter.
        if hop_ticks > 0 {
            hop_ticks -= 1;
            if hop_ticks == 0 {
                held &= !(buttons::L1 | buttons::R1);
            } else if rng.below(14) == 0 {
                held ^= (buttons::L1 | buttons::R1) & !hop_button;
            }
        } else if rng.below(40) < 3 {
            hop_button =
                [buttons::L1, buttons::R1, buttons::L1 | buttons::R1][rng.below(3) as usize];
            held = (held & !(buttons::L1 | buttons::R1)) | hop_button;
            hop_ticks = rng.range(2, 140);
        }
        let env = Env {
            elapsed_time_ms: 32,
            elapsed_event_time: t as i32 * 32,
            timer: t as u32,
            end_of_race: rng.below(500) == 0,
            respawns: false,
            kart_collisions: true,
            traffic_lights_timer: 0,
        };
        out.push(Frame {
            env,
            input: Input {
                buttons_held: held,
                buttons_tapped: held & !prev,
                stick_lx: lx,
                stick_ly: ly,
                stick_ry: ry,
            },
        });
    }
    out
}

/// A driver who floors it, hops and drifts for a long time: hard steering with
/// the hop button held, the other button tapped now and then for the boost.
fn drifter_frames(rng: &mut Rng, ticks: usize) -> Vec<Frame> {
    let mut out = Vec::new();
    let mut held = buttons::CROSS;
    let (mut wait, mut hold, mut hop_button) = (rng.range(30, 90), 0, buttons::R1);
    let mut lx = 0xffu8;
    for t in 0..ticks {
        let prev = held;
        if hold > 0 {
            hold -= 1;
            if hold == 0 {
                held &= !(buttons::L1 | buttons::R1);
                wait = rng.range(10, 60);
            } else if rng.below(25) == 0 {
                held ^= (buttons::L1 | buttons::R1) & !hop_button;
            }
        } else if wait > 0 {
            wait -= 1;
            if wait == 0 {
                hop_button = [buttons::L1, buttons::R1][rng.below(2) as usize];
                held |= hop_button;
                hold = rng.range(40, 200);
            }
        }
        if rng.below(45) == 0 {
            lx = [0, 0xff, 0x20, 0xe0, 0x80][rng.below(5) as usize];
        }
        let env = Env {
            elapsed_time_ms: 32,
            elapsed_event_time: t as i32 * 32,
            timer: t as u32,
            end_of_race: false,
            respawns: false,
            kart_collisions: true,
            traffic_lights_timer: 0,
        };
        out.push(Frame {
            env,
            input: Input {
                buttons_held: held,
                buttons_tapped: held & !prev,
                stick_lx: lx,
                ..Input::default()
            },
        });
    }
    out
}

fn driver_script(d: &Driver, character: usize) -> String {
    let mut s = String::from("S");
    let class = CHARACTER_ENGINES[character] as usize;
    for row in META_PHYS {
        write!(s, " {}", row[class]).unwrap();
    }
    write!(s, "\nC {}\nA", d.character).unwrap();
    for n in d.anim_num_frames {
        write!(s, " {n}").unwrap();
    }
    write!(s, "\nD").unwrap();
    for (_, v) in fields(d) {
        write!(s, " {v}").unwrap();
    }
    s.push('\n');
    s
}

fn tick_line(d: &Driver, c: &Collision, effects: &[Effect], mask_grab: bool) -> String {
    let mut o = String::new();
    for (_, v) in fields(d) {
        write!(o, " {v}").unwrap();
    }
    o.push_str(" |");
    for e in effects {
        o.push_str(&effect_line(e, d));
    }
    let s = &c.sps;
    write!(
        o,
        " | {} {} {} {} {} {}",
        s.hit_fraction,
        s.input1.hit_radius,
        s.collision.step_flags as i32,
        s.did_touch_quadblock,
        s.num_triangles_tested,
        mask_grab as u8
    )
    .unwrap();
    o
}

fn runs() -> usize {
    std::env::var("VEHICLE_TICK_RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16)
}

fn ticks() -> usize {
    std::env::var("VEHICLE_TICK_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600)
}

/// Spawn points and random places on the track.
fn make_runs(level: &Level, coll: &mut Collision, rng: &mut Rng) -> Vec<Run> {
    let mut out = Vec::new();
    for i in 0..runs() {
        let character = (i * 5 + rng.below(3) as usize) % 16;
        let mut driver = Driver::new(0, character);
        driver.anim_num_frames = [30, 12, 20, 16, 24, 8, 8, 8];
        spawn(&mut driver, coll, level, 0);
        if i > 0 {
            // Somewhere else on the track, just above it, facing any way.
            let q = &level.quad_blocks[rng.below(level.quad_blocks.len() as u64) as usize];
            let (a, b) = (q.bbox.min, q.bbox.max);
            let at = |rng: &mut Rng, lo: i16, hi: i16| rng.range(lo as i32, hi as i32);
            driver.pos_curr.x = at(rng, a.x, b.x) << 8;
            driver.pos_curr.z = at(rng, a.z, b.z) << 8;
            driver.pos_curr.y = (b.y as i32 + rng.range(0x10, 0x60)) << 8;
            driver.pos_prev = driver.pos_curr;
            driver.angle = rng.range(0, 0xfff) as i16;
            driver.rot_curr_y = driver.angle;
            driver.rot_prev = [0, driver.angle, 0, 0];
            driver.last_valid = None;
            driver.under_driver = None;
            driver.axis_angle1_normal_vec = SVec3 {
                x: 0,
                y: 0x1000,
                z: 0,
            };
            driver.axis_angle2_normal_vec = SVec3 {
                x: 0,
                y: 0x1000,
                z: 0,
            };
            driver.axis_angle3_normal_vec = SVec3 {
                x: 0,
                y: 0x1000,
                z: 0,
            };
            driver.axis_angle4_normal_vec = SVec3 {
                x: 0,
                y: 0x1000,
                z: 0,
            };
        }
        let frames = if i % 2 == 1 {
            drifter_frames(rng, ticks())
        } else {
            frames(rng, ticks())
        };
        out.push(Run {
            character,
            driver,
            frames,
        });
    }
    out
}

/// A copy of a reference source with `edits` applied, for the places where
/// ctr-native cannot be compared with the port as it is. Each edit must match
/// exactly once, so a changed reference is noticed. The copy goes next to the C
/// driver; the reference itself is left alone. Returns the copy's path.
fn patched(source: &str, edits: &[(&str, &str)]) -> Option<std::path::PathBuf> {
    let path = support::reference_dir().join(source);
    let mut text = std::fs::read_to_string(&path).ok()?;
    for (old, new) in edits {
        assert_eq!(
            text.matches(old).count(),
            1,
            "{source} changed in the reference; update this patch: {old}"
        );
        text = text.replace(old, new);
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "patched_{}",
        Path::new(source).file_name()?.to_str()?
    ));
    std::fs::write(&out, text).unwrap();
    Some(out)
}

/// `VehPhysProc.c` with the one retail behaviour ctr-native loses restored: at
/// `gasClearSteerFlags` it clears the reverse-steer latches in
/// `actionsFlagSetNext`, but `gasDone` then overwrites that with the unmasked
/// copy. (Retail has one register for both; the port follows retail.)
fn retail_veh_phys_proc() -> Option<std::path::PathBuf> {
    let header = format!(
        "#include \"{}\"",
        support::reference_dir()
            .join("game/Vehicle/VehCommon.h")
            .display()
    );
    patched(
        "game/Vehicle/VehPhysProc.c",
        &[
            (
                "actionsFlagSetNext = actionsFlagSetCopy & ~(ACTION_REVERSE_STEER_LEFT | ACTION_REVERSE_STEER_RIGHT);",
                "actionsFlagSetCopy &= ~(ACTION_REVERSE_STEER_LEFT | ACTION_REVERSE_STEER_RIGHT);",
            ),
            ("#include \"VehCommon.h\"", &header),
        ],
    )
}

/// `COLL.c` with `ScrubImpact`'s LLV0 light matrix loaded with the identity
/// before it is used. Retail uses whatever the renderer last left in the GTE
/// (and the rest of the tick overwrites it, too); the port passes
/// `scrub::IDENTITY_LIGHT_MATRIX` instead.
fn coll_with_identity_light_matrix() -> Option<std::path::PathBuf> {
    patched(
        "game/COLL.c",
        &[(
            "internal Vec3 CollMoved_ScrubImpact_GteLLV0(s32 x, s32 y, s32 z)\n{\n",
            "internal Vec3 CollMoved_ScrubImpact_GteLLV0(s32 x, s32 y, s32 z)\n{\n\tCTC2(0x1000, 8);\n\tCTC2(0, 9);\n\tCTC2(0x1000, 10);\n\tCTC2(0, 11);\n\tCTC2(0x1000, 12);\n",
        )],
    )
}

#[test]
fn matches_reference_on_tracks() {
    let driver_c = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/tick.c");
    let (Some(proc_c), Some(coll_c)) = (retail_veh_phys_proc(), coll_with_identity_light_matrix())
    else {
        return;
    };
    let Some(exe) = build_driver(
        &driver_c,
        "vehicle_tick",
        &[
            coll_c.to_str().unwrap(),
            "game/COLL/COLL_InstanceQuery.c",
            "game/Vehicle/VehCalc.c",
            "game/Vehicle/VehFrame.c",
            "game/Vehicle/VehFire.c",
            "game/231/RB_Player.c",
            "game/Vehicle/VehAfterColl.c",
            proc_c.to_str().unwrap(),
            "game/Vehicle/VehPhysGeneral.c",
            "game/PROC.c",
            "game/UI/UI_Meter.c",
            "game/MATH/MATH_2_FastSqrt.c",
            "game/Vehicle/VehPhysForce.c",
            "game/Vehicle/VehPhysCrash.c",
            "game/Vehicle/VehPhysJoystick.c",
        ],
    ) else {
        return;
    };
    let Some(mut b) = common::open_bigfile() else {
        return;
    };

    // The two sides must name the driver fields identically, in the same order.
    let names = run_reference(&exe, "N\n");
    let probe = Driver::new(0, 0);
    let rust_names: Vec<&str> = fields(&probe).iter().map(|(n, _)| *n).collect();
    assert_eq!(names.len(), 1);
    assert_eq!(
        names[0].split(' ').collect::<Vec<_>>(),
        rust_names,
        "the driver fields differ between the C and Rust test drivers"
    );

    let mut rng = Rng(0x7d1c_e5a9_0b13_4f65);
    // (ticks, ticks airborne, slams, mask grabs, drove into reverse, wall rubs, jumps, froze,
    // drifting, spinning, spin winding down, spin stopping, boosted by a drift)
    let mut totals = [0u32; 13];

    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let idx = BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev);
        let file = b.big.read(&mut b.disc, idx.get()).unwrap();
        let level = Level::parse(&DramFile::parse(&file).unwrap()).unwrap();

        // The scratchpad and the GTE carry over from run to run, in both.
        // (Spawning searches the level on a scratchpad of its own.)
        let runs = make_runs(&level, &mut Collision::default(), &mut rng);
        let mut script = describe_level(&level);
        script.push_str(&extras_script(&level));
        script.push('\n');
        script.push_str(&tables_script());
        for run in &runs {
            script.push_str(&driver_script(&run.driver, run.character));
            for f in &run.frames {
                script.push_str(&f.script());
                script.push('\n');
            }
        }
        let reference = run_reference(&exe, &script);
        let total: usize = runs.iter().map(|r| r.frames.len()).sum();
        assert_eq!(
            reference.len(),
            total,
            "level {id}: the C driver printed {} lines for {total} ticks",
            reference.len()
        );

        // Rust side, with the same carried-over collision state: the C driver
        // starts with a zeroed scratchpad.
        let mut collision = Collision::default();
        let mut line = 0;
        let mut failures = 0;
        for (r, run) in runs.iter().enumerate() {
            let mut d = run.driver.clone();
            for (t, f) in run.frames.iter().enumerate() {
                let mut effects = Vec::new();
                let result = tick(
                    &mut d,
                    &mut collision,
                    &level,
                    &f.env,
                    &f.input,
                    &mut effects,
                );
                let got = tick_line(&d, &collision, &effects, result.mask_grab);
                let want = reference[line].trim_end();
                line += 1;

                totals[0] += 1;
                totals[1] += (d.actions_flag_set & vehicle::actions::TOUCH_GROUND == 0) as u32;
                totals[2] += (d.phys_state == PhysState::SlamWall) as u32;
                totals[3] += result.mask_grab as u32;
                totals[4] += (d.speed_approx < -0x100) as u32;
                totals[5] += (d.wall_rub_timer != 0) as u32;
                totals[6] += (d.actions_flag_set & vehicle::actions::JUMP_STARTED != 0) as u32;
                totals[7] += (d.phys_state == PhysState::FreezeVShift) as u32;
                totals[8] += (d.phys_state == PhysState::PowerSlide) as u32;
                totals[9] += (d.phys_state == PhysState::SpinFirst) as u32;
                totals[10] += (d.phys_state == PhysState::SpinLast) as u32;
                totals[11] += (d.phys_state == PhysState::SpinStop) as u32;
                totals[12] +=
                    (d.phys_state == PhysState::PowerSlide && d.num_boosts_success > 0) as u32;

                if got.trim_start() != want.trim_start() {
                    failures += 1;
                    if failures <= 2 {
                        let names: Vec<&str> = fields(&d).iter().map(|(n, _)| *n).collect();
                        let rust: Vec<&str> = got.trim_start().split(' ').collect();
                        let c: Vec<&str> = want.trim_start().split(' ').collect();
                        let first = (0..names.len()).find(|&i| rust.get(i) != c.get(i));
                        eprintln!(
                            "level {id}, run {r} (character {}), tick {t}: MISMATCH at {}\n  rust: {}\n  ref:  {}",
                            run.character,
                            first.map_or("the effects or scratchpad".to_owned(), |i| format!(
                                "{} (rust {}, ref {})",
                                names[i], rust[i], c[i]
                            )),
                            got,
                            want
                        );
                    }
                    break; // later ticks of this run would only repeat the divergence
                }
            }
            // After a mismatch, skip this run's remaining C lines.
            line = starts_line(&runs, r + 1);
        }
        eprintln!(
            "level {id}: {} runs, {} ticks, {failures} runs diverge",
            runs.len(),
            total
        );
        assert_eq!(
            failures, 0,
            "level {id}: {failures} runs differ from the reference"
        );
    }
    eprintln!(
        "total (ticks, airborne, slam, mask grab, reverse, wall rub, jump, v-shift freeze, drift, spin, spin end, spin stop, drift boost): {totals:?}"
    );
    assert!(
        totals[1] > 0 && totals[2] > 0 && totals[5] > 0 && totals[6] > 0,
        "the runs do not cover airborne, slams, wall rubs and jumps: {totals:?}"
    );
    // Spinning out is compared when a run happens to reach it, but deliberately not required: it
    // is rare in random driving and matters little for the cups.
    assert!(
        totals[8] > 0 && totals[12] > 0,
        "the runs do not cover drifting and drift boosts: {totals:?}"
    );
}

/// The first C output line of run `r` (the runs print one line per tick).
fn starts_line(runs: &[Run], r: usize) -> usize {
    runs[..r.min(runs.len())]
        .iter()
        .map(|run| run.frames.len())
        .sum()
}
