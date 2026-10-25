//! The mines a kart drops with the TNT item (`VehPickupItem_ShootNow`'s `WEAPON_ID_MINE`): a TNT
//! crate, or a Nitro crate for a kart with 10 wumpa fruit. What they do (`RB_GenericMine_ThTick`):
//! they grow where they were dropped, fall to the ground and wait there for a kart.
//!
//! A kart that touches a Nitro crate is blasted, and the crate goes off. A TNT crate jumps onto the
//! head of the kart instead (`RB_TNT_ThTick_ThrowOnHead`), squashes and stretches there for three
//! seconds (`SitOnHead`) and goes off, unless the kart jumps it off (9 jumps) or is hit by
//! something else first. The mine pool keeps no more than [`POOL`] of them on the ground: dropping
//! one more sets the oldest off.
//!
//! Going off (`RB_Blowup_Init`) blasts every kart in reach and sets off the other mines near it,
//! which go with no blast of their own (`RB_Hazard_ThCollide_Generic`).
//!
//! The potion (`STATIC_BEAKER_GREEN`, and the red one of a kart with 10 wumpa fruit) is a mine
//! too: dropped, it lies where it fell; thrown (forward with Up, backward with Down) it flies
//! (`RB_Potion_ThTick_InAir`) and lies where it lands. A kart that touches one spins out, its
//! screen flashes, and the beaker shatters, with no blast.
//!
//! Not ported: the red beaker's rain cloud, the shatter's particles, the Tiger Temple door that a
//! shattering potion opens, the afterimage that follows a mine dropped at speed, the bots' TNT, a
//! mine dropped on a crate holding the crate's regrowth back, and the sounds.

use coll::{Collision, search_flags};
use gte::Gte;
use loader::lev::{Level, SVec3, quad_flags};
use math::mips;
use vehicle::{Driver, Matrix, kart_state, pick};

use crate::{Body, Hit, Reason, Tick, WEAPON_REACT, Weapons, in_reach, svec};

/// `RB_MinePool_Capacity`: how many mines lie around (those on a head do not count).
pub const POOL: usize = 10;

/// `MINE_PARENT_SAFETY_FRAMES`: how long a mine lets the kart that dropped it pass.
const PARENT_SAFETY_TICKS: i16 = 10;
/// `MINE_COLL_PROBE_TOP_Y_OFFSET`, `_BOTTOM_`, and the hit radius of the search.
const PROBE_TOP: i32 = -400;
const PROBE_BOTTOM: i32 = 64;
const PROBE_RADIUS: i32 = 0x40;

/// `RB_Hazard_CollideWithDrivers`' reach for a mine: a squared distance. A beaker's is smaller.
const HIT_RADIUS: u32 = 0x3840;
const BEAKER_HIT_RADIUS: u32 = 0x1900;
/// `VehPickupItem_PotionThrow`: the speed of a throw (`POTION_THROW_SPEED`, over 1 << 12), and
/// how high it goes up.
const THROW_SPEED: i32 = 0x78;
const THROW_SHIFT: u32 = 12;
const THROW_Y_VELOCITY: i16 = 0x30;
/// `mw->cooldown` of a beaker in the air, and once it has landed (3.84 s): a kart that touches it
/// within that time is hit as by a thrown one.
const AIR_COOLDOWN_MS: i16 = 0xf0;
const LANDED_COOLDOWN_MS: i16 = 0xf00;
/// `RB_Potion_ThTick_InAir`'s probe of the ground: below and above the beaker, and the deeper one
/// when that finds nothing; and the slacks of landing and of sliding along the ground.
const AIR_PROBE_BELOW: i32 = -0x40;
const AIR_PROBE_ABOVE: i32 = 0x100;
const AIR_PROBE_DEEP: i32 = -0x900;
const LANDING_SLACK: i32 = 0x30;
const SLIDE_RISE: i32 = 0x28;
/// A mine's size (`inst->scale`), full, and the step it grows by.
const FULL_SCALE: i16 = 0x1000;
const GROW_STEP: i16 = 0x200;
/// The size of a TNT crate on a head.
const HEAD_SCALE: i16 = 0x800;
/// How much a TNT crate shrinks by as it flies to a head.
const SHRINK_STEP: i16 = 0x100;
/// How fast a TNT crate jumps up on the way to a head and off it.
const JUMP_VELOCITY: i16 = 0x30;
const FALL_SPEED_MAX: i16 = -0x60;
/// `mw->stopFallAtY` of a TNT crate that has not found where to land yet.
const LAND_UNSET: i32 = 0x3fff;
/// How a TNT crate spins on its way to a head.
const SPIN_STEP: i16 = 0x100;
/// `numFramesOnHead` at which it goes off: 3 seconds.
const FUSE_TICKS: i16 = 0x5a;
/// The jumps that do not shake it off (`jumpsRemaining` is 8 or 9).
const JUMPS_BASE: i32 = 8;

/// `RB_Blowup_Init`'s reach for the karts, for Nitro and TNT; and for the other mines.
const BLOWUP_REACH: i32 = 0x19000;
const NITRO_BLOWUP_REACH: i32 = 0x4000;

