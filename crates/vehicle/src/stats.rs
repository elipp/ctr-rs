//! `data.metaPhys`: the per-engine-class physics constants, copied into the driver by
//! `VehBirth_SetConsts`, and `data.MetaDataCharacters[].engineID`.

/// `EngineClass`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineClass {
    Balanced = 0,
    Accel = 1,
    Speed = 2,
    Turn = 3,
}

/// `MetaDataCharacters[character].engineID`, in `GAME_CHARACTER_IDS` order
/// (crash, cortex, tiny, coco, ngin, dingo, polar, pura, pinstripe, papu, roo,
/// joe, ntropy, penta, fake crash, oxide).
pub const CHARACTER_ENGINES: [EngineClass; 16] = {
    use EngineClass::*;
    [
        Balanced, Balanced, Speed, Accel, Accel, Speed, Turn, Turn, Accel, Speed, Turn, Balanced,
        Speed, Turn, Balanced, Balanced,
    ]
};

/// The `Driver::const_*` fields. Named after the C fields; types are the C
/// types, which matter where the reference reads them with a cast.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub gravity: i16,
    pub jump_force: i16,
    pub pedal_friction_perpendicular: i16,
    pub pedal_friction_forward: i16,
    pub no_pedal_friction_perpendicular: i16,
    pub no_pedal_friction_forward: i16,
    pub brake_friction: i16,
    pub drift_curve: i16,
    pub drift_friction: i16,
    pub accel_class_stat: i16,
    pub accel_reserves: i16,
    pub speed_class_stat: i16,
    pub accel_speed_class_stat: i16,
    pub single_turbo_speed: i16,
    pub sacred_fire_speed: i16,
    pub backward_speed: i16,
    pub mask_speed: i16,
    pub damaged_speed: i16,
    pub turn_rate: u8,
    pub backward_turn_rate: u8,
    pub turn_decrease_rate: i16,
    pub turn_input_delay: i16,
    pub pre_turbo: i16,
    pub terminal_velocity: i16,
    pub terrain_friction_boost: i16,
    pub steer_accel_stage4_first_frame: i8,
    pub steer_accel_stage2_first_frame: i8,
    pub steer_accel_stage2_frame_length: i8,
    pub steer_accel_stage1_max_steer: i16,
    pub steer_accel_stage1_min_steer: i16,
    pub steer_accel_turn_vel_scale: i16,
    pub steer_accel_turn_vel_limit: i16,
    pub model_rot_vel_max: i16,
    pub model_rot_vel_min: i16,
    pub model_turn_counter_steer_strength: u8,
    pub model_turn_return_strength: u8,
    pub model_turn_negative_return_strength: u8,
    pub model_turn_velocity_lerp: u8,
    pub turn_resist_min: u8,
    pub turn_resist_max: u8,
    pub steer_vel_drift_switch_way: i8,
    pub steer_vel_drift_standard: i8,
    pub drift_turn_base: i8,
    pub drift_turn_startup_scale: i8,
    pub drift_turn_ramp_frames: u8,
    pub drifting_frames_till_spinout: u8,
    pub drift_spin_rate_accel: i16,
    pub drift_spin_rate_decel: i16,
    pub drifting_camera_spin_rate: i16,
    pub drift_camera_lerp_step: u8,
    pub drift_release_turn_assist_frames: u8,
    pub meta_phys_33: i16,
    pub meta_phys_34: i16,
    pub drift_turn_same_direction_angle: i16,
    pub drift_turn_opposite_direction_angle: i16,
    pub drift_turn_angle_scale: i16,
    pub turbo_max_room: u8,
    pub turbo_low_room_warning: u8,
    pub turbo_full_bar_reserve_gain: u8,
    pub drift_boost_duration_frames: u8,
    pub drift_boost_axis_kick_rate: u8,
    pub collision_weight: i16,
    pub slope_forward_speed_bonus: i16,
    pub side_speed_clamp: i16,
    pub prototype_key: i32,
}

