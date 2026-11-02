//! The speedometer's dial and needle (`UI_DrawSpeedBG`, `UI_DrawSpeedNeedle`) and the jump meter's
//! digits (`DebugFont_DrawNumbers`). The UI has no polygons or lines, so the dial is rasterized
//! here and the needle is a shader, both deliberately finer than retail's pixels and undithered.

use loader::icons::Icon;
use loader::vram::Vram;

use crate::hud_icons::half_transparent;
use crate::mesh::Atlas;

/// The part of retail's screen the dial and the needle reach, whatever the needle's angle.
pub const LEFT: i32 = 405;
pub const TOP: i32 = 145;
pub const WIDTH: usize = 105;
pub const HEIGHT: usize = 65;

/// Picture pixels per pixel of retail's screen: one each on a 2x display, where `hud::PIXEL` is 4
/// by 6 physical pixels.
const SHARP_X: i32 = 4;
const SHARP_Y: i32 = 6;
/// Samples per picture pixel along each axis, averaged for smooth edges.
const SAMPLES: i32 = 2;
const FINE_X: i32 = SHARP_X * SAMPLES;
const FINE_Y: i32 = SHARP_Y * SAMPLES;

/// `hudStructPtr[UI_HUD_SLOT_SPEEDOMETER]`
const SLOT_X: i32 = 414;
const SLOT_Y: i32 = 145;

/// Where `UI_DrawSpeedBG` puts the dial, whatever the slot says.
const DIAL_X: i32 = 480;
const DIAL_Y: i32 = 190;

/// The spokes of the dial, its outer and inner end each (`data.speedometerBG_vertData[0]`).
const SPOKES: [(i32, i32); 14] = [
    (-65, 16),
    (-54, 13),
    (-70, 0),
    (-58, 0),
    (-65, -16),
    (-54, -13),
    (-50, -29),
    (-42, -24),
    (-25, -39),
    (-21, -32),
    (0, -42),
    (0, -35),
    (25, -39),
    (21, -32),
];

/// The same with the inner ends further in, for the lines drawn across the band
/// (`data.speedometerBG_vertData[1]`).
const LONG_SPOKES: [(i32, i32); 14] = [
    (-65, 16),
    (-48, 11),
    (-70, 0),
    (-51, 0),
    (-65, -16),
    (-48, -11),
    (-50, -29),
    (-37, -21),
    (-25, -39),
    (-18, -29),
    (0, -42),
    (0, -31),
    (25, -39),
    (18, -29),
];

/// The band's colours from its low end to its high one (`DrawSpeedBG_Colors`).
const BAND: [[u8; 3]; 7] = [
    [0x00, 0xb5, 0x00],
    [0x00, 0xb5, 0x00],
    [0x00, 0xb5, 0x00],
    [0xff, 0xd1, 0x00],
    [0xdb, 0x00, 0x00],
    [0xdb, 0x00, 0x00],
    [0xdb, 0x00, 0x00],
];

const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const BLACK: [u8; 3] = [0, 0, 0];

/// A point of retail's screen.
type Point = (i32, i32);

/// A point of the canvas's samples, from the top left of the picture.
type Fine = (i32, i32);

fn fine((x, y): Point) -> Fine {
    ((x - LEFT) * FINE_X, (y - TOP) * FINE_Y)
}

struct Canvas {
    samples: Vec<u8>,
}

impl Canvas {
    const WIDTH: i32 = WIDTH as i32 * FINE_X;
    const HEIGHT: i32 = HEIGHT as i32 * FINE_Y;

    fn new() -> Canvas {
        Canvas {
            samples: vec![0; (Self::WIDTH * Self::HEIGHT * 4) as usize],
        }
    }

    fn put(&mut self, x: i32, y: i32, rgba: [u8; 4]) {
        if (0..Self::WIDTH).contains(&x) && (0..Self::HEIGHT).contains(&y) {
            self.samples[((y * Self::WIDTH + x) * 4) as usize..][..4].copy_from_slice(&rgba);
        }
    }

    /// A line as wide as a pixel of retail's screen, from the corner of one such pixel to another's.
    fn line(&mut self, from: Fine, to: Fine, color: [u8; 3]) {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let steps = dx.abs().max(dy.abs());
        for step in 0..=steps {
            let along = |d: i32| (d * step * 2 + steps * d.signum()).checked_div(steps * 2);
            let x = from.0 + along(dx).unwrap_or(0);
            let y = from.1 + along(dy).unwrap_or(0);
            for py in y..y + FINE_Y {
                for px in x..x + FINE_X {
                    self.put(px, py, opaque(color));
                }
            }
        }
    }

