//! Particles (`game/Particle.c`): short-lived sprites that every axis of
//! (position, rotation, scale, colour) moves with a start value, a velocity
//! and an acceleration. Emitter tables say how a new particle's axes are
//! chosen; here, for the kart's exhaust.
//!
//! These are cosmetic, so they run on the renderer's side with a private
//! random generator. (Retail draws from the game's shared generator
//! (`RngDeadCoed`), which gameplay such as the item roll also uses.)
//! Supported: the axis and function entries of emitters, no oscillators; and the
//! rotate-and-scale sprite (`ROT_Y` with `SCALE_X`) that `Particle_RenderList` draws.

use crate::mesh::SCALE;

/// `ParticleAxisIndex`
pub mod axis {
    pub const POS_X: usize = 0;
    pub const POS_Y: usize = 1;
    pub const POS_Z: usize = 2;
    pub const ROT_Y: usize = 4;
    pub const SCALE_X: usize = 5;
    pub const SCALE_Y: usize = 6;
    pub const COLOR_R: usize = 7;
    pub const COLOR_G: usize = 8;
    pub const COLOR_B: usize = 9;
    pub const COUNT: usize = 11;
}

/// `ParticleAxisFlags`: one bit per initialised axis (bit `n` is axis `n`).
mod axis_flags {
    pub const ROT_X: u16 = 0x8;
    pub const ROT_Y: u16 = 0x10;
    pub const SCALE_X: u16 = 0x20;
    pub const SCALE_Y: u16 = 0x40;
    pub const COLOR_R: u16 = 0x80;
    pub const COLOR_G: u16 = 0x100;
    pub const COLOR_B: u16 = 0x200;
}

/// `ParticleSetColorFlags`
pub mod color_flags {
    pub const DESTROY_ON_SCALE_EXPIRE: u16 = 0x1;
    pub const DESTROY_ON_COLOR_EXPIRE: u16 = 0x2;
    pub const DESTROY_NOW: u16 = 0x8;
    pub const DRAW_MODE_MASK: u16 = 0x60;
    pub const DRAW_MODE_ADD: u16 = 0x20;
    pub const DRAW_MODE_SUBTRACT: u16 = 0x40;
    /// The sprite is eight times the size of the icon's texels (half of it is `texels << 4`
    /// instead of `<< 1`).
    pub const LARGE_QUAD: u16 = 0x400;
    /// The position is relative to the driver the particle belongs to.
    pub const DRIVER_LOCAL: u16 = 0x800;
    /// Drawn as a line from where the particle was a tick ago, not as a sprite.
    pub const SPECIAL_LINE: u16 = 0x1000;
}

/// `ParticleEmitterFlags`
mod emitter_flags {
    pub const BASE_START: u16 = 0x1;
    pub const BASE_VELOCITY: u16 = 0x2;
    pub const BASE_ACCEL: u16 = 0x4;
    pub const RANDOM_START: u16 = 0x8;
    pub const RANDOM_VELOCITY: u16 = 0x10;
    pub const RANDOM_ACCEL: u16 = 0x20;
}

/// One line of an emitter table (`struct ParticleEmitter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// The particle's life and how it is drawn (`FuncInit`).
    Func { flags: u16, color_flags: u16, lifespan: i16 },
    /// How one axis starts (`AxisInit`): `[start, velocity, accel]` of the base
    /// value, and of the range that random values are drawn from.
    Axis { flags: u16, axis: usize, base: [i32; 3], random: [i32; 3] },
}