/// `s_tntTables`: the scale of a TNT crate on a head by the tick (`xz`, `y`), 90 of them, and then
/// how high the head is by character (16 of them). The last tick's scale is the first heights, as
/// in retail.
const TNT_TABLE: [i16; 0x5a * 2 + 0x10] = [
    2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048,
    2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2355, 2148, 2539, 2248, 2462, 2293, 2280, 2273,
    2061, 2220, 1878, 2148, 1802, 2070, 1815, 1998, 1850, 1945, 1898, 1925, 1951, 1934, 1999, 1957,
    2034, 1986, 2048, 2038, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048,
    2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2048, 2560, 2214, 2867, 2381,
    2739, 2457, 2434, 2423, 2070, 2335, 1766, 2215, 1638, 2085, 1649, 1965, 1679, 1877, 1728, 1843,
    1791, 1857, 1867, 1899, 1954, 1964, 2048, 2048, 2150, 2150, 2048, 2048, 1945, 1945, 2048, 2048,
    2150, 2150, 2048, 2048, 1945, 1945, 2048, 2048, 2150, 2150, 2123, 2123, 2074, 2074, 2048, 2048,
    2728, 2270, 3137, 2491, 2967, 2592, 2562, 2532, 2078, 2381, 1673, 2184, 1503, 1987, 1620, 1836,
    1878, 1775, 2136, 2014, 2253, 2253, 2052, 2052, 1851, 1851, 2067, 2067, 2283, 2283, 1943, 1943,
    2331, 2331, 1926, 1926, 2344, 2344, 1899, 1899, 2379, 2379, 1904, 1904, 2327, 2327, 1926, 1926,
    2379, 2379, 3072, 3072, 32, 32, 64, 32, 32, 48, 32, 32, 48, 64, 32, 48, 56, 24, 32, 56,
];

/// Which crate a mine is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MineKind {
    /// `STATIC_CRATE_TNT`
    Tnt,
    /// `PU_EXPLOSIVE_CRATE`
    Nitro,
    /// `STATIC_BEAKER_GREEN`
    GreenBeaker,
    /// `STATIC_BEAKER_RED`
    RedBeaker,
}

impl MineKind {
    /// A potion, which spins the kart out where the crates blast it.
    pub fn is_beaker(self) -> bool {
        matches!(self, MineKind::GreenBeaker | MineKind::RedBeaker)
    }
}

/// How a kart sends a potion off (`POTION_THROW_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Throw {
    /// Dropped behind the kart, like a crate.
    Drop,
    /// With Up held.
    Forward,
    /// With Down held.
    Backward,
}

/// What a kart sends off with its item: where from, and how.
pub struct Release<'a> {
    /// The kart's slot.
    pub shooter: usize,
    pub driver: &'a Driver,
    /// The kart's, for the matrix of what it drops.
    pub gte: &'a mut Gte,
    /// A potion, and how; `None` for the TNT item's crate.
    pub potion: Option<Throw>,
}

/// What a mine is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// `RB_GenericMine_ThTick`: lying on the ground.
    Ground,
    /// `RB_TNT_ThTick_ThrowOnHead`: a TNT crate jumping onto the head of `target`, `height` over
    /// the kart's instance, in the kart's own axes.
    Landing {
        target: usize,
        height: i16,
        spin: i16,
    },
    /// `RB_TNT_ThTick_SitOnHead`: on the head of `target` for `frames` ticks, with `jumps` jumps
    /// to go before the next one shakes it off.
    Sitting {
        target: usize,
        frames: i16,
        jumps: i32,
    },
    /// `RB_TNT_ThTick_ThrowOffHead`: shaken off by `target`, falling back down.
    Falling { target: usize },
    /// `RB_Potion_ThTick_InAir`: a thrown beaker, on its way down to the ground.
    InAir,
}

/// A TNT or Nitro crate (`struct MineWeapon` and the instance it moves).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mine {
    /// See [`crate::Projectile::id`].
    pub id: u32,
    pub kind: MineKind,
    /// `matrix.t`, in level units.
    pub pos: [i32; 3],
    /// `matrix` without the translation.
    pub rot: Matrix,
    /// `scale.x` (and `z`), and `scale.y`; 0x1000 is full size.
    pub scale_xz: i16,
    pub scale_y: i16,
    pub vel: [i16; 3],
    /// The kart that dropped it.
    pub parent: usize,
    pub state: State,
    /// `mw->cooldown`, in ms: counts down while a beaker flies and lies; a hit while it runs is
    /// credited to a thrown potion.
    cooldown: i16,
    parent_safety: i16,
    stop_fall_y: i32,
    /// `weaponSlot231 != NULL`: counted by the pool.
    pooled: bool,
    /// `boolDestroyed` by the pool, not yet gone off.
    overflowed: bool,
    dead: bool,
}

impl Mine {
    pub(crate) fn is_dead(&self) -> bool {
        self.dead
    }

    /// A TNT crate that is on or flying to a head, and so is not the pool's (`instTntRecv`).
    fn carried_by(&self) -> Option<usize> {
        match self.state {
            State::Landing { target, .. } | State::Sitting { target, .. } => Some(target),
            _ => None,
        }
    }

