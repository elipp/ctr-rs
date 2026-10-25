//! The weapons in flight: the bombs and the missiles a kart fires (`VehPickupItem_ShootNow`), what
//! they do each tick (`RB_MovingExplosive_ThTick`) and how they go off (`RB_MovingExplosive_Explode`
//! and `RB_Burst_Init`). Plain state and a pure `tick`, like [`Objects`] of the `instance` crate,
//! so it can go in the rollback state next to the karts.
//!
//! The weapons are not the level's objects: they are born when a kart fires and end when they go
//! off, they know the karts (they chase one, and hurt the ones they reach) and they search the
//! level's surfaces themselves. What a hit does to a kart is the caller's: [`Tick::hits`] says
//! who was hit by whom.
//!
//! The mines (TNT and Nitro crates the kart drops) are in [`mine`]; a weapon that runs into one,
//! and a burst that reaches one, sets it off.
//!
//! A weapon breaks the fruit and weapon crates it passes through, and so does its blast
//! ([`Tick::crate_hits`], for the objects to deal with). Not ported: the temple teeth, the warpball it would meet, the thrown TNT a missile would be drawn
//! away by, and the sounds. How the aim differs is in [`aim`].
//!
//! The projectiles keep what the sim needs. Their matrices (`inst->matrix`) are for drawing:
//! a bomb rolls by [`Projectile::roll`] about the heading `rot_y` on the plane of `normal`, and a
//! missile faces `rot_y` on that plane.

pub mod aim;
pub mod mine;

use coll::instance::instances_in_reach;
use coll::{Collision, model_id, search_flags};
use gte::Gte;
use loader::lev::{Level, SVec3, quad_flags};
use math::mips;
use vehicle::{Driver, Matrix, actions, kart_state};

/// `ACTIVE_MISSILE_LIMIT`: no more than this many bombs and missiles fly at once.
pub const ACTIVE_LIMIT: usize = 12;

/// `TRACKER_PARENT_SAFETY_FRAMES`: how long a weapon passes through the kart that fired it.
const PARENT_SAFETY_TICKS: i16 = 60;
/// `TRACKER_LAUNCH_VELOCITY_NUMERATOR` and `_SHIFT`: a bomb's speed, and a powered-up missile's.
const LAUNCH_VELOCITY: i32 = 3;
const LAUNCH_VELOCITY_SHIFT: u32 = 7;
/// `MISSILE_TRACKER_VELOCITY_NUMERATOR` and `_SHIFT`
const MISSILE_VELOCITY: i32 = 5;
const MISSILE_VELOCITY_SHIFT: u32 = 8;
/// `DRIVER_WUMPA_JUICED_COUNT`
const JUICED: i8 = 10;

/// How a weapon turns towards its target each tick, in angle units of 4096 to the turn
/// (`RB_Hazard_InterpolateValue`'s speed).
const BOMB_TURN: i32 = 4;
const MISSILE_TURN_POWERED: i32 = 0x80;
const MISSILE_TURN: i32 = 0x40;

/// The search for the ground: from `GROUND_PROBE_BELOW` under the weapon up to
/// `GROUND_PROBE_ABOVE` over it, and the second one, for a weapon that is in the air, down to
/// `AIR_PROBE_BELOW`.
const GROUND_PROBE_BELOW: i32 = -0x40;
const GROUND_PROBE_ABOVE: i32 = 0x100;
const AIR_PROBE_BELOW: i32 = -0x900;
/// How far over the ground a weapon rides.
const RIDE_HEIGHT: i32 = 0x30;
/// The fastest a weapon falls.
const FALL_SPEED_MAX: i16 = -0x60;
/// What a bomb's roll turns by each tick.
const ROLL_STEP: i16 = 0x200;

/// `RB_Hazard_CollideWithDrivers`' and `_CollideWithBucket`'s reach: a squared distance.
const HIT_RADIUS: u32 = 0x2400;

/// `COLL_STEP_TRIGGER_WEAPON_REACT`: the surface blows a weapon up.
const WEAPON_REACT: u32 = 0x4;

/// The models of the pickups a level places, which a weapon that reaches does not blow up on
/// (`RB_Hazard_CollLevInst`).
const PICKUP_MODELS: [i32; 3] = [2, 7, 8];

/// The crates a blast breaks: `PU_FRUIT_CRATE` and `PU_RANDOM_CRATE` (not the time crates).
const CRATE_MODELS: [i32; 2] = [7, 8];

