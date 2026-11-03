//! The warpball (the electric ball): `VehPickupItem_ShootNow`'s `WEAPON_ID_WARPBALL` and
//! `RB_Warpball_*`.
//!
//! It goes for the leader (the first in the race order that is not the kart that fired it), or,
//! fired with 10 wumpa fruit, for the nearest kart ahead along the track, and then the next one
//! ahead after each it hits, until it hits the leader. It finds its way by the track's checkpoint
//! nodes (`restart_points`): it steers to a node near it (the fallback path), then rides the path
//! from node to node, and steers straight at its target (the target path) once that is on the
//! next few nodes. Where it meets a wall, an instance, or no ground, it turns back along the
//! path; after too long turned back, or with nobody left to chase, it fades away.
//!
//! Only one is in play at a time: the roulette gives three missiles instead while one is held or
//! flies ([`Weapons::warpball_held`], `WARPBALL_HELD`), until a warpball has faded away.
//!
//! Not here: its glow (a particle) and its reflection, which are the drawing's.

use coll::{Collision, model_id, search_flags};
use coll::doors::Doors;
use loader::lev::{CheckpointNode, Level, SVec3, quad_flags};
use math::mips;
use vehicle::{Driver, Sound3D};

use crate::{
    Body, CRATE_MODELS, CrateHit, Hit, JUICED, Reason, Tick, WEAPON_REACT, Weapons,
    collide_with_drivers_in, heading_velocity, hits_level_instance, mine, shift_velocity, svec,
};

/// `TrackerWeaponFlags`, the warpball's.
pub mod flags {
    /// The kart fired it with 10 wumpa fruit.
    pub const POWERED_UP: u16 = 0x1;
    /// It steers straight at its target.
    pub const TARGET_PATH: u16 = 0x4;
    /// It steers to the node it is to ride the path from.
    pub const FALLBACK_PATH: u16 = 0x8;
    /// Its target was grabbed by the mask, and it went back to the path.
    pub const MASK_REPATH: u16 = 0x10;
    pub const HIT_DRIVER: u16 = 0x40;
    /// It has touched the ground, and turns back where it is stopped.
    pub const TURN_AROUND: u16 = 0x100;
    /// It turned back on the last tick.
    pub const BACKTRACKING: u16 = 0x200;
    /// It steers (at the target or at a node) instead of riding the path.
    pub const PATH_MODE: u16 = TARGET_PATH | FALLBACK_PATH;
    /// It keeps its target this tick.
    pub const TARGET_REFRESH_BLOCKED: u16 = TARGET_PATH | BACKTRACKING;
}

/// `WARPBALL_VELOCITY_NUMERATOR` and `_SHIFT`
const VELOCITY: i32 = 7;
const VELOCITY_SHIFT: u32 = 8;
/// `WARPBALL_PARENT_SAFETY_FRAMES`
const PARENT_SAFETY_TICKS: i16 = 10;
/// How far along the path it rides each tick, in level units per 32 milliseconds.
const PATH_SPEED: u32 = 0x70;
/// `RB_Hazard_CollideWithDrivers`' reach for it: a squared flat distance.
const HIT_RADIUS: u32 = 0x9000;
/// `RB_Hazard_CollideWithBucket`'s reach for the mines: a squared distance.
const MINE_RADIUS: u32 = 0x2400;
/// The search for the ground: this far under and over it, and the second, further down.
const PROBE: i32 = 0x80;
const AIR_PROBE: i32 = 0x900;
/// After turning back more than this many ticks, it fades away.
const TURN_AROUND_TICKS: i16 = 0x78;
/// The fastest it climbs or falls.
const CLIMB_MAX: i16 = 0x60;
/// `RB_Hazard_InterpolateValue`'s speeds: how fast it turns.
const TURN_SLOW: i16 = 0x100;
const TURN_FAST: i16 = 0x400;
const TURN_NEW: i16 = 0x40;
/// Its squared flat distances to a node or a target: at a node, near one, and near enough to
/// turn faster the nearer it is.
const AT_NODE: i32 = 0x4000;
const NEAR_NODE: i32 = 0x24000;
const NEAR: i32 = 0x90000;
/// What a powered-up one looks ahead of itself for its next target.
const TARGET_LOOKAHEAD: i32 = 0x200;

/// The sounds of its flight (a loop) and of its end. (The launch's is the kart's, see
/// `vehicle::item::launch_effects`.)
pub const LOOP_SOUND: u8 = 0x4e;
pub const DEATH_SOUND: u8 = 0x4f;

/// `R231.warpballFadeScale` and `R231.warpballFadeY`: its scale, and how far over where it ended
/// it is, for each tick of the fade.
const FADE_SCALE: [[i16; 3]; 6] = [
    [4505, 5120, 4096],
    [5226, 8192, 4096],
    [5600, 5501, 4096],
    [5272, 3183, 4096],
    [4242, 1411, 3337],
    [2878, 437, 1668],
];
const FADE_Y: [i16; 6] = [-64, -256, -87, 57, 167, 228];