    /// Whether a TNT crate has a kart it was on (`driverTarget`): these go off by themselves.
    fn has_target(&self) -> bool {
        self.state != State::Ground
    }
}

/// `LHMatrix_Parent`: the matrix of a kart's instance, and the point `height` above it in its
/// own axes.
fn on_head(body: &Body, height: i16) -> (Matrix, [i32; 3]) {
    let at = std::array::from_fn(|a| {
        mips::add_lo(
            mips::sra(mips::mul_lo(body.rot.m[a][1] as i32, height as i32), 12),
            body.at[a],
        )
    });
    (body.rot, at)
}

/// What a tick of the mines needs besides the mines.
pub(crate) struct Ctx<'a> {
    pub bodies: &'a [Body],
    pub elapsed_ms: i32,
    pub random_number: &'a mut i32,
    pub tick: &'a mut Tick,
    /// What a beaker in the air looks at for the ground. (A test that has no level leaves a
    /// beaker in the air where it is.)
    pub level: Option<&'a Level>,
    pub coll: &'a mut Collision,
}

impl Weapons {
    /// The mines there are.
    pub fn mines(&self) -> &[Mine] {
        &self.mines
    }

    /// Whether kart `slot` has a TNT crate on its head, or on its way there (`instTntRecv`).
    pub fn carries_tnt(&self, slot: usize) -> bool {
        self.mines
            .iter()
            .any(|m| !m.dead && m.carried_by() == Some(slot))
    }

    /// The `scale.y` of the TNT crate on the head of kart `slot`, or on its way there
    /// (`instTntRecv->scale.y`), if it has one.
    pub fn tnt_scale_y(&self, slot: usize) -> Option<i16> {
        self.mines
            .iter()
            .find(|m| !m.dead && m.carried_by() == Some(slot))
            .map(|m| m.scale_y)
    }

    /// `VehPickupItem_ShootNow` for the mine: a kart drops a TNT crate, or a Nitro crate with 10
    /// wumpa fruit, where it is; or a potion (red with 10 wumpa fruit, otherwise green), which it
    /// may throw as well. It goes off at once if it is dropped into something, which the next
    /// `tick` says.
    pub fn drop_mine(&mut self, release: Release, level: &Level, bodies: &[Body], elapsed_ms: i32) {
        let Release {
            shooter,
            driver,
            gte,
            potion,
        } = release;
        let juiced = driver.num_wumpas >= crate::JUICED;
        let kind = match (potion.is_some(), juiced) {
            (false, true) => MineKind::Nitro,
            (false, false) => MineKind::Tnt,
            (true, true) => MineKind::RedBeaker,
            (true, false) => MineKind::GreenBeaker,
        };
        let (facing, at) = vehicle::baked::inst_matrix(driver);
        let mut mine = Mine {
            id: self.next_id,
            kind,
            pos: at,
            rot: Matrix::default(),
            scale_xz: 0,
            scale_y: 0,
            vel: [0; 3],
            parent: shooter,
            state: State::Ground,
            cooldown: 0,
            parent_safety: PARENT_SAFETY_TICKS,
            stop_fall_y: 0,
            pooled: false,
            overflowed: false,
            dead: false,
        };
        self.next_id = self.next_id.wrapping_add(1);
        self.add_to_pool(&mut mine);

        // A thrown potion leaves the kart's hands at the kart's size, and flies.
        if let Some(throw @ (Throw::Forward | Throw::Backward)) = potion {
            let speed = if throw == Throw::Forward {
                THROW_SPEED
            } else {
                -THROW_SPEED
            };
            let velocity = |m: i16| mips::sra(m as i32 * speed, THROW_SHIFT) as i16;
            mine.rot = facing;
            mine.vel = [
                velocity(facing.m[0][2]),
                THROW_Y_VELOCITY,
                velocity(facing.m[2][2]),
            ];
            (mine.scale_xz, mine.scale_y) = (FULL_SCALE, FULL_SCALE);
            mine.cooldown = AIR_COOLDOWN_MS;
            mine.state = State::InAir;
            self.mines.push(mine);
            return;
        }

        // Where the ground is, and whether it was dropped into a level instance.
        let mut coll = Collision::default();
        {
            let q = &mut coll.sps.quad_block_coll;
            q.quad_flags_wanted = quad_flags::GROUND as u32;
            q.quad_flags_ignored = 0;
            q.search_flags = search_flags::TEST_INSTANCES | search_flags::HIGH_LOD;
        }
        let top = svec(at, PROBE_TOP);
        let bottom = svec(at, PROBE_BOTTOM);
        coll.search_quadblocks(level, top, bottom, PROBE_RADIUS);
        let mut destroyed = false;
        if coll.sps.did_touch_hitbox != 0 {
            // The crates the kart drops it on let it through; the rest of the level blows it up.
            let model = coll
                .sps
                .bsp_hitbox
                .and_then(|hit| hit.instance)
                .map(|i| level.instances[i].model_id);
            destroyed = !matches!(model, Some(7 | 8));
            coll.sps.quad_block_coll.search_flags = 0;
            coll.search_quadblocks(level, top, bottom, PROBE_RADIUS);
        }
        let normal = if coll.sps.did_touch_quadblock != 0 {
            mine.stop_fall_y = coll.sps.quad_block_coll.hit_pos.y as i32;
            coll.sps.hit.plane.normal
        } else {
            mine.stop_fall_y = mine.pos[1];
            SVec3 {
                x: 0,
                y: 0x1000,
                z: 0,
            }
        };
        vehicle::force::rot_axis_angle(&mut mine.rot, gte, normal, driver.angle as i32);

        let me = self.mines.len();
        self.mines.push(mine);
        if destroyed {
            let mut m = self.mines[me].clone();
            let mut random_number = 0;
            let mut tick = std::mem::take(&mut self.pending);
            let mut c = Ctx {
                bodies,
                elapsed_ms,
                random_number: &mut random_number,
                tick: &mut tick,
                level: Some(level),
                coll: &mut coll,
            };
            self.destroy(me, &mut m, &mut c);
            self.mines[me] = m;
            self.pending = tick;
        }
    }

