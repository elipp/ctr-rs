//! Karts against karts: the search `VehPhysForce_CollideDrivers` makes for another driver in
//! reach (`PROC_CollidePointWithBucket`), and what a touch does (`VehPhysCrash_AnyTwoCars`,
//! `VehPhysCrash_BounceSelf`, `VehPhysCrash_Attack`).
//!
//! The original walks the thread list: the sibling threads of the driver's, the drivers that tick
//! after it. So each pair is looked at once, by the driver that ticks first, which moves both.
//! [`Others`] is that list. Only human drivers are ported: there are no bots, so no
//! `VehPhysCrash_AI` and no `BOTS_*`. The shield bubble's hit in `Attack` is left out too (the
//! bubble does not hurt the kart it bumps).

use math::mips;

use crate::calc::map_to_range;
use crate::{Driver, Effect, Env, actions, forced_jump, kart_state, pick};

/// `THREAD_DRIVER_HIT_RADIUS` of a driver's thread.
const HIT_RADIUS: i32 = 0x40;
/// `VEH_PHYS_FORCE_COLLISION_BEST_DIST_INIT`
const BEST_DIST_INIT: i32 = 0x7fff_ffff;
/// Beyond this, the square of one axis is out of reach (`PROC_CollidePointWithSelf`).
const AXIS_DIST_SQ_LIMIT: i32 = 0x1000_0000;

const VECTOR_SPEED_SHIFT: u32 = 8;
const MATRIX_FRAC_SHIFT: u32 = 12;
const UNIT_VECTOR_SCALE: i32 = 0x1000;
const BOUNCE_Y_CLAMP: i32 = 0x3200;
const BOUNCE_DOT_DIVISOR: i32 = 0xc00;

const DAMAGE_TYPE_MASK: u8 = 2;
const DAMAGE_TYPE_TURBO: u8 = 3;
const DAMAGE_REASON_TURBO: u8 = 5;
const DAMAGE_REASON_MASK: u8 = 6;
const TURBO_DAMAGE_IMPACT: i32 = 0xa00;

const VOICELINE_CRASH: u8 = 1;
const VOICELINE_HARD_CRASH: u8 = 5;
const VOICELINE_PRIORITY: u8 = 0x10;

const FEEDBACK_MIN_IMPACT: i32 = 0x200;
const VOLUME_IMPACT_MAX: i32 = 0x1900;
const VOLUME_MIN: i32 = 0x3f;
const VOLUME_MAX: i32 = 0xff;
const HARD_CRASH_VOLUME: i32 = 0xdc;
const FEEDBACK_COOLDOWN_FRAMES: u32 = 3;

const RUMBLE_CHANNEL: u8 = 8;
const RUMBLE_FORCE: u8 = 0x7f;
const JOG_TURNING: u8 = 0x29;
const JOG_STRAIGHT: u8 = 0x19;
const JOG_DURATION: u8 = 0x60;

/// The frame the last bump was heard on (`audioDefaults[8]`), which keeps the sound from
/// repeating every frame. It only gates sounds and rumble, never the physics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Feedback {
    pub last_frame: i32,
}

/// The drivers a driver can bump into, in the order of the thread list after its own.
pub struct Others<'a, 'b> {
    pub drivers: &'a mut [&'b mut Driver],
    pub feedback: &'a mut Feedback,
}

/// `struct DriverCollisionSearch`: the closest driver in reach.
struct Search {
    /// Index into [`Others::drivers`].
    other: usize,
    best_dist_sq: i32,
    /// `bucket.dist`: from the other kart's instance to ours.
    dist: [i16; 3],
}

/// `PROC_CollidePointWithBucket` over the drivers: the closest one, the first on a tie.
fn closest(d: &Driver, others: &[&mut Driver]) -> Option<Search> {
    let pos = [d.pos_curr.x, d.pos_curr.y, d.pos_curr.z].map(|v| mips::sra(v, 8) as i16 as i32);
    let mut best: Option<Search> = None;
    let mut best_dist_sq = BEST_DIST_INIT;

    for (n, other) in others.iter().enumerate() {
        if other.collision_disabled {
            continue;
        }
        // `inst->matrix.t`
        let at = crate::baked::inst_matrix(other).1;
        let dist = [pos[0] - at[0], pos[1] - at[1], pos[2] - at[2]];

        let mut sum = mips::mul_lo(dist[0], dist[0]);
        if sum >= AXIS_DIST_SQ_LIMIT {
            continue;
        }
        let square = mips::mul_lo(dist[1], dist[1]);
        sum = mips::add_lo(sum, square);
        if square >= AXIS_DIST_SQ_LIMIT {
            continue;
        }
        let square = mips::mul_lo(dist[2], dist[2]);
        sum = mips::add_lo(sum, square);
        if square >= AXIS_DIST_SQ_LIMIT {
            continue;
        }
        if sum >= best_dist_sq {
            continue;
        }
        best_dist_sq = sum;
        best = Some(Search {
            other: n,
            best_dist_sq: sum,
            dist: dist.map(|v| v as i16),
        });
    }
    best
}