/// `TrackerWeaponFlags`
pub mod flags {
    /// The kart fired with 10 wumpa fruit.
    pub const POWERED_UP: u16 = 0x1;
    /// The kart that threw the bomb asked for it to go off.
    pub const DETONATE: u16 = 0x2;
    /// The weapon reached the kart it was chasing.
    pub const HIT_TARGET: u16 = 0x10;
    /// The bomb was thrown backwards.
    pub const BOMB_BACKWARD: u16 = 0x20;
}

/// `VEH_PICK_REASON_*`: what a hit was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// A TNT crate that went off on the head it sat on.
    Tnt = 0,
    Bomb = 1,
    /// A Nitro or TNT crate that was dropped.
    Mine = 2,
    Missile = 3,
    /// A potion that was thrown a moment ago.
    Potion = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bomb,
    Missile,
}

impl Kind {
    /// The `Input1.modelID` of the weapon's hits.
    fn model(self) -> i16 {
        match self {
            Kind::Bomb => model_id::BOMB,
            Kind::Missile => model_id::ROCKET,
        }
    }

    fn reason(self) -> Reason {
        match self {
            Kind::Bomb => Reason::Bomb,
            Kind::Missile => Reason::Missile,
        }
    }

    /// `RB_Burst_Init`'s `hitRadiusSquared`: a missile's blast is small, a bomb's wide, and wider
    /// still when it was thrown with 10 wumpa fruit.
    fn blast_radius_squared(self, powered: bool) -> i32 {
        match (self, powered) {
            (Kind::Missile, _) => 0x4000,
            (Kind::Bomb, true) => 0x40000,
            (Kind::Bomb, false) => 0x19000,
        }
    }
}

/// What a weapon sees of a kart (by slot): where its model is and what keeps it from being aimed at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Body {
    /// The kart's `instSelf->matrix.t`, in level units.
    pub at: [i32; 3],
    /// Its `instSelf->matrix`, which a TNT crate on its head follows.
    pub rot: Matrix,
    /// `posCurr`, 24.8.
    pub pos_curr: [i32; 3],
    /// `KS_MASK_GRABBED`
    pub mask_grabbed: bool,
    /// `invisibleTimer != 0`
    pub invisible: bool,
    pub kart_state: u8,
    /// `squishTimer != 0`
    pub squished: bool,
    /// `ACTION_JUMP_STARTED`
    pub jump_started: bool,
    /// The Aku Aku mask or invincibility absorbs a hit (`VehPickState_NewState` is false for it).
    pub immune: bool,
    pub character: u8,
}

impl Body {
    pub fn of(d: &Driver) -> Body {
        let inst = vehicle::baked::inst_matrix(d);
        Body {
            at: inst.1,
            rot: inst.0,
            pos_curr: [d.pos_curr.x, d.pos_curr.y, d.pos_curr.z],
            mask_grabbed: d.kart_state == kart_state::MASK_GRABBED,
            // (The invisibility item is not ported.)
            invisible: false,
            kart_state: d.kart_state,
            squished: d.squish_timer != 0,
            jump_started: d.actions_flag_set & actions::JUMP_STARTED != 0,
            immune: d.kart_state == kart_state::MASK_GRABBED
                || d.actions_flag_set & actions::MASK_WEAPON != 0
                || d.invincible_timer != 0,
            character: d.character,
        }
    }
}

/// A bomb or a missile in flight (`struct TrackerWeapon` and the instance it moves).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projectile {
    /// Which weapon this is for as long as it flies, so that the outside can follow it between
    /// ticks (the weapons do not move in the list when others end).
    pub id: u32,
    pub kind: Kind,
    /// `matrix.t`, in level units.
    pub pos: [i32; 3],
    /// `vel`
    pub vel: [i16; 3],
    pub flags: u16,
    /// The heading, 4096 to the turn.
    pub rot_y: i16,
    /// How far the bomb has rolled (`dir.x`).
    pub roll: i16,
    /// The normal of the surface it rides on, for the missile to face along.
    pub normal: SVec3,
    /// The slot of the kart that fired it.
    pub parent: usize,
    /// The slot of the kart it chases.
    pub target: Option<usize>,
    /// `parentSafetyFrames`: ticks left in which it goes through its parent.
    pub parent_safety: i16,
    /// `blindFrames`: ticks left in which it does not chase.
    pub blind: i16,
    /// `timeAlive`, in milliseconds.
    pub time_alive: i32,
    /// Gone (`THREAD_FLAG_DEAD`): removed at the end of the tick.
    dead: bool,
}