/// The scale of a new instance.
const FULL_SCALE: i16 = 0x1000;

/// `0xff`: no node.
const NO_NODE: u8 = 0xff;

/// A search along the path that would not end on a track that has no way to the finish: retail
/// loops for good there. None of the tracks need this many steps.
const PATH_STEPS_MAX: usize = 0x100;

/// What retail reads of a node past the end of the list: a node with no way on.
const NOWHERE: CheckpointNode = CheckpointNode {
    pos: SVec3 { x: 0, y: 0, z: 0 },
    dist_to_finish: 0,
    next_forward: NO_NODE,
    next_left: NO_NODE,
    next_backward: NO_NODE,
    next_right: NO_NODE,
};

/// A warpball in play (`struct TrackerWeapon` and its instance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warpball {
    /// Which weapon this is, from the same count as the [`crate::Projectile`]s'.
    pub id: u32,
    /// `matrix.t`, in level units.
    pub pos: [i32; 3],
    pub vel: [i16; 3],
    pub flags: u16,
    /// The heading (`dir.y`), 4096 to the turn.
    pub rot_y: i16,
    /// The slot of the kart that fired it.
    pub parent: usize,
    /// The slot of the kart it chases.
    pub target: Option<usize>,
    /// `parentSafetyFrames`
    pub parent_safety: i16,
    /// `distanceToTarget`: the squared flat distance to the target's `posCurr`.
    pub distance_to_target: u32,
    /// `scale`, which the fade changes.
    pub scale: [i16; 3],
    /// Fading away (`RB_Warpball_FadeAway` is its tick): it hits nothing, and its loop is quiet.
    pub fading: bool,
    /// `pathProgress`: how far along the segment from `node_curr` to `node_next`.
    path_progress: i32,
    /// `fadeFrame`
    fade_frame: i32,
    /// `driversHit`: a bit for each slot it does not go for (the parent's, and those it hit and
    /// the ones behind them).
    drivers_hit: u32,
    /// `distFromGround`: its height where it ended.
    dist_from_ground: i32,
    /// `ptrNodeCurr` and `ptrNodeNext`, as indices into `restart_points`.
    node_curr: usize,
    node_next: usize,
    /// `nodeCurrIndex` and `nodeNextIndex`
    node_curr_index: u8,
    node_next_index: u8,
    /// `turnAroundFrames`
    turn_around_frames: i16,
    /// Gone (`THREAD_FLAG_DEAD`): removed at the end of the tick.
    dead: bool,
}

impl Warpball {
    /// `ptrNodeCurr`, as an index into `restart_points` (it may be past the list's end).
    pub fn node_curr(&self) -> usize {
        self.node_curr
    }
}

/// The node at `index`.
fn node(nodes: &[CheckpointNode], index: usize) -> CheckpointNode {
    nodes.get(index).copied().unwrap_or(NOWHERE)
}

/// `driversInRaceOrder`: the slot in each place.
fn race_order(bodies: &[Body]) -> [Option<usize>; 8] {
    let mut order = [None; 8];
    for (slot, body) in bodies.iter().enumerate().take(8) {
        if let Ok(place) = usize::try_from(body.rank)
            && place < 8
        {
            order[place] = Some(slot);
        }
    }
    order
}

/// `RB_Warpball_NewPathNode`: the node after `cn`, the left branch if `d` (the target) took it or
/// is on it a few nodes on.
fn new_path_node(nodes: &[CheckpointNode], cn: usize, d: Option<&Body>) -> usize {
    let c = node(nodes, cn);
    if let Some(d) = d {
        if d.branch_choice == c.next_left {
            return c.next_left as usize;
        }
        if c.next_left != NO_NODE {
            let mut curr = c;
            for _ in 0..3 {
                let next = if curr.next_left == NO_NODE {
                    curr.next_forward
                } else {
                    curr.next_left
                };
                curr = node(nodes, next as usize);
                if d.branch_choice == curr.next_forward {
                    return c.next_left as usize;
                }
            }
        }
    }
    c.next_forward as usize
}

/// `(next - curr, its length)` of a segment of the path (`RB_Warpball_NodeDeltaLength`).
fn segment(nodes: &[CheckpointNode], curr: usize, next: usize) -> ([i32; 3], i32) {
    let (c, n) = (node(nodes, curr).pos, node(nodes, next).pos);
    let delta = [
        n.x as i32 - c.x as i32,
        n.y as i32 - c.y as i32,
        n.z as i32 - c.z as i32,
    ];
    let squared = delta
        .iter()
        .fold(0u32, |sum, &d| sum.wrapping_add((d as u32).wrapping_mul(d as u32)));
    (delta, math::square_root0_stub(squared as i32))
}

