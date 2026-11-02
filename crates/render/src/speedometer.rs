//! The speedometer's dial and needle (`UI_DrawSpeedBG`, `UI_DrawSpeedNeedle`) and the jump meter's
//! digits (`DebugFont_DrawNumbers`). The UI has no polygons or lines, so they are rasterized like
//! the PS1 does, without its dithering.

use loader::icons::Icon;
use loader::vram::Vram;

use crate::hud_icons::half_transparent;
use crate::mesh::Atlas;

/// The part of retail's screen the dial and the needle reach, whatever the needle's angle.
pub const LEFT: i32 = 405;
pub const TOP: i32 = 145;
pub const WIDTH: usize = 105;
pub const HEIGHT: usize = 65;

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

struct Canvas(Vec<u8>);

impl Canvas {
    fn new() -> Canvas {
        Canvas(vec![0; WIDTH * HEIGHT * 4])
    }

    fn put(&mut self, x: i32, y: i32, rgba: [u8; 4]) {
        let (x, y) = (x - LEFT, y - TOP);
        if (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            self.0[(y as usize * WIDTH + x as usize) * 4..][..4].copy_from_slice(&rgba);
        }
    }

    /// A line, both its ends included.
    fn line(&mut self, from: Point, to: Point, color: [u8; 3]) {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let steps = dx.abs().max(dy.abs());
        for step in 0..=steps {
            let along = |d: i32| (d * step * 2 + steps * d.signum()).checked_div(steps * 2);
            let x = from.0 + along(dx).unwrap_or(0);
            let y = from.1 + along(dy).unwrap_or(0);
            self.put(x, y, opaque(color));
        }
    }

