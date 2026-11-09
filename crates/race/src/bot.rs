//! The bots: karts the race drives itself, with a pad of its own making, along the level's bot
//! paths ([`loader::lev::NavPath`]). Unlike retail's bots (`BOTS_Driver_Convert`), which move along
//! the path and skip the physics, these drive the same kart a player does.
//!
//! So far: where on its path a bot is ([`Tracker`]), and where it should head for
//! ([`Tracker::ahead`]). All of it is integers, so every client tracks the same.

use loader::lev::NavFrame;
use math::Vec3;

/// `Driver::pos_curr` is in 1/256 of the units of the level (and of the path).
const WORLD_POS_SHIFT: u32 = 8;

/// The most frames a tracker moves in a tick before it gives up and searches. A kart does less
/// than a frame a tick, so more than this is a teleport (a respawn, a mask grab).
const MAX_STEPS: usize = 16;

/// Further than this from the segment it is on (in level units), a kart has left the path, and the
/// tracker searches for it again.
pub const LOST_DISTANCE: i64 = 1500;

/// How many frames either side of the last one the tracker searches first, so that a kart is not
/// put on another part of a track that passes close (a bridge over it, the other side of a
/// hairpin).
const SEARCH_WINDOW: usize = 48;

type P = [i64; 3];

fn frame_pos(f: &NavFrame) -> P {
    [f.pos.x as i64, f.pos.y as i64, f.pos.z as i64]
}

fn level_pos(pos: Vec3) -> P {
    [pos.x, pos.y, pos.z].map(|v| (v >> WORLD_POS_SHIFT) as i64)
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: P, b: P) -> i64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The square of the distance from `p` to the segment `a`..`b`.
fn segment_distance2(p: P, a: P, b: P) -> i64 {
    let d = sub(b, a);
    let len2 = dot(d, d);
    let ap = sub(p, a);
    if len2 == 0 {
        return dot(ap, ap);
    }
    let t = dot(ap, d).clamp(0, len2);
    // The closest point is a + d * t / len2.
    let off = [0, 1, 2].map(|k| ap[k] - d[k] * t / len2);
    dot(off, off)
}

/// Where a kart is on a bot path: between [`Tracker::frame`] and the frame after it. The paths are
/// loops, the last frame leading to the first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tracker {
    /// The last frame the kart has passed.
    pub frame: u16,
    /// How many times the kart has been searched for on the path (after a respawn, or being
    /// knocked off it), for the steering to tell that the path is not where it was.
    pub relocations: u16,
}

impl Tracker {
    /// A tracker for a kart at `pos` (in the units of `Driver::pos_curr`), searched for on the
    /// whole of `path`.
    pub fn find(path: &[NavFrame], pos: Vec3) -> Tracker {
        let mut t = Tracker::default();
        if path.len() >= 2 {
            let p = level_pos(pos);
            t.frame = nearest(path, p, 0..path.len()).0 as u16;
            t.settle(path, p);
        }
        t
    }

    /// Moves the tracker along with the kart, now at `pos`. Returns whether it had to search for
    /// the kart again.
    pub fn update(&mut self, path: &[NavFrame], pos: Vec3) -> bool {
        let n = path.len();
        if n < 2 {
            return false;
        }
        if self.frame as usize >= n {
            *self = Tracker::find(path, pos);
            return true;
        }
        let p = level_pos(pos);
        if self.settle(path, p) && self.distance2(path, p) <= LOST_DISTANCE * LOST_DISTANCE {
            return false;
        }

        // Lost: near where it was, or anywhere.
        let here = self.frame as usize;
        let (mut best, best_d2) = if n > 2 * SEARCH_WINDOW + 1 {
            let start = here + n - SEARCH_WINDOW;
            nearest(path, p, (start..start + 2 * SEARCH_WINDOW + 1).map(|i| i % n))
        } else {
            (here, i64::MAX)
        };
        if best_d2 > LOST_DISTANCE * LOST_DISTANCE {
            best = nearest(path, p, 0..n).0;
        }
        self.frame = best as u16;
        self.settle(path, p);
        self.relocations = self.relocations.wrapping_add(1);
        true
    }

