//! Wall and floor scrapes: the surface table (`data.MetaDataScrub`) and
//! `COLL_MOVED_ScrubImpact`, which turns a hit from the moved search into a
//! change of the driver's velocity.

use coll::{Collision, clip};
use gte::{ctrl, data};
use loader::lev::{Level, SVec3, quad_flags};
use math::mips;

use crate::crash;
use crate::{Driver, Effect, actions, calc, kart_state};

/// `ScrubFlags`
pub mod scrub_flags {
    pub const APPLY_IMPACT: u32 = 0x1;
    pub const SLAM_ON_HARD_IMPACT: u32 = 0x2;
    pub const SKIP_WALL_RUB_TIMER: u32 = 0x4;
    pub const KEEP_RESERVES: u32 = 0x8;
}

/// `struct Scrub`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scrub {
    pub name: &'static str,
    pub flags: u32,
    pub speed_limit: i32,
    /// Given to the trig table.
    pub impact_angle: i32,
}

/// `data.MetaDataScrub`, indexed by scrub id.
pub const SCRUBS: [Scrub; 7] = [
    Scrub { name: "SOLID", flags: 0x3, speed_limit: 0xd00, impact_angle: 700 },
    Scrub { name: "RUBBER", flags: 0x1, speed_limit: 0xd00, impact_angle: 0 },
    Scrub { name: "SANDBAG", flags: 0x3, speed_limit: 0xd00, impact_angle: 0x100 },
    Scrub { name: "SLOWDOWN", flags: 0xc, speed_limit: 0xf00, impact_angle: 0 },
    Scrub { name: "NO EFFECT", flags: 0xc, speed_limit: 0x3e800, impact_angle: 0 },
    Scrub { name: "ROAD", flags: 0xd, speed_limit: 0x3e800, impact_angle: 0 },
    Scrub { name: "NO SCRUB", flags: 0x3, speed_limit: 0x3e800, impact_angle: 700 },
];

/// The scrub id for a quad block the player ran into.
pub mod scrub_id {
    pub const SOLID: usize = 0;
    pub const ROAD: usize = 5;
    /// `QUADBLOCK_FLAG_REFLECT_SPLIT_LINE_1` walls.
    pub const NO_SCRUB: usize = 6;
}

/// `VehAfterColl_GetSurface`: an out-of-range id is the first entry.
pub fn surface(id: usize) -> &'static Scrub {
    SCRUBS.get(id).unwrap_or(&SCRUBS[0])
}

/// `op` with sf=1: the cross product of the IR vector with the matrix diagonal.
const OP_SF1: u32 = 0x0178_000c;

/// `DRIVER_WALL_RUB_TIMER_START`
const WALL_RUB_TIMER_START: i16 = 0xf0;

/// `COLL_MOVED_ScrubImpact` result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Impact {
    /// The driver was not moving into the surface.
    None = 0,
    /// Velocity was changed.
    Applied = 1,
    /// The driver slammed into a wall and is now crashing: the search ends.
    Slammed = 2,
}

/// A GTE light matrix, row-major, 4.12.
pub type LightMatrix = [i16; 9];

/// The light matrix the simulation uses for [`scrub_impact`].
///
/// TODO: the original never sets this matrix for physics: `ScrubImpact`
/// transforms the impact with whatever light matrix the renderer last loaded
/// into the GTE (`gte_SetLightMatrix` in `PushBuffer.c`, `FLARE.c`, `Torch.c`,
/// `Particle.c`, `Vector.c`, `233/CS_Instance.c`, ...). That may depend on the
/// camera, which a lockstep simulation cannot share, so we use the identity:
/// the impact is then checked as-is against the slam-wall threshold. The
/// threshold (0x1900000, about 0x1400 squared) suggests a rotation was
/// intended, so the camera dependence is probably accidental. To replicate
/// retail instead: (1) statically find the last light-matrix writer before the
/// physics threads run in a frame (is `PROC` update before or after the draw
/// pass?); (2) if that is ambiguous, log `CFC2(8..=12)` on each `ScrubImpact`
/// call in `ctr-native`, with the camera yaw, over a few laps. Slam-wall
/// behaviour may differ from retail at some camera angles until then.
pub const IDENTITY_LIGHT_MATRIX: LightMatrix = [0x1000, 0, 0, 0, 0x1000, 0, 0, 0, 0x1000];