    /// A triangle of a colour at each corner. Like the PS1's, it leaves out its right and bottom
    /// edges, so that triangles sharing an edge neither overlap nor leave a gap.
    fn triangle(&mut self, corners: [(Fine, [u8; 4]); 3]) {
        let [mut a, b, mut c] = corners;
        let edge =
            |p: Fine, q: Fine, x: i32, y: i32| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
        let mut area = edge(a.0, b.0, c.0.0, c.0.1);
        if area == 0 {
            return;
        }
        if area < 0 {
            std::mem::swap(&mut a, &mut c);
            area = -area;
        }
        let excludes = |p: Fine, q: Fine| {
            let (dx, dy) = (q.0 - p.0, q.1 - p.1);
            !(dy < 0 || (dy == 0 && dx > 0))
        };
        let excluded = [excludes(b.0, c.0), excludes(c.0, a.0), excludes(a.0, b.0)];
        let (min_x, max_x) = (a.0.0.min(b.0.0).min(c.0.0), a.0.0.max(b.0.0).max(c.0.0));
        let (min_y, max_y) = (a.0.1.min(b.0.1).min(c.0.1), a.0.1.max(b.0.1).max(c.0.1));
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let weights = [
                    edge(b.0, c.0, x, y),
                    edge(c.0, a.0, x, y),
                    edge(a.0, b.0, x, y),
                ];
                if (0..3).any(|n| weights[n] < 0 || (weights[n] == 0 && excluded[n])) {
                    continue;
                }
                let channel = |n: usize| {
                    let sum = weights[0] * a.1[n] as i32
                        + weights[1] * b.1[n] as i32
                        + weights[2] * c.1[n] as i32;
                    (sum / area) as u8
                };
                self.put(x, y, std::array::from_fn(channel));
            }
        }
    }

    /// Averages the samples, weighted by alpha so that the transparent ones add no black.
    fn atlas(self) -> Atlas {
        let (width, height) = (WIDTH * SHARP_X as usize, HEIGHT * SHARP_Y as usize);
        let samples = SAMPLES as usize;
        let mut rgba = vec![0; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0u32; 4];
                for sample_y in y * samples..(y + 1) * samples {
                    for sample_x in x * samples..(x + 1) * samples {
                        let at = (sample_y * Self::WIDTH as usize + sample_x) * 4;
                        let alpha = self.samples[at + 3] as u32;
                        for (total, &channel) in sum.iter_mut().zip(&self.samples[at..at + 3]) {
                            *total += channel as u32 * alpha;
                        }
                        sum[3] += alpha;
                    }
                }
                let out = &mut rgba[(y * width + x) * 4..][..4];
                for (channel, &total) in out.iter_mut().zip(&sum[..3]) {
                    *channel = total.checked_div(sum[3]).unwrap_or(0) as u8;
                }
                out[3] = (sum[3] / (samples * samples) as u32) as u8;
            }
        }
        Atlas {
            width,
            height,
            rgba,
        }
    }
}

fn opaque([r, g, b]: [u8; 3]) -> [u8; 4] {
    [r, g, b, 255]
}

/// The dial: a dark fan, the band of colours on it, and the lines over them. Retail links its
/// primitives so that the first it adds is drawn last, so this goes through them backwards.
pub fn dial() -> Atlas {
    let mut canvas = Canvas::new();
    let at = |(x, y): Point| fine((x + DIAL_X, y + DIAL_Y));
    let spoke = |n: usize| at(SPOKES[n]);
    let long_spoke = |n: usize| at(LONG_SPOKES[n]);
    let shadow = |(x, y): Fine| (x + FINE_X, y + FINE_Y);

    let dark = half_transparent([0, 0, 0, 255]);
    let hub = at((SPOKES[13].0, SPOKES[1].1));
    for i in (0..SPOKES.len() - 2).step_by(2) {
        canvas.triangle([(spoke(i + 1), dark), (spoke(i + 3), dark), (hub, dark)]);
    }

    for i in (0..SPOKES.len() - 2).step_by(2).rev() {
        let (low, high) = (opaque(BAND[i / 2]), opaque(BAND[i / 2 + 1]));
        let quad = [spoke(i), spoke(i + 1), spoke(i + 2), spoke(i + 3)];
        canvas.triangle([(quad[0], low), (quad[1], low), (quad[2], high)]);
        canvas.triangle([(quad[1], low), (quad[2], high), (quad[3], high)]);
        canvas.line(shadow(quad[1]), shadow(quad[3]), BLACK);
        canvas.line(shadow(quad[0]), shadow(quad[2]), BLACK);
        canvas.line(quad[1], quad[3], WHITE);
        canvas.line(quad[0], quad[2], WHITE);
    }

    for i in (0..LONG_SPOKES.len()).step_by(2).rev() {
        let (from, to) = (long_spoke(i), long_spoke(i + 1));
        canvas.line(shadow(from), shadow(to), BLACK);
        canvas.line(from, to, WHITE);
    }
    canvas.atlas()
}