/// `data.emSet_Exhaust_High`: the exhaust of a single player.
pub const EXHAUST_HIGH: [Entry; 7] = [
    Entry::Func { flags: 1, color_flags: 0x08a3, lifespan: 5 },
    Entry::Axis { flags: 18, axis: 0, base: [0, -250, 0], random: [0, 500, 0] },
    Entry::Axis { flags: 22, axis: 1, base: [0, -250, -256], random: [0, 500, 0] },
    Entry::Axis { flags: 18, axis: 2, base: [0, -250, 0], random: [0, 500, 0] },
    Entry::Axis { flags: 3, axis: 5, base: [1536, 400, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 3, axis: 7, base: [16384, -2560, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 26, axis: 4, base: [0, 256, 0], random: [-512, -512, 0] },
];

/// `data.emSet_GroundSparks`: the burst of a hard landing.
pub const GROUND_SPARKS: [Entry; 8] = [
    Entry::Func { flags: 1, color_flags: 0x58a2, lifespan: 8 },
    Entry::Axis { flags: 18, axis: 0, base: [0, -500, 0], random: [0, 1000, 0] },
    Entry::Axis { flags: 18, axis: 2, base: [0, -500, 0], random: [0, 1000, 0] },
    Entry::Axis { flags: 55, axis: 1, base: [2048, 1000, -300], random: [0, 800, 100] },
    Entry::Axis { flags: 17, axis: 5, base: [131072, 0, 0], random: [0, 200, 0] },
    Entry::Axis { flags: 3, axis: 7, base: [65280, -7680, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 3, axis: 8, base: [65280, -7680, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 3, axis: 9, base: [32768, -3840, 0], random: [0, 0, 0] },
];

/// `data.emSet_WallSparks`: a spark from a scrape along a wall.
pub const WALL_SPARKS: [Entry; 8] = [
    Entry::Func { flags: 1, color_flags: 0x58a2, lifespan: 8 },
    Entry::Axis { flags: 18, axis: 0, base: [0, -500, 0], random: [0, 1000, 0] },
    Entry::Axis { flags: 18, axis: 2, base: [0, -500, 0], random: [0, 1000, 0] },
    Entry::Axis { flags: 22, axis: 1, base: [0, 2000, -200], random: [0, 400, 0] },
    Entry::Axis { flags: 1, axis: 5, base: [131072, 0, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 3, axis: 7, base: [65280, -2048, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 3, axis: 8, base: [65280, -3072, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 1, axis: 9, base: [1, 0, 0], random: [0, 0, 0] },
];

/// `emSet_Fireball`: a spark of the fire a Mystery Caves fireball trails. It starts one unit from
/// where it is placed, shrinks and fades from white through orange to red.
pub const FIREBALL: [Entry; 9] = [
    Entry::Func { flags: 1, color_flags: 0x04a1, lifespan: 4 },
    Entry::Axis { flags: 1, axis: 0, base: [1, 0, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 1, axis: 2, base: [1, 0, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 3, axis: 1, base: [1, 1, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 9, axis: 4, base: [1, 0, 0], random: [0x1000, 0, 0] },
    Entry::Axis { flags: 3, axis: 5, base: [0xa00, -0xb0, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 0xb, axis: 7, base: [0xff00, 0xc000, 0], random: [0x5f00, 0, 0] },
    Entry::Axis { flags: 3, axis: 8, base: [0x8000, 0xe000, 0], random: [0, 0, 0] },
    Entry::Axis { flags: 3, axis: 9, base: [0x4000, 0xf000, 0], random: [0, 0, 0] },
];

/// `MixRNG_Particles`' source: a stand-in for retail's `RngDeadCoed`.
#[derive(Debug, Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed | 1)
    }

    /// `MixRNG_Particles(range)`: a value in `[0, range)`, or `[range, 0]` when negative.
    pub fn mix(&mut self, range: i32) -> i32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (((self.0 >> 8) & 0xffff) as i32 * range) >> 16
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Axis {
    pub start: i32,
    pub velocity: i16,
    pub accel: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Particle {
    pub axes: [Axis; axis::COUNT],
    pub axis_flags: u16,
    pub color_flags: u16,
    pub frames_left: i16,
    /// Where the particle was a tick ago, for the ones drawn as lines.
    pub previous: [i32; 3],
}

/// How the sprite is combined with the picture behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawMode {
    Average,
    Add,
    Subtract,
    Opaque,
}

impl Particle {
    /// `Particle_Init`.
    pub fn new(set: &[Entry], rng: &mut Rng) -> Particle {
        let mut p = Particle { axes: Default::default(), axis_flags: 0, color_flags: 0, frames_left: 0, previous: [0; 3] };
        for entry in set {
            match *entry {
                Entry::Func { color_flags, lifespan, .. } => {
                    p.color_flags = color_flags;
                    p.frames_left = lifespan;
                }
                Entry::Axis { flags, axis, base, random } => {
                    let mut value = |base_flag, random_flag, i: usize| {
                        let mut v = 0i32;
                        if flags & base_flag != 0 {
                            v = base[i] as i32;
                        }
                        if flags & random_flag != 0 {
                            v = v.wrapping_add(rng.mix(random[i] as i32));
                        }
                        v
                    };
                    p.axes[axis] = Axis {
                        start: value(emitter_flags::BASE_START, emitter_flags::RANDOM_START, 0),
                        velocity: value(emitter_flags::BASE_VELOCITY, emitter_flags::RANDOM_VELOCITY, 1) as i16,
                        accel: value(emitter_flags::BASE_ACCEL, emitter_flags::RANDOM_ACCEL, 2) as i16,
                    };
                    p.axis_flags |= 1 << axis;
                }
            }
        }
        p
    }

    /// `RB_Fireball_ThTick`'s spark: at `position` (level units) and rising at `velocity_y`
    /// (1/256 units a tick).
    pub fn fireball(position: [i32; 3], velocity_y: i16, rng: &mut Rng) -> Particle {
        let mut p = Particle::new(&FIREBALL, rng);
        for a in 0..3 {
            p.axes[a].start += position[a] * 0x100;
        }
        p.axes[axis::POS_Y].velocity = velocity_y;
        p
    }

    /// `VehEmitter_Exhaust`: a puff at `position` (1/256 game units, relative
    /// to the kart) that has been flying with `velocity` since the pipe.
    pub fn exhaust(position: [i32; 3], velocity: [i32; 3], dark: bool, rng: &mut Rng) -> Particle {
        let mut p = Particle::new(&EXHAUST_HIGH, rng);
        for a in 0..3 {
            p.axes[a].start += position[a] - velocity[a];
            p.axes[a].velocity = velocity[a] as i16;
        }
        if dark {
            p.color_flags = p.color_flags & !color_flags::DRAW_MODE_MASK | color_flags::DRAW_MODE_SUBTRACT;
        }
        p
    }

    /// `VehEmitter_Sparks_Ground`: one of the sparks of a landing. `back`, `side` and `push`
    /// are the kart's axes in the world (see `vehicle::sparks::Landing`).
    pub fn landing_spark(back: [i32; 3], side: [i32; 3], push: [i32; 3], rng: &mut Rng) -> Particle {
        let mut p = Particle::new(&GROUND_SPARKS, rng);
        let raw = rng.mix(0x1000);
        let mut r = raw >> 1 & 0x7ff;
        if raw & 1 != 0 {
            r = -r;
        }
        for a in 0..3 {
            let velocity = (p.axes[a].velocity as i32 + push[a] + ((r * side[a]) >> 12)) as i16;
            p.axes[a].velocity = velocity;
            p.axes[a].start += velocity as i32 + back[a];
        }
        p
    }

    /// `VehEmitter_Sparks_Wall`: a spark from the tire at `tire`; `rotate` turns its velocity
    /// into the world's axes.
    pub fn wall_spark(tire: [i32; 3], rotate: impl Fn([i32; 3]) -> [i32; 3], rng: &mut Rng) -> Particle {
        let mut p = Particle::new(&WALL_SPARKS, rng);
        let velocity = rotate(std::array::from_fn(|a| p.axes[a].velocity as i32));
        for a in 0..3 {
            p.axes[a].start += tire[a];
            p.axes[a].velocity = velocity[a] as i16;
        }
        p
    }

    /// `Particle_UpdateList` for one particle, once per game tick. False once it is gone.
    pub fn update(&mut self) -> bool {
        self.frames_left -= 1;
        if self.frames_left == -1 || self.color_flags & color_flags::DESTROY_NOW != 0 {
            return false;
        }
        if self.color_flags & color_flags::SPECIAL_LINE != 0 {
            self.previous = self.position();
        }
        for (n, a) in self.axes.iter_mut().enumerate() {
            if self.axis_flags >> n & 1 != 0 {
                a.start = a.start.wrapping_add(a.velocity as i32);
                a.velocity = a.velocity.wrapping_add(a.accel);
            }
        }
        if self.color_flags & color_flags::DESTROY_ON_SCALE_EXPIRE != 0 {
            let small = |flag, axis: usize| self.axis_flags & flag != 0 && self.axes[axis].start < 1;
            if small(axis_flags::SCALE_X, axis::SCALE_X) || small(axis_flags::SCALE_Y, axis::SCALE_Y) {
                return false;
            }
        }
        if self.color_flags & color_flags::DESTROY_ON_COLOR_EXPIRE != 0 {
            let channel = |flag, axis: usize| {
                if self.axis_flags & flag != 0 && self.axes[axis].start > 0 { self.axes[axis].start } else { 0 }
            };
            let color = channel(axis_flags::COLOR_R, axis::COLOR_R)
                | channel(axis_flags::COLOR_G, axis::COLOR_G)
                | channel(axis_flags::COLOR_B, axis::COLOR_B);
            if color < 0x800 {
                return false;
            }
        }
        true
    }

    /// Where the particle is, in 1/256 game units, and how it moves per tick.
    pub fn position(&self) -> [i32; 3] {
        std::array::from_fn(|a| self.axes[a].start)
    }

    pub fn velocity(&self) -> [i32; 3] {
        std::array::from_fn(|a| self.axes[a].velocity as i32)
    }

    pub fn draw_mode(&self) -> DrawMode {
        match self.color_flags & color_flags::DRAW_MODE_MASK {
            0 => DrawMode::Average,
            color_flags::DRAW_MODE_ADD => DrawMode::Add,
            color_flags::DRAW_MODE_SUBTRACT => DrawMode::Subtract,
            _ => DrawMode::Opaque,
        }
    }

    /// `Particle_SetColors`: the grey the sprite is tinted with, 0x80 being its own colours.
    /// (Particles with their own green and blue are not supported.)
    pub fn tint(&self) -> u8 {
        if self.axis_flags & axis_flags::COLOR_R == 0 {
            return 0x80;
        }
        (self.axes[axis::COLOR_R].start.clamp(0, 0xff00) >> 8) as u8
    }

    /// Whether the particle is drawn as a line, from its head to its tail.
    pub fn is_line(&self) -> bool {
        self.color_flags & color_flags::SPECIAL_LINE != 0
    }

    /// Where the tail of a line is, relative to its head, in 1/256 game units: the way the
    /// particle came in the last tick, `scale` times (65536 is once).
    pub fn line_tail(&self) -> [i32; 3] {
        let scale = if self.axis_flags & axis_flags::SCALE_X != 0 { self.axes[axis::SCALE_X].start } else { 0 };
        std::array::from_fn(|a| (((self.previous[a] - self.axes[a].start) as i64 * scale as i64) >> 16) as i32)
    }

    /// The colour of a line particle (`Particle_SetColors`): the grey of its red channel,
    /// or its own green and blue where it has them.
    pub fn rgb(&self) -> [u8; 3] {
        let channel = |axis: usize| (self.axes[axis].start.clamp(0, 0xff00) >> 8) as u8;
        if self.axis_flags & axis_flags::COLOR_R == 0 {
            return [0x80; 3];
        }
        let red = channel(axis::COLOR_R);
        let green = if self.axis_flags & axis_flags::COLOR_G != 0 { channel(axis::COLOR_G) } else { red };
        let blue = if self.axis_flags & axis_flags::COLOR_B != 0 { channel(axis::COLOR_B) } else { red };
        [red, green, blue]
    }

    /// The sprite's corners (top left, top right, bottom left, bottom right) as
    /// offsets from its centre across the screen, in quarter game units with y
    /// down, for an icon `width` by `height` texels (`Particle_RenderList`).
    /// Only the rotated and scaled sprite is supported.
    pub fn corners(&self, width: i32, height: i32) -> Option<[[i32; 2]; 4]> {
        let wanted = axis_flags::ROT_Y | axis_flags::SCALE_X;
        if self.axis_flags & (axis_flags::ROT_X | axis_flags::ROT_Y | axis_flags::SCALE_X | axis_flags::SCALE_Y) != wanted {
            return None;
        }
        let (sin, cos) = math::sin_cos(self.axes[axis::ROT_Y].start as u32);
        let scale = self.axes[axis::SCALE_X].start;
        let (c, s) = ((cos * scale) >> 12, (sin * scale) >> 12);
        // The GTE rotation matrix: x is stretched to twice the width.
        let (r11, r12, r21, r22) = ((c << 1) as i16 as i32, (s << 1) as i16 as i32, -s as i16 as i32, c as i16 as i32);
        let shift = if self.color_flags & color_flags::LARGE_QUAD != 0 { 4 } else { 1 };
        let (half_width, half_height) = (width << shift, height << shift);
        Some([(-1, -1), (1, -1), (-1, 1), (1, 1)].map(|(sx, sy)| {
            let (x, y) = (sx * half_width, sy * half_height);
            [(r11 * x + r12 * y) >> 12, (r21 * x + r22 * y) >> 12]
        }))
    }
}

/// Quarter game units (retail's camera-relative space for particles) in world units.
pub fn quarter_units_to_world(v: f32) -> f32 {
    v / 4.0 * SCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn puff(dark: bool) -> Particle {
        Particle::exhaust([3685, 11466, -11466], [0, 1024, -1024], dark, &mut Rng::new(1))
    }

    #[test]
    fn a_fireball_spark_is_a_large_quad() {
        let spark = Particle::fireball([0, 0, 0], 0, &mut Rng::new(1));
        assert_eq!(spark.color_flags & color_flags::LARGE_QUAD, color_flags::LARGE_QUAD);
        // Sixteen texels wide at scale 0xa00 (a third of the matrix's unit, and the x doubling).
        let [[left, _], [right, _], ..] = spark.corners(16, 16).unwrap();
        let normal = Particle::exhaust([0, 0, 0], [0, 0, 0], false, &mut Rng::new(1));
        let [[n_left, _], [n_right, _], ..] = normal.corners(16, 16).unwrap();
        // The exhaust starts at scale 1536 and a puff is not large; size per scale unit is 8 times.
        let per_scale = |l: i32, r: i32, scale: i32| (r - l) as f64 / scale as f64;
        assert!((per_scale(left, right, 0xa00) / per_scale(n_left, n_right, 1536) - 8.0).abs() < 0.5);
    }

    #[test]
    fn a_new_puff_reaches_its_pipe_with_its_first_step() {
        let mut p = puff(false);
        assert_eq!(p.position(), [3685, 11466 - 1024, -11466 + 1024]);
        assert!(p.update());
        assert_eq!(p.position(), [3685, 11466, -11466]);
        // Gravity: the vertical speed drops by 256 per tick.
        assert_eq!(p.velocity()[1], 1024 - 256);
        assert_eq!(p.draw_mode(), DrawMode::Add);
        assert_eq!(puff(true).draw_mode(), DrawMode::Subtract);
    }

    #[test]
    fn a_puff_grows_fades_and_is_gone_after_five_ticks() {
        let mut p = puff(false);
        let mut tints = Vec::new();
        let mut scales = Vec::new();
        while p.update() {
            tints.push(p.tint());
            scales.push(p.axes[axis::SCALE_X].start);
        }
        assert_eq!(tints, [54, 44, 34, 24, 14]);
        assert_eq!(scales, [1936, 2336, 2736, 3136, 3536]);
    }

    #[test]
    fn random_values_stay_in_range() {
        let mut rng = Rng::new(7);
        for _ in 0..1000 {
            let p = Particle::new(&EXHAUST_HIGH, &mut rng);
            // Start in [-512, 0], speed 256 plus another such value.
            assert!((-512..=0).contains(&p.axes[axis::ROT_Y].start));
            assert!((-256..=256).contains(&(p.axes[axis::ROT_Y].velocity as i32)));
        }
    }

    #[test]
    fn the_sprite_is_a_flat_wide_rectangle_when_not_rotated() {
        let mut p = puff(false);
        p.axes[axis::ROT_Y].start = 0;
        let [tl, tr, bl, br] = p.corners(32, 32).unwrap();
        // 2*scale/4096 * 2 * 32 wide on each side; half as tall.
        assert_eq!((tl[0], tr[0], bl[0], br[0]), (-tr[0], tr[0], -tr[0], tr[0]));
        assert_eq!(tr[0], 2 * 2 * 32 * p.axes[axis::SCALE_X].start / 4096);
        assert_eq!(tl[1], -tr[0] / 2);
        assert_eq!(bl[1], -tl[1]);
    }

    #[test]
    fn a_spark_is_a_streak_that_fades_out_in_eight_ticks() {
        let mut p = Particle::landing_spark([0, 0, -0x1800], [0x1800, 0, 0], [0, 0, -0x200], &mut Rng::new(3));
        assert!(p.is_line());
        let mut ticks = 0;
        while p.update() {
            ticks += 1;
            // The tail trails behind the way the spark moves, about twice its last step.
            let tail = p.line_tail();
            let step: Vec<_> = (0..3).map(|a| p.axes[a].start - p.previous[a]).collect();
            assert!((0..3).all(|a| (tail[a] + 2 * step[a]).abs() <= 2 * step[a].abs() / 4 + 2), "{tail:?} {step:?}");
        }
        assert_eq!(ticks, 8);
    }
}

#[cfg(test)]
mod spark_colour_tests {
    use super::*;

    #[test]
    fn sparks_are_yellow() {
        let mut p = Particle::landing_spark([0; 3], [0; 3], [0; 3], &mut Rng::new(3));
        assert_eq!(p.rgb(), [255, 255, 128]);
        p.update();
        assert_eq!(p.rgb(), [225, 225, 113]);
        let w = Particle::wall_spark([0; 3], |v| v, &mut Rng::new(3));
        assert_eq!(w.rgb(), [255, 255, 0]);
    }
}