    /// `RB_MinePool_Add`: a new mine on the ground, and the oldest one set off if there are too
    /// many.
    fn add_to_pool(&mut self, mine: &mut Mine) {
        let pooled = self.mines.iter().filter(|m| m.pooled).count();
        if pooled >= POOL
            && let Some(oldest) = self
                .mines
                .iter_mut()
                .filter(|m| m.pooled)
                .min_by_key(|m| m.id)
        {
            oldest.pooled = false;
            oldest.overflowed = true;
        }
        mine.pooled = true;
    }

    /// One game tick of every mine, in the order they were dropped.
    pub(crate) fn tick_mines(&mut self, c: &mut Ctx) {
        for i in 0..self.mines.len() {
            if self.mines[i].dead {
                continue;
            }
            let mut m = self.mines[i].clone();
            self.tick_mine(i, &mut m, c);
            self.mines[i] = m;
        }
        self.mines.retain(|m| !m.dead);
    }

    fn tick_mine(&mut self, me: usize, m: &mut Mine, c: &mut Ctx) {
        match m.state {
            State::Ground => self.on_ground(me, m, c),
            State::Landing { .. } => self.landing(me, m, c),
            State::Sitting { .. } => self.sitting(me, m, c),
            State::Falling { .. } => self.falling(me, m, c),
            State::InAir => self.in_air(me, m, c),
        }
    }

    /// `RB_GenericMine_ThTick`.
    fn on_ground(&mut self, me: usize, m: &mut Mine, c: &mut Ctx) {
        let dt = c.elapsed_ms;
        m.cooldown = (m.cooldown as i32 - dt).max(0) as i16;
        m.pos[1] = mips::add_lo(m.pos[1], mips::sra(m.vel[1] as i32 * dt, 5));
        if m.pos[1] < m.stop_fall_y {
            m.pos[1] = m.stop_fall_y;
        }
        m.vel[1] = (m.vel[1] as i32 - mips::sra(dt << 2, 5)).max(FALL_SPEED_MAX as i32) as i16;
        if m.scale_xz < FULL_SCALE {
            m.scale_xz += GROW_STEP;
            m.scale_y += GROW_STEP;
        } else {
            (m.scale_xz, m.scale_y) = (FULL_SCALE, FULL_SCALE);
        }

        if let Some(slot) = collide_with_drivers(m, c.bodies) {
            let body = c.bodies[slot];
            let hit = |damage| Hit {
                victim: slot,
                attacker: Some(m.parent),
                reason: Reason::Mine,
                damage,
            };
            let carried = !m.kind.is_beaker()
                && self
                    .mines
                    .iter_mut()
                    .enumerate()
                    .find(|(j, o)| *j != me && !o.dead && o.carried_by() == Some(slot))
                    .map(|(_, o)| o.dead = true)
                    .is_some();
            if m.kind.is_beaker() {
                // The kart spins out, its screen flashes, and the beaker shatters. (A potion
                // that has only just been thrown is credited as a thrown one.)
                c.tick.hits.push(Hit {
                    reason: if m.cooldown != 0 {
                        Reason::Potion
                    } else {
                        Reason::Mine
                    },
                    ..hit(pick::damage::SPIN)
                });
                c.tick.flashes.push(slot);
                self.destroy(me, m, c);
            } else if carried {
                // The kart has a TNT crate on its head already: both go off, on it.
                c.tick.hits.push(Hit {
                    attacker: None,
                    ..hit(pick::damage::BLAST)
                });
                self.destroy(me, m, c);
            } else if body.squished {
                c.tick.hits.push(Hit {
                    attacker: None,
                    ..hit(pick::damage::BLAST)
                });
                self.destroy(me, m, c);
            } else if m.kind == MineKind::Nitro {
                c.tick.hits.push(hit(pick::damage::BLAST));
                self.destroy(me, m, c);
            } else {
                // TNT: onto the head, unless the kart is shielded from it.
                c.tick.hits.push(hit(pick::damage::NONE));
                if body.immune {
                    self.destroy(me, m, c);
                } else {
                    m.pooled = false;
                    m.vel = [0, JUMP_VELOCITY, 0];
                    m.stop_fall_y = LAND_UNSET;
                    m.state = State::Landing {
                        target: slot,
                        height: 0,
                        spin: 0,
                    };
                    self.landing(me, m, c);
                    return;
                }
            }
        }

        if m.parent_safety != 0 {
            m.parent_safety -= 1;
        }
        // The pool set it off for a newer mine.
        if m.overflowed && !m.dead {
            self.go_off(me, m, c);
            m.dead = true;
        }
    }

