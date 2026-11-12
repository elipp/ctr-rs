//! Our own changes to the bot paths (`Level::nav_paths`): frames moved from where the level has
//! them, mostly to take tight corners wider. They are kept in `nav_amendments.txt` next to this
//! file, compiled in ([`Amendments::builtin`]), and the viewer edits and saves them (`nav.rs` of
//! the client).
//!
//! A move names the track, the path and the frame, where the frame was and where it goes. Where
//! it was is checked: a level whose frame is elsewhere (another version of the track, a map made
//! from a patch) gets nothing from that move.
//!
//! Moving a frame changes what the level has worked out from the positions: the distance to the
//! next frame (`dist_to_next_*`), the heading (`rot[1]`) and the climb (`rot[3]`). Those are
//! changed by as much as the way to the next frame changed, so that what the level's own numbers
//! have on top of the geometry (rounding, smoothing) is kept. All of it is integer math: the
//! paths are the same on every machine.
//!
//! A frame's flags (`NavFrame::flags`: jump, drift, turbo) can be changed the same way: what they
//! were is checked, and they are set to what they become.
//!
//! The file has a change on each line, `#` starts a comment:
//!
//! ```text
//! # track path frame   was x y z   ->   to x y z
//! 3 0 212   -1234 56 789   ->   -1300 56 800
//! # track path frame   flags was   ->   to
//! 3 0 215   flags 0x0000   ->   0x0400
//! ```

use std::fmt::Write as _;
use std::sync::OnceLock;

use crate::lev::{NavPath, SVec3};

/// The file the viewer saves to: the one [`Amendments::builtin`] is compiled from.
pub const SOURCE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/nav_amendments.txt");

const BUILTIN: &str = include_str!("nav_amendments.txt");

/// One frame moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Move {
    /// The track's number (a `LevelId`, or a map's id).
    pub track: u32,
    pub path: usize,
    pub frame: usize,
    /// Where the level has the frame.
    pub was: SVec3,
    pub to: SVec3,
}

/// One frame's flags changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    pub track: u32,
    pub path: usize,
    pub frame: usize,
    /// The level's flags of the frame.
    pub was: u16,
    pub to: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Amendments {
    /// By track, path and frame; one move to a frame at most.
    moves: Vec<Move>,
    /// The same, for the flags.
    flags: Vec<Flags>,
}