    /// A triangle of a colour at each corner. Like the PS1's, it leaves out its right and bottom
    /// edges, so that triangles sharing an edge neither overlap nor leave a gap.
    fn triangle(&mut self, corners: [(Point, [u8; 4]); 3]) {
        let [mut a, b, mut c] = corners;
        let edge =
            |p: Point, q: Point, x: i32, y: i32| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
        let mut area = edge(a.0, b.0, c.0.0, c.0.1);
        if area == 0 {
            return;
        }
        if area < 0 {
            std::mem::swap(&mut a, &mut c);
            area = -area;
        }
        let excludes = |p: Point, q: Point| {
            let (dx, dy) = (q.0 - p.0, q.1 - p.1);
            !(dy < 0 || (dy == 0 && dx > 0))
        };
        let (min_x, max_x) = (a.0.0.min(b.0.0).min(c.0.0), a.0.0.max(b.0.0).max(c.0.0));
        let (min_y, max_y) = (a.0.1.min(b.0.1).min(c.0.1), a.0.1.max(b.0.1).max(c.0.1));
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let weights = [
                    (edge(b.0, c.0, x, y), excludes(b.0, c.0)),
                    (edge(c.0, a.0, x, y), excludes(c.0, a.0)),
                    (edge(a.0, b.0, x, y), excludes(a.0, b.0)),
                ];
                if weights.iter().any(|&(w, out)| w < 0 || (w == 0 && out)) {
                    continue;
                }
                let channel = |n: usize| {
                    let sum = weights[0].0 * a.1[n] as i32
                        + weights[1].0 * b.1[n] as i32
                        + weights[2].0 * c.1[n] as i32;
                    (sum / area) as u8
                };
                self.put(x, y, std::array::from_fn(channel));
            }
        }
    }

    fn atlas(self) -> Atlas {
        Atlas {
            width: WIDTH,
            height: HEIGHT,
            rgba: self.0,
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
    let at = |(x, y): Point| (x + DIAL_X, y + DIAL_Y);
    let spoke = |n: usize| at(SPOKES[n]);
    let long_spoke = |n: usize| at(LONG_SPOKES[n]);
    let shadow = |(x, y): Point| (x + 1, y + 1);

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

/// The needle at `angle`: `0x1000` is a turn, 0 points down and `0x400` right.
/// It is a kite of two halves, the dark one over the bright one.
pub fn needle(angle: i32) -> Atlas {
    const CENTER: Point = (SLOT_X + 65, SLOT_Y + 41);
    const LENGTH: i32 = 60;
    const NEEDLE_WIDTH: i32 = 6;

    // The screen's pixels are taller than wide.
    let flat = |n: i32| n * 320 / 512;
    let (sin, cos) = math::sin_cos(angle as u32);
    let (sin_side, cos_side) = math::sin_cos((angle + 0x400) as u32);
    let (sx, sy) = CENTER;

    let tip = (sx + ((sin * LENGTH) >> 12), sy + flat((cos * LENGTH) >> 12));
    let tail = (
        sx - ((sin * NEEDLE_WIDTH) >> 12),
        sy - flat((cos * NEEDLE_WIDTH) >> 12),
    );
    let dark_side = (
        sx + ((sin_side * NEEDLE_WIDTH) >> 12),
        sy + flat((cos_side * (NEEDLE_WIDTH + 2)) >> 12),
    );
    let bright_side = (
        sx - ((sin_side * NEEDLE_WIDTH) >> 12),
        sy - flat((cos_side * (NEEDLE_WIDTH + 2)) >> 12),
    );

    let mut canvas = Canvas::new();
    canvas.triangle([
        (tip, opaque(WHITE)),
        (tail, opaque([156, 105, 0])),
        (bright_side, opaque([255, 255, 0])),
    ]);
    canvas.triangle([
        (tip, opaque([91, 91, 0])),
        (tail, opaque([50, 43, 1])),
        (dark_side, opaque([255, 187, 0])),
    ]);
    canvas.atlas()
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

    fn inked(atlas: &Atlas, x: i32, y: i32) -> bool {
        let at = ((y - TOP) as usize * WIDTH + (x - LEFT) as usize) * 4;
        atlas.rgba[at + 3] != 0
    }

    fn ink_on_the_border(atlas: &Atlas) -> bool {
        let (right, bottom) = (LEFT + WIDTH as i32 - 1, TOP + HEIGHT as i32 - 1);
        (LEFT..=right).any(|x| inked(atlas, x, TOP) || inked(atlas, x, bottom))
            || (TOP..=bottom).any(|y| inked(atlas, LEFT, y) || inked(atlas, right, y))
    }

    #[test]
    fn triangles_sharing_an_edge_cover_a_square_once() {
        const SIDE: i32 = 8;
        let (x0, y0) = (LEFT + 3, TOP + 3);
        let (x1, y1) = (x0 + SIDE, y0 + SIDE);
        let mut canvas = Canvas::new();
        let color = [10, 20, 30, 255];
        canvas.triangle([((x0, y0), color), ((x1, y0), color), ((x0, y1), color)]);
        canvas.triangle([((x1, y0), color), ((x0, y1), color), ((x1, y1), color)]);
        let count = canvas.0.chunks(4).filter(|p| p[3] != 0).count();
        assert_eq!(count, (SIDE * SIDE) as usize);
    }

    #[test]
    fn a_triangle_shades_between_its_corners() {
        let mut canvas = Canvas::new();
        let (x, y) = (LEFT + 2, TOP + 2);
        canvas.triangle([
            ((x, y), [0, 0, 0, 255]),
            ((x + 40, y), [200, 0, 0, 255]),
            ((x, y + 40), [0, 0, 0, 255]),
        ]);
        let at = ((2 * WIDTH) + 22) * 4;
        assert_eq!(canvas.0[at], 100);
    }

    #[test]
    fn a_line_includes_both_ends() {
        let mut canvas = Canvas::new();
        canvas.line((LEFT, TOP), (LEFT + 4, TOP + 2), WHITE);
        let atlas = canvas.atlas();
        assert!(inked(&atlas, LEFT, TOP));
        assert!(inked(&atlas, LEFT + 4, TOP + 2));
        assert_eq!(atlas.rgba.chunks(4).filter(|p| p[3] != 0).count(), 5);
    }

    #[test]
    fn the_dial_is_within_its_picture() {
        assert!(!ink_on_the_border(&dial()));
    }

    #[test]
    fn the_needle_is_within_the_picture_at_every_angle() {
        for angle in 1792..=3472 {
            assert!(!ink_on_the_border(&needle(angle)), "{angle}");
        }
    }

    #[test]
    fn the_dial_has_its_colours_at_its_ends() {
        let dial = dial();
        let color = |x: i32, y: i32| {
            let at = ((y + DIAL_Y - TOP) as usize * WIDTH + (x + DIAL_X - LEFT) as usize) * 4;
            let p = &dial.rgba[at..at + 4];
            [p[0], p[1], p[2], p[3]]
        };
        assert_eq!(color(-60, 8), [0x00, 0xb5, 0x00, 255]);
        assert_eq!(color(11, -35), [0xdb, 0x00, 0x00, 255]);
        // The dark fan inside the band is half transparent.
        let [r, g, b, a] = color(-20, -10);
        assert_eq!([r, g, b], [0, 0, 0]);
        assert!(a > 0 && a < 255);
    }

    #[test]
    fn the_needle_points_along_the_band() {
        const SLOWEST: i32 = 3472;
        const FASTEST: i32 = 1792;
        assert!(inked(&needle(SLOWEST), 479 - 25, 186 + 11));
        assert!(inked(&needle(FASTEST), 479 + 15, 186 - 23));
    }
}