    /// A crate blows up, with its blast; a beaker only shatters.
    fn go_off(&mut self, me: usize, m: &mut Mine, c: &mut Ctx) {
        if m.kind.is_beaker() {
            c.tick.detonations.push(Detonation::of(m, false));
        } else {
            self.blowup(me, m, c);
        }
    }

    /// `RB_GenericMine_ThDestroy`: it goes off where it is.
    fn destroy(&mut self, me: usize, m: &mut Mine, c: &mut Ctx) {
        self.go_off(me, m, c);
        m.scale_xz = 0;
        m.scale_y = 0;
        m.pooled = false;
        m.dead = true;
    }

    /// `RB_Blowup_Init`: the blast (a Nitro crate's is smaller on karts), and every mine within
    /// reach, this one too, set off (`RB_Hazard_ThCollide_Generic`).
    fn blowup(&mut self, me: usize, m: &mut Mine, c: &mut Ctx) {
        c.tick.detonations.push(Detonation::of(m, true));
        // (A beaker does not blow up: `go_off` does not come here for it.)
        let reach = match m.kind {
            MineKind::Nitro => NITRO_BLOWUP_REACH,
            _ => BLOWUP_REACH,
        };
        for (victim, body) in c.bodies.iter().enumerate() {
            if in_reach(m.pos, body.at, reach) {
                c.tick.flashes.push(victim);
                c.tick.hits.push(Hit {
                    victim,
                    attacker: Some(m.parent),
                    reason: Reason::Mine,
                    damage: pick::damage::BLAST,
                });
            }
        }
        for (j, other) in self.mines.iter_mut().enumerate() {
            if j != me && in_reach(m.pos, other.pos, BLOWUP_REACH) {
                other.set_off(c.tick);
            }
        }
        m.set_off(c.tick);
    }

    /// `RB_TNT_ThTick_ThrowOnHead`.
    fn landing(&mut self, me: usize, m: &mut Mine, c: &mut Ctx) {
        let State::Landing {
            target,
            mut height,
            mut spin,
        } = m.state
        else {
            return;
        };
        let Some(body) = c.bodies.get(target).copied() else {
            m.dead = true;
            return;
        };
        height = height.wrapping_add(mips::sra(m.vel[1] as i32 * c.elapsed_ms, 5) as i16);
        if m.vel[1] < 0 {
            let head = TNT_TABLE[0x5a * 2 + (body.character as usize & 0xf)];
            if height < head {
                height = head;
                if m.scale_xz == HEAD_SCALE {
                    // It has landed.
                    let jumps = JUMPS_BASE - math::mix_rng_scramble(c.random_number) % 2;
                    m.state = State::Sitting {
                        target,
                        frames: 0,
                        jumps,
                    };
                    self.sitting(me, m, c);
                    return;
                }
            }
        }
        let (matrix, at) = on_head(&body, height);
        m.pos = at;
        m.rot = vehicle::rotmat::matrix_rotate(
            &mut Gte::default(),
            &matrix,
            &vehicle::rotmat::rot_to_matrix(SVec3 {
                x: 0,
                y: spin,
                z: 0,
            }),
        );
        m.vel[1] =
            (m.vel[1] as i32 - mips::sra(c.elapsed_ms << 2, 5)).max(FALL_SPEED_MAX as i32) as i16;
        spin = spin.wrapping_add(SPIN_STEP);
        if m.scale_xz > HEAD_SCALE {
            m.scale_xz -= SHRINK_STEP;
            m.scale_y -= SHRINK_STEP;
        } else {
            (m.scale_xz, m.scale_y) = (HEAD_SCALE, HEAD_SCALE);
        }
        m.state = State::Landing {
            target,
            height,
            spin,
        };
    }