/// What a warpball's tick sees.
struct Ctx<'a> {
    nodes: &'a [CheckpointNode],
    bodies: &'a [Body],
    order: [Option<usize>; 8],
    elapsed_ms: i32,
}

impl Ctx<'_> {
    fn body(&self, slot: Option<usize>) -> Option<&Body> {
        slot.and_then(|s| self.bodies.get(s))
    }

    /// `RB_Warpball_GetDriverTarget`
    fn driver_target(&self, w: &Warpball) -> Option<usize> {
        if w.flags & flags::POWERED_UP == 0 {
            // The first in the race that is not its parent.
            return self
                .order
                .iter()
                .flatten()
                .copied()
                .find(|&slot| slot != w.parent && !self.bodies[slot].finished);
        }
        // The nearest kart ahead of a point a little ahead of it along the track.
        let node1 = node(self.nodes, node(self.nodes, w.node_curr).next_forward as usize);
        let node2 = node(self.nodes, node1.next_forward as usize);
        let track = (node(self.nodes, 0).dist_to_finish as i32) << 3;
        let mut path = math::SVec3 {
            x: node1.pos.x.wrapping_sub(node2.pos.x),
            y: node1.pos.y.wrapping_sub(node2.pos.y),
            z: node1.pos.z.wrapping_sub(node2.pos.z),
        };
        math::vector_normalize(&mut path);
        let orb = [
            (w.pos[0] as u16).wrapping_sub(node1.pos.x as u16) as i16,
            (w.pos[1] as u16).wrapping_sub(node1.pos.y as u16) as i16,
            (w.pos[2] as u16).wrapping_sub(node1.pos.z as u16) as i16,
        ];
        // The GTE's dot product (`mvmva` of the path as a row of the matrix).
        let projected = (path.x as i32 * orb[0] as i32)
            .wrapping_add(path.y as i32 * orb[1] as i32)
            .wrapping_add(path.z as i32 * orb[2] as i32)
            >> 12;
        let mut along = ((node1.dist_to_finish as i32) << 3)
            .wrapping_add(projected)
            .wrapping_add(TARGET_LOOKAHEAD);
        // DIVERGENCE: retail divides by a track of length 0 (there is none).
        if track != 0 {
            along = along.wrapping_rem(track);
        }
        let mut best = i32::MAX;
        let mut best_slot = None;
        for (slot, body) in self.bodies.iter().enumerate().take(8) {
            if w.drivers_hit & (1 << slot) != 0 || body.finished || body.mask_grabbed {
                continue;
            }
            let mut distance = mips::sub_lo(along, body.distance_to_finish as i32);
            if distance < 0 {
                distance = mips::add_lo(distance, track);
            }
            if distance < best {
                best = distance;
                best_slot = Some(slot);
            }
        }
        best_slot
    }

    /// `RB_Warpball_SetTargetDriver`: takes the target path if the node of its target is a few
    /// nodes behind or ahead of the ball's.
    fn set_target_driver(&self, w: &mut Warpball) {
        let Some(target) = self.body(w.target) else {
            return;
        };
        let nodes = self.nodes;
        let target_distance = target.distance_to_finish as i32;
        // The last node before the target's distance, or the finish.
        let mut target_node = target.checkpoint as usize;
        let mut prev = target_node;
        for _ in 0..PATH_STEPS_MAX {
            if ((node(nodes, target_node).dist_to_finish as i32) << 3) < target_distance
                || target_node == 0
            {
                break;
            }
            prev = target_node;
            target_node = new_path_node(nodes, prev, Some(target));
        }
        let target_node = prev;

        let take = |w: &mut Warpball| {
            w.flags = (w.flags | flags::TARGET_PATH) & !flags::FALLBACK_PATH;
        };
        // Back along the path, then along the right branch found on the way.
        let mut starts = [Some(w.node_curr), None];
        let mut i = 0;
        while i < 2 && w.flags & flags::TARGET_PATH == 0 {
            if let Some(start) = starts[i] {
                let mut path_node = start;
                for _ in 0..3 {
                    if path_node == target_node {
                        take(w);
                        break;
                    }
                    let n = node(nodes, path_node);
                    if n.next_right != NO_NODE {
                        starts[1] = Some(n.next_right as usize);
                    }
                    path_node = n.next_backward as usize;
                }
            }
            i += 1;
        }
        // And ahead.
        let mut path_node = w.node_curr;
        for _ in 0..3 {
            if path_node == target_node {
                take(w);
                return;
            }
            path_node = new_path_node(nodes, path_node, Some(target));
        }
    }

    /// `RB_Warpball_SeekDriver`: `nodeCurrIndex` becomes the first node from `checkpoint` on that
    /// is behind `d` (or the finish).
    fn seek_driver(&self, w: &mut Warpball, checkpoint: u8, d: Option<&Body>) {
        let Some(d) = d else {
            return;
        };
        if checkpoint == NO_NODE {
            return;
        }
        let target = self.body(w.target);
        let mut cn = checkpoint as usize;
        for _ in 0..PATH_STEPS_MAX {
            if (d.distance_to_finish as i32) > ((node(self.nodes, cn).dist_to_finish as i32) << 3)
                || cn == 0
            {
                break;
            }
            cn = new_path_node(self.nodes, cn, target);
        }
        w.node_curr_index = cn as u8;
    }

    /// `RB_Warpball_Start`: on to the next node, and the one after.
    fn start(&self, w: &mut Warpball) {
        let target = self.body(w.target);
        w.node_curr = new_path_node(self.nodes, w.node_curr, target);
        w.node_next = new_path_node(self.nodes, w.node_curr, target);
    }
}

