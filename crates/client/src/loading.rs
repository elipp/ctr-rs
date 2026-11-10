//! Loading, and the loading screen. Building a scene (`scene.rs`) is not the end of it: the HUD's
//! pictures and cameras are set up by commands over the next frames, and a second change before
//! they land despawns what they are about to touch. So after each change the game clock stops,
//! and nothing goes on (the keys do not change the level again, an online race does not say it is
//! ready) until the scene is whole ([`Loading`]).
//!
//! The scene is whole when a few frames have passed and what a race needs is there: our kart
//! and the chase camera. If that does not come in [`GIVE_UP_FRAMES`] the game goes on anyway
//! and says what was missing in the log, rather than hold it up for good.
//!
//! A load takes no time to speak of, so it is not covered: the screen is only up between the
//! races of a cup, and for `/debug loading-screen`. It is retail's (`RaceFlag_DrawSelf`): the grey
//! checkered flag waving over all of it, and "LOADING..." flying in over it a letter at a time
//! (`RaceFlag_DrawLoadingString`). When it is done the flag speeds off to the left, and the
//! letters after it, with the game running under them. Retail draws the flag as quads through the GTE; here the wave is retail's, worked
//! out on the CPU column by column, and the shader (`loading.wgsl`) finds the flag under each
//! pixel. The flag goes on past retail's edges, for windows wider than its screen.
//!
//! Between the races of a cup the flag comes in first, as retail's does
//! (`RaceFlag_BeginTransition`): the next race's start brings it in over the race before, the
//! level loads once it covers the window ([`entering`]), and it stays [`CUP_MIN_SECONDS`] at
//! least, and until everyone is ready to start.
//!
//! `/debug loading-screen` puts the screen up for a while on its own, over the game as it goes on.

use std::f32::consts::TAU;

use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::window::PrimaryWindow;
use render::font::{Font, Gradient};

use crate::font::{CtrText, Fonts, TexelSize};
use crate::{drive, scene::KartScene};

/// The frames the scene always gets to settle, whatever else is there.
const MIN_FRAMES: u32 = 3;

/// The frames after which the screen goes away even if the scene is not whole (3 seconds at 60).
const GIVE_UP_FRAMES: u32 = 180;

/// The longest a frame counts for (in ms): the frame after a level loaded stood still for the
/// load, and the flag should not jump for it.
const MAX_FRAME_MS: f32 = 50.0;

/// What retail's animations count in: `elapsedTimeMS >> 5` is a frame.
const FRAME_MS: f32 = 32.0;

/// `matrixTitleFlag`: how the flag is turned to the screen (4096 is 1).
const FLAG_MATRIX: [[i32; 3]; 3] = [
    [-0xe93, -0x2aa, -0x607],
    [0xea, -0x8cc, 0x1a9],
    [-0x662, 0x214, 0xe84],
];

/// `checkerFlagVariables`: the wave's angle of amplitude, its amplitude, its angle of speed, its
/// speed, and its phase (32 to a unit of angle).
const WAVE: [f32; 5] = [0.0, 150.0, 0.0, 180.0, 0.0];

/// The columns of the flag: retail's 35, and more on either side for a wide window. The first is
/// column [`FIRST_COLUMN`] of retail's (whose first is 0). `loading.wgsl` has the same.
const COLUMNS: usize = 88;
const FIRST_COLUMN: i32 = -40;

/// Retail's flag starts this far across (in its space), and is 100 per column.
const FLAG_X: f32 = -3006.0;
const COLUMN_WIDTH: f32 = 100.0;

/// The flag's position as it slides off: it starts at speed 300, which goes up by 10 a frame to
/// 1000, and it is off when its first column is [`OFF_SCREEN`] across from where it started.
const LEAVE_SPEED: f32 = 300.0;
const LEAVE_ACCEL: f32 = 10.0;
const LEAVE_MAX_SPEED: f32 = 1000.0;
const OFF_SCREEN: f32 = 5000.0;

/// Where the flag comes in from: retail's 5000, and the columns past its last that this one has.
/// It goes an eighth of the way that is left a frame, and is on the screen within 8 of it.
const ENTER_FROM: f32 = 5000.0 + COLUMN_WIDTH * (FIRST_COLUMN + COLUMNS as i32 - 1 - 34) as f32;
const ENTER_SHARE: f32 = 8.0;
const ENTER_SNAP: f32 = 8.0;
/// Close enough in that the flag covers the window, and the level can load under it: its last
/// column is past the window's left edge.
const COVERED_AT: f32 = 500.0;

