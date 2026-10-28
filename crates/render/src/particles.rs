//! Particles (`game/Particle.c`): short-lived sprites that every axis of
//! (position, rotation, scale, colour) moves with a start value, a velocity
//! and an acceleration. Emitter tables say how a new particle's axes are
//! chosen; here, for the kart's exhaust.
//!
//! These are cosmetic, so they run on the renderer's side with a private
//! random generator. (Retail draws from the game's shared generator
//! (`RngDeadCoed`), which gameplay such as the item roll also uses.)
//! Supported: the axis and function entries of emitters, and the square-wave oscillator; and the
//! rotate-and-scale sprite (`ROT_Y` with `SCALE_X`) that `Particle_RenderList` draws.

use crate::mesh::SCALE;

/// `ParticleAxisIndex`
pub mod axis {
    pub const POS_X: usize = 0;
    pub const POS_Y: usize = 1;
    pub const POS_Z: usize = 2;
    pub const ROT_Y: usize = 4;
    /// What a heat particle keeps the radii of the rings of its lens in.
    pub const HEAT_RADIUS_0: usize = 3;
    pub const HEAT_RADIUS_1: usize = 4;
    pub const HEAT_RADIUS_2: usize = 5;
    pub const SCALE_X: usize = 5;
    pub const SCALE_Y: usize = 6;
    pub const COLOR_R: usize = 7;
    pub const COLOR_G: usize = 8;
    pub const COLOR_B: usize = 9;
    /// The frame of the icon group's icons, in 1/256 icons.
    pub const ICON_FRAME: usize = 10;
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
    pub const ICON_FRAME: u16 = 0x400;
}

/// `ParticleSetColorFlags`
pub mod color_flags {
    pub const DESTROY_ON_SCALE_EXPIRE: u16 = 0x1;
    pub const DESTROY_ON_COLOR_EXPIRE: u16 = 0x2;
    pub const DESTROY_NOW: u16 = 0x8;
    pub const DRAW_MODE_MASK: u16 = 0x60;
    pub const DRAW_MODE_ADD: u16 = 0x20;
    pub const DRAW_MODE_SUBTRACT: u16 = 0x40;
    /// The icon frame starts over when it runs past the last icon (it is `COLOR_G` for the
    /// particles that have no colour axes).
    pub const ICON_WRAP: u16 = 0x100;
    /// The sprite is eight times the size of the icon's texels (half of it is `texels << 4`
    /// instead of `<< 1`).
    pub const LARGE_QUAD: u16 = 0x400;
    /// The position is relative to the driver the particle belongs to.
    pub const DRIVER_LOCAL: u16 = 0x800;
    /// Drawn as a line from where the particle was a tick ago, not as a sprite.
    pub const SPECIAL_LINE: u16 = 0x1000;
    /// A line's tail fades to black. Without it, the tail has the colour the head had.
    pub const LINE_KEEP_PREVIOUS: u16 = 0x4000;
    /// The sprite or line is blended with the picture, not drawn over it.
    pub const SEMI_TRANSPARENT: u16 = 0x80;
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
    Func {
        flags: u16,
        color_flags: u16,
        lifespan: i16,
    },
    /// How one axis starts (`AxisInit`): `[start, velocity, accel]` of the base
    /// value, and of the range that random values are drawn from.
    Axis {
        flags: u16,
        axis: usize,
        base: [i32; 3],
        random: [i32; 3],
    },
    /// An oscillator on an axis (`PARTICLE_EMITTER_FLAG_OSCILLATOR`): a wave added to it.
    /// Only the square wave is supported.
    Oscillator {
        axis: usize,
        flags: u16,
        period: u16,
        phase: i16,
        scale: u16,
        offset: i16,
        min: i16,
        max: i16,
    },
}

/// `ParticleOscillatorFlags` and `ParticleOscillatorMode`
mod oscillator_flags {
    pub const MODE_MASK: u16 = 0x7;
    pub const MODE_SQUARE: u16 = 4;
    pub const SKIP_PREVIOUS_SUBTRACT: u16 = 0x8;
    pub const APPLY_TO_VELOCITY: u16 = 0x10;
}

/// `struct ParticleOscillator`: what a particle's axis is pushed by on every tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Oscillator {
    axis: usize,
    flags: u16,
    period: u16,
    phase: i16,
    scale: u16,
    offset: i16,
    min: i16,
    max: i16,
    /// What it added last tick, which it takes back first.
    previous: i16,
}

impl Oscillator {
    /// `Particle_UpdateList`'s oscillator, for a square wave: its value at `clock` ticks.
    fn value(&self, clock: u32) -> i16 {
        let time = (clock as i32).wrapping_add(self.phase as i32);
        let wave = if (i32::from(self.period).wrapping_mul(time) >> 6) & 0x400 != 0 {
            0x1000
        } else {
            -0x1000
        };
        let value = ((wave + self.offset as i32).wrapping_mul(self.scale as i32)) >> 12;
        // (Clamped with the minimum last, as the game does.)
        let value = value.min(self.max as i32);
        value.max(self.min as i32) as i16
    }
}