/// `RB_Warpball_FadeAway`, a tick of the fade: true when it is over (and the warpball gone).
fn fade_away(w: &mut Warpball) -> bool {
    if w.fade_frame as u32 >= FADE_SCALE.len() as u32 {
        w.dead = true;
        return true;
    }
    let frame = w.fade_frame as usize;
    w.scale = FADE_SCALE[frame];
    w.pos[1] = mips::add_lo(w.dist_from_ground, FADE_Y[frame] as i32);
    w.fade_frame = mips::add_lo(w.fade_frame, 1);
    false
}

/// `RB_Warpball_Death`: it starts to fade, with the first tick of the fade at once
/// (`ThTick_SetAndExec`). The rest of the tick that killed it goes on.
fn death(w: &mut Warpball, tick: &mut Tick) {
    w.dist_from_ground = w.pos[1];
    w.fade_frame = 0;
    tick.sounds.push(Sound3D {
        id: DEATH_SOUND,
        pos: w.pos,
    });
    w.fading = true;
    fade_away(w);
}

/// `matrix.t += vel * elapsed / 32`
fn advance(w: &mut Warpball, elapsed_ms: i32) {
    for axis in 0..3 {
        w.pos[axis] = mips::add_lo(
            w.pos[axis],
            mips::mul_lo(w.vel[axis] as i32, elapsed_ms) >> 5,
        );
    }
}

/// What a tick of a warpball did beyond itself.
#[derive(Default)]
struct Step {
    /// Its parent's `instBombThrow` was cleared: the bomb it threw can no longer be set off.
    unlink_bomb: bool,
    /// The mine it ran into (an index into the mines).
    mine_hit: Option<usize>,
}

/// `RB_Warpball_TurnAround`: back the way it came, if it has touched the ground (or has nobody
/// to chase), and back along the path a node every 4 ticks. Too long at it, or with nobody to
/// chase, it ends.
fn turn_around(w: &mut Warpball, c: &Ctx, tick: &mut Tick, step: &mut Step) {
    let was = w.flags;
    if was & flags::TURN_AROUND == 0 && w.target.is_some() {
        return;
    }
    if was & flags::TARGET_PATH != 0 {
        w.flags = (was & !flags::TARGET_PATH) | flags::BACKTRACKING | flags::FALLBACK_PATH;
    }
    w.vel = w.vel.map(i16::wrapping_neg);
    advance(w, c.elapsed_ms);
    w.turn_around_frames = w.turn_around_frames.wrapping_add(1);
    if w.turn_around_frames > TURN_AROUND_TICKS || w.target.is_none() {
        step.unlink_bomb = true;
        // (Its end is heard twice.)
        tick.sounds.push(Sound3D {
            id: DEATH_SOUND,
            pos: w.pos,
        });
        death(w, tick);
    }
    if w.turn_around_frames & 3 == 0 {
        w.node_next = w.node_curr;
        w.node_curr = node(c.nodes, w.node_curr).next_backward as usize;
    }
    let n = node(c.nodes, w.node_curr);
    w.rot_y = math::ratan2(
        mips::sub_lo(n.pos.x as i32, w.pos[0]),
        mips::sub_lo(n.pos.z as i32, w.pos[2]),
    ) as i16;
}

/// `RB_Warpball_SetQuadblockIndex`: the node of the ground it found.
fn set_quadblock_index(w: &mut Warpball, level: &Level, coll: &Collision) {
    let quad = coll
        .sps
        .hit
        .quad
        .and_then(|q| level.quad_blocks.get(q as usize));
    if let Some(quad) = quad
        && quad.checkpoint_index != NO_NODE
    {
        w.node_next_index = quad.checkpoint_index;
    }
}