/// The needle's corners, as fractions of the picture. A kite of two halves, split from tip to tail.
pub struct Needle {
    pub tip: [f32; 2],
    pub tail: [f32; 2],
    pub bright_side: [f32; 2],
    pub dark_side: [f32; 2],
}

/// The needle at `angle`: `0x1000` is a turn, 0 points down and `0x400` right.
pub fn needle(angle: i32) -> Needle {
    const CENTER: Point = (SLOT_X + 65 - LEFT, SLOT_Y + 41 - TOP);
    const LENGTH: f32 = 60.0;
    const NEEDLE_WIDTH: f32 = 6.0;

    let unit = |angle: i32| {
        let (sin, cos) = math::sin_cos(angle as u32);
        (sin as f32 / 4096.0, cos as f32 / 4096.0)
    };
    let (sin, cos) = unit(angle);
    let (sin_side, cos_side) = unit(angle + 0x400);
    // The screen's pixels are taller than wide.
    let corner = |x: f32, y: f32| {
        [
            (CENTER.0 as f32 + x) / WIDTH as f32,
            (CENTER.1 as f32 + y * 0.625) / HEIGHT as f32,
        ]
    };
    Needle {
        tip: corner(sin * LENGTH, cos * LENGTH),
        tail: corner(-sin * NEEDLE_WIDTH, -cos * NEEDLE_WIDTH),
        dark_side: corner(sin_side * NEEDLE_WIDTH, cos_side * (NEEDLE_WIDTH + 2.0)),
        bright_side: corner(-sin_side * NEEDLE_WIDTH, -cos_side * (NEEDLE_WIDTH + 2.0)),
    }
}

/// `gGT->ptrIcons[DEBUG_FONT_ICON_INDEX]`, and how its digits lie in it
/// (`DEBUG_FONT_DIGIT_TILE_SIZE`, `DEBUG_FONT_DIGIT_ZERO_ATLAS_OFFSET`): on its second row.
const DEBUG_FONT: i32 = 0x42;
pub const DIGIT_SIZE: usize = 7;
const DIGIT_ZERO: usize = 5;

