//! The crash state (`game/Vehicle/VehPhysProc.c`, `VehPhysProc_SlamWall_*`).

use crate::{Driver, PhysState, kart_state};

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
}