    /// Steps forward over the frames the kart has passed, or back over the ones it has not
    /// reached. Returns false when that took more than [`MAX_STEPS`].
    fn settle(&mut self, path: &[NavFrame], p: P) -> bool {
        let n = path.len();
        let mut i = self.frame as usize;
        let mut settled = false;
        for _ in 0..MAX_STEPS {
            let next = (i + 1) % n;
            if !passed(path, next, p) {
                settled = true;
                break;
            }
            i = next;
        }
        if settled && i == self.frame as usize {
            // Not forward: maybe back. The same test, so the two never undo each other.
            settled = false;
            for _ in 0..MAX_STEPS {
                if passed(path, i, p) {
                    settled = true;
                    break;
                }
                i = (i + n - 1) % n;
            }
        }
        self.frame = i as u16;
        settled
    }

    /// The square of the kart's distance from the segment it is on.
    fn distance2(&self, path: &[NavFrame], p: P) -> i64 {
        let i = self.frame as usize;
        let a = frame_pos(&path[i]);
        let b = frame_pos(&path[(i + 1) % path.len()]);
        segment_distance2(p, a, b)
    }

    /// The point of the path `distance` (in level units) ahead of the kart at `pos`, measured
    /// along the path from where the kart is level with it; in the units of `Driver::pos_curr`.
    /// With it, the frame the point is past (for its flags).
    pub fn ahead(&self, path: &[NavFrame], pos: Vec3, distance: i32) -> (Vec3, usize) {
        let n = path.len();
        if n == 0 {
            return (pos, 0);
        }
        let to_world = |q: P| Vec3 {
            x: (q[0] << WORLD_POS_SHIFT) as i32,
            y: (q[1] << WORLD_POS_SHIFT) as i32,
            z: (q[2] << WORLD_POS_SHIFT) as i32,
        };
        let mut i = self.frame as usize % n;
        if n == 1 {
            return (to_world(frame_pos(&path[0])), 0);
        }
        let p = level_pos(pos);
        let mut a = frame_pos(&path[i]);
        let mut b = frame_pos(&path[(i + 1) % n]);
        let mut d = sub(b, a);
        let mut len = dot(d, d).isqrt();
        // How far along this segment the kart is.
        let along = if len == 0 {
            0
        } else {
            (dot(sub(p, a), d) / len).clamp(0, len)
        };
        let mut left = along + distance.max(0) as i64;
        for _ in 0..n {
            if left <= len {
                break;
            }
            left -= len;
            i = (i + 1) % n;
            a = b;
            b = frame_pos(&path[(i + 1) % n]);
            d = sub(b, a);
            len = dot(d, d).isqrt();
        }
        let left = left.min(len);
        let point = if len == 0 {
            a
        } else {
            [0, 1, 2].map(|k| a[k] + d[k] * left / len)
        };
        (to_world(point), i)
    }
}

/// The direction from `a` to `b`, 4096 long (zero if they are the same point).
fn unit(a: P, b: P) -> P {
    let d = sub(b, a);
    let len = dot(d, d).isqrt();
    if len == 0 {
        return [0; 3];
    }
    d.map(|v| v * 4096 / len)
}

/// Whether `p` is past frame `j` of `path`: on the far side of the plane through it that halves
/// the turn there. (Past it along the segment to it would not do: a kart that cuts the inside of a
/// corner never gets there.)
fn passed(path: &[NavFrame], j: usize, p: P) -> bool {
    let n = path.len();
    let here = frame_pos(&path[j]);
    let before = frame_pos(&path[(j + n - 1) % n]);
    let after = frame_pos(&path[(j + 1) % n]);
    let a = unit(before, here);
    let b = unit(here, after);
    dot(sub(p, here), [a[0] + b[0], a[1] + b[1], a[2] + b[2]]) >= 0
}