/// `RB_Warpball_ThTick`
fn fly(
    w: &mut Warpball,
    c: &Ctx,
    coll: &mut Collision,
    level: &Level,
    doors: &Doors,
    mines: &[mine::Mine],
    tick: &mut Tick,
) -> Step {
    let mut step = Step::default();

    // A new target, unless it is on the target path or turned back.
    if let Some(target) = c.body(w.target) {
        if target.mask_grabbed && w.flags & flags::TARGET_PATH != 0 {
            w.flags = (w.flags & !flags::TARGET_PATH) | flags::FALLBACK_PATH | flags::MASK_REPATH;
            w.node_curr = w.node_next_index as usize;
            w.node_next = new_path_node(c.nodes, w.node_curr, Some(target));
            w.target = c.driver_target(w);
            c.set_target_driver(w);
        }
        if w.flags & flags::TARGET_REFRESH_BLOCKED == 0 {
            w.target = c.driver_target(w);
            if w.target.is_some() {
                c.set_target_driver(w);
            }
        }
    }
    w.flags &= !flags::BACKTRACKING;

    match c.body(w.target) {
        Some(target) if w.flags & flags::PATH_MODE != 0 => {
            // It steers, at its target or at the node of the path.
            let mut dist_x = mips::sub_lo(target.pos_curr[0] >> 8, w.pos[0]);
            let mut dist_z = mips::sub_lo(target.pos_curr[2] >> 8, w.pos[2]);
            let mut dist_y = mips::sub_lo(target.pos_curr[1] >> 8, w.pos[1]);
            w.distance_to_target =
                mips::add_lo(mips::mul_lo(dist_x, dist_x), mips::mul_lo(dist_z, dist_z)) as u32;
            let mut dist_xz = w.distance_to_target as i32;
            let mut turn = TURN_SLOW;
            if w.flags & flags::TARGET_PATH == 0 {
                let n = node(c.nodes, w.node_curr);
                dist_x = mips::sub_lo(n.pos.x as i32, w.pos[0]);
                dist_z = mips::sub_lo(n.pos.z as i32, w.pos[2]);
                dist_y = mips::sub_lo(n.pos.y as i32, w.pos[1]);
                dist_xz = (dist_x as u32)
                    .wrapping_mul(dist_x as u32)
                    .wrapping_add((dist_z as u32).wrapping_mul(dist_z as u32))
                    as i32;
                // At the node it rides the path from there.
                if dist_xz < AT_NODE {
                    w.flags = (w.flags & !flags::FALLBACK_PATH) | flags::TURN_AROUND;
                } else if dist_xz < NEAR_NODE {
                    turn = TURN_FAST;
                }
            }
            if dist_xz < NEAR {
                turn = (TURN_FAST as i32 - (dist_xz >> 9)) as i16;
                if turn < TURN_SLOW {
                    turn = TURN_SLOW;
                }
            }
            if w.parent_safety > 0 {
                turn = TURN_NEW;
            }
            let desired = math::ratan2(dist_x, dist_z) as i16;
            w.rot_y = math::interpolate(w.rot_y, desired, turn as i32);
            w.vel[0] = heading_velocity(math::sin(w.rot_y as u32), VELOCITY, VELOCITY_SHIFT);
            w.vel[2] = heading_velocity(math::cos(w.rot_y as u32), VELOCITY, VELOCITY_SHIFT);
            let climb = mips::sll(c.elapsed_ms, 2) >> 5;
            if dist_y > 0 {
                w.vel[1] = (w.vel[1] as i32).wrapping_add(climb) as i16;
                if dist_y < w.vel[1] as i32 {
                    w.vel[1] = dist_y as i16;
                }
                if w.vel[1] > CLIMB_MAX {
                    w.vel[1] = CLIMB_MAX;
                }
            } else if dist_y < 0 {
                w.vel[1] = (w.vel[1] as i32).wrapping_sub(climb) as i16;
                if (w.vel[1] as i32) < dist_y {
                    w.vel[1] = dist_y as i16;
                }
                if w.vel[1] < -CLIMB_MAX {
                    w.vel[1] = -CLIMB_MAX;
                }
            }
            advance(w, c.elapsed_ms);
        }
        Some(target) => {
            // It rides the path: from node to node, as far as it goes this tick.
            let dist_x = mips::sub_lo(target.pos_curr[0] >> 8, w.pos[0]);
            let dist_z = mips::sub_lo(target.pos_curr[2] >> 8, w.pos[2]);
            w.distance_to_target =
                mips::add_lo(mips::mul_lo(dist_x, dist_x), mips::mul_lo(dist_z, dist_z)) as u32;
            let (mut curr, mut next) = (w.node_curr, w.node_next);
            let (mut delta, mut length) = segment(c.nodes, curr, next);
            let mut progress = mips::add_lo(
                w.path_progress,
                ((c.elapsed_ms as u32).wrapping_mul(PATH_SPEED) as i32) >> 5,
            );
            if length <= progress {
                progress = mips::sub_lo(progress, length);
                for _ in 0..PATH_STEPS_MAX {
                    curr = next;
                    next = new_path_node(c.nodes, curr, Some(target));
                    (delta, length) = segment(c.nodes, curr, next);
                    let reached = progress < length;
                    progress = mips::sub_lo(progress, length);
                    if reached {
                        break;
                    }
                }
                progress = mips::add_lo(progress, length);
            }
            w.path_progress = progress;
            w.node_curr = curr;
            w.node_next = next;
            let fraction = if length != 0 {
                mips::div(mips::sll(progress, 12), length)
            } else {
                0
            };
            let from = node(c.nodes, curr).pos;
            let from = [from.x as i32, from.y as i32, from.z as i32];
            for axis in 0..3 {
                w.pos[axis] = mips::add_lo(from[axis], mips::mul_lo(delta[axis], fraction) >> 12);
            }
            let yaw = math::ratan2(delta[0], delta[2]);
            w.rot_y = yaw as i16;
            w.vel[0] = heading_velocity(math::sin(yaw as u32), VELOCITY, VELOCITY_SHIFT);
            w.vel[2] = heading_velocity(math::cos(yaw as u32), VELOCITY, VELOCITY_SHIFT);
            w.vel[1] = 0;
        }
        None => advance(w, c.elapsed_ms),
    }

    // The ground under it, and the instances it passes through.
    {
        let q = &mut coll.sps.quad_block_coll;
        q.quad_flags_wanted = (quad_flags::GROUND | quad_flags::TRIGGER) as u32;
        q.quad_flags_ignored = 0;
        q.search_flags =
            search_flags::TEST_INSTANCES | search_flags::HIGH_LOD | search_flags::FORCE_INSTANCE_HIT;
    }
    let pos_top = svec(w.pos, -PROBE);
    let pos_bottom = svec(w.pos, PROBE);
    coll.search_quadblocks(level, pos_top, pos_bottom, 0);

    if coll.sps.collision.step_flags & WEAPON_REACT != 0 {
        turn_around(w, c, tick, &mut step);
    }

    if coll.sps.did_touch_hitbox != 0 {
        // `RB_Hazard_CollLevInst`: a crate breaks (for nothing) and lets it through; a temple door
        // that is shut turns it back (without opening); the rest of the instances turn it back.
        let instance = coll.sps.bsp_hitbox.and_then(|hit| hit.instance);
        if let Some(instance) = instance
            && CRATE_MODELS.contains(&level.instances[instance].model_id)
        {
            tick.crate_hits.push(CrateHit {
                instance,
                model: model_id::WARPBALL,
                owner: w.parent,
            });
        }
        let stopped = match instance.and_then(|i| doors.door(i)) {
            Some(door) => door.stops(w.pos),
            None => hits_level_instance(level, coll),
        };
        if stopped {
            turn_around(w, c, tick, &mut step);
        }
    }

    if coll.sps.did_touch_quadblock != 0 {
        w.flags |= flags::TURN_AROUND;
        set_quadblock_index(w, level, coll);
        w.vel[1] = 0;
        let ground = coll.sps.quad_block_coll.hit_pos.y as i32;
        if w.flags & flags::PATH_MODE != 0 && w.pos[1] < ground {
            w.pos[1] = ground;
        }
    } else {
        // Further down, from where it is now.
        let far = svec(w.pos, -AIR_PROBE);
        coll.search_quadblocks(level, far, pos_bottom, 0);
        let touched = coll.sps.did_touch_quadblock != 0;
        if touched {
            w.flags |= flags::TURN_AROUND;
            set_quadblock_index(w, level, coll);
        }
        if !touched && (w.flags & flags::PATH_MODE != 0 || w.target.is_none()) {
            turn_around(w, c, tick, &mut step);
        }
    }

    // A kart it runs into is hurt; a mine goes off.
    if let Some(victim) =
        collide_with_drivers_in(w.pos, w.parent, w.parent_safety, c.bodies, HIT_RADIUS)
    {
        if victim != w.parent {
            let had_target_path = w.flags & flags::TARGET_PATH != 0;
            tick.hits.push(Hit {
                victim,
                attacker: Some(w.parent),
                reason: Reason::Warpball,
                damage: vehicle::pick::damage::BLAST,
                rain: false,
            });
            tick.damage_colors
                .push((victim, vehicle::DAMAGE_COLOR_TIMER));
            w.flags |= flags::HIT_DRIVER;
            let hit = &c.bodies[victim];
            // It ends at its target, unless powered up, and at the leader.
            if (w.flags & flags::POWERED_UP == 0 && w.target == Some(victim)) || hit.rank == 0 {
                step.unlink_bomb = true;
                death(w, tick);
                return step;
            }
            // It goes for none of them again, nor for the ones behind.
            w.drivers_hit |= 1 << victim;
            for place in hit.rank.max(0) as usize..8 {
                if let Some(slot) = c.order[place] {
                    w.drivers_hit |= 1 << slot;
                }
            }
            if w.target == Some(victim) {
                w.flags &= !flags::TARGET_PATH;
                w.target = c.driver_target(w);
                if w.target.is_some() {
                    c.set_target_driver(w);
                } else {
                    step.unlink_bomb = true;
                    death(w, tick);
                }
                if had_target_path {
                    c.seek_driver(w, hit.checkpoint, Some(hit));
                }
                if w.flags & flags::TARGET_PATH == 0 && had_target_path {
                    if w.node_curr_index != NO_NODE {
                        w.node_curr = w.node_curr_index as usize;
                        w.node_next = new_path_node(c.nodes, w.node_curr, c.body(w.target));
                    }
                    w.flags |= flags::FALLBACK_PATH;
                }
            }
        }
    } else {
        // The newest mine first, as its bucket has them.
        step.mine_hit = mines.iter().rposition(|m| {
            let [dx, dy, dz] = std::array::from_fn(|a| m.pos[a].wrapping_sub(w.pos[a]));
            let distance = dx
                .wrapping_mul(dx)
                .wrapping_add(dz.wrapping_mul(dz))
                .wrapping_add(dy.wrapping_mul(dy)) as u32;
            distance < MINE_RADIUS
        });
    }

    if w.parent_safety != 0 {
        w.parent_safety -= 1;
    }
    step
}