impl Projectile {
    /// The rotation of its instance (`inst->matrix`), for drawing. A bomb turns about the way it
    /// heads as it rolls (`ConvertRotToMatrix` of the heading and the roll). A missile faces its
    /// heading on the plane of the surface it flies over (`VehPhysForce_RotAxisAngle`).
    pub fn matrix(&self) -> Matrix {
        match self.kind {
            Kind::Bomb => vehicle::rotmat::rot_to_matrix(SVec3 {
                x: self.roll,
                y: self.rot_y,
                z: 0,
            }),
            Kind::Missile => {
                let mut matrix = Matrix::default();
                vehicle::force::rot_axis_angle(
                    &mut matrix,
                    &mut Gte::default(),
                    self.normal,
                    self.rot_y as i32,
                );
                matrix
            }
        }
    }

    fn powered(&self) -> bool {
        self.flags & flags::POWERED_UP != 0
    }
}

/// A kart that was hurt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub victim: usize,
    /// None for a hit nobody is to blame for.
    pub attacker: Option<usize>,
    pub reason: Reason,
    /// `VEH_PICK_DAMAGE_*`: what the hit does (`vehicle::pick::damage`).
    pub damage: u8,
}

/// A weapon that went off, for the outside to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Explosion {
    pub kind: Kind,
    pub powered: bool,
    pub pos: [i32; 3],
    pub parent: usize,
}

/// A crate that a blast reached (`RB_Burst_CollLevInst`): the caller breaks it as a hit of `model`
/// by the driver `owner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrateHit {
    /// An index into [`Level::instances`].
    pub instance: usize,
    /// `Input1.modelID`: the weapon that blasted.
    pub model: i16,
    /// The kart that fired it.
    pub owner: usize,
}

/// What one tick of the weapons did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tick {
    pub hits: Vec<Hit>,
    pub explosions: Vec<Explosion>,
    /// The mines that went off.
    pub detonations: Vec<mine::Detonation>,
    /// The crates the blasts reached, in the order they went off.
    pub crate_hits: Vec<CrateHit>,
    /// The karts a blast went off on, whose screens flash white (`RB_Burst_CollThread`).
    pub flashes: Vec<usize>,
}

/// All the weapons in flight.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Weapons {
    projectiles: Vec<Projectile>,
    mines: Vec<mine::Mine>,
    /// What happened outside a tick (a mine dropped into something goes off at once), which the
    /// next one tells.
    pending: Tick,
    /// The `id` of the next weapon fired.
    next_id: u32,
}

fn shift_velocity(direction: i16, numerator: i32, shift: u32) -> i16 {
    mips::sra(mips::mul_lo(direction as i32, numerator), shift) as i16
}

/// `(sin(rot_y) * numerator) >> shift`, a component of a velocity.
fn heading_velocity(trig: i32, numerator: i32, shift: u32) -> i16 {
    mips::sra(mips::mul_lo(trig, numerator), shift) as i16
}

fn svec(pos: [i32; 3], dy: i32) -> SVec3 {
    SVec3 {
        x: pos[0] as i16,
        y: (pos[1] + dy) as i16,
        z: pos[2] as i16,
    }
}

impl Weapons {
    /// The weapons in flight.
    pub fn projectiles(&self) -> &[Projectile] {
        &self.projectiles
    }

    /// Whether `slot` has a bomb that it threw still in flight (`instBombThrow`).
    pub fn has_bomb(&self, slot: usize) -> bool {
        self.projectiles
            .iter()
            .any(|p| p.kind == Kind::Bomb && p.parent == slot && !p.dead)
    }