    /// `RB_TNT_ThTick_SitOnHead`.
    fn sitting(&mut self, me: usize, m: &mut Mine, c: &mut Ctx) {
        let State::Sitting {
            target,
            mut frames,
            mut jumps,
        } = m.state
        else {
            return;
        };
        let Some(body) = c.bodies.get(target).copied() else {
            m.dead = true;
            return;
        };
        let head = TNT_TABLE[0x5a * 2 + (body.character as usize & 0xf)];
        (m.rot, m.pos) = on_head(&body, head);

        match body.kart_state {
            // Whatever hurts the kart sets the crate off.
            kart_state::CRASHING | kart_state::MASK_GRABBED | kart_state::SPINNING => {
                self.blowup(me, m, c);
                self.vanish(m);
                return;
            }
            kart_state::BLASTED => {
                c.tick.detonations.push(Detonation::of(m, false));
                self.vanish(m);
                return;
            }
            _ => {}
        }

        if body.jump_started {
            if jumps != 0 {
                jumps -= 1;
            } else {
                // Shaken off: it jumps up from the head and falls back.
                m.scale_xz = HEAD_SCALE;
                m.scale_y = HEAD_SCALE;
                m.vel = [0, JUMP_VELOCITY, 0];
                m.stop_fall_y = LAND_UNSET;
                m.state = State::Falling { target };
                self.falling(me, m, c);
                return;
            }
        }

        if frames >= FUSE_TICKS {
            // The fuse is out.
            c.tick.hits.push(Hit {
                victim: target,
                attacker: Some(m.parent),
                reason: Reason::Tnt,
                damage: pick::damage::BLAST,
            });
            self.blowup(me, m, c);
            self.vanish(m);
            return;
        }
        frames += 1;
        let row = frames as usize * 2;
        m.scale_xz = TNT_TABLE[row];
        m.scale_y = TNT_TABLE[row + 1];
        m.state = State::Sitting {
            target,
            frames,
            jumps,
        };
    }

    /// `RB_TNT_ThTick_ThrowOffHead`.
    fn falling(&mut self, me: usize, m: &mut Mine, c: &mut Ctx) {
        let State::Falling { target } = m.state else {
            return;
        };
        m.pos[1] = mips::add_lo(m.pos[1], mips::sra(m.vel[1] as i32 * c.elapsed_ms, 5));
        if m.stop_fall_y == LAND_UNSET
            && let Some(body) = c.bodies.get(target)
        {
            m.stop_fall_y = body.at[1];
        }
        if m.pos[1] <= m.stop_fall_y {
            self.blowup(me, m, c);
            self.vanish(m);
            return;
        }
        m.vel[1] =
            (m.vel[1] as i32 - mips::sra(c.elapsed_ms << 2, 5)).max(FALL_SPEED_MAX as i32) as i16;
    }

    /// `RB_Potion_ThTick_InAir`: a thrown beaker flies, and lands, or shatters on whatever it
    /// hits (a level instance, a spot where the ground gives way, or nothing under it at all).
    fn in_air(&mut self, me: usize, m: &mut Mine, c: &mut Ctx) {
        let dt = c.elapsed_ms;
        for axis in 0..3 {
            m.pos[axis] = mips::add_lo(m.pos[axis], mips::sra(m.vel[axis] as i32 * dt, 5));
        }
        m.vel[1] = (m.vel[1] as i32 - mips::sra(dt << 2, 5)).max(FALL_SPEED_MAX as i32) as i16;
        m.cooldown = (m.cooldown as i32 - dt).max(0) as i16;

        let Some(level) = c.level else {
            return;
        };
        {
            let q = &mut c.coll.sps.quad_block_coll;
            q.quad_flags_wanted = (quad_flags::GROUND | quad_flags::TRIGGER) as u32;
            q.quad_flags_ignored = 0;
            q.search_flags = search_flags::TEST_INSTANCES
                | search_flags::HIGH_LOD
                | search_flags::FORCE_INSTANCE_HIT;
        }
        let bottom = svec(m.pos, AIR_PROBE_BELOW);
        let top = svec(m.pos, AIR_PROBE_ABOVE);
        c.coll.search_quadblocks(level, bottom, top, 0);
        let sps = &c.coll.sps;
        let reacted = sps.collision.step_flags & WEAPON_REACT != 0;
        let hitbox = sps.did_touch_hitbox != 0;
        let ground = sps.did_touch_quadblock != 0;
        let (hit_y, normal) = (sps.quad_block_coll.hit_pos.y as i32, sps.hit.plane.normal);

        // (Retail goes on to look at the rest after this, and may shatter it twice.) The door of
        // the Tiger Temple that a potion opens is not ported: it is a level instance like the rest.
        if reacted || hitbox {
            self.destroy(me, m, c);
            return;
        }
        if ground {
            vehicle::force::rot_axis_angle(&mut m.rot, &mut c.coll.gte, normal, 0);
            let before = m.pos[1];
            // Still well above it.
            if hit_y + LANDING_SLACK < before {
                return;
            }
            if m.cooldown == 0 {
                // It lands, and lies there.
                m.pos[1] = hit_y;
                m.stop_fall_y = hit_y;
                m.cooldown = LANDED_COOLDOWN_MS;
                m.vel = [0; 3];
                m.state = State::Ground;
                self.on_ground(me, m, c);
                return;
            }
            // Too soon after the throw to land: it slides along the ground.
            if before <= hit_y {
                m.pos[1] = hit_y;
            }
            let rise = m.pos[1] - before + SLIDE_RISE;
            if (m.vel[1] as i32) < rise {
                m.vel[1] = rise as i16;
            }
            return;
        }
        // Nothing within reach below: look deeper before it shatters in the air.
        let deep = svec(m.pos, AIR_PROBE_DEEP);
        c.coll.search_quadblocks(level, deep, top, 0);
        if c.coll.sps.did_touch_quadblock == 0 {
            self.destroy(me, m, c);
        }
    }

