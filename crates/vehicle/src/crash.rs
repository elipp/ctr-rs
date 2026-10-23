//! The crash state (`game/Vehicle/VehPhysProc.c`, `VehPhysProc_SlamWall_*`).

use math::{Vec3, mips};

use crate::{Driver, PhysState, calc, kart_state};

/// `VEH_PHYS_PROC_STEER_RESET_FRAMES`
const STEER_RESET_FRAMES: i16 = 10000;
/// `VEH_PHYS_PROC_CRASH_SCALE_XY`
const CRASH_SCALE_XY: i16 = 0xccc;

/// `VehPhysProc_SlamWall_Init`: the driver hit a wall hard. Stops the kart and
/// switches it to the slam-wall procedures.
pub fn slam_wall_init(d: &mut Driver) {
    d.kart_state = kart_state::CRASHING;
    d.num_frames_spent_steering = STEER_RESET_FRAMES;
    d.turbo_meter_room_left = 0;
    d.rotation_spin_rate = 0;
    d.turn_angle_lerp_vel = 0;
    d.turn_wobble_angle = 0;
    d.turn_wobble_velocity = 0;
    d.turn_wobble_timer = 0;
    d.amp_turn_state = 0;
    d.speed = 0;
    d.speed_approx = 0;

    d.x_speed = 0;
    d.y_speed = 0;
    d.z_speed = 0;
    d.velocity = math::Vec3::default();
    d.speedometer_needle_value = 0;
    d.reserves = 0;
    d.turbo_outside_timer = 0;
    d.veh_fire_audio_cooldown = 0;
    d.screen_offset_y = 0;
    d.distance_from_ground = 0;
    d.reserved_0x40e = 0;
    d.jump_squish_stretch2 = 0;
    d.jump_squish_stretch = 0;
    d.base_speed = 0;
    d.fire_speed = 0;

    d.inst_scale_y = CRASH_SCALE_XY;
    d.inst_scale_x = CRASH_SCALE_XY;

    d.phys_state = PhysState::SlamWall;
    // Nothing pending: a drift that started this tick is over before it armed.
    d.init_pending = false;
}

/// `VEH_PHYS_CRASH_FAST_SQRT_ITERATIONS`
const FAST_SQRT_ITERATIONS: u32 = 0x10;
/// `VEH_PHYS_CRASH_VECTOR_SPEED_SHIFT`
const VECTOR_SPEED_SHIFT: u32 = 8;
/// `VEH_PHYS_CRASH_MATRIX_FRAC_SHIFT`
const MATRIX_FRAC_SHIFT: u32 = 12;

fn length_sq3(x: i32, y: i32, z: i32) -> u32 {
    mips::add_lo(
        mips::add_lo(mips::mul_lo(x, x), mips::mul_lo(y, y)),
        mips::mul_lo(z, z),
    ) as u32
}

fn dot3(a: [i32; 3], b: [i32; 3]) -> i32 {
    mips::add_lo(
        mips::add_lo(mips::mul_lo(a[0], b[0]), mips::mul_lo(a[1], b[1])),
        mips::mul_lo(a[2], b[2]),
    )
}

/// `VehPhysCrash_ConvertVecToSpeed`: a velocity as the driver's speed (its
/// length), the two angles of its direction, the height it moves above the
/// surface (`jump_height_curr`: the part along the surface normal), and the
/// speed along the ground (`speed_approx`, negative when reversing).
pub fn convert_vec_to_speed(d: &mut Driver, vel: &Vec3) {
    let speed_2d = calc::fast_sqrt(length_sq3(vel.x, 0, vel.z), FAST_SQRT_ITERATIONS) as i32;
    let speed = calc::fast_sqrt(length_sq3(vel.x, vel.y, vel.z), FAST_SQRT_ITERATIONS)
        >> VECTOR_SPEED_SHIFT;
    d.speed = speed as i16;
    d.axis_rotation_y = math::ratan2(mips::sll(vel.y, VECTOR_SPEED_SHIFT), speed_2d) as i16;
    d.axis_rotation_x = math::ratan2(vel.x, vel.z) as i16;

    // The second column of matrixMovingDir is the surface normal, the third the way forwards.
    let m = &d.matrix_moving_dir.m;
    let normal = [m[0][1] as i32, m[1][1] as i32, m[2][1] as i32];
    let forward = [m[0][2] as i32, m[1][2] as i32, m[2][2] as i32];

    let dot = dot3([vel.x, vel.y, vel.z], normal);
    let along_normal = mips::sra(dot, MATRIX_FRAC_SHIFT);
    let proj = normal.map(|n| mips::sra(mips::mul_lo(n, along_normal), MATRIX_FRAC_SHIFT));

    let jump_speed = calc::fast_sqrt(length_sq3(proj[0], proj[1], proj[2]), FAST_SQRT_ITERATIONS)
        >> VECTOR_SPEED_SHIFT;
    d.jump_height_curr = jump_speed as i16;
    if along_normal < 0 {
        d.jump_height_curr = mips::neg_lo(jump_speed as i32) as i16;
    }

    let ground = [
        mips::sub_lo(vel.x, proj[0]),
        mips::sub_lo(vel.y, proj[1]),
        mips::sub_lo(vel.z, proj[2]),
    ];
    let approx = calc::fast_sqrt(
        length_sq3(ground[0], ground[1], ground[2]),
        FAST_SQRT_ITERATIONS,
    ) >> VECTOR_SPEED_SHIFT;
    d.speed_approx = approx as i16;
    if dot3(ground, forward) < 0 {
        d.speed_approx = mips::neg_lo(approx as i32) as i16;
    }
}