    /// `VehPickupItem_ShootNow` for the bomb and the missile: kart `shooter` (`driver`) fires one,
    /// `backward` being the bomb thrown backwards (the shot with Down held). `bodies` are all the
    /// karts. False if too many are in flight already, when nothing is fired.
    pub fn launch(
        &mut self,
        kind: Kind,
        shooter: usize,
        driver: &Driver,
        bodies: &[Body],
        gte: &mut Gte,
        backward: bool,
    ) -> bool {
        if self.projectiles.len() >= ACTIVE_LIMIT {
            return false;
        }
        // The weapon starts where the kart's model is, turned along the heading on its surface.
        let at = vehicle::baked::inst_matrix(driver).1;
        let mut matrix = Matrix::default();
        vehicle::force::rot_axis_angle(
            &mut matrix,
            gte,
            driver.axis_angle1_normal_vec,
            driver.rot_curr_y as i32,
        );
        let (forward_x, forward_z) = (matrix.m[0][2], matrix.m[2][2]);
        let juiced = driver.num_wumpas >= JUICED;

        let mut p = Projectile {
            id: self.next_id,
            kind,
            pos: at,
            vel: [0; 3],
            flags: 0,
            rot_y: driver.rot_curr_y,
            roll: 0,
            normal: driver.axis_angle1_normal_vec,
            parent: shooter,
            target: aim::target(shooter, bodies, driver.rot_curr_y),
            parent_safety: PARENT_SAFETY_TICKS,
            blind: 0,
            time_alive: 0,
            dead: false,
        };
        match kind {
            Kind::Bomb => {
                p.vel[0] = shift_velocity(forward_x, LAUNCH_VELOCITY, LAUNCH_VELOCITY_SHIFT);
                p.vel[2] = shift_velocity(forward_z, LAUNCH_VELOCITY, LAUNCH_VELOCITY_SHIFT);
                if juiced {
                    p.flags |= flags::POWERED_UP;
                }
                if backward {
                    p.flags |= flags::BOMB_BACKWARD;
                    let slow = |v: i16| -((((v as i32) >> 1) * 3) / 5) as i16;
                    p.vel[0] = slow(p.vel[0]);
                    p.vel[2] = slow(p.vel[2]);
                }
            }
            Kind::Missile if juiced => {
                p.vel[0] = shift_velocity(forward_x, LAUNCH_VELOCITY, LAUNCH_VELOCITY_SHIFT);
                p.vel[2] = shift_velocity(forward_z, LAUNCH_VELOCITY, LAUNCH_VELOCITY_SHIFT);
                p.flags |= flags::POWERED_UP;
            }
            Kind::Missile => {
                p.vel[0] = shift_velocity(forward_x, MISSILE_VELOCITY, MISSILE_VELOCITY_SHIFT);
                p.vel[2] = shift_velocity(forward_z, MISSILE_VELOCITY, MISSILE_VELOCITY_SHIFT);
            }
        }
        self.next_id = self.next_id.wrapping_add(1);
        self.projectiles.push(p);
        true
    }

    /// The kart in `slot` presses the fire button with a bomb in flight: it goes off at the end of
    /// this tick's flight.
    pub fn detonate(&mut self, slot: usize) {
        for p in &mut self.projectiles {
            if p.kind == Kind::Bomb && p.parent == slot {
                p.flags |= flags::DETONATE;
            }
        }
    }

    /// One game tick of every weapon, in the order they were fired. `bodies` are the karts as
    /// they are now.
    /// `random_number` is `sdata->randomNumber`.
    pub fn tick(
        &mut self,
        level: &Level,
        bodies: &[Body],
        elapsed_ms: i32,
        random_number: &mut i32,
    ) -> Tick {
        let mut tick = std::mem::take(&mut self.pending);
        let mut coll = Collision::default();
        // The mines come first (their bucket does).
        self.tick_mines(&mut mine::Ctx {
            bodies,
            elapsed_ms,
            random_number,
            tick: &mut tick,
            level: Some(level),
            coll: &mut coll,
        });
        for i in 0..self.projectiles.len() {
            if self.projectiles[i].dead {
                continue;
            }
            let mut p = self.projectiles[i].clone();
            let step = fly(
                &mut p,
                &mut coll,
                level,
                bodies,
                &self.projectiles,
                &self.mines,
                elapsed_ms,
                &mut tick.crate_hits,
            );
            for j in step.missiles_hit {
                self.projectiles[j].dead = true;
            }
            for j in step.mines_hit {
                self.mines[j].set_off(&mut tick);
            }
            if step.explodes {
                self.burst(i, &p, Some(level), bodies, &mut tick);
                p.dead = true;
            }
            self.projectiles[i] = p;
        }
        self.projectiles.retain(|p| !p.dead);
        tick
    }