/// The part of `VehPhysForce_CollideDrivers` that searches for a driver and bumps into it.
pub fn collide(d: &mut Driver, others: &mut Others, env: &Env, effects: &mut Vec<Effect>) {
    if !env.kart_collisions || d.collision_disabled {
        return;
    }
    let Some(search) = closest(d, others.drivers) else {
        return;
    };
    let radius_sum = mips::add_lo(HIT_RADIUS, HIT_RADIUS);
    if search.best_dist_sq < mips::mul_lo(radius_sum, radius_sum) {
        let other = &mut *others.drivers[search.other];
        any_two_cars(d, other, &search, env, others.feedback, effects);
    }
}

/// `VehPhysCrash_BounceSelf`: takes the part of `vel` (relative to `origin`) that goes into the
/// other kart (along `normal`) away. The result is the game's: always 0.
fn bounce_self(
    normal: [i16; 3],
    origin: [i32; 3],
    vel: &mut [i32; 3],
    other_driver: bool,
    impact: &mut i32,
) -> i32 {
    let diff = [0, 1, 2].map(|i| mips::sub_lo(vel[i], origin[i]));
    let dot = mips::sra(
        mips::add_lo(
            mips::add_lo(
                mips::mul_lo(diff[0], normal[0] as i32),
                mips::mul_lo(diff[1], normal[1] as i32),
            ),
            mips::mul_lo(diff[2], normal[2] as i32),
        ),
        MATRIX_FRAC_SHIFT,
    );

    if other_driver {
        if dot <= 0 {
            return 0;
        }
    } else if dot >= 0 {
        return 0;
    }

    let abs_dot = if dot < 0 { mips::neg_lo(dot) } else { dot };
    if *impact < abs_dot {
        *impact = abs_dot;
    }

    let scale = |n: i16| mips::mul_lo(dot, n as i32) / BOUNCE_DOT_DIVISOR;
    let diff = [0, 1, 2].map(|i| mips::sub_lo(diff[i], scale(normal[i])));

    vel[0] = mips::add_lo(diff[0], origin[0]);
    let old_y = vel[1];
    let mut y = mips::add_lo(diff[1], origin[1]);
    if old_y < y && y > BOUNCE_Y_CLAMP {
        y = BOUNCE_Y_CLAMP;
    }
    vel[1] = y;
    vel[2] = mips::add_lo(diff[2], origin[2]);
    0
}