/// `data.emSet_Exhaust_High`: the exhaust of a single player.
pub const EXHAUST_HIGH: [Entry; 7] = [
    Entry::Func {
        flags: 1,
        color_flags: 0x08a3,
        lifespan: 5,
    },
    Entry::Axis {
        flags: 18,
        axis: 0,
        base: [0, -250, 0],
        random: [0, 500, 0],
    },
    Entry::Axis {
        flags: 22,
        axis: 1,
        base: [0, -250, -256],
        random: [0, 500, 0],
    },
    Entry::Axis {
        flags: 18,
        axis: 2,
        base: [0, -250, 0],
        random: [0, 500, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 5,
        base: [1536, 400, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 7,
        base: [16384, -2560, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 26,
        axis: 4,
        base: [0, 256, 0],
        random: [-512, -512, 0],
    },
];

/// `data.emSet_GroundSparks`: the burst of a hard landing.
pub const GROUND_SPARKS: [Entry; 8] = [
    Entry::Func {
        flags: 1,
        color_flags: 0x58a2,
        lifespan: 8,
    },
    Entry::Axis {
        flags: 18,
        axis: 0,
        base: [0, -500, 0],
        random: [0, 1000, 0],
    },
    Entry::Axis {
        flags: 18,
        axis: 2,
        base: [0, -500, 0],
        random: [0, 1000, 0],
    },
    Entry::Axis {
        flags: 55,
        axis: 1,
        base: [2048, 1000, -300],
        random: [0, 800, 100],
    },
    Entry::Axis {
        flags: 17,
        axis: 5,
        base: [131072, 0, 0],
        random: [0, 200, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 7,
        base: [65280, -7680, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 8,
        base: [65280, -7680, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 9,
        base: [32768, -3840, 0],
        random: [0, 0, 0],
    },
];

/// The axes of the emitter sets of the ground (`emSet_DirtLR`, `emSet_GrassL`, `emSet_GrassR`,
/// `emSet_SnowLR`): a short streak thrown up from a tire, back and to the sides.
const fn ground_axes(velocity_y: i32) -> [Entry; 3] {
    [
        Entry::Axis {
            flags: 27,
            axis: 0,
            base: [-2560, -500, 0],
            random: [5120, 1000, 0],
        },
        Entry::Axis {
            flags: 27,
            axis: 2,
            base: [-2560, -500, 0],
            random: [5120, 1000, 0],
        },
        Entry::Axis {
            flags: 23,
            axis: 1,
            base: [2560, velocity_y, -300],
            random: [0, 400, 0],
        },
    ]
}

/// `data.emSet_DirtLR`: dirt, a streak that goes from orange to brown, added to the picture.
pub const TERRAIN_DIRT: [Entry; 7] = {
    let [x, z, y] = ground_axes(1500);
    [
        Entry::Func {
            flags: 1,
            color_flags: 0x58a2,
            lifespan: 10,
        },
        x,
        z,
        y,
        Entry::Axis {
            flags: 3,
            axis: 7,
            base: [65280, -2048, 0],
            random: [0, 0, 0],
        },
        Entry::Axis {
            flags: 3,
            axis: 8,
            base: [32768, -1024, 0],
            random: [0, 0, 0],
        },
        Entry::Axis {
            flags: 1,
            axis: 9,
            base: [1, 0, 0],
            random: [0, 0, 0],
        },
    ]
};

/// `data.emSet_SnowLR`: snow, a white streak.
pub const TERRAIN_SNOW: [Entry; 5] = {
    let [x, z, y] = ground_axes(2000);
    [
        Entry::Func {
            flags: 1,
            color_flags: 0x58a2,
            lifespan: 10,
        },
        x,
        z,
        y,
        Entry::Axis {
            flags: 1,
            axis: 7,
            base: [65280, 0, 0],
            random: [0, 0, 0],
        },
    ]
};

/// The grass's streaks (`emSet_GrassL` and `emSet_GrassR`): solid ones, twice as long as the
/// distance they cover in a tick, that jump up two units and down again every other tick.
const fn grass(red: i32, green: i32) -> [Entry; 8] {
    let [x, z, y] = ground_axes(2000);
    [
        Entry::Func {
            flags: 1,
            color_flags: 0x1800,
            lifespan: 10,
        },
        x,
        z,
        y,
        Entry::Oscillator {
            axis: 1,
            flags: oscillator_flags::MODE_SQUARE,
            period: 0x9600,
            phase: 0,
            scale: 0x100,
            offset: 0x1000,
            min: 1,
            max: 0x200,
        },
        Entry::Axis {
            flags: 1,
            axis: 5,
            base: [131072, 0, 0],
            random: [0, 0, 0],
        },
        Entry::Axis {
            flags: 1,
            axis: 7,
            base: [red, 0, 0],
            random: [0, 0, 0],
        },
        Entry::Axis {
            flags: 1,
            axis: 8,
            base: [green, 0, 0],
            random: [0, 0, 0],
        },
    ]
}

/// `data.emSet_GrassL`: a light green (the blue is the red).
pub const TERRAIN_GRASS_LIGHT: [Entry; 8] = grass(15360, 65280);

/// `data.emSet_GrassR`: a dark green.
pub const TERRAIN_GRASS_DARK: [Entry; 8] = grass(1, 40960);

/// `data.emSet_WallSparks`: a spark from a scrape along a wall.
pub const WALL_SPARKS: [Entry; 8] = [
    Entry::Func {
        flags: 1,
        color_flags: 0x58a2,
        lifespan: 8,
    },
    Entry::Axis {
        flags: 18,
        axis: 0,
        base: [0, -500, 0],
        random: [0, 1000, 0],
    },
    Entry::Axis {
        flags: 18,
        axis: 2,
        base: [0, -500, 0],
        random: [0, 1000, 0],
    },
    Entry::Axis {
        flags: 22,
        axis: 1,
        base: [0, 2000, -200],
        random: [0, 400, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 5,
        base: [131072, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 7,
        base: [65280, -2048, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 8,
        base: [65280, -3072, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 9,
        base: [1, 0, 0],
        random: [0, 0, 0],
    },
];

/// `emSet_Fireball`: a spark of the fire a Mystery Caves fireball trails. It starts one unit from
/// where it is placed, shrinks and fades from white through orange to red.
pub const FIREBALL: [Entry; 9] = [
    Entry::Func {
        flags: 1,
        color_flags: 0x04a1,
        lifespan: 4,
    },
    Entry::Axis {
        flags: 1,
        axis: 0,
        base: [1, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 2,
        base: [1, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 1,
        base: [1, 1, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 9,
        axis: 4,
        base: [1, 0, 0],
        random: [0x1000, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 5,
        base: [0xa00, -0xb0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 0xb,
        axis: 7,
        base: [0xff00, 0xc000, 0],
        random: [0x5f00, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 8,
        base: [0x8000, 0xe000, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 9,
        base: [0x4000, 0xf000, 0],
        random: [0, 0, 0],
    },
];

/// `emSet_fjFire`: a lick of the fire of a Tiger Temple flame jet. It starts a little off the
/// nozzle in x and z, shrinks as it goes and fades out within seven ticks.
pub const FLAME_JET_FIRE: [Entry; 7] = [
    Entry::Func {
        flags: 1,
        color_flags: 0x00a1,
        lifespan: 7,
    },
    Entry::Axis {
        flags: 0x1b,
        axis: 0,
        base: [-0x40, -0x320, 0],
        random: [0x80, 0x640, 0],
    },
    Entry::Axis {
        flags: 0x1b,
        axis: 2,
        base: [-0x40, -0x320, 0],
        random: [0x80, 0x640, 0],
    },
    Entry::Axis {
        flags: 5,
        axis: 1,
        base: [-0x40, 0, 1],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 0x1a,
        axis: 4,
        base: [0, 0x10, 0],
        random: [0x800, 0x10, 0],
    },
    Entry::Axis {
        flags: 0xb,
        axis: 5,
        base: [0x4e2, 0x100, 0],
        random: [0x80, 0, 0],
    },
    Entry::Axis {
        flags: 0xd,
        axis: 7,
        base: [0x8000, 0, -0x800],
        random: [0x5f00, 0, 0],
    },
];

/// `emSet_PlantTires`: a tire (one of the wheel icons, spinning) that a plant spits out. It
/// leaves the mouth sideways and falls; `Particle_FuncPtr_SpitTire` bounces it off the ground.
pub const PLANT_TIRES: [Entry; 7] = [
    Entry::Func {
        flags: 1,
        color_flags: 0x0121,
        lifespan: 0x50,
    },
    Entry::Axis {
        flags: 0x13,
        axis: 0,
        base: [1, -0x320, 0],
        random: [0, 0x640, 0],
    },
    Entry::Axis {
        flags: 0x13,
        axis: 2,
        base: [1, -0x320, 0],
        random: [0, 0x640, 0],
    },
    Entry::Axis {
        flags: 0x17,
        axis: 1,
        base: [1, -0x640, -0x320],
        random: [0, 0x320, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 5,
        base: [0x1000, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 0x1a,
        axis: 4,
        base: [0, 0xc0, 0],
        random: [0x400, 0x40, 0],
    },
    Entry::Axis {
        flags: 0xa,
        axis: 10,
        base: [0, 0x100, 0],
        random: [0xe00, 0, 0],
    },
];

/// `emSet_PotionShatter`: a shard of the cloud a beaker bursts into. It shoots up, and once it
/// slows (`Particle_FuncPtr_PotionShatter`) it drifts off sideways, growing and fading. Its
/// colour (axes 7 to 9) is the beaker's, set when it is born.
pub const POTION_SHATTER: [Entry; 8] = [
    Entry::Func {
        flags: 1,
        color_flags: 0x00a1,
        lifespan: 20,
    },
    Entry::Axis {
        flags: 1,
        axis: 0,
        base: [1, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 2,
        base: [1, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 0x17,
        axis: 1,
        base: [1, 0xed8, -280],
        random: [0, 400, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 5,
        base: [0x1000, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 7,
        base: [1, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 8,
        base: [0xc800, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 9,
        base: [1, 0, 0],
        random: [0, 0, 0],
    },
];

/// `emSet_fjHeat`: the shimmer of a Tiger Temple flame jet's heat. It is not drawn as a sprite
/// (`Particle_Type` 1 puts it in the heat-warp list): the three radii (`ROT_X`, `ROT_Y` and
/// `SCALE_X`) are those of the rings of the lens that `Torch_Main` warps the picture with.
pub const FLAME_JET_HEAT: [Entry; 9] = [
    Entry::Func {
        flags: 1,
        color_flags: 0x00a1,
        lifespan: 6,
    },
    Entry::Axis {
        flags: 0x1b,
        axis: 0,
        base: [-0x40, -0x320, 0],
        random: [0x80, 0x640, 0],
    },
    Entry::Axis {
        flags: 0x1b,
        axis: 2,
        base: [-0x40, -0x320, 0],
        random: [0x80, 0x640, 0],
    },
    Entry::Axis {
        flags: 5,
        axis: 1,
        base: [-0x40, 0, 1],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 0xb,
        axis: 3,
        base: [0x1200, 1, 0],
        random: [0x1000, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 4,
        base: [1, 1, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 3,
        axis: 5,
        base: [1, 1, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 7,
        base: [0x8000, 0, 0],
        random: [0, 0, 0],
    },
    Entry::Axis {
        flags: 1,
        axis: 8,
        base: [0x8000, 0, 0],
        random: [0, 0, 0],
    },
];

/// `MixRNG_Particles`' source: a stand-in for retail's `RngDeadCoed`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// How many icons its group has, which its icon frame runs through.
    pub icons: i32,
    /// A tire a plant spat, which bounces (`Particle_FuncPtr_SpitTire`).
    pub spit: Option<SpitTire>,
    /// A shard of a beaker that burst (`Particle_FuncPtr_PotionShatter`).
    pub shatter: Option<Shatter>,
    /// The wave on one of its axes, and the game's `frameTimer_Confetti` that it runs by (the
    /// tick the particle began on, counting on).
    pub oscillator: Option<Oscillator>,
    pub clock: u32,
}

/// What `Particle_FuncPtr_PotionShatter` needs: which colour fades, and its own source of
/// randomness (retail draws from the game's `MixRNG_Scramble`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shatter {
    pub green: bool,
    pub rng: Rng,
}

/// What `Particle_FuncPtr_SpitTire` needs: the height of the plant it is `owner`'s, and its own
/// source of randomness (retail draws from the game's `MixRNG_Scramble`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpitTire {
    pub plant_y: i32,
    pub rng: Rng,
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
        let mut p = Particle {
            axes: Default::default(),
            axis_flags: 0,
            color_flags: 0,
            frames_left: 0,
            previous: [0; 3],
            icons: 0,
            spit: None,
            shatter: None,
            oscillator: None,
            clock: 0,
        };
        for entry in set {
            match *entry {
                Entry::Oscillator {
                    axis,
                    flags,
                    period,
                    phase,
                    scale,
                    offset,
                    min,
                    max,
                } => {
                    p.oscillator = Some(Oscillator {
                        axis,
                        flags,
                        period,
                        phase,
                        scale,
                        offset,
                        min,
                        max,
                        previous: 0,
                    });
                    // An axis that nothing else started begins at zero.
                    p.axis_flags |= 1 << axis;
                }
                Entry::Func {
                    color_flags,
                    lifespan,
                    ..
                } => {
                    p.color_flags = color_flags;
                    p.frames_left = lifespan;
                }
                Entry::Axis {
                    flags,
                    axis,
                    base,
                    random,
                } => {
                    let mut value = |base_flag, random_flag, i: usize| {
                        let mut v = 0i32;
                        if flags & base_flag != 0 {
                            v = base[i];
                        }
                        if flags & random_flag != 0 {
                            v = v.wrapping_add(rng.mix(random[i]));
                        }
                        v
                    };
                    p.axes[axis] = Axis {
                        start: value(emitter_flags::BASE_START, emitter_flags::RANDOM_START, 0),
                        velocity: value(
                            emitter_flags::BASE_VELOCITY,
                            emitter_flags::RANDOM_VELOCITY,
                            1,
                        ) as i16,
                        accel: value(emitter_flags::BASE_ACCEL, emitter_flags::RANDOM_ACCEL, 2)
                            as i16,
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
        for (axis, pos) in p.axes.iter_mut().zip(position) {
            axis.start += pos * 0x100;
        }
        p.axes[axis::POS_Y].velocity = velocity_y;
        p
    }

    /// `RB_FlameJet_Particles`' heat: the lens that goes with `fire` (the flame particle that
    /// was just born). It starts where the flame did, 16 units higher, and moves with it; its
    /// three rings grow towards 0x4a, 0x46 and 0x44 units over the seven ticks it lives.
    pub fn flame_jet_heat(fire: &Particle, rng: &mut Rng) -> Particle {
        let mut p = Particle::new(&FLAME_JET_HEAT, rng);
        for a in 0..3 {
            p.axes[a].start += fire.axes[a].start;
        }
        p.axes[axis::POS_Y].start += 0x1000;
        p.axes[axis::POS_X].velocity = fire.axes[axis::POS_X].velocity;
        p.axes[axis::POS_Y].velocity = 0;
        p.axes[axis::POS_Z].velocity = fire.axes[axis::POS_Z].velocity;
        p.axes[axis::POS_Y].accel = fire.axes[axis::POS_Y].accel;
        let size = p.axes[axis::HEAT_RADIUS_0].start;
        p.axes[axis::HEAT_RADIUS_1].start = size - 0x400;
        p.axes[axis::HEAT_RADIUS_2].start = size - 0x600;
        for (axis, target) in [
            (axis::HEAT_RADIUS_0, 0x4a00),
            (axis::HEAT_RADIUS_1, 0x4600),
            (axis::HEAT_RADIUS_2, 0x4400),
        ] {
            p.axes[axis].velocity = ((target - p.axes[axis].start) / 7) as i16;
        }
        p
    }

    /// The radii of the three rings of a heat particle's lens, in game units (the second byte of
    /// the axes that hold them).
    pub fn heat_radii(&self) -> [i32; 3] {
        [
            axis::HEAT_RADIUS_0,
            axis::HEAT_RADIUS_1,
            axis::HEAT_RADIUS_2,
        ]
        .map(|a| self.axes[a].start >> 8 & 0xff)
    }

    /// `RB_FlameJet_Particles`' fire: at the nozzle `position` (level units), drifting `dir`
    /// (x and z, 1/256 units a tick) and flickering up and down with the game's `ticks`.
    pub fn flame_jet(position: [i32; 3], dir: [i16; 2], ticks: u32, rng: &mut Rng) -> Particle {
        let mut p = Particle::new(&FLAME_JET_FIRE, rng);
        // It starts 50 units above the nozzle.
        for (axis, pos) in p
            .axes
            .iter_mut()
            .zip([position[0], position[1] + 0x32, position[2]])
        {
            axis.start += pos * 0x100;
        }
        p.axes[axis::POS_X].velocity = dir[0];
        p.axes[axis::POS_Y].velocity = 0;
        p.axes[axis::POS_Z].velocity = dir[1];
        let sin = math::sin((ticks.wrapping_mul(0x100) + rng.mix(0x100) as u32) & 0xfff);
        p.axes[axis::POS_Y].accel = (sin >> 4) as i16;
        // Every other tick's flame turns the other way.
        if ticks & 1 != 0 {
            let spin = &mut p.axes[axis::ROT_Y];
            spin.start = -spin.start;
            spin.velocity = spin.velocity.wrapping_neg();
        }
        p
    }

    /// One of the four tires of `RB_Plant_ThTick_Eat`'s spit: at the plant's mouth, `facing` being
    /// the x and z of the third column of its matrix. `icons` is how many wheel icons there are.
    pub fn plant_tire(plant: [i32; 3], facing: [i16; 2], icons: usize, rng: &mut Rng) -> Particle {
        let mut p = Particle::new(&PLANT_TIRES, rng);
        let [fx, fz] = facing.map(i32::from);
        p.axes[axis::POS_X].start += (plant[0] + (fx * 9 >> 7)) * 0x100;
        p.axes[axis::POS_Y].start += (plant[1] + 0x20) * 0x100;
        p.axes[axis::POS_Z].start += (plant[2] + (fz * 9 >> 7)) * 0x100;
        // Each horizontal axis keeps its own launch, with more to it along where it faces.
        let launch = |rng: &mut Rng, f: i32| ((rng.mix(10) + 0x10) * f >> 12) * 0x100;
        let x = launch(rng, fx);
        let z = launch(rng, fz);
        p.axes[axis::POS_X].velocity = p.axes[axis::POS_X].velocity.wrapping_add(x as i16);
        p.axes[axis::POS_Z].velocity = p.axes[axis::POS_Z].velocity.wrapping_add(z as i16);
        p.icons = icons as i32;
        p.spit = Some(SpitTire {
            plant_y: plant[1],
            rng: Rng::new((rng.mix(0x7fff) | rng.mix(0x7fff) << 15) as u32),
        });
        p
    }

    /// One of the five shards of `RB_Explosion_InitPotion`, at the beaker's `position` (level
    /// units): green for a green beaker, red for a red one.
    pub fn potion_shatter(position: [i32; 3], green: bool, rng: &mut Rng) -> Particle {
        let mut p = Particle::new(&POTION_SHATTER, rng);
        for (axis, pos) in p.axes.iter_mut().zip(position) {
            axis.start += pos * 0x100;
        }
        let (red, grn) = if green { (1, 0xc800) } else { (0xc800, 1) };
        p.axes[axis::COLOR_R].start = red;
        p.axes[axis::COLOR_G].start = grn;
        p.axes[axis::COLOR_B].start = 1;
        p.shatter = Some(Shatter {
            green,
            rng: Rng::new((rng.mix(0x7fff) | rng.mix(0x7fff) << 15) as u32),
        });
        p
    }

    /// `Particle_FuncPtr_PotionShatter`: once it has slowed on the way up it is blown sideways
    /// and starts to grow, and from then on its colour fades.
    fn potion_shatter_step(&mut self) {
        let Some(shatter) = &mut self.shatter else {
            return;
        };
        if self.axes[axis::POS_Y].velocity < 0x578 && self.axes[axis::POS_X].velocity == 0 {
            self.axes[axis::POS_X].velocity = (shatter.rng.mix(800) - 400) as i16;
            self.axes[axis::POS_Z].velocity = (shatter.rng.mix(800) - 400) as i16;
            self.axes[axis::SCALE_X].velocity = (shatter.rng.mix(0x100) + 0x100) as i16;
        }
        if self.axes[axis::POS_X].velocity == 0 {
            return;
        }
        let fading = if shatter.green {
            axis::COLOR_G
        } else {
            axis::COLOR_R
        };
        if self.axes[fading].start > 0 {
            self.axes[fading].start -= 0x1200;
        }
    }

    /// `Particle_FuncPtr_SpitTire`: once it has fallen below the mouth it is thrown up again, in
    /// a new direction, three times. The third time it also shrinks away.
    fn spit_tire(&mut self) {
        let Some(spit) = &mut self.spit else { return };
        let mouth = spit.plant_y + 0x10;
        if self.axes[axis::POS_Y].start >> 8 >= mouth {
            return;
        }
        for a in [axis::POS_X, axis::POS_Z] {
            self.axes[a].velocity = (spit.rng.mix(0x1640) - 0xb20) as i16;
        }
        let scale = &mut self.axes[axis::SCALE_X];
        let up = match scale.start {
            0x1000 => {
                scale.start = 0xfff;
                spit.rng.mix(0x12c0) + 0x1900
            }
            0xfff => {
                scale.start = 0xffe;
                spit.rng.mix(800) + 8000
            }
            0xffe => {
                scale.velocity = -2047;
                spit.rng.mix(800) + 6000
            }
            _ => return,
        };
        self.axes[axis::POS_Y].velocity = up as i16;
        self.axes[axis::POS_Y].start = mouth << 8;
    }

    /// The icon of its group that it shows, if it steps through them.
    pub fn icon_frame(&self) -> Option<usize> {
        (self.axis_flags & axis_flags::ICON_FRAME != 0 && self.icons > 0)
            .then(|| (self.axes[axis::ICON_FRAME].start >> 8).clamp(0, self.icons - 1) as usize)
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
            p.color_flags =
                p.color_flags & !color_flags::DRAW_MODE_MASK | color_flags::DRAW_MODE_SUBTRACT;
        }
        p
    }

    /// `VehEmitter_Sparks_Ground`: one of the sparks of a landing. `back`, `side` and `push`
    /// are the kart's axes in the world (see `vehicle::sparks::Landing`).
    pub fn landing_spark(
        back: [i32; 3],
        side: [i32; 3],
        push: [i32; 3],
        rng: &mut Rng,
    ) -> Particle {
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

    /// `VehEmitter_Terrain_Ground`: a streak of what the ground is made of, from the tire at
    /// `tire` (game units from the kart); `rotate` turns its velocity into the world's axes.
    /// `timer` is the game's frame counter, which the wave of the grass runs by.
    pub fn terrain_spray(
        set: &[Entry],
        tire: [i32; 3],
        rotate: impl Fn([i32; 3]) -> [i32; 3],
        timer: u32,
        rng: &mut Rng,
    ) -> Particle {
        let mut p = Particle::new(set, rng);
        p.clock = timer;
        let velocity = rotate(std::array::from_fn(|a| p.axes[a].velocity as i32));
        for a in 0..3 {
            p.axes[a].start += tire[a] << 8;
            p.axes[a].velocity = velocity[a] as i16;
        }
        p
    }

    /// `VehEmitter_Sparks_Wall`: a spark from the tire at `tire`; `rotate` turns its velocity
    /// into the world's axes.
    pub fn wall_spark(
        tire: [i32; 3],
        rotate: impl Fn([i32; 3]) -> [i32; 3],
        rng: &mut Rng,
    ) -> Particle {
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
                if let Some(osc) = self.oscillator.as_mut().filter(|o| o.axis == n)
                    && osc.flags & oscillator_flags::MODE_MASK == oscillator_flags::MODE_SQUARE
                {
                    let to_velocity = osc.flags & oscillator_flags::APPLY_TO_VELOCITY != 0;
                    let value = osc.value(self.clock);
                    if osc.flags & oscillator_flags::SKIP_PREVIOUS_SUBTRACT == 0 {
                        if to_velocity {
                            a.velocity = a.velocity.wrapping_sub(osc.previous);
                        } else {
                            a.start = a.start.wrapping_sub(osc.previous as i32);
                        }
                    }
                    if to_velocity {
                        a.velocity = a.velocity.wrapping_add(value);
                    } else {
                        a.start = a.start.wrapping_add(value as i32);
                    }
                    osc.previous = value;
                }
            }
        }
        self.clock = self.clock.wrapping_add(1);
        self.spit_tire();
        self.potion_shatter_step();
        if self.color_flags & color_flags::DESTROY_ON_SCALE_EXPIRE != 0 {
            let small =
                |flag, axis: usize| self.axis_flags & flag != 0 && self.axes[axis].start < 1;
            if small(axis_flags::SCALE_X, axis::SCALE_X)
                || small(axis_flags::SCALE_Y, axis::SCALE_Y)
            {
                return false;
            }
        }
        if self.color_flags & color_flags::DESTROY_ON_COLOR_EXPIRE != 0 {
            let channel = |flag, axis: usize| {
                if self.axis_flags & flag != 0 && self.axes[axis].start > 0 {
                    self.axes[axis].start
                } else {
                    0
                }
            };
            let color = channel(axis_flags::COLOR_R, axis::COLOR_R)
                | channel(axis_flags::COLOR_G, axis::COLOR_G)
                | channel(axis_flags::COLOR_B, axis::COLOR_B);
            if color < 0x800 {
                return false;
            }
        }
        // The icon frame wraps round, or stops at the first or the last icon.
        if self.axis_flags & axis_flags::ICON_FRAME != 0 && self.icons > 0 {
            let limit = self.icons << 8;
            let frame = &mut self.axes[axis::ICON_FRAME].start;
            let wrap = self.color_flags & color_flags::ICON_WRAP != 0;
            if *frame < 0 {
                *frame = if wrap { *frame + limit } else { 0 };
            } else if *frame >= limit {
                *frame = if wrap { *frame - limit } else { limit - 1 };
            }
        }
        true
    }

    /// The particle `ticks` further on than it is, which may be a fraction of a tick: every
    /// axis that is set moves on with its velocity and acceleration, so that its position, size
    /// and colour are what they would be between two ticks. (Its life is not touched.)
    pub fn ahead(&self, ticks: f32) -> Particle {
        let mut p = self.clone();
        for (n, a) in p.axes.iter_mut().enumerate() {
            if self.axis_flags >> n & 1 != 0 {
                let moved =
                    a.velocity as f32 * ticks + a.accel as f32 * ticks * (ticks - 1.0) / 2.0;
                a.start = a.start.wrapping_add(moved.round() as i32);
            }
        }
        p
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

    /// Whether a line fades to black at its tail, and not the colour of its head.
    pub fn line_fades(&self) -> bool {
        self.color_flags & color_flags::LINE_KEEP_PREVIOUS != 0
    }

    /// Whether it is blended with the picture behind it.
    pub fn semi_transparent(&self) -> bool {
        self.color_flags & color_flags::SEMI_TRANSPARENT != 0
    }

    /// Where the tail of a line is, relative to its head, in 1/256 game units: the way the
    /// particle came in the last tick, `scale` times (65536 is once).
    pub fn line_tail(&self) -> [i32; 3] {
        // (Without a scale, the line is as long as the way the particle came.)
        let scale = if self.axis_flags & axis_flags::SCALE_X != 0 {
            self.axes[axis::SCALE_X].start
        } else {
            0x10000
        };
        std::array::from_fn(|a| {
            (((self.previous[a] - self.axes[a].start) as i64 * scale as i64) >> 16) as i32
        })
    }

    /// The colour of a line particle (`Particle_SetColors`): the grey of its red channel,
    /// or its own green and blue where it has them.
    pub fn rgb(&self) -> [u8; 3] {
        let channel = |axis: usize| (self.axes[axis].start.clamp(0, 0xff00) >> 8) as u8;
        if self.axis_flags & axis_flags::COLOR_R == 0 {
            return [0x80; 3];
        }
        let red = channel(axis::COLOR_R);
        let green = if self.axis_flags & axis_flags::COLOR_G != 0 {
            channel(axis::COLOR_G)
        } else {
            red
        };
        let blue = if self.axis_flags & axis_flags::COLOR_B != 0 {
            channel(axis::COLOR_B)
        } else {
            red
        };
        [red, green, blue]
    }

    /// The sprite's corners (top left, top right, bottom left, bottom right) as
    /// offsets from its centre across the screen, in quarter game units with y
    /// down, for an icon `width` by `height` texels (`Particle_RenderList`).
    /// Only the scaled sprite, turned or not, is supported.
    pub fn corners(&self, width: i32, height: i32) -> Option<[[i32; 2]; 4]> {
        if self.axis_flags & (axis_flags::ROT_X | axis_flags::SCALE_Y) != 0
            || self.axis_flags & axis_flags::SCALE_X == 0
        {
            return None;
        }
        // (Without a rotation the sprite is not turned.)
        let angle = if self.axis_flags & axis_flags::ROT_Y != 0 {
            self.axes[axis::ROT_Y].start as u32
        } else {
            0
        };
        let (sin, cos) = math::sin_cos(angle);
        let scale = self.axes[axis::SCALE_X].start;
        let (c, s) = ((cos * scale) >> 12, (sin * scale) >> 12);
        // The GTE rotation matrix: x is stretched to twice the width.
        let (r11, r12, r21, r22) = (
            (c << 1) as i16 as i32,
            (s << 1) as i16 as i32,
            -s as i16 as i32,
            c as i16 as i32,
        );
        let shift = if self.color_flags & color_flags::LARGE_QUAD != 0 {
            4
        } else {
            1
        };
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

    fn spray(set: &[Entry], timer: u32) -> Particle {
        Particle::terrain_spray(set, [-30, 10, -20], |v| v, timer, &mut Rng::new(7))
    }

    #[test]
    fn a_streak_of_dirt_starts_at_the_tire_and_lives_ten_ticks() {
        let mut p = spray(&TERRAIN_DIRT, 0);
        assert!(p.is_line() && p.line_fades() && p.semi_transparent());
        // The tire's place, and the random spread of the entry.
        assert!((p.position()[0] - (-30 * 256 - 2560)).abs() <= 5120);
        let mut ticks = 0;
        while p.update() {
            ticks += 1;
        }
        assert_eq!(ticks, 10);
    }

    #[test]
    fn snow_is_white_and_dirt_orange() {
        assert_eq!(spray(&TERRAIN_SNOW, 0).rgb(), [255, 255, 255]);
        let [r, g, b] = spray(&TERRAIN_DIRT, 0).rgb();
        assert!(r == 255 && g == 128 && b == 0, "{r} {g} {b}");
    }

    #[test]
    fn grass_is_solid_green_in_two_shades_and_flickers_up_and_down() {
        let light = spray(&TERRAIN_GRASS_LIGHT, 0);
        let dark = spray(&TERRAIN_GRASS_DARK, 0);
        assert!(light.is_line() && !light.line_fades() && !light.semi_transparent());
        assert_eq!(light.rgb(), [60, 255, 60]);
        assert_eq!(dark.rgb(), [0, 160, 0]);
        // Without the wave the height follows the velocity; with it, it is 1 or 0x200 above.
        let mut p = spray(&TERRAIN_GRASS_LIGHT, 0);
        let mut plain = p.clone();
        plain.oscillator = None;
        let mut offsets = Vec::new();
        for _ in 0..8 {
            p.update();
            plain.update();
            offsets.push(p.position()[1] - plain.position()[1]);
        }
        assert!(offsets.iter().all(|&o| o == 1 || o == 0x200), "{offsets:?}");
        assert!(
            offsets.contains(&1) && offsets.contains(&0x200),
            "{offsets:?}"
        );
    }

    fn puff(dark: bool) -> Particle {
        Particle::exhaust(
            [3685, 11466, -11466],
            [0, 1024, -1024],
            dark,
            &mut Rng::new(1),
        )
    }

    #[test]
    fn a_fireball_spark_is_a_large_quad() {
        let spark = Particle::fireball([0, 0, 0], 0, &mut Rng::new(1));
        assert_eq!(
            spark.color_flags & color_flags::LARGE_QUAD,
            color_flags::LARGE_QUAD
        );
        // Sixteen texels wide at scale 0xa00 (a third of the matrix's unit, and the x doubling).
        let [[left, _], [right, _], ..] = spark.corners(16, 16).unwrap();
        let normal = Particle::exhaust([0, 0, 0], [0, 0, 0], false, &mut Rng::new(1));
        let [[n_left, _], [n_right, _], ..] = normal.corners(16, 16).unwrap();
        // The exhaust starts at scale 1536 and a puff is not large; size per scale unit is 8 times.
        let per_scale = |l: i32, r: i32, scale: i32| (r - l) as f64 / scale as f64;
        assert!(
            (per_scale(left, right, 0xa00) / per_scale(n_left, n_right, 1536) - 8.0).abs() < 0.5
        );
    }

    #[test]
    fn a_particle_ahead_by_whole_ticks_is_the_one_that_was_updated() {
        let mut stepped = puff(false);
        let ahead = stepped.ahead(1.0);
        stepped.update();
        assert_eq!(ahead.position(), stepped.position());
        assert_eq!(ahead.tint(), stepped.tint());
        assert_eq!(
            ahead.axes[axis::SCALE_X].start,
            stepped.axes[axis::SCALE_X].start
        );
        // A half tick is between the two.
        let half = puff(false).ahead(0.5);
        let (start, end) = (puff(false), stepped);
        assert!(half.tint() < start.tint() && half.tint() > end.tint());
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
    fn a_shard_of_a_beaker_rises_then_drifts_growing_and_fades_its_colour() {
        for green in [true, false] {
            let mut p = Particle::potion_shatter([100, 50, -30], green, &mut Rng::new(3));
            assert_eq!(p.rgb(), if green { [0, 0xc8, 0] } else { [0xc8, 0, 0] });
            let mut ticks = 0;
            while p.update() {
                ticks += 1;
            }
            // It lives 20 ticks, and was blown sideways once it slowed.
            assert_eq!(ticks, 20);
            assert_ne!(p.velocity()[0], 0);
            assert!(p.axes[axis::SCALE_X].start > 0x1000);
            let [r, g, _] = p.rgb();
            assert!(if green { g } else { r } < 0x20);
            assert!(p.corners(16, 16).is_some());
        }
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
        let mut p = Particle::landing_spark(
            [0, 0, -0x1800],
            [0x1800, 0, 0],
            [0, 0, -0x200],
            &mut Rng::new(3),
        );
        assert!(p.is_line());
        let mut ticks = 0;
        while p.update() {
            ticks += 1;
            // The tail trails behind the way the spark moves, about twice its last step.
            let tail = p.line_tail();
            let step: Vec<_> = (0..3).map(|a| p.axes[a].start - p.previous[a]).collect();
            assert!(
                (0..3).all(|a| (tail[a] + 2 * step[a]).abs() <= 2 * step[a].abs() / 4 + 2),
                "{tail:?} {step:?}"
            );
        }
        assert_eq!(ticks, 8);
    }
}

#[cfg(test)]
mod spark_colour_tests {
    use super::*;

    #[test]
    fn a_spat_tire_bounces_three_times_and_is_gone() {
        let mut rng = Rng::new(7);
        let mut tire = Particle::plant_tire([100, 50, -30], [4096, 0], 14, &mut rng);
        let mouth = 50 + 0x10;
        assert_eq!(tire.icon_frame().map(|f| f < 14), Some(true));
        let (mut bounces, mut last_up) = (0, false);
        let mut ticks = 0;
        while tire.update() {
            ticks += 1;
            assert!(ticks < 0x50, "it lives at most 80 ticks");
            // A bounce puts it back at the mouth, going up.
            let up = tire.axes[axis::POS_Y].velocity > 0;
            if up && !last_up {
                bounces += 1;
                assert_eq!(tire.axes[axis::POS_Y].start >> 8, mouth);
            }
            last_up = up;
        }
        assert_eq!(bounces, 3);
        assert!(tire.icon_frame().is_some_and(|f| f < 14));
    }

    #[test]
    fn the_heat_of_a_flame_jet_grows_its_three_rings_with_the_outer_one_widest() {
        let mut rng = Rng::new(3);
        let fire = Particle::flame_jet([10, 20, 30], [100, -50], 4, &mut rng);
        let mut heat = Particle::flame_jet_heat(&fire, &mut rng);
        let [a, b, c] = heat.heat_radii();
        // Born between 18 and 34 units, the rings 4 and 6 units under it.
        assert!((0x12..0x22).contains(&a), "{a:#x}");
        assert_eq!((a - b, a - c), (4, 6));
        // It starts 16 units over the flame and goes where it goes.
        assert_eq!(
            heat.axes[axis::POS_Y].start - fire.axes[axis::POS_Y].start >> 8,
            16 + (-0x40 >> 8)
        );
        assert_eq!(heat.velocity()[0], 100);
        let mut ticks = 0;
        while heat.update() {
            ticks += 1;
        }
        // Seven ticks, and by then the rings are near 0x4a, 0x46 and 0x44.
        assert_eq!(ticks, 6);
        let [a, b, c] = heat.heat_radii();
        assert!(
            a >= 0x40 && a <= 0x4a && b >= 0x3c && c >= 0x3a,
            "{a:#x} {b:#x} {c:#x}"
        );
    }

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