    /// `RB_Burst_Init`'s damage: the karts within reach of the explosion at `p` are hit (not the
    /// one that fired it), and the missiles there are destroyed with no blast of their own.
    fn burst(
        &mut self,
        me: usize,
        p: &Projectile,
        level: Option<&Level>,
        bodies: &[Body],
        tick: &mut Tick,
    ) {
        let radius_squared = p.kind.blast_radius_squared(p.powered());
        if let Some(level) = level {
            tick.break_crates(level, p.pos, radius_squared, p.kind.model(), p.parent);
        }
        tick.explosions.push(Explosion {
            kind: p.kind,
            powered: p.powered(),
            pos: p.pos,
            parent: p.parent,
        });
        for (victim, body) in bodies.iter().enumerate() {
            if victim != p.parent && in_reach(p.pos, body.at, radius_squared) {
                tick.flashes.push(victim);
                tick.hits.push(Hit {
                    victim,
                    attacker: Some(p.parent),
                    reason: p.kind.reason(),
                    damage: vehicle::pick::damage::BLAST,
                });
            }
        }
        for mine in &mut self.mines {
            if in_reach(p.pos, mine.pos, radius_squared) {
                mine.set_off(tick);
            }
        }
        for (j, other) in self.projectiles.iter_mut().enumerate() {
            if j != me
                && other.kind == Kind::Missile
                && !other.dead
                && in_reach(p.pos, other.pos, radius_squared)
            {
                other.dead = true;
            }
        }
    }
}

impl Tick {
    /// `RB_Burst_CollLevInst` over the search of a blast at `pos`: the fruit and weapon crates
    /// within `radius_squared` are broken by it, as a hit of `model` by the kart `owner`.
    pub(crate) fn break_crates(
        &mut self,
        level: &Level,
        pos: [i32; 3],
        radius_squared: i32,
        model: i16,
        owner: usize,
    ) {
        // (The squares are of whole radii: 0x80, 0x140 and 0x200.)
        let radius = radius_squared.isqrt() as i16;
        for instance in instances_in_reach(level, svec(pos, 0), radius, radius_squared) {
            if CRATE_MODELS.contains(&level.instances[instance].model_id) {
                self.crate_hits.push(CrateHit {
                    instance,
                    model,
                    owner,
                });
            }
        }
    }
}

/// `PROC_CollideHitboxWithBucket`'s test of an instance at `at` against a sphere at `pos`: the
/// squared distance below `radius_squared`, given up on early where it would overflow.
fn in_reach(pos: [i32; 3], at: [i32; 3], radius_squared: i32) -> bool {
    let mut dist = 0i32;
    for axis in [0, 1, 2] {
        let delta = mips::sub_lo(pos[axis], at[axis]);
        let square = mips::mul_lo(delta, delta);
        dist = mips::add_lo(dist, square);
        if square > 0x0fff_ffff {
            return false;
        }
    }
    dist < radius_squared
}

/// What one weapon's flight came to.
struct Step {
    explodes: bool,
    /// The missiles (indices into the weapons) a bomb ran into, which are destroyed.
    missiles_hit: Vec<usize>,
    /// The mines (indices into the mines) it ran into, which go off.
    mines_hit: Vec<usize>,
}

impl Step {
    fn explode() -> Step {
        Step {
            explodes: true,
            missiles_hit: Vec::new(),
            mines_hit: Vec::new(),
        }
    }
}

/// `RB_Hazard_CollideWithDrivers`: the first kart, by slot, within the weapon's reach on the
/// ground plane and not too far above or below it. Karts held by the mask do not count, nor does
/// the kart that fired it while the weapon is new.
fn collide_with_drivers(p: &Projectile, bodies: &[Body]) -> Option<usize> {
    bodies.iter().enumerate().position(|(slot, body)| {
        if body.mask_grabbed || (p.parent_safety != 0 && slot == p.parent) {
            return false;
        }
        let [dx, dy, dz] = std::array::from_fn(|a| body.at[a].wrapping_sub(p.pos[a]));
        let flat = dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) as u32;
        if flat >= HIT_RADIUS {
            return false;
        }
        dy.wrapping_mul(dy) < (HIT_RADIUS << 2) as i32
    })
}