/// The frame of `path` among `frames` that is nearest to `p`, and the square of its distance.
fn nearest(path: &[NavFrame], p: P, frames: impl Iterator<Item = usize>) -> (usize, i64) {
    let mut best = (0, i64::MAX);
    for i in frames {
        let off = sub(p, frame_pos(&path[i]));
        let d2 = dot(off, off);
        if d2 < best.1 {
            best = (i, d2);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::lev::SVec3;

    /// A square loop of side `side` in the ground plane, a frame every 100 units, going +x, +z,
    /// -x, -z from the origin.
    fn square_of(side: i32) -> Vec<NavFrame> {
        let mut frames = Vec::new();
        let corners = [(0, 0), (side, 0), (side, side), (0, side)];
        let steps = side / 100;
        for k in 0..4 {
            let (x0, z0) = corners[k];
            let (x1, z1) = corners[(k + 1) % 4];
            for s in 0..steps {
                frames.push(NavFrame {
                    pos: SVec3 {
                        x: (x0 + (x1 - x0) * s / steps) as i16,
                        y: 0,
                        z: (z0 + (z1 - z0) * s / steps) as i16,
                    },
                    ..Default::default()
                });
            }
        }
        frames
    }

    fn square() -> Vec<NavFrame> {
        square_of(1000)
    }

    fn world(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3 {
            x: x << 8,
            y: y << 8,
            z: z << 8,
        }
    }

    #[test]
    fn finds_and_follows() {
        let path = square();
        let mut t = Tracker::find(&path, world(150, 0, 20));
        assert_eq!(t.frame, 1);
        // Driving along: a frame at a time.
        for x in (150..960).step_by(30) {
            assert!(!t.update(&path, world(x, 0, 20)));
            assert_eq!(t.frame as i32, x / 100);
        }
        // Cutting the corner, and on up the second side.
        assert!(!t.update(&path, world(990, 0, 20)));
        assert_eq!(t.frame, 10);
        assert!(!t.update(&path, world(990, 0, 120)));
        assert_eq!(t.frame, 11);
        assert_eq!(t.relocations, 0);
    }

    #[test]
    fn wraps_and_backs_up() {
        let path = square();
        let mut t = Tracker::find(&path, world(10, 0, 50));
        assert_eq!(t.frame, 39);
        assert!(!t.update(&path, world(50, 0, -10)));
        assert_eq!(t.frame, 0);
        // Reversing over the line.
        assert!(!t.update(&path, world(-10, 0, 150)));
        assert_eq!(t.frame, 38);
    }

    #[test]
    fn searches_after_a_teleport() {
        // Big enough that the far side is further than LOST_DISTANCE.
        let path = square_of(4000);
        let mut t = Tracker::find(&path, world(150, 0, 0));
        assert_eq!(t.frame, 1);
        // Put on the far side: too many frames on to step over.
        assert!(t.update(&path, world(1950, 0, 4000)));
        assert_eq!(t.frame, 100);
        assert_eq!(t.relocations, 1);
        // Knocked off the track, far.
        assert!(t.update(&path, world(2050, 0, 6000)));
        assert_eq!(t.frame, 99);
        assert_eq!(t.relocations, 2);
    }

    #[test]
    fn looks_ahead_along_the_path() {
        let path = square();
        let t = Tracker::find(&path, world(950, 0, 30));
        let (point, frame) = t.ahead(&path, world(950, 0, 30), 250);
        assert_eq!(point, world(1000, 0, 200));
        assert_eq!(frame, 11);
        // Over the end of the loop.
        let t = Tracker::find(&path, world(0, 0, 150));
        let (point, frame) = t.ahead(&path, world(0, 0, 150), 300);
        assert_eq!(point, world(150, 0, 0));
        assert_eq!(frame, 1);
    }
}
