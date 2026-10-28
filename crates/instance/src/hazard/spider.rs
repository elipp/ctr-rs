//! `RB_Spider_ThTick`: the spiders of Cortex Castle hang at the roof and wiggle, drop to the
//! ground on a thread, wiggle there, and climb back. A kart under one (or on the ground where it
//! lands) is spun out. The spiders are put out of step by the digit their names end in.
//!
//! The web that runs up to the roof and the shadow on the ground are the drawing's.

use loader::lev::InstDef;
use vehicle::rotmat::rot_to_matrix;
use vehicle::{Matrix, Sound3D};

use crate::{Contact, Context, Damage, Instance, Pose, Tick};

/// The model's animations (`INSTANCE_GetNumAnimFrames`).
pub mod anim {
    /// The drop, played backwards to climb.
    pub const DROP: u8 = 0;
    pub const WIGGLE: u8 = 1;
}

/// How many wiggles it makes at either end.
const WIGGLES: u8 = 5;
/// `LinkedCollide_Radius`'s radius: a squared distance.
const HIT_RADIUS: u32 = 0x9000;
const HIT_SOUND: i32 = 0x7b;
/// The sound of the drop starting, and of the climb at its frame `CLIMB_SOUND_FRAME`.
const DROP_SFX: u8 = 0x7a;
const CLIMB_SFX: u8 = 0x79;
const CLIMB_SOUND_FRAME: i16 = 0xc;
/// Spiders wait before they first move, by the digit of their names (`spiderID`), so that they
/// do not hang in step.
const DELAYS: [(usize, u16); 2] = [(3, 91), (2, 69)];
/// `spiderArr`: how high above its place the spider is at each frame of the drop.
const HEIGHTS: [i16; 26] = [
    0x4c0, 0x439, 0x3a6, 0x306, 0x266, 0x1c8, 0x139, 0xb9, 0x59, 0x17, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0,
];

/// How high above the place of the definition the web starts, and how far above the spider it
/// ends (`RB_Spider_DrawWebs`).
const WEB_TOP: i32 = 0x540;
const WEB_BOTTOM: i32 = 0x60;
/// The shadow lies this far under the place of the definition, and starts at this scale
/// (`RB_Spider_LInB`).
const SHADOW_DROP: i32 = 8;
pub const SHADOW_SCALE: i32 = 0x2000;

/// `struct Spider` and the parts of its `Instance` the code changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spider {
    /// `matrix.t`: where it is, in level units.
    pub pos: [i32; 3],
    /// The height it was placed at, which the drop is measured from.
    base_y: i32,
    /// Ticks to wait before the first move.
    delay: u16,
    near_roof: bool,
    wiggles: u8,
    pub anim_index: u8,
    pub anim_frame: i16,
    /// The scale of the shadow's width and depth, which the first frames of the drop change.
    shadow_scale: i32,
    matrix: Matrix,
}

impl Spider {
    /// `RB_Spider_LInB`: at the roof, `0x4c0` above the place of the definition. `id` is the digit
    /// its name ends in.
    pub fn new(def: &InstDef, id: Option<usize>) -> Spider {
        let base_y = def.pos.y as i32;
        Spider {
            pos: [def.pos.x as i32, base_y + HEIGHTS[0] as i32, def.pos.z as i32],
            base_y,
            delay: DELAYS
                .iter()
                .find(|&&(n, _)| Some(n) == id)
                .map_or(0, |&(_, delay)| delay),
            near_roof: true,
            wiggles: 0,
            anim_index: anim::WIGGLE,
            anim_frame: 0,
            shadow_scale: SHADOW_SCALE,
            matrix: rot_to_matrix(def.rot),
        }
    }

    /// Where the drop puts it for the frame it is at.
    fn settle(&mut self) {
        let height = HEIGHTS.get(self.anim_frame as usize).copied().unwrap_or(0);
        self.pos[1] = self.base_y + height as i32;
        if self.anim_frame < 0xb {
            self.shadow_scale = self.anim_frame as i32 * 0x1000 / 10 + 0x1800;
        }
    }

    /// The two ends of the web it hangs from, roof first.
    pub fn web(&self) -> ([i32; 3], [i32; 3]) {
        let at = |y| [self.pos[0], y, self.pos[2]];
        (at(self.base_y + WEB_TOP), at(self.pos[1] + WEB_BOTTOM))
    }

    /// Where its shadow lies on the ground, and the scale of its width and depth.
    pub fn shadow(&self) -> ([i32; 3], i32) {
        (
            [self.pos[0], self.base_y - SHADOW_DROP, self.pos[2]],
            self.shadow_scale,
        )
    }