/// The least time the flag stays between the races of a cup once it covers the window.
const CUP_MIN_SECONDS: f32 = 2.0;

/// The text, in the large font and the menus' orange, with its top at line 108.
const TEXT: &str = "LOADING...";
const TEXT_Y: f32 = 108.0;

/// Where a letter flies in from (across), where it sits (the centre), and how fast it goes (a
/// frame).
const LETTER_FROM: f32 = 572.0;
const LETTER_AT: f32 = 256.0;
const LETTER_SPEED: f32 = 60.0;
/// The frames a letter is behind the one before it, its frame at which it flies off, and the
/// frame after which it is gone.
const LETTER_STEP: f32 = 4.0;
const LETTER_LEAVES: f32 = 75.0;
const LETTER_GONE: f32 = 80.0;

/// How fast the text slides off once the scene is whole (a frame), and how far it goes.
const TEXT_SLIDE: f32 = 40.0;
const TEXT_SLIDE_END: f32 = -1000.0;

/// A scene is being settled: present from the change until [`finish`].
#[derive(Resource)]
pub struct Loading {
    frames: u32,
}

/// What the shader is given (`loading.wgsl`'s `Flag`).
#[derive(ShaderType, Debug, Clone)]
struct FlagUniform {
    /// Each column's depth and light.
    columns: [Vec4; COLUMNS],
    unproject: Mat4,
    /// Where the first column of retail's is across, the ripple's angle, the column of the
    /// trailing edge, and 1 while the flag covers what it misses.
    flag: Vec4,
    /// The window's size, and one of retail's pixels in it.
    screen: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct FlagMaterial {
    #[uniform(0)]
    flag: FlagUniform,
}

impl UiMaterial for FlagMaterial {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_PKG_NAME"), "/loading.wgsl").into()
    }
}

/// The screen that covers the window, and the state of its flag and text.
#[derive(Component)]
struct LoadingScreen {
    material: Handle<FlagMaterial>,
    /// `checkerFlagVariables`, as they go.
    wave: [f32; 5],
    /// `RaceFlag_ElapsedTime`: the ripple's clock.
    elapsed: f32,
    /// `RaceFlag_Position`: 0 on the screen, more as it comes in, less as it slides off.
    position: f32,
    speed: f32,
    /// The scene is whole: the flag and the text go.
    leaving: bool,
    /// What sends it off.
    hold: Hold,
    /// `RaceFlag_LoadingTextAnimFrame`: the first letter's frame, less than 0 for no text.
    text_frame: f32,
    /// `RaceFlag_Transition`: the text slid to the left, once the scene is whole.
    text_slide: f32,
    /// Where each letter is from the left of the line, and how wide the line is (in texels).
    letters: Vec<f32>,
    width: f32,
}

/// What a screen waits for before it goes.
#[derive(Clone, Copy, PartialEq)]
enum Hold {
    /// `/debug loading-screen`: these seconds, whatever loads.
    For(f32),
    /// Between the races of a cup: these seconds once it covers the window, the load, and the
    /// next race's go (everyone is ready).
    Cup(f32),
}

/// The letter of the text at `.0`.
#[derive(Component)]
struct Letter(usize);

/// The sine of retail's angles (4096 to a turn), as big as theirs (4096).
fn sine(angle: f32) -> f32 {
    (angle * TAU / 4096.0).sin() * 4096.0
}

/// `matrixTitleFlag`, from the flag's space to the screen's.
fn rotation() -> Mat3 {
    let rows = FLAG_MATRIX.map(|row| row.map(|m| m as f32 / 4096.0));
    Mat3::from_cols_array_2d(&rows).transpose()
}

/// One step of a wave (`RaceFlag_StepWave`): its phase on by its speed `times` over, and its amplitude and
/// speed changing each time the phase goes round. The height and the light where it is, both 0
/// to 8190.
fn step_wave(wave: &mut [f32; 5], times: f32) -> (f32, f32) {
    wave[4] += wave[3] * times;
    let mut phase = wave[4] / 32.0;
    if phase > 4095.0 {
        wave[4] %= 4096.0 * 32.0;
        phase = wave[4] / 32.0;
        wave[0] += 512.0;
        wave[1] = ((sine(wave[0]) + 4095.0) * 32.0 / 8192.0).floor() + 150.0;
        wave[2] += 200.0;
        wave[3] = ((sine(wave[2]) + 4095.0) * 64.0 / 8192.0).floor() + 180.0;
    }
    (sine(phase) + 4095.0, sine(phase + 3200.0) + 4095.0)
}