/// `VehPhysCrash_AnyTwoCars`, for two human drivers: `d` is the one whose procedure runs.
fn any_two_cars(
    d: &mut Driver,
    other: &mut Driver,
    search: &Search,
    env: &Env,
    feedback: &mut Feedback,
    effects: &mut Vec<Effect>,
) {
    let distance = math::fast_sqrt(search.best_dist_sq as u32, 0);

    let mut hit_dir = [0i16, 0, UNIT_VECTOR_SCALE as i16];
    if distance != 0 {
        let toward = |v: i16| mips::div(mips::sll(v as i32, MATRIX_FRAC_SHIFT), distance) as i16;
        hit_dir = [
            toward(search.dist[0]),
            toward(search.dist[1]),
            toward(search.dist[2]),
        ];
    }

    let hit_strength = mips::sub_lo(mips::add_lo(HIT_RADIUS, HIT_RADIUS), distance);
    if hit_strength <= 0 {
        return;
    }

    let mut impact = 0;

    // Both go away at the speed of the pair, weighted by how heavy they are.
    let (weight, other_weight) = (
        d.stats.collision_weight as i32,
        other.stats.collision_weight as i32,
    );
    let mut self_vel = [d.velocity.x, d.velocity.y, d.velocity.z];
    let mut other_vel = [other.velocity.x, other.velocity.y, other.velocity.z];
    let weighted = [0, 1, 2].map(|i| {
        mips::div(
            mips::add_lo(
                mips::mul_lo(self_vel[i], weight),
                mips::mul_lo(other_vel[i], other_weight),
            ),
            mips::add_lo(weight, other_weight),
        )
    });
    if bounce_self(hit_dir, weighted, &mut other_vel, true, &mut impact) < 0 {
        impact = 0;
    }
    if bounce_self(hit_dir, weighted, &mut self_vel, false, &mut impact) > 0 {
        impact = 0;
    }

    let push = |n: i16| mips::sra(mips::mul_lo(n as i32, hit_strength), VECTOR_SPEED_SHIFT);
    for i in 0..3 {
        self_vel[i] = mips::add_lo(self_vel[i], push(hit_dir[i]));
        other_vel[i] = mips::sub_lo(other_vel[i], push(hit_dir[i]));
    }
    (d.velocity.x, d.velocity.y, d.velocity.z) = (self_vel[0], self_vel[1], self_vel[2]);
    (other.velocity.x, other.velocity.y, other.velocity.z) =
        (other_vel[0], other_vel[1], other_vel[2]);

    // The bump is heard and felt, no more often than every few frames.
    let current_frame = env.timer as i32;
    let cooling_down =
        (mips::sub_lo(current_frame, feedback.last_frame) as u32) < FEEDBACK_COOLDOWN_FRAMES;
    let can_play_feedback = !cooling_down;

    if impact > FEEDBACK_MIN_IMPACT {
        let volume = map_to_range(impact, 0, VOLUME_IMPACT_MAX, VOLUME_MIN, VOLUME_MAX);
        let hurt = |k: &Driver| k.kart_state == kart_state::BLASTED || k.invincible_timer != 0;
        if can_play_feedback && !hurt(d) && !hurt(other) {
            let echo = d.actions_flag_set & actions::ENGINE_ECHO != 0;
            effects.push(Effect::DriverCrashing {
                echo,
                volume: volume as u8,
            });
            feedback.last_frame = current_frame;

            if volume as u32 > HARD_CRASH_VOLUME as u32 {
                effects.push(Effect::Voiceline {
                    id: VOICELINE_HARD_CRASH,
                    driver_id: d.driver_id,
                    priority: VOICELINE_PRIORITY,
                });
            }
        }

        for k in [&*other, &*d] {
            effects.push(Effect::ShockFreq {
                driver_id: k.driver_id,
                a: RUMBLE_CHANNEL,
                b: 0,
            });
            effects.push(Effect::ShockForce {
                driver_id: k.driver_id,
                a: RUMBLE_CHANNEL,
                b: RUMBLE_FORCE,
            });
            let pattern = if k.simp_turn_state > 0 {
                JOG_TURNING
            } else {
                JOG_STRAIGHT
            };
            effects.push(Effect::JogCon {
                driver_id: k.driver_id,
                pattern,
                strength: JOG_DURATION,
            });
        }

        d.actions_flag_set |= actions::HUMAN_HUMAN_COLLISION;
        other.actions_flag_set |= actions::HUMAN_HUMAN_COLLISION;
    }

    let result = attack(d, other, can_play_feedback, impact, effects);
    attack(other, d, result, impact, effects);
}

/// `VehPhysCrash_Attack`: `driver2` may hurt `driver1` by touching it: with the Aku Aku mask, or
/// with a boost (if `driver1` is not boosting). The result is `can_play_feedback`.
fn attack(
    driver1: &mut Driver,
    driver2: &mut Driver,
    can_play_feedback: bool,
    impact: i32,
    effects: &mut Vec<Effect>,
) -> bool {
    if driver1.actions_flag_set & actions::MASK_WEAPON != 0 {
        return can_play_feedback;
    }

    if driver2.actions_flag_set & actions::MASK_WEAPON != 0 {
        driver1.pending_damage_type = DAMAGE_TYPE_MASK;
        driver1.pending_damage_reason = DAMAGE_REASON_MASK;
        driver1.pending_damage_attacker = Some(driver2.driver_id);

        if can_play_feedback
            && driver1.kart_state != kart_state::BLASTED
            && driver1.invincible_timer == 0
        {
            let echo = driver1.actions_flag_set & actions::ENGINE_ECHO != 0;
            effects.push(Effect::DriverCrashing {
                echo,
                volume: VOLUME_MAX as u8,
            });
            effects.push(Effect::Voiceline {
                id: VOICELINE_CRASH,
                driver_id: driver1.driver_id,
                priority: VOICELINE_PRIORITY,
            });
        }
    }

    if impact > TURBO_DAMAGE_IMPACT
        && driver2.reserves != 0
        && driver2.actions_flag_set & actions::TURBO_ITEM != 0
        && driver1.reserves == 0
    {
        driver2.forced_jump_type = forced_jump::HIGH;

        driver1.pending_damage_type = DAMAGE_TYPE_TURBO;
        driver1.pending_damage_reason = DAMAGE_REASON_TURBO;
        driver1.pending_damage_attacker = Some(driver2.driver_id);
    }

    can_play_feedback
}

/// What the victim's next update does with a hit that another driver queued
/// (`VehPickupItem_ShootOnCirclePress`): the damage, then nothing pending.
pub fn take_pending_damage(d: &mut Driver, effects: &mut Vec<Effect>) {
    if d.pending_damage_type == 0 {
        return;
    }
    let damage = d.pending_damage_type;
    d.pending_damage_type = 0;
    pick::new_state(d, damage, effects);
}