    /// One game tick. `frames` is the number of frames of its drop and wiggle animations. Where
    /// it looks for drivers goes in `out`, unless it is still waiting.
    pub fn tick(&mut self, frames: [i32; 2], out: &mut Tick) {
        if self.delay != 0 {
            self.delay -= 1;
            return;
        }
        let last = |a: u8| self.anim_frame as i32 + 1 >= frames[a as usize];
        let sound = |id| Sound3D { id, pos: self.pos };
        if self.wiggles < WIGGLES {
            if !last(anim::WIGGLE) {
                self.anim_frame += 1;
            } else {
                self.anim_frame = 0;
                self.wiggles += 1;
                if self.wiggles == WIGGLES {
                    // On to the drop, or up from its end.
                    self.anim_index = anim::DROP;
                    if self.near_roof {
                        out.sounds.push(sound(DROP_SFX));
                    } else {
                        self.anim_frame = (frames[anim::DROP as usize] - 1) as i16;
                    }
                }
            }
        } else if self.near_roof {
            if !last(anim::DROP) {
                self.anim_frame += 1;
            } else {
                self.wiggles = 0;
                self.near_roof = false;
                self.anim_index = anim::WIGGLE;
            }
            self.settle();
        } else {
            // The climb plays the drop backwards, and stops short of its first frame.
            if self.anim_frame - 1 > 0 {
                self.anim_frame -= 1;
                if self.anim_frame == CLIMB_SOUND_FRAME {
                    out.sounds.push(sound(CLIMB_SFX));
                }
            } else {
                self.anim_frame = 0;
                self.wiggles = 0;
                self.near_roof = true;
                self.anim_index = anim::WIGGLE;
            }
            self.settle();
        }
        out.contacts.push(Contact {
            voice: true,
            ..Contact::sphere(self.pos, Damage::Spin, HIT_RADIUS, HIT_SOUND)
        });
    }
}

impl Instance for Spider {
    fn th_tick(&mut self, cx: &Context, out: &mut Tick) {
        self.tick([cx.frames[0], cx.frames[1]], out);
    }

    fn pose(&self) -> Option<Pose> {
        Some(Pose {
            pos: self.pos,
            matrix: self.matrix,
            animation: (self.anim_index, self.anim_frame),
            hidden: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::lev::SVec3;

    /// The drop is 20 frames, the wiggle 6.
    const FRAMES: [i32; 2] = [20, 6];

    fn def(name: &str) -> InstDef {
        InstDef {
            name: name.into(),
            model: 0,
            model_name: "spider".into(),
            scale: Default::default(),
            color_rgba: 0,
            flags: 0,
            pos: SVec3 {
                x: 100,
                y: 50,
                z: -200,
            },
            rot: Default::default(),
            model_id: 0,
        }
    }

    fn run(spider: &mut Spider, ticks: usize) -> Tick {
        let mut out = Tick::default();
        for _ in 0..ticks {
            spider.tick(FRAMES, &mut out);
        }
        out
    }

    #[test]
    fn it_starts_at_the_roof() {
        let spider = Spider::new(&def("spider#0"), Some(0));
        assert_eq!(spider.pos, [100, 50 + 0x4c0, -200]);
        assert_eq!((spider.anim_index, spider.anim_frame), (anim::WIGGLE, 0));
    }

    #[test]
    fn the_later_spiders_wait_and_do_not_hurt_meanwhile() {
        let mut spider = Spider::new(&def("spider#3"), Some(3));
        assert!(run(&mut spider, 91).contacts.is_empty());
        assert_eq!(run(&mut spider, 1).contacts.len(), 1);
        let mut spider = Spider::new(&def("spider#2"), Some(2));
        assert!(run(&mut spider, 69).contacts.is_empty());
        assert_eq!(run(&mut spider, 1).contacts.len(), 1);
    }

    #[test]
    fn it_wiggles_five_times_then_drops_to_the_ground() {
        let mut spider = Spider::new(&def("spider#0"), Some(0));
        // Five wiggles of six frames, the last frame of each starting the next.
        let out = run(&mut spider, 6 * 5);
        assert_eq!((spider.anim_index, spider.anim_frame), (anim::DROP, 0));
        assert_eq!(out.sounds, [Sound3D { id: DROP_SFX, pos: spider.pos }]);
        // Up there still: the roof is where the drop starts.
        assert_eq!(spider.pos[1], 50 + 0x4c0);
        run(&mut spider, 10);
        assert_eq!(spider.pos[1], 50);
        // The drop ends, and it wiggles on the ground.
        run(&mut spider, 10);
        assert_eq!(spider.anim_index, anim::WIGGLE);
        assert_eq!(spider.pos[1], 50);
    }

    #[test]
    fn it_climbs_back_with_a_sound_and_a_full_cycle_repeats() {
        let mut spider = Spider::new(&def("spider#0"), Some(0));
        let mut out = Tick::default();
        let mut ticks = 0;
        // Roof wiggles, drop, ground wiggles, climb: back at the roof, ready to start again.
        let mut climbed = false;
        while !(climbed && spider.near_roof) {
            spider.tick(FRAMES, &mut out);
            ticks += 1;
            climbed |= out.sounds.iter().any(|s| s.id == CLIMB_SFX);
            assert!(ticks < 500);
        }
        assert_eq!(spider.pos[1], 50 + 0x4c0);
        let ids: Vec<u8> = out.sounds.iter().map(|s| s.id).collect();
        assert_eq!(ids, [DROP_SFX, CLIMB_SFX]);
        // It hurt all the way.
        assert_eq!(out.contacts.len(), ticks);
    }

    #[test]
    fn what_it_does_is_a_spin_with_its_voice() {
        let mut spider = Spider::new(&def("spider#0"), Some(0));
        let contact = run(&mut spider, 1).contacts[0];
        assert_eq!(
            (contact.damage, contact.hit_radius, contact.sound, contact.voice),
            (Damage::Spin, 0x9000, 0x7b, true)
        );
    }
}