/// Where a letter `frame`s into its flight is across (`RaceFlag_DrawLoadingString`), if it is on
/// the screen: in from the right, at the centre, and off to the left.
fn letter_x(frame: f32) -> Option<f32> {
    if !(0.0..LETTER_GONE).contains(&frame) {
        None
    } else if frame < LETTER_LEAVES {
        Some((LETTER_FROM - LETTER_SPEED * frame).max(LETTER_AT))
    } else {
        Some(LETTER_AT - LETTER_SPEED * (frame - LETTER_LEAVES))
    }
}

/// The window's logical size, and the size of one of retail's pixels in it.
#[derive(Clone, Copy)]
struct Screen {
    size: Vec2,
    pixel: Vec2,
}

impl Screen {
    /// Where a point of retail's screen is on the window: their centres are each other's.
    fn to_window(self, at: Vec2) -> Vec2 {
        (at - Vec2::new(256.0, 120.0)) * self.pixel + self.size / 2.0
    }

    /// The columns before retail's first that the flag needs to go past the right of the window
    /// (as it would be at rest). The far end is on the screen of a very wide window, which the
    /// flag never reaches: it stops at [`FIRST_COLUMN`].
    fn trailing_columns(self) -> i32 {
        let half = self.size / 2.0 / self.pixel;
        let (right, top, bottom) = (256.0 + half.x, 120.0 - half.y, 120.0 + half.y);
        let r = rotation();
        // On the screen before the right of the window: a column a little nearer than the
        // middle of the flag's depth still is not.
        let in_sight = |k: i32| {
            (-32..=70).step_by(2).any(|p| {
                let x = FLAG_X - COLUMN_WIDTH * k as f32;
                let v = r * Vec3::new(x, -910.0 + 94.0 * p as f32, 720.0);
                if v.z <= 1.0 {
                    return false;
                }
                let s = Vec2::new(256.0, 120.0) + 256.0 * v.truncate() / v.z;
                (top..=bottom).contains(&s.y) && s.x <= right
            })
        };
        let k = (0..-FIRST_COLUMN).find(|&k| !in_sight(k)).unwrap_or(-FIRST_COLUMN);
        (k + 2).min(-FIRST_COLUMN)
    }
}

fn screen(window: &Window, texel: &TexelSize) -> Screen {
    Screen {
        size: window.size(),
        pixel: texel.0,
    }
}

impl LoadingScreen {
    fn new(material: Handle<FlagMaterial>, letters: Vec<f32>, width: f32) -> LoadingScreen {
        LoadingScreen {
            material,
            wave: WAVE,
            elapsed: 0.0,
            position: 0.0,
            speed: LEAVE_SPEED,
            leaving: false,
            hold: Hold::For(0.0),
            // (Retail starts the text at the stages of a load that take long.)
            text_frame: 0.0,
            text_slide: 0.0,
            letters,
            width,
        }
    }

    /// The flag on by a frame of `ms` (`RaceFlag_GetOT`, `RaceFlag_DrawSelf`), and what the
    /// shader needs to draw it. None once it is off the screen.
    fn step_flag(&mut self, ms: f32, screen: Screen) -> Option<FlagUniform> {
        let trailing = screen.trailing_columns();
        if !self.leaving && self.position > 0.0 {
            // (`RaceFlag_GetOT`'s steps are whole units, and at least one.)
            if self.position < ENTER_SNAP {
                self.position = 0.0;
            } else {
                let step = (self.position / ENTER_SHARE * ms / FRAME_MS).max(ms / FRAME_MS);
                self.position = (self.position - step).max(0.0);
            }
        }
        if self.leaving {
            if self.speed < LEAVE_MAX_SPEED {
                self.speed += ms * LEAVE_ACCEL / FRAME_MS;
            }
            // Off when the trailing edge is where retail's first column would be.
            if self.position <= -(OFF_SCREEN + COLUMN_WIDTH * trailing as f32) {
                return None;
            }
            self.position -= self.speed / 4.0 * ms / FRAME_MS;
        }

        // The first column goes on with the time; each after it is a fixed step on from the one
        // before.
        let mut columns = [Vec4::ZERO; COLUMNS];
        let first = step_wave(&mut self.wave, ms);
        let mut wave = self.wave;
        for (i, column) in columns.iter_mut().enumerate() {
            let (height, light) = if i == 0 {
                first
            } else {
                step_wave(&mut wave, 64.0)
            };
            let depth = height * wave[1] / 8192.0 + 640.0;
            *column = Vec4::new(depth, light, 0.0, 0.0);
        }
        let ripple = self.elapsed / 32.0;
        self.elapsed = (self.elapsed + ms * 100.0) % (4096.0 * 32.0);

        Some(FlagUniform {
            columns,
            unproject: Mat4::from_mat3(rotation().inverse()),
            flag: Vec4::new(
                FLAG_X - self.position,
                ripple,
                -trailing as f32,
                if self.leaving || self.position > 0.0 {
                    0.0
                } else {
                    1.0
                },
            ),
            screen: Vec4::new(screen.size.x, screen.size.y, screen.pixel.x, screen.pixel.y),
        })
    }