/// One `MetaPhys` row per field of [`Stats`], with the value for each engine class
/// (balanced, accel, speed, turn).
pub const META_PHYS: [[i32; 4]; 65] = [
    [900, 900, 900, 900],         // gravity
    [4596, 4596, 4596, 4596],     // jump_force
    [350, 350, 350, 350],         // pedal_friction_perpendicular
    [120, 120, 120, 120],         // pedal_friction_forward
    [425, 425, 425, 425],         // no_pedal_friction_perpendicular
    [120, 120, 120, 120],         // no_pedal_friction_forward
    [260, 260, 260, 260],         // brake_friction
    [120, 120, 120, 120],         // drift_curve
    [120, 120, 120, 120],         // drift_friction
    [480, 544, 448, 512],         // accel_class_stat
    [1152, 1152, 1152, 1152],     // accel_reserves
    [13140, 13520, 13900, 12950], // speed_class_stat
    [14640, 15020, 15400, 14450], // accel_speed_class_stat
    [2048, 2048, 2048, 2048],     // single_turbo_speed
    [4096, 4096, 4096, 4096],     // sacred_fire_speed
    [5120, 5120, 5120, 5120],     // backward_speed
    [3900, 3900, 3900, 3900],     // mask_speed
    [4096, 4096, 4096, 4096],     // damaged_speed
    [28, 26, 24, 30],             // turn_rate
    [48, 48, 48, 48],             // backward_turn_rate
    [4080, 9080, 5666, 7252],     // turn_decrease_rate
    [5000, 4500, 4000, 5500],     // turn_input_delay
    [1000, 750, 500, 1250],       // pre_turbo
    [20480, 20480, 20480, 20480], // terminal_velocity
    [256, 256, 256, 256],         // terrain_friction_boost
    [64, 64, 64, 64],             // steer_accel_stage4_first_frame
    [4, 4, 4, 4],                 // steer_accel_stage2_first_frame
    [8, 8, 8, 8],                 // steer_accel_stage2_frame_length
    [3072, 3072, 3072, 3072],     // steer_accel_stage1_max_steer
    [0, 0, 0, 0],                 // steer_accel_stage1_min_steer
    [128, 128, 128, 128],         // steer_accel_turn_vel_scale
    [0, 0, 0, 0],                 // steer_accel_turn_vel_limit
    [350, 350, 350, 350],         // model_rot_vel_max
    [450, 450, 450, 450],         // model_rot_vel_min
    [30, 30, 30, 30],             // model_turn_counter_steer_strength
    [20, 20, 20, 20],             // model_turn_return_strength
    [60, 60, 60, 60],             // model_turn_negative_return_strength
    [10, 10, 10, 10],             // model_turn_velocity_lerp
    [32, 32, 32, 32],             // turn_resist_min
    [224, 224, 224, 224],         // turn_resist_max
    [-10, -10, -10, -10],         // steer_vel_drift_switch_way
    [22, 22, 22, 22],             // steer_vel_drift_standard
    [14, 10, 5, 18],              // drift_turn_base
    [100, 100, 100, 100],         // drift_turn_startup_scale
    [60, 60, 60, 60],             // drift_turn_ramp_frames
    [60, 60, 60, 60],             // drifting_frames_till_spinout
    [3000, 3000, 3000, 3000],     // drift_spin_rate_accel
    [500, 500, 500, 500],         // drift_spin_rate_decel
    [384, 384, 384, 384],         // drifting_camera_spin_rate
    [16, 16, 16, 16],             // drift_camera_lerp_step
    [15, 15, 15, 15],             // drift_release_turn_assist_frames
    [8192, 8192, 8192, 8192],     // meta_phys_33
    [8192, 8192, 8192, 8192],     // meta_phys_34
    [300, 300, 300, 300],         // drift_turn_same_direction_angle
    [-128, -128, -128, -128],     // drift_turn_opposite_direction_angle
    [768, 768, 768, 768],         // drift_turn_angle_scale
    [30, 30, 30, 30],             // turbo_max_room
    [15, 15, 15, 15],             // turbo_low_room_warning
    [60, 60, 60, 60],             // turbo_full_bar_reserve_gain
    [15, 15, 15, 15],             // drift_boost_duration_frames
    [36, 36, 36, 36],             // drift_boost_axis_kick_rate
    [256, 256, 300, 200],         // collision_weight
    [5120, 5120, 5120, 5120],     // slope_forward_speed_bonus
    [2560, 2560, 2560, 2560],     // side_speed_clamp
    [15, 15, 15, 15],             // prototype_key
];