/// The debug font's digits 0 to 9, each a picture of its own. `None` if the pack lacks the font.
pub fn digits(icons: &[Icon], vram: &Vram) -> Option<Vec<Atlas>> {
    let icon = icons.iter().find(|i| i.global_index == DEBUG_FONT)?;
    let (u0, v0) = icon.layout.uv[0];
    Some(
        (0..10)
            .map(|digit| {
                let mut rgba = vec![0; DIGIT_SIZE * DIGIT_SIZE * 4];
                for y in 0..DIGIT_SIZE {
                    for x in 0..DIGIT_SIZE {
                        let u = u0.wrapping_add(((DIGIT_ZERO + digit) * DIGIT_SIZE + x) as u8);
                        let v = v0.wrapping_add((DIGIT_SIZE + y) as u8);
                        // Retail tints the font black (packet `0x2e`, colour 0): its plain texels
                        // come out black, and its STP ones, added in the page's blend mode, change
                        // nothing behind them.
                        if let Ok(Some((_, false))) = vram.texel_stp(&icon.layout, u, v) {
                            rgba[(y * DIGIT_SIZE + x) * 4..][..4].copy_from_slice(&[0, 0, 0, 255]);
                        }
                    }
                }
                Atlas {
                    width: DIGIT_SIZE,
                    height: DIGIT_SIZE,
                    rgba,
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The picture's pixel in the middle of a pixel of retail's screen.
    fn pixel(atlas: &Atlas, x: i32, y: i32) -> [u8; 4] {
        let px = (x - LEFT) * SHARP_X + SHARP_X / 2;
        let py = (y - TOP) * SHARP_Y + SHARP_Y / 2;
        let at = (py as usize * atlas.width + px as usize) * 4;
        std::array::from_fn(|n| atlas.rgba[at + n])
    }

    fn inked(atlas: &Atlas, x: i32, y: i32) -> bool {
        pixel(atlas, x, y)[3] != 0
    }

    fn ink_on_the_border(atlas: &Atlas) -> bool {
        let (w, h) = (atlas.width, atlas.height);
        let alpha = |x: usize, y: usize| atlas.rgba[(y * w + x) * 4 + 3];
        (0..w).any(|x| alpha(x, 0) != 0 || alpha(x, h - 1) != 0)
            || (0..h).any(|y| alpha(0, y) != 0 || alpha(w - 1, y) != 0)
    }

    #[test]
    fn triangles_sharing_an_edge_cover_a_square_once() {
        const SIDE: i32 = 8;
        let (x0, y0) = (LEFT + 3, TOP + 3);
        let (x1, y1) = (x0 + SIDE, y0 + SIDE);
        let corner = |x, y| fine((x, y));
        let mut canvas = Canvas::new();
        let color = [10, 20, 30, 255];
        canvas.triangle([
            (corner(x0, y0), color),
            (corner(x1, y0), color),
            (corner(x0, y1), color),
        ]);
        canvas.triangle([
            (corner(x1, y0), color),
            (corner(x0, y1), color),
            (corner(x1, y1), color),
        ]);
        let count = canvas.samples.chunks(4).filter(|p| p[3] != 0).count();
        assert_eq!(count, (SIDE * FINE_X * SIDE * FINE_Y) as usize);
    }

    #[test]
    fn a_triangle_shades_between_its_corners() {
        let mut canvas = Canvas::new();
        let (x, y) = (LEFT + 2, TOP + 2);
        canvas.triangle([
            (fine((x, y)), [0, 0, 0, 255]),
            (fine((x + 40, y)), [200, 0, 0, 255]),
            (fine((x, y + 40)), [0, 0, 0, 255]),
        ]);
        let (sample_x, sample_y) = fine((x + 20, y));
        let at = ((sample_y * Canvas::WIDTH + sample_x) * 4) as usize;
        assert_eq!(canvas.samples[at], 100);
    }

    #[test]
    fn a_line_covers_both_ends() {
        let mut canvas = Canvas::new();
        canvas.line(fine((LEFT, TOP)), fine((LEFT + 4, TOP + 2)), WHITE);
        let atlas = canvas.atlas();
        assert!(inked(&atlas, LEFT, TOP));
        assert!(inked(&atlas, LEFT + 4, TOP + 2));
        assert!(!inked(&atlas, LEFT + 5, TOP + 2));
    }

    #[test]
    fn the_dial_is_within_its_picture() {
        assert!(!ink_on_the_border(&dial()));
    }

    const SLOWEST: i32 = 3472;
    const FASTEST: i32 = 1792;

    #[test]
    fn the_needle_is_within_the_picture_at_every_angle() {
        for angle in FASTEST..=SLOWEST {
            let n = needle(angle);
            for [x, y] in [n.tip, n.tail, n.bright_side, n.dark_side] {
                assert!(
                    (0.0..1.0).contains(&x) && (0.0..1.0).contains(&y),
                    "{angle}"
                );
            }
        }
    }

    // The shader relies on this: its coverage is the least of the four edges, inside positive.
    #[test]
    fn the_needle_is_convex_and_wound_one_way_at_every_angle() {
        for angle in 0..0x1000 {
            let n = needle(angle);
            let corners = [n.tip, n.bright_side, n.tail, n.dark_side];
            for i in 0..4 {
                let [a, b, c] = [corners[i], corners[(i + 1) % 4], corners[(i + 2) % 4]];
                let turn = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
                assert!(turn > 0.0, "{angle}");
            }
        }
    }

    #[test]
    fn the_dial_has_its_colours_at_its_ends() {
        let dial = dial();
        let color = |x: i32, y: i32| pixel(&dial, x + DIAL_X, y + DIAL_Y);
        assert_eq!(color(-60, 8), [0x00, 0xb5, 0x00, 255]);
        assert_eq!(color(11, -35), [0xdb, 0x00, 0x00, 255]);
        // The dark fan inside the band is half transparent.
        let [r, g, b, a] = color(-20, -10);
        assert_eq!([r, g, b], [0, 0, 0]);
        assert!(a > 0 && a < 255);
    }

    #[test]
    fn the_needle_points_along_the_band() {
        let center = needle(SLOWEST).tail;
        let slowest = needle(SLOWEST).tip;
        assert!(slowest[0] < center[0] && slowest[1] > center[1]);
        let fastest = needle(FASTEST).tip;
        assert!(fastest[0] > center[0] && fastest[1] < center[1]);
    }
}