fn load_light_matrix(gte: &mut gte::Gte, m: &LightMatrix) {
    let pack = |lo: i16, hi: i16| mips::pack_s16_pair(lo as i32, hi as i32);
    gte.ctc2(pack(m[0], m[1]), ctrl::L_MATRIX);
    gte.ctc2(pack(m[2], m[3]), ctrl::L_MATRIX + 1);
    gte.ctc2(pack(m[4], m[5]), ctrl::L_MATRIX + 2);
    gte.ctc2(pack(m[6], m[7]), ctrl::L_MATRIX + 3);
    gte.ctc2(m[8] as i32 as u32, ctrl::L33);
}

/// `CollMoved_ScrubImpact_GteLLV0`: `x, y, z` through the light matrix.
fn light_matrix_times(gte: &mut gte::Gte, light_matrix: &LightMatrix, x: i32, y: i32, z: i32) -> math::Vec3 {
    load_light_matrix(gte, light_matrix);
    gte.mtc2(mips::pack_s16_pair(x, y), data::VXY0);
    gte.mtc2(z as u32, data::VZ0);
    gte.cop2(0x04a6012);
    math::Vec3 { x: gte.mfc2(data::MAC1) as i32, y: gte.mfc2(data::MAC2) as i32, z: gte.mfc2(data::MAC3) as i32 }
}

/// `CollMoved_ScrubImpact_ProjectWallVelocity`: the velocity's horizontal
/// part crossed with the wall normal twice, i.e. projected along the wall.
fn project_wall_velocity(gte: &mut gte::Gte, normal: SVec3, old_vel_x: i32, old_vel_z: i32) -> math::Vec3 {
    gte.mtc2(old_vel_x as u32, data::IR1);
    gte.mtc2(0, data::IR2);
    gte.mtc2(old_vel_z as u32, data::IR3);
    gte.ctc2(normal.x as i32 as u32, ctrl::R_MATRIX);
    gte.ctc2(normal.y as i32 as u32, ctrl::R22R23);
    gte.ctc2(normal.z as i32 as u32, ctrl::R33);

    gte.cop2(OP_SF1);

    let r11 = gte.cfc2(ctrl::R_MATRIX);
    let r22 = gte.cfc2(ctrl::R22R23);
    let r33 = gte.cfc2(ctrl::R33);

    gte.mtc2(r11, data::IR1);
    gte.mtc2(r22, data::IR2);
    gte.mtc2(r33, data::IR3);

    let cross = [gte.mfc2(data::MAC1), gte.mfc2(data::MAC2), gte.mfc2(data::MAC3)];

    gte.ctc2(cross[0], ctrl::R_MATRIX);
    gte.ctc2(cross[1], ctrl::R22R23);
    gte.ctc2(cross[2], ctrl::R33);

    gte.cop2(OP_SF1);

    math::Vec3 { x: gte.mfc2(data::MAC1) as i32, y: gte.mfc2(data::MAC2) as i32, z: gte.mfc2(data::MAC3) as i32 }
}

fn abs(value: i32) -> i32 {
    if value < 0 { mips::neg_lo(value) } else { value }
}