/// `RB_MovingExplosive_ThTick`, for the weapon `p`: steer, move, and look for what it hits. `all`
/// are the weapons as they were at the start of the tick; the crates it breaks go into `crate_hits`.
fn fly(
    p: &mut Projectile,
    coll: &mut Collision,
    level: &Level,
    bodies: &[Body],
    all: &[Projectile],
    mines: &[mine::Mine],
    elapsed_ms: i32,
    crate_hits: &mut Vec<CrateHit>,
) -> Step {
    p.time_alive = mips::add_lo(p.time_alive, elapsed_ms);

    // A kart that has gone invisible is lost.
    if p.target
        .is_some_and(|t| bodies.get(t).is_none_or(|b| b.invisible))
    {
        p.target = None;
    }

    match p.target.and_then(|t| bodies.get(t)) {
        Some(target) if p.blind == 0 => {
            let delta_x = mips::sub_lo(mips::sra(target.pos_curr[0], 8), p.pos[0]);
            let delta_z = mips::sub_lo(mips::sra(target.pos_curr[2], 8), p.pos[2]);
            let desired = math::ratan2(delta_x, delta_z) as i16;
            match p.kind {
                Kind::Bomb => {
                    p.rot_y = math::interpolate(p.rot_y, desired, BOMB_TURN);
                    let (sin, cos) = (math::sin(p.rot_y as u32), math::cos(p.rot_y as u32));
                    p.vel[0] = heading_velocity(sin, LAUNCH_VELOCITY, LAUNCH_VELOCITY_SHIFT);
                    p.vel[2] = heading_velocity(cos, LAUNCH_VELOCITY, LAUNCH_VELOCITY_SHIFT);
                    if p.flags & flags::BOMB_BACKWARD != 0 {
                        p.vel[2] = p.vel[2].wrapping_neg();
                        p.vel[0] = p.vel[0].wrapping_neg();
                    }
                }
                Kind::Missile => {
                    if p.powered() {
                        p.rot_y = math::interpolate(p.rot_y, desired, MISSILE_TURN_POWERED);
                        let (sin, cos) = (math::sin(p.rot_y as u32), math::cos(p.rot_y as u32));
                        p.vel[0] = heading_velocity(sin, LAUNCH_VELOCITY, LAUNCH_VELOCITY_SHIFT);
                        p.vel[2] = heading_velocity(cos, LAUNCH_VELOCITY, LAUNCH_VELOCITY_SHIFT);
                    } else {
                        p.rot_y = math::interpolate(p.rot_y, desired, MISSILE_TURN);
                        let (sin, cos) = (math::sin(p.rot_y as u32), math::cos(p.rot_y as u32));
                        p.vel[0] = heading_velocity(sin, MISSILE_VELOCITY, MISSILE_VELOCITY_SHIFT);
                        p.vel[2] = heading_velocity(cos, MISSILE_VELOCITY, MISSILE_VELOCITY_SHIFT);
                    }
                }
            }
        }
        _ => {
            if p.blind != 0 {
                p.blind -= 1;
            }
        }
    }

    let advance = |p: &mut Projectile| {
        for axis in 0..3 {
            p.pos[axis] = mips::add_lo(p.pos[axis], mips::sra(p.vel[axis] as i32 * elapsed_ms, 5));
        }
    };
    advance(p);

    // The bomb rolls the way it travels.
    if p.kind == Kind::Bomb {
        p.roll = if p.flags & flags::BOMB_BACKWARD != 0 {
            p.roll.wrapping_sub(ROLL_STEP)
        } else {
            p.roll.wrapping_add(ROLL_STEP)
        };
    }

    // The surface under it, and the instances' hitboxes it passes through.
    {
        let q = &mut coll.sps.quad_block_coll;
        q.quad_flags_wanted = (quad_flags::GROUND | quad_flags::TRIGGER) as u32;
        q.quad_flags_ignored = 0;
        q.search_flags = search_flags::TEST_INSTANCES
            | search_flags::HIGH_LOD
            | search_flags::FORCE_INSTANCE_HIT;
    }
    let pos_a = svec(p.pos, GROUND_PROBE_BELOW);
    let pos_b = svec(p.pos, GROUND_PROBE_ABOVE);
    coll.search_quadblocks(level, pos_a, pos_b, 0);

    if coll.sps.collision.step_flags & WEAPON_REACT != 0 {
        // Back a tick, and off it goes.
        for v in &mut p.vel {
            *v = v.wrapping_neg();
        }
        advance(p);
        return Step::explode();
    }

    if coll.sps.did_touch_hitbox != 0 {
        // The crates it passes through break (`RB_Hazard_CollLevInst`'s `LInC`).
        if let Some(instance) = coll.sps.bsp_hitbox.and_then(|hit| hit.instance)
            && CRATE_MODELS.contains(&level.instances[instance].model_id)
        {
            crate_hits.push(CrateHit {
                instance,
                model: p.kind.model(),
                owner: p.parent,
            });
        }
        if hits_level_instance(level, coll) {
            return Step::explode();
        }
    } else if coll.sps.did_touch_quadblock != 0 {
        // It rides on the ground.
        p.vel[1] = 0;
        p.normal = coll.sps.hit.plane.normal;
        let hit = coll.sps.quad_block_coll.hit_pos;
        p.pos = [hit.x as i32, hit.y as i32 + RIDE_HEIGHT, hit.z as i32];
    } else {
        // Nothing under it: look further down before it falls, and go off if there is no ground.
        let far = svec(p.pos, AIR_PROBE_BELOW);
        coll.search_quadblocks(level, far, pos_b, 0);
        if coll.sps.did_touch_quadblock == 0 {
            return Step::explode();
        }
        let fall = if p.kind == Kind::Missile {
            elapsed_ms << 3
        } else {
            elapsed_ms << 2
        };
        p.vel[1] = (p.vel[1] as i32 - (fall >> 5)) as i16;
        if p.vel[1] < FALL_SPEED_MAX {
            p.vel[1] = FALL_SPEED_MAX;
        }
    }

    // A kart in the way.
    if let Some(victim) = collide_with_drivers(p, bodies) {
        if p.target == Some(victim) {
            p.flags |= flags::HIT_TARGET;
        }
        return Step::explode();
    }

    // A mine in the way goes off, and so does the weapon.
    if let Some(hit) = mines.iter().position(|m| {
        let [dx, dy, dz] = std::array::from_fn(|a| m.pos[a].wrapping_sub(p.pos[a]));
        let distance = dx
            .wrapping_mul(dx)
            .wrapping_add(dz.wrapping_mul(dz))
            .wrapping_add(dy.wrapping_mul(dy)) as u32;
        !m.is_dead() && distance < HIT_RADIUS
    }) {
        return Step {
            explodes: true,
            missiles_hit: Vec::new(),
            mines_hit: vec![hit],
        };
    }

    if p.kind == Kind::Bomb {
        // A bomb is stopped by a missile, which goes with it.
        let missile = all.iter().position(|m| {
            m.kind == Kind::Missile && {
                let [dx, dy, dz] = std::array::from_fn(|a| m.pos[a].wrapping_sub(p.pos[a]));
                let distance = dx
                    .wrapping_mul(dx)
                    .wrapping_add(dz.wrapping_mul(dz))
                    .wrapping_add(dy.wrapping_mul(dy)) as u32;
                distance < HIT_RADIUS
            }
        });
        if let Some(missile) = missile {
            return Step {
                explodes: true,
                missiles_hit: vec![missile],
                mines_hit: Vec::new(),
            };
        }
        if p.flags & flags::DETONATE != 0 {
            return Step::explode();
        }
    }
    p.parent_safety = (p.parent_safety - 1).max(0);
    Step {
        explodes: false,
        missiles_hit: Vec::new(),
        mines_hit: Vec::new(),
    }
}