impl Amendments {
    /// The ones compiled in.
    pub fn builtin() -> &'static Amendments {
        static BUILT: OnceLock<Amendments> = OnceLock::new();
        BUILT.get_or_init(|| {
            Amendments::parse(BUILTIN).unwrap_or_else(|e| panic!("nav_amendments.txt: {e}"))
        })
    }

    pub fn parse(text: &str) -> Result<Amendments, String> {
        let mut a = Amendments::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or_default();
            let words: Vec<&str> = line.split_whitespace().filter(|w| *w != "->").collect();
            if words.is_empty() {
                continue;
            }
            let bad = |what: &str| format!("line {}: {what}: {}", n + 1, line.trim());
            if words.len() != 9 {
                return Err(bad("wants track, path, frame, was x y z, to x y z"));
            }
            let int = |i: usize| words[i].parse::<i64>().map_err(|_| bad("not a number"));
            let frame = |i: usize| -> Result<usize, String> {
                usize::try_from(int(i)?).map_err(|_| bad("out of range"))
            };
            let track = || u32::try_from(int(0)?).map_err(|_| bad("out of range"));
            if words.get(3) == Some(&"flags") {
                if words.len() != 6 {
                    return Err(bad("wants track, path, frame, flags, was, to"));
                }
                let hex = |i: usize| {
                    let w = words[i];
                    u16::from_str_radix(w.strip_prefix("0x").unwrap_or(w), 16)
                        .map_err(|_| bad("not a hex number"))
                };
                a.set_flags(Flags {
                    track: track()?,
                    path: frame(1)?,
                    frame: frame(2)?,
                    was: hex(4)?,
                    to: hex(5)?,
                });
                continue;
            }
            let short = |i: usize| -> Result<i16, String> {
                i16::try_from(int(i)?).map_err(|_| bad("out of range"))
            };
            let at = |i: usize| -> Result<SVec3, String> {
                Ok(SVec3 {
                    x: short(i)?,
                    y: short(i + 1)?,
                    z: short(i + 2)?,
                })
            };
            a.set(Move {
                track: track()?,
                path: frame(1)?,
                frame: frame(2)?,
                was: at(3)?,
                to: at(6)?,
            });
        }
        Ok(a)
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from(
            "# Bot path frames moved from where the level has them (see `loader::nav_amend`).\n\
             # Edited in the viewer: `/debug viewer`, `E`.\n\
             # track path frame   was x y z   ->   to x y z\n",
        );
        for m in &self.moves {
            let _ = writeln!(
                s,
                "{} {} {}   {} {} {}   ->   {} {} {}",
                m.track, m.path, m.frame, m.was.x, m.was.y, m.was.z, m.to.x, m.to.y, m.to.z
            );
        }
        if !self.flags.is_empty() {
            s += "# track path frame   flags was   ->   to\n";
        }
        for f in &self.flags {
            let _ = writeln!(
                s,
                "{} {} {}   flags {:#06x}   ->   {:#06x}",
                f.track, f.path, f.frame, f.was, f.to
            );
        }
        s
    }

    /// The moves of `track`.
    pub fn of(&self, track: u32) -> impl Iterator<Item = &Move> {
        self.moves.iter().filter(move |m| m.track == track)
    }

    /// The flag changes of `track`.
    pub fn flags_of(&self, track: u32) -> impl Iterator<Item = &Flags> {
        self.flags.iter().filter(move |f| f.track == track)
    }

    /// Takes the place of the change of the same frame's flags; a change to what they were is
    /// none.
    pub fn set_flags(&mut self, f: Flags) {
        let key = |f: &Flags| (f.track, f.path, f.frame);
        match self.flags.binary_search_by_key(&key(&f), key) {
            Ok(i) if f.to == f.was => {
                self.flags.remove(i);
            }
            Ok(i) => self.flags[i] = f,
            Err(_) if f.to == f.was => {}
            Err(i) => self.flags.insert(i, f),
        }
    }

    /// Takes the place of the move of the same frame; a move to where the frame was is none.
    pub fn set(&mut self, m: Move) {
        let key = |m: &Move| (m.track, m.path, m.frame);
        match self.moves.binary_search_by_key(&key(&m), key) {
            Ok(i) if m.to == m.was => {
                self.moves.remove(i);
            }
            Ok(i) => self.moves[i] = m,
            Err(_) if m.to == m.was => {}
            Err(i) => self.moves.insert(i, m),
        }
    }

    /// Takes the move and the flag change of the frame back, if it has them.
    pub fn clear(&mut self, track: u32, path: usize, frame: usize) {
        self.moves
            .retain(|m| (m.track, m.path, m.frame) != (track, path, frame));
        self.flags
            .retain(|f| (f.track, f.path, f.frame) != (track, path, frame));
    }

    /// Moves the frames of `paths` (the level's own, as parsed) by the moves of `track`, and
    /// changes their flags. What is returned says which were left out, and why.
    pub fn apply(&self, track: u32, paths: &mut [NavPath]) -> Vec<String> {
        let mut refused = Vec::new();
        for (p, path) in paths.iter_mut().enumerate() {
            for c in self.flags_of(track).filter(|c| c.path == p) {
                match path.frames.get_mut(c.frame) {
                    Some(f) if f.flags == c.was => f.flags = c.to,
                    Some(f) => refused.push(format!(
                        "path {p} frame {} has flags {:#06x}, not {:#06x}",
                        c.frame, f.flags, c.was
                    )),
                    None => refused.push(format!("path {p} has no frame {}", c.frame)),
                }
            }
            let mut moved = vec![false; path.frames.len()];
            let before: Vec<SVec3> = path.frames.iter().map(|f| f.pos).collect();
            for m in self.of(track).filter(|m| m.path == p) {
                let Some(f) = path.frames.get_mut(m.frame) else {
                    refused.push(format!(
                        "path {p} has no frame {} (it has {})",
                        m.frame,
                        before.len()
                    ));
                    continue;
                };
                if f.pos != m.was {
                    refused.push(format!(
                        "path {p} frame {} is at {:?}, not {:?}",
                        m.frame, f.pos, m.was
                    ));
                    continue;
                }
                f.pos = m.to;
                moved[m.frame] = true;
            }
            let n = path.frames.len();
            for i in 0..n {
                let next = (i + 1) % n;
                if moved[i] || moved[next] {
                    let old = Leg::between(before[i], before[next]);
                    let new = Leg::between(path.frames[i].pos, path.frames[next].pos);
                    let f = &mut path.frames[i];
                    f.dist_to_next_xyz = shift(f.dist_to_next_xyz, new.xyz - old.xyz);
                    f.dist_to_next_xz = shift(f.dist_to_next_xz, new.xz - old.xz);
                    f.rot[1] = turn(f.rot[1], new.yaw - old.yaw);
                    f.rot[3] = turn(f.rot[3], new.pitch - old.pitch);
                }
            }
        }
        refused
    }
}