impl Weapons {
    /// The warpballs in play, the fading ones too.
    pub fn warpballs(&self) -> &[Warpball] {
        &self.warpballs
    }

    /// `VehPickupItem_ShootNow` for the warpball: kart `shooter` (`driver`) fires one from where
    /// it is. `bodies` are all the karts.
    pub fn launch_warpball(
        &mut self,
        shooter: usize,
        driver: &Driver,
        bodies: &[Body],
        level: &Level,
    ) {
        let c = Ctx {
            nodes: &level.restart_points,
            bodies,
            order: race_order(bodies),
            elapsed_ms: 0,
        };
        let me = bodies.get(shooter).copied().unwrap_or_else(|| Body::of(driver));
        let mut w = Warpball {
            id: self.next_id,
            pos: [
                mips::sra(driver.pos_curr.x, 8),
                mips::sra(driver.pos_curr.y, 8),
                mips::sra(driver.pos_curr.z, 8),
            ],
            vel: [0; 3],
            flags: 0,
            rot_y: 0,
            parent: shooter,
            target: None,
            parent_safety: 0,
            distance_to_target: 0,
            scale: [FULL_SCALE; 3],
            fading: false,
            path_progress: 0,
            fade_frame: 0,
            drivers_hit: 0,
            dist_from_ground: 0,
            node_curr: 0,
            node_next: 0,
            node_curr_index: 0,
            node_next_index: 0,
            turn_around_frames: 0,
            dead: false,
        };
        // The kart a place ahead, to find the path by.
        w.target = usize::try_from(me.rank - 1)
            .ok()
            .and_then(|place| c.order.get(place).copied().flatten());
        c.seek_driver(&mut w, me.checkpoint, Some(&me));
        w.node_next_index = w.node_curr_index;
        w.node_curr = w.node_curr_index as usize;
        if driver.num_wumpas >= JUICED {
            w.flags |= flags::POWERED_UP;
        }
        w.flags |= flags::FALLBACK_PATH;
        w.drivers_hit = 1 << shooter;
        w.target = if me.rank == 0 {
            None
        } else {
            c.driver_target(&w)
        };
        if w.target.is_some() {
            c.set_target_driver(&mut w);
        }
        if w.flags & flags::TARGET_PATH != 0 {
            w.flags &= !flags::FALLBACK_PATH;
        } else {
            c.start(&mut w);
        }
        w.node_next = new_path_node(c.nodes, w.node_curr, c.body(w.target));
        let (matrix, _) = vehicle::baked::inst_matrix(driver);
        w.vel = [
            shift_velocity(matrix.m[0][2], VELOCITY, VELOCITY_SHIFT),
            0,
            shift_velocity(matrix.m[2][2], VELOCITY, VELOCITY_SHIFT),
        ];
        w.parent_safety = PARENT_SAFETY_TICKS;
        w.rot_y = driver.angle;
        self.next_id = self.next_id.wrapping_add(1);
        self.warpballs.push(w);
    }

