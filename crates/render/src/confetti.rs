//! The confetti over the winner's screen (`DrawConfetti`, `gGT->confetti`): small flat quads that
//! sway and tumble as they fall, in a box of 0x800 units each way around the camera that wraps,
//! so the camera never leaves them behind. No piece is kept from one frame to the next: where
//! they are comes from a pseudo-random generator seeded the same every frame, the camera, and the
//! frame counter (`frameTimer_Confetti`), which is how far they have fallen and turned. This is
//! where they are, in game units from the camera; the viewer draws them.

use crate::weather::{Camera, next_byte};

/// `PLAYLEVEL_CONFETTI_PARTICLES`: the pieces of a race won, and how fast they come.
pub const PARTICLES: i16 = 250;

/// `MainInit`'s `confetti.velY`: how far the pieces fall each frame of the counter.
const VEL_Y: i32 = -10;

/// The camera is this far from the box's edges, and the box's middle as far ahead of it.
const BOX_MIDDLE: i32 = 0x400;

/// How far apart the frame counter is for one piece and the next: they turn out of step.
const PHASE_STEP: u32 = 23;

/// `sDrawConfettiColorTable8008a2a0`: the shade of a piece, by how square it is to the camera.
const SHADES: [u8; 36] = [
    0x39, 0x39, 0x3a, 0x3b, 0x3d, 0x3f, 0x41, 0x44, 0x47, 0x4b, 0x4f, 0x53, 0x58, 0x5d, 0x63, 0x69,
    0x6f, 0x76, 0x7d, 0x85, 0x8d, 0x95, 0x9e, 0xa7, 0xb1, 0xbb, 0xc5, 0xd0, 0xdb, 0xe7, 0xf3, 0xff,
    0xff, 0xff, 0xff, 0xff,
];

/// A piece: its four corners in game units from the camera, in the order of the GPU's
/// four-point polygon (0 1 2 and 1 2 3 are its triangles), and its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub corners: [[i32; 3]; 4],
    pub rgb: [u8; 3],
}

/// `gGT->confetti`
#[derive(Clone, Debug)]
pub struct Confetti {
    count: i32,
    max: i16,
    vanish_rate: i16,
}

impl Default for Confetti {
    /// `MainInit`: none, and none to come.
    fn default() -> Confetti {
        Confetti {
            count: 0,
            max: 0,
            vanish_rate: 0,
        }
    }
}

impl Confetti {
    /// `PlayLevel_UpdateLapStats`, for a human who finished first: all of them at once.
    pub fn start(&mut self) {
        self.max = PARTICLES;
        self.vanish_rate = PARTICLES;
    }

    /// How many pieces there are now.
    pub fn count(&self) -> i32 {
        self.count
    }

    /// The part of `DrawConfetti` before the drawing, on a frame that is not paused: the count
    /// goes toward the maximum.
    pub fn step(&mut self) {
        let (max, rate) = (i32::from(self.max), i32::from(self.vanish_rate));
        let diff = max - self.count;
        if diff < 0 {
            self.count -= rate;
            if diff + rate > 0 {
                self.count = max;
            }
        } else if diff > 0 {
            self.count += rate;
            if diff - rate < 0 {
                self.count = max;
            }
        }
    }