    /// The text on by a frame of `ms` (`RaceFlag_DrawLoadingString`): where each letter is on
    /// retail's screen, if it is on it.
    fn step_text(&mut self, ms: f32) -> Vec<Option<Vec2>> {
        // A flag that came in starts the text once it covers the window.
        if self.text_frame < 0.0 && !self.leaving && self.position > 0.0 && self.position <= COVERED_AT {
            self.text_frame = 0.0;
        }
        if self.text_frame < 0.0 {
            return vec![None; self.letters.len()];
        }
        if !self.leaving {
            self.text_slide = 0.0;
        } else if self.text_slide > TEXT_SLIDE_END {
            self.text_slide -= TEXT_SLIDE * ms / FRAME_MS;
        }
        let left = self.text_slide - (self.width / 2.0).floor();
        let at = self
            .letters
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let frame = self.text_frame - LETTER_STEP * i as f32;
                letter_x(frame).map(|lx| Vec2::new(left + x + lx, TEXT_Y))
            })
            .collect();
        // Round again from the first letter once the last is gone, as long as the load goes on.
        let last = self.text_frame - LETTER_STEP * self.letters.len() as f32;
        if last < LETTER_GONE {
            self.text_frame += ms / FRAME_MS;
        } else {
            self.text_frame = if self.leaving { -1.0 } else { 0.0 };
        }
        at
    }
}

/// Stops the game clock until the scene that was just built has settled. The flag of a cup, if it
/// is up, waits for it (see [`Hold::Cup`]).
pub fn begin(world: &mut World) {
    if world.contains_resource::<Loading>() {
        return;
    }
    world.insert_resource(Loading { frames: 0 });
    world.resource_mut::<Time<Virtual>>().pause();
}

/// Whether the flag is coming in before the next race of a cup, and does not cover the window
/// yet.
pub fn entering(world: &mut World) -> bool {
    world
        .query::<&LoadingScreen>()
        .iter(world)
        .any(|s| !s.leaving && s.position > COVERED_AT)
}

/// Brings the flag in over the window before the next race of a cup.
pub fn bring_in(world: &mut World) {
    spawn_screen(world, Hold::Cup(CUP_MIN_SECONDS), ENTER_FROM);
}

/// Puts the screen up over the window (from `position`, 0 for on it), until `hold` sends it off.
fn spawn_screen(world: &mut World, hold: Hold, position: f32) {
    // A screen still up, or sliding off from the scene before, is covered by this one.
    let leaving: Vec<Entity> = world
        .query_filtered::<Entity, With<LoadingScreen>>()
        .iter(world)
        .collect();
    for screen in leaving {
        world.entity_mut(screen).despawn();
    }

    let line = world
        .get_resource::<Fonts>()
        .map(|fonts| fonts.atlas(Font::Large).layout(TEXT));
    let (letters, width) = match &line {
        Some(line) => (
            line.glyphs.iter().map(|g| g.x as f32).collect(),
            line.width as f32,
        ),
        None => (Vec::new(), 0.0),
    };
    let texel = *world.resource::<TexelSize>();
    let at = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .map(|window| screen(window, &texel))
        .ok();
    let mut flag = LoadingScreen::new(Handle::default(), letters, width);
    flag.hold = hold;
    flag.position = position;
    if position > 0.0 {
        // (The text waits for the flag to cover the window.)
        flag.text_frame = -1.0;
    }
    let uniform = flag
        .step_flag(
            0.0,
            at.unwrap_or(Screen {
                size: Vec2::new(1024.0, 720.0),
                pixel: crate::hud::PIXEL,
            }),
        )
        .expect("the flag is on the screen");
    flag.material = world
        .resource_mut::<Assets<FlagMaterial>>()
        .add(FlagMaterial { flag: uniform });
    let material = MaterialNode(flag.material.clone());
    let screen = world
        .spawn((
            flag,
            material,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            GlobalZIndex(i32::MAX - 3),
            Pickable::IGNORE,
        ))
        .id();
    // (The glyphs of the line are its letters, one to a character.)
    for (i, c) in TEXT.chars().enumerate().take(line.map_or(0, |l| l.glyphs.len())) {
        world.spawn((
            Letter(i),
            CtrText::new(c.to_string(), Font::Large, Gradient::ORANGE),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
            ChildOf(screen),
        ));
    }
}