impl Stats {
    /// `VehBirth_SetConsts`: the constants of an engine class.
    pub fn for_class(class: EngineClass) -> Stats {
        let c = class as usize;
        Stats {
            gravity: META_PHYS[0][c] as i16,
            jump_force: META_PHYS[1][c] as i16,
            pedal_friction_perpendicular: META_PHYS[2][c] as i16,
            pedal_friction_forward: META_PHYS[3][c] as i16,
            no_pedal_friction_perpendicular: META_PHYS[4][c] as i16,
            no_pedal_friction_forward: META_PHYS[5][c] as i16,
            brake_friction: META_PHYS[6][c] as i16,
            drift_curve: META_PHYS[7][c] as i16,
            drift_friction: META_PHYS[8][c] as i16,
            accel_class_stat: META_PHYS[9][c] as i16,
            accel_reserves: META_PHYS[10][c] as i16,
            speed_class_stat: META_PHYS[11][c] as i16,
            accel_speed_class_stat: META_PHYS[12][c] as i16,
            single_turbo_speed: META_PHYS[13][c] as i16,
            sacred_fire_speed: META_PHYS[14][c] as i16,
            backward_speed: META_PHYS[15][c] as i16,
            mask_speed: META_PHYS[16][c] as i16,
            damaged_speed: META_PHYS[17][c] as i16,
            turn_rate: META_PHYS[18][c] as u8,
            backward_turn_rate: META_PHYS[19][c] as u8,
            turn_decrease_rate: META_PHYS[20][c] as i16,
            turn_input_delay: META_PHYS[21][c] as i16,
            pre_turbo: META_PHYS[22][c] as i16,
            terminal_velocity: META_PHYS[23][c] as i16,
            terrain_friction_boost: META_PHYS[24][c] as i16,
            steer_accel_stage4_first_frame: META_PHYS[25][c] as i8,
            steer_accel_stage2_first_frame: META_PHYS[26][c] as i8,
            steer_accel_stage2_frame_length: META_PHYS[27][c] as i8,
            steer_accel_stage1_max_steer: META_PHYS[28][c] as i16,
            steer_accel_stage1_min_steer: META_PHYS[29][c] as i16,
            steer_accel_turn_vel_scale: META_PHYS[30][c] as i16,
            steer_accel_turn_vel_limit: META_PHYS[31][c] as i16,
            model_rot_vel_max: META_PHYS[32][c] as i16,
            model_rot_vel_min: META_PHYS[33][c] as i16,
            model_turn_counter_steer_strength: META_PHYS[34][c] as u8,
            model_turn_return_strength: META_PHYS[35][c] as u8,
            model_turn_negative_return_strength: META_PHYS[36][c] as u8,
            model_turn_velocity_lerp: META_PHYS[37][c] as u8,
            turn_resist_min: META_PHYS[38][c] as u8,
            turn_resist_max: META_PHYS[39][c] as u8,
            steer_vel_drift_switch_way: META_PHYS[40][c] as i8,
            steer_vel_drift_standard: META_PHYS[41][c] as i8,
            drift_turn_base: META_PHYS[42][c] as i8,
            drift_turn_startup_scale: META_PHYS[43][c] as i8,
            drift_turn_ramp_frames: META_PHYS[44][c] as u8,
            drifting_frames_till_spinout: META_PHYS[45][c] as u8,
            drift_spin_rate_accel: META_PHYS[46][c] as i16,
            drift_spin_rate_decel: META_PHYS[47][c] as i16,
            drifting_camera_spin_rate: META_PHYS[48][c] as i16,
            drift_camera_lerp_step: META_PHYS[49][c] as u8,
            drift_release_turn_assist_frames: META_PHYS[50][c] as u8,
            meta_phys_33: META_PHYS[51][c] as i16,
            meta_phys_34: META_PHYS[52][c] as i16,
            drift_turn_same_direction_angle: META_PHYS[53][c] as i16,
            drift_turn_opposite_direction_angle: META_PHYS[54][c] as i16,
            drift_turn_angle_scale: META_PHYS[55][c] as i16,
            turbo_max_room: META_PHYS[56][c] as u8,
            turbo_low_room_warning: META_PHYS[57][c] as u8,
            turbo_full_bar_reserve_gain: META_PHYS[58][c] as u8,
            drift_boost_duration_frames: META_PHYS[59][c] as u8,
            drift_boost_axis_kick_rate: META_PHYS[60][c] as u8,
            collision_weight: META_PHYS[61][c] as i16,
            slope_forward_speed_bonus: META_PHYS[62][c] as i16,
            side_speed_clamp: META_PHYS[63][c] as i16,
            prototype_key: META_PHYS[64][c] as i32,
        }
    }
}