    /// The rest of `DrawConfetti`: the pieces at frame `timer` of the counter, as `camera` sees
    /// them. (Retail leaves out the ones the GTE cannot project, behind the camera or off the
    /// screen; those are the viewer's to clip.)
    pub fn pieces(&self, camera: &Camera, timer: u32) -> Vec<Piece> {
        let (sin, cos) = math::sin_cos(camera.yaw as u16 as u32);
        let center_x = (sin >> 2) + BOX_MIDDLE;
        let center_z = (cos >> 2) + BOX_MIDDLE;
        let base_x = center_x - i32::from(camera.pos.x);
        let base_y = (VEL_Y as u32).wrapping_mul(timer) as i32 - i32::from(camera.pos.y);
        let base_z = center_z - i32::from(camera.pos.z);

        let (mut state0, mut state1) = (0x3012_5400_u32, 0x4935_83fe_u32);
        let mut timer = timer;
        let mut cycle = 6;
        let mut out = Vec::with_capacity(self.count.max(0) as usize);
        for _ in 0..self.count.max(0) {
            let [rx, ry, rz] = [(); 3].map(|_| next_byte(&mut state0, &mut state1) as i32 >> 21);
            cycle -= 1;
            if cycle == 0 {
                cycle = 6;
            }
            timer = timer.wrapping_add(PHASE_STEP);

            let mut x = ((rx + base_x) & 0x7ff) - center_x;
            let y = ((ry + base_y) & 0x7ff) - BOX_MIDDLE;
            let mut z = ((rz + base_z) & 0x7ff) - center_z;

            // It sways round in a circle, the odd colours the other way, and tumbles eight times
            // as fast.
            let angle = if cycle & 1 != 0 {
                timer.wrapping_neg()
            } else {
                timer
            } << 4;
            let (sin_a, cos_a) = math::sin_cos(angle);
            x += sin_a >> 5;
            z += cos_a >> 5;
            let (sin_b, cos_b) = math::sin_cos(angle << 3);

            let shade = SHADES[(cos_b.unsigned_abs() >> 7) as usize];
            let rgb = [1, 2, 4].map(|bit| if cycle & bit != 0 { shade } else { 0 });

            let half_y = cos_b >> 8;
            let skew_x = (sin_b * sin_a) >> 20;
            let half_x = cos_a >> 8;
            let skew_z = (sin_b * cos_a) >> 20;
            let half_z = sin_a >> 8;
            out.push(Piece {
                corners: [
                    [x - half_x - skew_x, y - half_y, z - half_z + skew_z],
                    [x + half_x - skew_x, y - half_y, z + half_z + skew_z],
                    [x - half_x + skew_x, y + half_y, z - half_z - skew_z],
                    [x + half_x + skew_x, y + half_y, z + half_z - skew_z],
                ],
                rgb,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::lev::SVec3;

    fn camera() -> Camera {
        Camera {
            pos: SVec3 {
                x: 1000,
                y: 300,
                z: -2000,
            },
            yaw: 0x300,
        }
    }

    fn started() -> Confetti {
        let mut confetti = Confetti::default();
        confetti.start();
        confetti.step();
        confetti
    }

    #[test]
    fn there_is_none_until_a_win_and_then_all_at_once() {
        let mut confetti = Confetti::default();
        confetti.step();
        assert_eq!(confetti.count(), 0);
        assert!(confetti.pieces(&camera(), 0).is_empty());
        confetti.start();
        confetti.step();
        assert_eq!(confetti.count(), i32::from(PARTICLES));
        assert_eq!(confetti.pieces(&camera(), 0).len(), PARTICLES as usize);
    }

    #[test]
    fn the_pieces_are_in_a_box_around_the_camera() {
        for piece in started().pieces(&camera(), 1234) {
            for [x, y, z] in piece.corners {
                assert!((-0x800 - 0x100..0x800 + 0x100).contains(&x), "{piece:?}");
                assert!((-0x400 - 0x100..0x400 + 0x100).contains(&y), "{piece:?}");
                assert!((-0x800 - 0x100..0x800 + 0x100).contains(&z), "{piece:?}");
            }
        }
    }

    #[test]
    fn a_piece_stays_where_it_is_in_the_world_as_the_camera_moves() {
        let confetti = started();
        let mut moved = camera();
        moved.pos.x += 40;
        moved.pos.z -= 24;
        let (a, b) = (confetti.pieces(&camera(), 50), confetti.pieces(&moved, 50));
        let same = a
            .iter()
            .zip(&b)
            .filter(|(a, b)| {
                (0..4).all(|k| {
                    a.corners[k][0] == b.corners[k][0] + 40
                        && a.corners[k][1] == b.corners[k][1]
                        && a.corners[k][2] == b.corners[k][2] - 24
                })
            })
            .count();
        // All but the few that wrapped round the box.
        assert!(same > 230, "{same}");
    }

    #[test]
    fn they_fall_ten_a_frame_and_spin() {
        let confetti = started();
        let (a, b) = (
            confetti.pieces(&camera(), 100),
            confetti.pieces(&camera(), 101),
        );
        let middle = |p: &Piece| {
            std::array::from_fn::<i32, 3, _>(|a| p.corners.iter().map(|c| c[a]).sum::<i32>() / 4)
        };
        let fell = a
            .iter()
            .zip(&b)
            .filter(|(a, b)| (middle(a)[1] - middle(b)[1] - 10).abs() <= 1)
            .count();
        assert!(fell > 240, "{fell}");
        assert!(a.iter().zip(&b).all(|(a, b)| a.corners != b.corners));
    }

    #[test]
    fn the_colours_are_six_and_never_grey() {
        let pieces = started().pieces(&camera(), 7);
        for piece in &pieces {
            let lit: Vec<_> = piece.rgb.iter().filter(|&&c| c != 0).collect();
            assert!(lit.len() == 1 || lit.len() == 2, "{piece:?}");
            assert!(lit.iter().all(|&&c| c == *lit[0] && c >= 0x39));
        }
        // Magenta, blue, yellow, green, red, cyan, over and over.
        let on = |p: &Piece| p.rgb.map(|c| c != 0);
        assert_eq!(on(&pieces[0]), [true, false, true]);
        assert_eq!(on(&pieces[1]), [false, false, true]);
        assert_eq!(on(&pieces[5]), [false, true, true]);
        assert_eq!(on(&pieces[6]), on(&pieces[0]));
    }
}