/// What the scene lacks before it is whole, if anything.
fn missing(
    driving: bool,
    karts: &Query<(), With<KartScene>>,
    chase: &Query<(), With<drive::ChaseCamera>>,
) -> Option<&'static str> {
    // (The viewer has neither.)
    if !driving {
        return None;
    }
    if karts.is_empty() {
        Some("our kart")
    } else if chase.is_empty() {
        Some("the chase camera")
    } else {
        None
    }
}

/// Counts the frames of the load, and lets the game go on when the scene is whole.
fn finish(
    mut loading: ResMut<Loading>,
    driving: Option<Res<drive::Drive>>,
    karts: Query<(), With<KartScene>>,
    chase: Query<(), With<drive::ChaseCamera>>,
    mut time: ResMut<Time<Virtual>>,
    mut commands: Commands,
) {
    loading.frames += 1;
    let lacks = missing(driving.is_some(), &karts, &chase);
    if loading.frames < MIN_FRAMES || (lacks.is_some() && loading.frames < GIVE_UP_FRAMES) {
        return;
    }
    if let Some(what) = lacks {
        warn!(
            "the scene is not whole after {} frames: no {what}",
            loading.frames
        );
    }
    time.unpause();
    commands.remove_resource::<Loading>();
}

/// Waves the flag and flies the letters, and takes the screen away once the flag is off it.
fn animate(
    real: Res<Time<Real>>,
    texel: Res<TexelSize>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut screens: Query<(Entity, &mut LoadingScreen)>,
    mut letters: Query<(&Letter, &ChildOf, &mut Node, &mut Visibility)>,
    mut materials: ResMut<Assets<FlagMaterial>>,
    loading: Option<Res<Loading>>,
    lobby: Option<Res<crate::lobby::Lobby>>,
    mut commands: Commands,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let at = screen(window, &texel);
    let ms = (real.delta_secs() * 1000.0).min(MAX_FRAME_MS);
    for (entity, mut flag) in &mut screens {
        let covers = flag.position <= COVERED_AT;
        match &mut flag.hold {
            Hold::For(left) => {
                *left -= real.delta_secs();
                if *left <= 0.0 {
                    flag.leaving = true;
                }
            }
            Hold::Cup(left) => {
                if covers {
                    *left -= real.delta_secs();
                }
                // (A race that was stopped meanwhile has no go to wait for.)
                let ready = lobby
                    .as_ref()
                    .and_then(|lobby| lobby.online.as_ref())
                    .is_none_or(|online| online.go);
                if *left <= 0.0 && loading.is_none() && ready {
                    flag.leaving = true;
                }
            }
        }
        let text = flag.step_text(ms);
        let Some(uniform) = flag.step_flag(ms, at) else {
            commands.entity(entity).despawn();
            continue;
        };
        if let Some(mut material) = materials.get_mut(&flag.material) {
            material.flag = uniform;
        }
        for (letter, parent, mut node, mut visibility) in &mut letters {
            if parent.parent() != entity {
                continue;
            }
            match text.get(letter.0).copied().flatten() {
                Some(spot) => {
                    let spot = at.to_window(spot);
                    node.left = Val::Px(spot.x);
                    node.top = Val::Px(spot.y);
                    visibility.set_if_neq(Visibility::Inherited);
                }
                None => {
                    visibility.set_if_neq(Visibility::Hidden);
                }
            }
        }
    }
}

/// `/debug loading-screen <duration>`.
fn debug_screen(mut actions: MessageReader<crate::command::Action>, mut commands: Commands) {
    for action in actions.read() {
        if let crate::command::Action::DebugLoadingScreen(time) = action {
            let held = time.as_secs_f32();
            commands.queue(move |world: &mut World| spawn_screen(world, Hold::For(held), 0.0));
        }
    }
}

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "loading.wgsl");
        app.add_plugins(UiMaterialPlugin::<FlagMaterial>::default())
            .add_systems(
                Update,
                (
                    debug_screen,
                    finish.run_if(resource_exists::<Loading>),
                    animate,
                )
                    .chain(),
            );
    }
}
