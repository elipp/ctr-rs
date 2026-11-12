//! Our own changes to the bot paths (`Level::nav_paths`): frames moved from where the level has
//! them, mostly to take tight corners wider. They are kept in `nav_amendments.ron` next to this
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
//! The file is RON, a frame to a line, by track (`IMPLICIT_SOME` is on: no `Some(..)`):
//!
//! ```text
//! {
//!     3: [
//!         (path: 0, frame: 212, pos: (was: (-1234, 56, 789), to: (-1300, 56, 800))),
//!         (path: 0, frame: 215, flags: (was: 0x0000, to: 0x0400)),
//!     ],
//! }
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

use crate::lev::{NavPath, SVec3};

/// The file the viewer saves to: the one [`Amendments::builtin`] is compiled from.
pub const SOURCE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/nav_amendments.ron");

const BUILTIN: &str = include_str!("nav_amendments.ron");

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

/// A frame's line in the file.
#[derive(serde::Deserialize)]
struct Entry {
    path: usize,
    frame: usize,
    #[serde(default)]
    pos: Option<Change<(i16, i16, i16)>>,
    #[serde(default)]
    flags: Option<Change<u16>>,
}

#[derive(serde::Deserialize)]
struct Change<T> {
    was: T,
    to: T,
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
            Amendments::parse(BUILTIN).unwrap_or_else(|e| panic!("nav_amendments.ron: {e}"))
        })
    }

    pub fn parse(text: &str) -> Result<Amendments, String> {
        let file: BTreeMap<u32, Vec<Entry>> = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(text)
            .map_err(|e| e.to_string())?;
        let mut a = Amendments::default();
        for (track, entries) in file {
            for e in entries {
                if let Some(Change { was, to }) = e.pos {
                    let v = |(x, y, z)| SVec3 { x, y, z };
                    a.set(Move {
                        track,
                        path: e.path,
                        frame: e.frame,
                        was: v(was),
                        to: v(to),
                    });
                }
                if let Some(Change { was, to }) = e.flags {
                    a.set_flags(Flags {
                        track,
                        path: e.path,
                        frame: e.frame,
                        was,
                        to,
                    });
                }
            }
        }
        Ok(a)
    }

    /// The file: written here, not by `ron`, for a frame to a line and the flags in hex.
    pub fn to_text(&self) -> String {
        let mut s = String::from(
            "// Bot path frames changed from where the level has them (see `loader::nav_amend`).\n\
             // Edited in the viewer: `/debug viewer`, `E`. By track (`LevelId` or map id).\n\
             {\n",
        );
        // Every frame that has a change, in order, with its move and its flags.
        let mut frames: BTreeMap<(u32, usize, usize), (Option<&Move>, Option<&Flags>)> =
            BTreeMap::new();
        for m in &self.moves {
            frames.entry((m.track, m.path, m.frame)).or_default().0 = Some(m);
        }
        for f in &self.flags {
            frames.entry((f.track, f.path, f.frame)).or_default().1 = Some(f);
        }
        let mut track = None;
        for ((t, path, frame), (m, f)) in frames {
            if track != Some(t) {
                if track.is_some() {
                    s += "    ],\n";
                }
                let _ = writeln!(s, "    {t}: [");
                track = Some(t);
            }
            let _ = write!(s, "        (path: {path}, frame: {frame}");
            if let Some(m) = m {
                let v = |p: SVec3| format!("({}, {}, {})", p.x, p.y, p.z);
                let _ = write!(s, ", pos: (was: {}, to: {})", v(m.was), v(m.to));
            }
            if let Some(f) = f {
                let _ = write!(s, ", flags: (was: {:#06x}, to: {:#06x})", f.was, f.to);
            }
            s += "),\n";
        }
        if track.is_some() {
            s += "    ],\n";
        }
        s += "}\n";
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
    fn the_file_is_as_it_is_written() {
        assert_eq!(Amendments::builtin().to_text(), BUILTIN);
        let example = "{ 3: [
            (path: 0, frame: 212, pos: (was: (-1234, 56, 789), to: (-1300, 56, 800))),
            (path: 0, frame: 215, flags: (was: 0x0000, to: 0x0400)),
        ] }";
        let a = Amendments::parse(example).unwrap();
        assert_eq!((a.of(3).count(), a.flags_of(3).count()), (1, 1));
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