    /// One game tick of the warpballs, after the other weapons.
    pub(crate) fn tick_warpballs(
        &mut self,
        level: &Level,
        bodies: &[Body],
        elapsed_ms: i32,
        doors: &Doors,
        coll: &mut Collision,
        tick: &mut Tick,
    ) {
        let c = Ctx {
            nodes: &level.restart_points,
            bodies,
            order: race_order(bodies),
            elapsed_ms,
        };
        for i in 0..self.warpballs.len() {
            let mut w = self.warpballs[i].clone();
            if w.dead {
                continue;
            }
            if w.fading {
                // Only one is ever in play: the roulette may give it again.
                if fade_away(&mut w) {
                    self.warpball_held = false;
                }
            } else {
                let step = fly(&mut w, &c, coll, level, doors, &self.mines, tick);
                if let Some(m) = step.mine_hit {
                    self.mines[m].set_off(tick);
                }
                if step.unlink_bomb {
                    self.unlink_bombs(w.parent);
                }
            }
            self.warpballs[i] = w;
        }
        self.warpballs.retain(|w| !w.dead);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight track of `n` nodes 1000 units apart along +z, the finish at node 0.
    fn track(n: usize) -> Vec<CheckpointNode> {
        (0..n)
            .map(|i| CheckpointNode {
                pos: SVec3 {
                    x: 0,
                    y: 0,
                    z: (i * 1000) as i16,
                },
                dist_to_finish: ((n - i) * 1000 / 8) as u16,
                next_forward: ((i + 1) % n) as u8,
                next_left: NO_NODE,
                next_backward: ((i + n - 1) % n) as u8,
                next_right: NO_NODE,
            })
            .collect()
    }

    fn body(rank: i16, distance_to_finish: u32) -> Body {
        Body {
            rank,
            distance_to_finish,
            ..Body::default()
        }
    }

    fn ball(parent: usize, flags: u16) -> Warpball {
        Warpball {
            id: 0,
            pos: [0; 3],
            vel: [0; 3],
            flags,
            rot_y: 0,
            parent,
            target: None,
            parent_safety: 0,
            distance_to_target: 0,
            scale: [FULL_SCALE; 3],
            fading: false,
            path_progress: 0,
            fade_frame: 0,
            drivers_hit: 1 << parent,
            dist_from_ground: 0,
            node_curr: 0,
            node_next: 1,
            node_curr_index: 0,
            node_next_index: 0,
            turn_around_frames: 0,
            dead: false,
        }
    }

    #[test]
    fn it_goes_for_the_leader_that_is_not_its_parent() {
        let nodes = track(10);
        let bodies = [body(2, 0), body(0, 0), body(1, 0)];
        let c = Ctx {
            nodes: &nodes,
            bodies: &bodies,
            order: race_order(&bodies),
            elapsed_ms: 32,
        };
        assert_eq!(c.driver_target(&ball(0, 0)), Some(1));
        // The leader's own goes for the second.
        assert_eq!(c.driver_target(&ball(1, 0)), Some(2));
        // One that finished is not chased.
        let mut finished = bodies;
        finished[1].finished = true;
        let c = Ctx {
            bodies: &finished,
            ..c
        };
        assert_eq!(c.driver_target(&ball(0, 0)), Some(2));
    }

    #[test]
    fn a_powered_up_one_goes_for_the_nearest_ahead_it_has_not_hit() {
        let nodes = track(10);
        // The track is 10000 long. The ball is at node 1 (9000 from the finish), its parent a
        // little behind; kart 1 is 2000 ahead of the ball, kart 2 4000.
        let bodies = [body(2, 9100), body(1, 7000), body(0, 5000)];
        let c = Ctx {
            nodes: &nodes,
            bodies: &bodies,
            order: race_order(&bodies),
            elapsed_ms: 32,
        };
        let mut w = ball(0, flags::POWERED_UP);
        w.node_curr = 1;
        w.pos = [0, 0, 1000];
        assert_eq!(c.driver_target(&w), Some(1));
        w.drivers_hit |= 1 << 1;
        assert_eq!(c.driver_target(&w), Some(2));
    }

    #[test]
    fn the_path_takes_the_left_branch_the_target_took() {
        let mut nodes = track(10);
        nodes[2].next_left = 7;
        let mut target = body(0, 0);
        // On the main path, a few nodes on.
        target.branch_choice = 4;
        assert_eq!(new_path_node(&nodes, 2, Some(&target)), 3);
        target.branch_choice = 7;
        assert_eq!(new_path_node(&nodes, 2, Some(&target)), 7);
        assert_eq!(new_path_node(&nodes, 2, None), 3);
    }

    #[test]
    fn it_fades_for_six_ticks_and_rises_and_shrinks() {
        let mut w = ball(0, 0);
        w.pos = [0, 100, 0];
        let mut tick = Tick::default();
        death(&mut w, &mut tick);
        assert!(w.fading);
        assert_eq!(w.pos[1], 100 + FADE_Y[0] as i32);
        assert_eq!(w.scale, FADE_SCALE[0]);
        assert_eq!(tick.sounds.len(), 1);
        for frame in 1..6 {
            assert!(!fade_away(&mut w));
            assert_eq!(w.pos[1], 100 + FADE_Y[frame] as i32);
        }
        assert!(fade_away(&mut w));
        assert!(w.dead);
    }
}