    /// A TNT crate that has gone off, or been taken away: gone.
    fn vanish(&mut self, m: &mut Mine) {
        m.scale_xz = 0;
        m.scale_y = 0;
        m.dead = true;
    }
}

impl Mine {
    /// `RB_Hazard_ThCollide_Generic`: something else sets the mine off. It goes with an explosion
    /// and no blast, unless it is a TNT crate with a kart (which goes off by itself).
    pub(crate) fn set_off(&mut self, tick: &mut Tick) {
        if self.dead || (self.kind == MineKind::Tnt && self.has_target()) {
            return;
        }
        tick.detonations.push(Detonation::of(self, false));
        self.pooled = false;
        self.scale_xz = 0;
        self.scale_y = 0;
        self.dead = true;
    }
}

/// `RB_Hazard_CollideWithDrivers` for a mine: the first kart, by slot, within reach in all three
/// directions. Karts held by the mask do not count, nor does the one that dropped it while it is new.
fn collide_with_drivers(m: &Mine, bodies: &[Body]) -> Option<usize> {
    let radius = if m.kind.is_beaker() {
        BEAKER_HIT_RADIUS
    } else {
        HIT_RADIUS
    };
    bodies.iter().enumerate().position(|(slot, body)| {
        if body.mask_grabbed || (m.parent_safety != 0 && slot == m.parent) {
            return false;
        }
        let [dx, dy, dz] = std::array::from_fn(|a| body.at[a].wrapping_sub(m.pos[a]));
        let distance = dx
            .wrapping_mul(dx)
            .wrapping_add(dz.wrapping_mul(dz))
            .wrapping_add(dy.wrapping_mul(dy)) as u32;
        distance < radius && dy.wrapping_mul(dy) < (radius << 2) as i32
    })
}

/// A mine that went off, for the outside to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detonation {
    pub kind: MineKind,
    pub pos: [i32; 3],
    pub rot: Matrix,
    /// `RB_Blowup_Init`: with a shockwave, and the explosion turned to the camera; otherwise
    /// `RB_Explosion_InitGeneric`'s plain one.
    pub blowup: bool,
}