/// `COLL_MOVED_ScrubImpact`: reacts to the hit left in `coll.sps` by the moved
/// search. Removes the velocity component into the surface (beyond the scrub
/// depth), starts the wall-rub timer, and may slam the driver into a wall.
/// `effects` collects what the original plays and rumbles.
///
/// The impact is passed through `light_matrix`; see [`IDENTITY_LIGHT_MATRIX`].
pub fn scrub_impact(
    d: &mut Driver,
    coll: &mut Collision,
    level: &Level,
    light_matrix: &LightMatrix,
    scrub: &Scrub,
    effects: &mut Vec<Effect>,
) -> Impact {
    let sps = &coll.sps;
    let hit_flags = sps.hit.quad.map(|q| level.quad_blocks[q as usize].flags);
    let mut normal = sps.hit.plane.normal;

    if d.v_shift_count != 0
        && sps.did_touch_quadblock != 0
        && hit_flags.is_some_and(|f| f & quad_flags::GROUND != 0)
        && sps.hit.reorder_result as i32 != clip::FACE
        && sps.hit.quad != d.under_driver
        && abs(d.speed_approx as i32) < 0x300
        && abs(d.jump_height_curr as i32) < 0x300
        && d.fire_speed == 0
    {
        let diff_x = mips::sub_lo(mips::sra(d.pos_curr.x, 8), sps.hit.hit_pos.x as i32);
        let diff_z = mips::sub_lo(mips::sra(d.pos_curr.z, 8), sps.hit.hit_pos.z as i32);
        let diff_y = mips::sub_lo(mips::sra(d.pos_curr.y, 8), sps.hit.hit_pos.y as i32);

        if (diff_x | diff_y | diff_z) != 0 {
            let squares = (mips::mul_lo(diff_x, diff_x) as u32)
                .wrapping_add(mips::mul_lo(diff_y, diff_y) as u32)
                .wrapping_add(mips::mul_lo(diff_z, diff_z) as u32);
            let len = calc::fast_sqrt(squares, 0) as i32;

            normal.x = mips::div(mips::sll(diff_x, 12), len) as i16;
            normal.y = mips::div(mips::sll(diff_y, 12), len) as i16;
            normal.z = mips::div(mips::sll(diff_z, 12), len) as i16;
        }
    }

    let v = d.velocity;
    let mut dot = mips::sra(
        mips::add_lo(
            mips::add_lo(
                mips::mul_lo(mips::sra(v.x, 3), normal.x as i32),
                mips::mul_lo(mips::sra(v.y, 3), normal.y as i32),
            ),
            mips::mul_lo(mips::sra(v.z, 3), normal.z as i32),
        ),
        9,
    );

    if dot < -0xa00 {
        d.actions_flag_set |= actions::TURBO_INPUT_LATCH;
    }

    dot = mips::sub_lo(dot, sps.input1.scrub_depth as i32);

    if dot >= 0 {
        return Impact::None;
    }

    let scrub_flags = scrub.flags;
    if scrub_flags & scrub_flags::SKIP_WALL_RUB_TIMER == 0 {
        d.actions_flag_set |= actions::DRIVING_AGAINST_WALL;
    }
    if scrub_flags & scrub_flags::KEEP_RESERVES == 0 {
        d.reserves = 0;
        d.turbo_outside_timer = 0;
    }

    let scrub_speed = scrub.speed_limit;
    let faster_rub_active = if d.wall_rub_timer == 0 { 0x3e7ff < scrub_speed } else { (d.wall_rub_speed_limit as i32) < scrub_speed };
    if !faster_rub_active {
        d.wall_rub_timer = WALL_RUB_TIMER_START;
        d.wall_rub_speed_limit = scrub_speed as i16;
        d.pos_wall_coll = sps.hit.hit_pos;
    }

    if scrub_flags & scrub_flags::APPLY_IMPACT == 0 {
        return Impact::None;
    }

    let impact = |axis: i16| mips::sra(mips::mul_lo(dot, axis as i32), 12);
    let mut impact = math::Vec3 { x: impact(normal.x), y: impact(normal.y), z: impact(normal.z) };
    let mut speed_sq = 0;

    if scrub.impact_angle != 0 {
        speed_sq = mips::sra(
            mips::add_lo(
                mips::add_lo(mips::mul_lo(v.x, v.x), mips::mul_lo(v.y, v.y)),
                mips::mul_lo(v.z, v.z),
            ),
            15,
        );
    }

    let old_vel_x = v.x;
    let old_vel_z = v.z;
    d.velocity.x = mips::sub_lo(old_vel_x, impact.x);
    d.velocity.z = mips::sub_lo(old_vel_z, impact.z);
    d.velocity.y = mips::sub_lo(d.velocity.y, impact.y);

    impact = light_matrix_times(&mut coll.gte, light_matrix, impact.x, impact.y, impact.z);

    let sps = &mut coll.sps;
    if sps.did_touch_quadblock != 0
        && sps.quad_block_coll.search_flags & coll::search_flags::WALL_PROJECTION_DONE == 0
        && d.actions_flag_set_prev_frame & actions::TOUCH_GROUND == 0
        && hit_flags.is_some_and(|f| f & quad_flags::GROUND != 0)
    {
        let wall_velocity = project_wall_velocity(&mut coll.gte, normal, old_vel_x, old_vel_z);

        let wall_speed_sq = (mips::mul_lo(wall_velocity.x, wall_velocity.x) as u32)
            .wrapping_add(mips::mul_lo(wall_velocity.y, wall_velocity.y) as u32)
            .wrapping_add(mips::mul_lo(wall_velocity.z, wall_velocity.z) as u32);
        let wall_speed = calc::fast_sqrt(wall_speed_sq, 0x10) >> 8;
        let speed_approx = d.speed_approx as i32;

        if wall_speed != 0 && speed_approx > 0 {
            coll.sps.quad_block_coll.search_flags |= coll::search_flags::WALL_PROJECTION_DONE;
            let scaled = |component: i32| mips::div(mips::mul_lo(component, speed_approx), wall_speed as i32);
            d.velocity.x = scaled(wall_velocity.x);
            d.velocity.y = scaled(wall_velocity.y);
            d.velocity.z = scaled(wall_velocity.z);
            d.velocity.x = mips::sub_lo(d.velocity.x, mips::sra(normal.x as i32, 1));
            d.velocity.y = mips::sub_lo(d.velocity.y, mips::sra(normal.y as i32, 1));
            d.velocity.z = mips::sub_lo(d.velocity.z, mips::sra(normal.z as i32, 1));
        }
    }

    let transformed_impact_xz = mips::add_lo(mips::mul_lo(impact.x, impact.x), mips::mul_lo(impact.z, impact.z));

    if scrub_flags & scrub_flags::SLAM_ON_HARD_IMPACT != 0 && dot < -0x13ff && transformed_impact_xz > 0x190_0000 {
        if d.kart_state != kart_state::MASK_GRABBED {
            let pattern = if d.simp_turn_state < 1 { 0x1f } else { 0x2f };
            effects.push(Effect::JogCon { driver_id: d.driver_id, pattern, strength: 0x60 });
        }

        if scrub.impact_angle != 0 {
            let trig = math::sin(scrub.impact_angle as u32);
            let scaled_speed = mips::sra(mips::mul_lo(speed_sq, trig), 12);
            let angle_limit = mips::sra(mips::mul_lo(scaled_speed, trig), 12);
            let dot_sq = mips::sra(mips::mul_lo(dot, dot), 15);

            if angle_limit >= dot_sq {
                return Impact::Applied;
            }
        }

        if d.kart_state != kart_state::MASK_GRABBED && transformed_impact_xz > 0x190_0000 {
            let echo = d.actions_flag_set & actions::ENGINE_ECHO != 0;
            effects.push(Effect::Sound { id: 6, echo });
            effects.push(Effect::Voiceline { id: 6, driver_id: d.driver_id, priority: 0x10 });
            effects.push(Effect::ShockFreq { driver_id: d.driver_id, a: 8, b: 0 });
            effects.push(Effect::ShockForce { driver_id: d.driver_id, a: 8, b: 0x7f });

            if d.kart_state == kart_state::DRIFTING {
                let turn_angle = d.turn_angle_curr;
                d.turn_angle_curr = 0;
                d.angle = d.angle.wrapping_add(turn_angle);
                d.rot_curr_w = d.rot_curr_w.wrapping_sub(turn_angle);
            }

            d.anim_index = 2;
            d.anim_frame = 0;
            d.matrix_array = crate::MatrixArray::CrashFall;
            d.matrix_index = 0;

            crash::slam_wall_init(d);
            return Impact::Slammed;
        }
    }

    Impact::Applied
}