/// `RB_Hazard_CollLevInst`: whether the instance hitbox the search ended on blows the weapon up.
/// The pickups let it through (a crate breaks as it passes, see [`CrateHit`]); the rest of the level's
/// instances stop it.
fn hits_level_instance(level: &Level, coll: &Collision) -> bool {
    let Some(instance) = coll.sps.bsp_hitbox.and_then(|hit| hit.instance) else {
        return true;
    };
    !PICKUP_MODELS.contains(&level.instances[instance].model_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(x: i32, z: i32) -> Body {
        Body {
            at: [x, 0, z],
            pos_curr: [x << 8, 0, z << 8],
            mask_grabbed: false,
            invisible: false,
            ..Body::default()
        }
    }

    fn weapon(kind: Kind, parent: usize) -> Projectile {
        Projectile {
            id: 0,
            kind,
            pos: [0, 0, 0],
            vel: [0; 3],
            flags: 0,
            rot_y: 0,
            roll: 0,
            normal: SVec3::default(),
            parent,
            target: None,
            parent_safety: PARENT_SAFETY_TICKS,
            blind: 0,
            time_alive: 0,
            dead: false,
        }
    }

    #[test]
    fn a_burst_hits_the_karts_in_reach_but_not_the_one_that_fired() {
        let mut weapons = Weapons {
            projectiles: vec![weapon(Kind::Bomb, 0)],
            ..Weapons::default()
        };
        // The bomb's blast reaches 0x19000 squared: 400 units. Kart 1 is at 300, kart 2 at 500.
        let bodies = [body(0, 0), body(300, 0), body(500, 0)];
        let mut tick = Tick::default();
        let p = weapons.projectiles[0].clone();
        weapons.burst(0, &p, None, &bodies, &mut tick);
        assert_eq!(
            tick.hits,
            vec![Hit {
                victim: 1,
                attacker: Some(0),
                reason: Reason::Bomb,
                damage: vehicle::pick::damage::BLAST,
            }]
        );
        assert_eq!(tick.explosions.len(), 1);
    }

    #[test]
    fn a_powered_up_bomb_reaches_further_and_a_missile_less() {
        let at = |kind, powered| {
            let mut p = weapon(kind, 0);
            if powered {
                p.flags |= flags::POWERED_UP;
            }
            let mut weapons = Weapons {
                projectiles: vec![p.clone()],
                ..Weapons::default()
            };
            let mut tick = Tick::default();
            weapons.burst(
                0,
                &p,
                None,
                &[body(0, 0), body(450, 0), body(100, 0)],
                &mut tick,
            );
            tick.hits.iter().map(|h| h.victim).collect::<Vec<_>>()
        };
        assert_eq!(at(Kind::Bomb, false), vec![2]);
        assert_eq!(at(Kind::Bomb, true), vec![1, 2]);
        assert_eq!(at(Kind::Missile, false), vec![2]);
    }

    #[test]
    fn a_burst_destroys_the_missiles_near_it() {
        let mut near = weapon(Kind::Missile, 1);
        near.pos = [100, 0, 0];
        let mut far = weapon(Kind::Missile, 1);
        far.pos = [900, 0, 0];
        let bomb = weapon(Kind::Bomb, 0);
        let mut weapons = Weapons {
            projectiles: vec![bomb.clone(), near, far],
            ..Weapons::default()
        };
        weapons.burst(0, &bomb, None, &[body(0, 0)], &mut Tick::default());
        assert_eq!(
            weapons
                .projectiles
                .iter()
                .map(|p| p.dead)
                .collect::<Vec<_>>(),
            [false, true, false]
        );
    }

    #[test]
    fn a_weapon_goes_through_the_kart_that_fired_it_for_a_while() {
        let bodies = [body(10, 0), body(5000, 0)];
        let mut p = weapon(Kind::Bomb, 0);
        assert_eq!(collide_with_drivers(&p, &bodies), None);
        p.parent_safety = 0;
        assert_eq!(collide_with_drivers(&p, &bodies), Some(0));
        // Others are hit at once, by the flat distance.
        let mut p = weapon(Kind::Missile, 1);
        assert_eq!(collide_with_drivers(&p, &bodies), Some(0));
        p.pos = [0, 0x2000, 0];
        assert_eq!(collide_with_drivers(&p, &bodies), None);
    }

    #[test]
    fn firing_is_refused_when_too_many_are_in_flight() {
        let mut weapons = Weapons {
            projectiles: vec![weapon(Kind::Missile, 0); ACTIVE_LIMIT],
            ..Weapons::default()
        };
        let d = Driver::default();
        assert!(!weapons.launch(Kind::Bomb, 0, &d, &[body(0, 0)], &mut Gte::default(), false));
        weapons.projectiles.pop();
        assert!(weapons.launch(Kind::Bomb, 0, &d, &[body(0, 0)], &mut Gte::default(), false));
    }

    #[test]
    fn a_bomb_thrown_backwards_is_slower_and_a_powered_one_is_marked() {
        let mut d = Driver::default();
        d.axis_angle1_normal_vec = SVec3 {
            x: 0,
            y: 0x1000,
            z: 0,
        };
        d.num_wumpas = 10;
        let mut weapons = Weapons::default();
        let bodies = [body(0, 0)];
        weapons.launch(Kind::Bomb, 0, &d, &bodies, &mut Gte::default(), false);
        weapons.launch(Kind::Bomb, 0, &d, &bodies, &mut Gte::default(), true);
        let [ahead, back] = &weapons.projectiles[..] else {
            panic!()
        };
        // Facing +z at a speed of 3/128 of the unit vector.
        assert_eq!(ahead.vel, [0, 0, 96]);
        assert!(ahead.powered() && ahead.flags & flags::BOMB_BACKWARD == 0);
        assert_eq!(back.vel, [0, 0, -((96 >> 1) * 3 / 5) as i16]);
        assert!(back.flags & flags::BOMB_BACKWARD != 0);
        assert!(weapons.has_bomb(0) && !weapons.has_bomb(1));
    }

    #[test]
    fn a_detonated_bomb_is_marked_and_missiles_are_not() {
        let mut weapons = Weapons {
            projectiles: vec![weapon(Kind::Bomb, 0), weapon(Kind::Missile, 0)],
            ..Weapons::default()
        };
        weapons.detonate(0);
        assert!(weapons.projectiles[0].flags & flags::DETONATE != 0);
        assert_eq!(weapons.projectiles[1].flags, 0);
    }
}