impl Detonation {
    fn of(m: &Mine, blowup: bool) -> Detonation {
        Detonation {
            kind: m.kind,
            pos: m.pos,
            rot: m.rot,
            blowup,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(x: i32, z: i32) -> Body {
        Body {
            at: [x, 0, z],
            ..Body::default()
        }
    }

    fn mine(kind: MineKind) -> Mine {
        Mine {
            id: 0,
            kind,
            pos: [0, 0, 0],
            rot: Matrix::default(),
            scale_xz: FULL_SCALE,
            scale_y: FULL_SCALE,
            vel: [0; 3],
            parent: 0,
            state: State::Ground,
            cooldown: 0,
            parent_safety: 0,
            stop_fall_y: 0,
            pooled: true,
            overflowed: false,
            dead: false,
        }
    }

    fn run(weapons: &mut Weapons, bodies: &[Body], ticks: usize) -> Vec<Tick> {
        let mut rng = 1;
        let mut coll = Collision::default();
        (0..ticks)
            .map(|_| {
                let mut tick = Tick::default();
                weapons.tick_mines(&mut Ctx {
                    bodies,
                    elapsed_ms: 32,
                    random_number: &mut rng,
                    tick: &mut tick,
                    level: None,
                    coll: &mut coll,
                });
                tick
            })
            .collect()
    }

    #[test]
    fn a_beaker_spins_the_kart_that_touches_it_out_and_flashes_its_screen() {
        for kind in [MineKind::GreenBeaker, MineKind::RedBeaker] {
            let mut weapons = Weapons::default();
            weapons.mines.push(mine(kind));
            let ticks = run(&mut weapons, &[body(1000, 0), body(0x30, 0x30)], 1);
            assert_eq!(ticks[0].hits.len(), 1);
            assert_eq!(ticks[0].hits[0].victim, 1);
            assert_eq!(ticks[0].hits[0].damage, pick::damage::SPIN);
            assert_eq!(ticks[0].hits[0].reason, Reason::Mine);
            assert_eq!(ticks[0].flashes, [1]);
            // It shatters with no blast of its own: one detonation, and no one else is hit.
            assert_eq!(ticks[0].detonations.len(), 1);
            assert!(!ticks[0].detonations[0].blowup);
            assert!(weapons.mines.is_empty());
        }
    }

    #[test]
    fn a_beaker_that_was_just_thrown_is_credited_to_the_throw() {
        let mut weapons = Weapons::default();
        let mut thrown = mine(MineKind::GreenBeaker);
        thrown.cooldown = LANDED_COOLDOWN_MS;
        weapons.mines.push(thrown);
        let ticks = run(&mut weapons, &[body(1000, 0), body(10, 0)], 1);
        assert_eq!(ticks[0].hits[0].reason, Reason::Potion);
    }

    #[test]
    fn a_beaker_reaches_less_far_than_a_crate() {
        let mut weapons = Weapons::default();
        weapons.mines.push(mine(MineKind::GreenBeaker));
        // A distance past a beaker's reach (0x1900 squared) and inside a crate's (0x3840).
        let between = 0x70;
        assert!((between * between) as u32 > BEAKER_HIT_RADIUS);
        assert!((between * between) as u32 <= HIT_RADIUS);
        let ticks = run(&mut weapons, &[body(1000, 0), body(between, 0)], 1);
        assert!(ticks[0].hits.is_empty());
        assert_eq!(weapons.mines.len(), 1);
    }

    #[test]
    fn a_pool_overflow_shatters_a_beaker_without_a_blast() {
        let mut weapons = Weapons::default();
        let mut beaker = mine(MineKind::GreenBeaker);
        beaker.overflowed = true;
        weapons.mines.push(beaker);
        let ticks = run(&mut weapons, &[body(1000, 0)], 1);
        assert!(ticks[0].hits.is_empty() && ticks[0].flashes.is_empty());
        assert_eq!(ticks[0].detonations.len(), 1);
        assert!(!ticks[0].detonations[0].blowup);
    }

    #[test]
    fn a_nitro_crate_blasts_the_kart_that_touches_it() {
        let mut weapons = Weapons::default();
        weapons.mines.push(mine(MineKind::Nitro));
        let ticks = run(&mut weapons, &[body(1000, 0), body(10, 0)], 1);
        assert_eq!(ticks[0].hits.len(), 1);
        assert_eq!(ticks[0].hits[0].victim, 1);
        assert_eq!(ticks[0].hits[0].damage, pick::damage::BLAST);
        assert!(weapons.mines.is_empty());
    }

    #[test]
    fn a_tnt_crate_lands_on_the_head_and_goes_off_after_three_seconds() {
        let mut weapons = Weapons::default();
        weapons.mines.push(mine(MineKind::Tnt));
        let bodies = [body(1000, 0), body(10, 0)];
        let first = run(&mut weapons, &bodies, 1);
        assert_eq!(first[0].hits[0].damage, pick::damage::NONE);
        assert!(weapons.carries_tnt(1));
        // It flies up, comes down and sits for the fuse (90 ticks), then blows.
        let ticks = run(&mut weapons, &bodies, 200);
        let blown = ticks.iter().position(|t| !t.detonations.is_empty());
        assert!(blown.is_some_and(|n| (90..140).contains(&n)), "{blown:?}");
        assert!(!weapons.carries_tnt(1));
        assert!(weapons.mines.is_empty());
    }

    #[test]
    fn jumping_nine_times_shakes_the_tnt_off() {
        let mut weapons = Weapons::default();
        weapons.mines.push(mine(MineKind::Tnt));
        let mut bodies = [body(1000, 0), body(10, 0)];
        run(&mut weapons, &bodies, 40);
        assert!(matches!(weapons.mines[0].state, State::Sitting { .. }));
        bodies[1].jump_started = true;
        run(&mut weapons, &bodies, 9);
        assert!(matches!(weapons.mines[0].state, State::Sitting { .. }));
        run(&mut weapons, &bodies, 1);
        assert!(matches!(weapons.mines[0].state, State::Falling { .. }));
        assert!(!weapons.carries_tnt(1));
    }

    #[test]
    fn the_oldest_mine_goes_off_when_the_pool_is_full() {
        let mut weapons = Weapons::default();
        for id in 0..POOL as u32 {
            let mut m = mine(MineKind::Nitro);
            m.id = id;
            m.pos = [id as i32 * 1000, 0, 0];
            weapons.mines.push(m);
        }
        let mut newest = mine(MineKind::Nitro);
        newest.id = 99;
        newest.pos = [50_000, 0, 0];
        weapons.add_to_pool(&mut newest);
        weapons.mines.push(newest);
        let ticks = run(&mut weapons, &[body(-9000, 0)], 1);
        assert_eq!(ticks[0].detonations.len(), 2);
        assert_eq!(weapons.mines.len(), POOL);
        assert!(weapons.mines.iter().all(|m| m.id != 0));
    }

    #[test]
    fn a_blast_sets_off_the_mines_near_it_with_no_blast_of_their_own() {
        let mut weapons = Weapons::default();
        weapons.mines.push(mine(MineKind::Nitro));
        let mut near = mine(MineKind::Tnt);
        near.pos = [100, 0, 0];
        weapons.mines.push(near);
        let ticks = run(&mut weapons, &[body(10, 0)], 1);
        assert!(weapons.mines.is_empty());
        // The blown one's blast and its own plain explosion, and the neighbour's plain one.
        assert_eq!(ticks[0].detonations.iter().filter(|d| d.blowup).count(), 1);
        assert_eq!(ticks[0].detonations.len(), 3);
        assert_eq!(
            ticks[0]
                .hits
                .iter()
                .filter(|h| h.damage == pick::damage::BLAST)
                .count(),
            2
        );
    }

    #[test]
    fn the_table_has_a_scale_for_each_tick_and_a_height_for_each_character() {
        assert_eq!(TNT_TABLE.len(), 0x5a * 2 + 16);
        assert!(TNT_TABLE[0x5a * 2..].iter().all(|&h| h > 0 && h < 100));
    }
}