/// The way from one frame to the next: its lengths and its angles in 0x1000 = a full turn.
struct Leg {
    xyz: i64,
    xz: i64,
    yaw: i32,
    pitch: i32,
}

impl Leg {
    fn between(a: SVec3, b: SVec3) -> Leg {
        let (dx, dy, dz) = (
            b.x as i64 - a.x as i64,
            b.y as i64 - a.y as i64,
            b.z as i64 - a.z as i64,
        );
        let xz = ((dx * dx + dz * dz) as u64).isqrt() as i64;
        let xyz = ((dx * dx + dy * dy + dz * dz) as u64).isqrt() as i64;
        Leg {
            xyz,
            xz,
            // As `BOTS_SetRotation` aims at a frame (the offset does not matter: only the change
            // is used).
            yaw: math::ratan2(-dx as i32, -dz as i32),
            pitch: math::ratan2(dy as i32, xz as i32),
        }
    }
}

fn shift(v: i16, by: i64) -> i16 {
    (v as i64 + by).clamp(0, i16::MAX as i64) as i16
}

/// `a` (0x100 = a full turn) turned by `by` (0x1000 = a full turn), the short way.
fn turn(a: u8, by: i32) -> u8 {
    let by = (by + 0x800).rem_euclid(0x1000) - 0x800;
    ((a as i32 * 16 + by + 8) >> 4) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lev::NavFrame;

    fn at(x: i16, z: i16) -> SVec3 {
        SVec3 { x, y: 0, z }
    }

    #[test]
    fn text_round_trips() {
        let mut a = Amendments::default();
        a.set(Move {
            track: 3,
            path: 1,
            frame: 7,
            was: at(10, 20),
            to: at(-30, 40),
        });
        a.set_flags(Flags {
            track: 3,
            path: 1,
            frame: 8,
            was: 0,
            to: 0x0c00,
        });
        assert_eq!(Amendments::parse(&a.to_text()).unwrap(), a);
        assert!(Amendments::parse(Amendments::builtin().to_text().as_str()).is_ok());
    }

    #[test]
    fn a_move_home_is_none() {
        let mut a = Amendments::default();
        let m = Move {
            track: 0,
            path: 0,
            frame: 0,
            was: at(1, 1),
            to: at(2, 2),
        };
        a.set(m);
        a.set(Move { to: m.was, ..m });
        assert_eq!(a, Amendments::default());
    }

    #[test]
    fn moving_fixes_the_legs_around() {
        let frames: Vec<NavFrame> = [at(0, 0), at(0, 100), at(0, 200), at(0, 300)]
            .into_iter()
            .map(|pos| NavFrame {
                pos,
                dist_to_next_xyz: 100,
                dist_to_next_xz: 100,
                ..Default::default()
            })
            .collect();
        let mut paths = [NavPath {
            frames,
            ..Default::default()
        }];
        let mut a = Amendments::default();
        a.set(Move {
            track: 9,
            path: 0,
            frame: 2,
            was: at(0, 200),
            to: at(100, 200),
        });
        // A track the moves are not for, and a frame that is not where the move says.
        assert!(a.apply(8, &mut paths).is_empty());
        let mut wrong = paths.clone();
        wrong[0].frames[2].pos = at(1, 1);
        assert_eq!(a.apply(9, &mut wrong).len(), 1);

        assert!(a.apply(9, &mut paths).is_empty());
        let f = &paths[0].frames;
        assert_eq!(f[2].pos, at(100, 200));
        // 100 → hypot(100, 100), and the turn toward it and back again.
        assert_eq!(f[1].dist_to_next_xz, 141);
        assert_eq!(f[2].dist_to_next_xz, 141);
        assert_ne!(f[1].rot[1], 0);
        assert_eq!(f[1].rot[1].wrapping_add(f[2].rot[1]), 0);
        assert_eq!((f[0].dist_to_next_xz, f[0].rot[1]), (100, 0));
        assert_eq!((f[3].dist_to_next_xz, f[3].rot[1]), (100, 0));
    }
}
