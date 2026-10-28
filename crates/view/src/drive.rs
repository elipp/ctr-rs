//! Drive mode (`--drive`): the kart on the keyboard, simulated by the
//! `vehicle` crate once per game tick (30 Hz), drawn between the last two
//! ticks, with a chase camera.
//!
//! * W / Up: gas (Cross), S / Down: brake and reverse (Square, stick back)
//! * A / D, Left / Right: steer
//! * Space / Shift: hop (R1), Q / Ctrl: hop (L1). Land while steering hard
//!   with one still held to drift; tap the other when the meter is nearly
//!   empty for the boost
//! * M: use the item in hand (Circle): the mask is longer with 10 wumpa fruit. A bomb or a
//!   potion is thrown backward with S held; a potion forward with E held (dropped with neither)
//! * `/debug set wumpa <N>`: (debug) set the wumpa count (0 to 10, clamped)
//! * `/debug set weapon <N>`: (debug) drop what is in hand and roll item N (0 to 4, 6, 7, 10 or 11), as the
//!   roulette would (the item ids; 10 and 11 are the triple bomb and missile). Not online, not in
//!   a demo
//! * Left mouse button held and mouse moves: turn the camera around the kart; scroll wheel:
//!   closer or farther; double click or Home: back to the default view
//! * R: back to the starting grid
//! * Escape, P or Start: the pause menu (`pause.rs`)
//! * F5 / 1: start recording a demo from the grid, again to save it to `demo.ctr`
//! * F6 / 2: play `demo.ctr` back from the grid (any F5 or R ends it, the kart stays where it is)
//!
//! Not in the sim yet: turbo pads (they are applied in kart-against-kart), items, the
//! start-of-race countdown, other karts. Falling off the track is a mask grab: Aku Aku
//! or Uka Uka (`mask.rs`) carries the kart back to the track.

use crate::toast::Toast;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use race::rollback::Step;
use race::{KartSpec, Pad};
use vehicle::demo::Demo;
use vehicle::{Driver, Effect, Input, buttons};

use crate::Track;
use crate::command::Action;
use crate::font::CtrText;
use crate::hud;
use crate::kart::{KartMotion, KartSlot};
use crate::wumpa_icon::{COUNTER_GAP, COUNTER_X, MIDDLE_Y};
use render::font::{Font, Gradient};
use render::mesh::SCALE;

/// The kart model's entity: its pose is the interpolated pose of the sim.
#[derive(Component)]
pub struct Kart;

/// Marks the camera that shows the sim's `camera::CameraDc`.
#[derive(Component)]
pub struct ChaseCamera;

impl ChaseCamera {
    pub fn new() -> ChaseCamera {
        ChaseCamera
    }
}

/// Where the camera is, what it looks at, and how it rolls.
#[derive(Clone, Copy)]
struct View {
    eye: Vec3,
    target: Vec3,
    /// Turns, around the view direction.
    roll: f32,
}

fn view_of(camera: &camera::CameraDc) -> View {
    let at = |x: i32, y: i32, z: i32| Vec3::new(x as f32, y as f32, z as f32) * SCALE;
    let p = camera.pb_pos;
    // The push buffer looks along its rotation, away from where the camera
    // would sit behind the look-at point (it stays put while the mask holds the kart).
    let turns = |angle: i32| (angle & 0xfff) as f32 / 4096.0 * std::f32::consts::TAU;
    let (yaw, pitch) = (
        turns(camera.pb_rot.y as i32),
        turns(0x800 - camera.pb_rot.x as i32),
    );
    let back = Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    );
    let eye = at(p.x as i32, p.y as i32, p.z as i32);
    View {
        eye,
        target: eye - back * 256.0 * SCALE,
        roll: camera.pb_rot.z as f32 / 4096.0,
    }
}

#[derive(Component)]
struct Hud;

/// The wumpa count: the number after the "x".
#[derive(Component)]
pub(crate) struct WumpaHud;

/// The time at the checkpoint just passed: green if we were the first there, else red and how
/// far behind the first.
#[derive(Component)]
struct SplitHud;

/// How long a checkpoint time stays up, in milliseconds of the race. (The last one, at the finish
/// line, stays.)
const SPLIT_SHOW_MS: i32 = 3000;

/// How many ticks we may play past the last pad of another player before we wait for it: the
/// prediction is that long at most. The clock is kept with the others' without this (see
/// `lobby::pace`), so it only matters when one of them stalls for good: 2 seconds.
const MAX_LEAD: u32 = 60;

/// How often the toast "Waiting for ..." is put up again while we wait: every 3 seconds, and it
/// stays that long.
const WAIT_TOAST_TICKS: u32 = 90;

/// The ticks between the beeps of an online race: a second.
const TICKS_PER_BEEP: u32 = 60;

/// The pitch of the first player's beep; each next player is a major third higher.
const BEEP_HZ: f32 = 440.0;

/// What a tick can ask of the player's ears and eyes outside the sim. (The traffic lights are
/// drawn by `lights.rs`; retail also plays a sound with each, `OtherFX` 0x45 for a red light and
/// 0x46 for the green, which is not here yet.)
#[derive(bevy::ecs::system::SystemParam)]
struct Cues<'w, 's> {
    commands: Commands<'w, 's>,
    pitches: ResMut<'w, Assets<bevy::audio::Pitch>>,
    toasts: MessageWriter<'w, Toast>,
}

impl Cues<'_, '_> {
    fn beep(&mut self, hertz: f32, millis: u64) {
        let pitch = self.pitches.add(bevy::audio::Pitch::new(
            hertz,
            std::time::Duration::from_millis(millis),
        ));
        self.commands
            .spawn((AudioPlayer(pitch), PlaybackSettings::DESPAWN));
    }
}

/// How long F7 and F8 freeze an online client for.
const STALLS_MS: [u64; 2] = [500, 2000];

/// The boost reserves, as a number at the bottom right.
#[derive(Component)]
struct ReservesHud;

/// "REC" or "DEMO" while a demo is being recorded or played.
#[derive(Component)]
struct DemoHud;

/// Where the kart is drawn.
#[derive(Clone, Copy)]
pub(crate) struct Pose {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

/// What the viewer keeps of driving: the camera, the poses to draw between ticks, the demo. The
/// race itself, our kart in it included, is [`crate::props::Sim`].
#[derive(Resource)]
pub struct Drive {
    pub(crate) character: usize,
    anim_num_frames: [u16; 8],
    demo: DemoMode,
    camera: camera::CameraDc,
    previous_view: View,
    current_view: View,
}

impl Drive {
    /// Driving for `driver`, the kart of `character` on the grid.
    pub fn new(
        track: &Track,
        driver: &Driver,
        character: usize,
        anim_num_frames: [u16; 8],
    ) -> Drive {
        let mut camera = camera::CameraDc::default();
        camera.follow_driver(
            driver,
            &track.0,
            &camera::Input {
                elapsed_time_ms: 32,
                ..Default::default()
            },
        );
        let view = view_of(&camera);
        Drive {
            camera,
            previous_view: view,
            current_view: view,
            character,
            anim_num_frames,
            demo: DemoMode::Off,
        }
    }

    /// Our kart, as a race takes it.
    pub fn spec(&self) -> KartSpec {
        KartSpec {
            character: self.character,
            anim_num_frames: self.anim_num_frames,
        }
    }
}

/// The rotation a game matrix stands for: its second column is up and its third forward. A matrix
/// that is flat (a squished kart) has none.
pub(crate) fn rotation_of(matrix: &vehicle::Matrix) -> Quat {
    let column = |c: usize| {
        Vec3::new(
            matrix.m[0][c] as f32,
            matrix.m[1][c] as f32,
            matrix.m[2][c] as f32,
        ) / 4096.0
    };
    let (up, forward) = (column(1), column(2));
    if up.length_squared() == 0.0 || forward.length_squared() == 0.0 {
        return Quat::IDENTITY;
    }
    let right = up.cross(forward).normalize_or_zero();
    let forward = right.cross(up.normalize_or_zero());
    Quat::from_mat3(&Mat3::from_cols(
        right,
        up.normalize_or_zero(),
        forward.normalize_or_zero(),
    ))
}

/// The pose of the model, from the matrix the sim gives the kart's instance (the
/// surface and heading, and the wheelie on top): its third column is forward and its
/// second up. The squash of the wheelie and the instance's own squash and stretch go in the scale.
pub(crate) fn pose_of(d: &Driver) -> Pose {
    let (matrix, t) = vehicle::baked::inst_matrix(d);
    let column = |c: usize| {
        Vec3::new(
            matrix.m[0][c] as f32,
            matrix.m[1][c] as f32,
            matrix.m[2][c] as f32,
        ) / 4096.0
    };
    let (side, up, forward) = (column(0), column(1), column(2));
    let rotation = rotation_of(&matrix);
    // The squash and stretch of the instance, on top of the wheelie's.
    let inst = Vec3::new(
        d.inst_scale_x as f32,
        d.inst_scale_y as f32,
        d.inst_scale_z as f32,
    ) / vehicle::force::KART_SCALE_BASE as f32;
    let basis = Vec3::new(side.length(), up.length(), forward.length());
    // A squished kart is flat: its height is 0, which a transform would not invert.
    let scale = basis * inst;
    Pose {
        translation: Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32) * SCALE,
        rotation,
        scale: if basis.min_element() > 0.0 && inst.x > 0.0 && inst.z > 0.0 {
            scale.max(Vec3::splat(0.001))
        } else {
            Vec3::ONE
        },
    }
}

/// The pose of the animation at `frame` (`Instance::animFrame`). An animation that is
/// interpolated stores every other frame, and an odd frame is the average of its neighbours.
pub(crate) fn animation_pose(
    header: &loader::model::ModelHeader,
    index: u8,
    frame: i16,
) -> Option<loader::model::Frame> {
    let animation = header.animations.get(index as usize)?;
    let last = animation.frames.len().checked_sub(1)?;
    let frame = frame.max(0) as usize;
    if !animation.interpolated {
        return animation.frames.get(frame.min(last)).cloned();
    }
    let from = &animation.frames[(frame >> 1).min(last)];
    if frame & 1 == 0 {
        return Some(from.clone());
    }
    let to = &animation.frames[((frame >> 1) + 1).min(last)];
    Some(loader::model::Frame {
        origin: from.origin,
        vertices: from
            .vertices
            .iter()
            .zip(&to.vertices)
            .map(|(a, b)| std::array::from_fn(|i| (a[i] + b[i]) >> 1))
            .collect(),
    })
}

/// Where the sim gets its input from.
enum DemoMode {
    Off,
    Recording(Demo),
    Playing(Demo),
}

const DEMO_FILE: &str = "demo.ctr";

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(crate::exhaust::Particles::new())
            .init_resource::<crate::flash::Flash>()
            .add_systems(Startup, spawn_hud)
            .add_systems(
                FixedUpdate,
                (drive_tick, crate::flash::step, crate::projectile::tick).chain(),
            )
            .add_systems(
                Update,
                (
                    crate::kart::reap,
                    crate::kart::animate,
                    crate::kart::show,
                    chase,
                    debug_actions,
                    crate::projectile::place,
                    crate::rain::draw,
                    crate::flash::show,
                    hud,
                    wumpa_hud,
                    split_hud,
                    reserves_hud,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    crate::shadow::update,
                    crate::flame::update,
                    crate::skid::update,
                    crate::mask::update.after(crate::kart::show),
                    crate::shield::update.after(crate::kart::show).after(chase),
                    crate::names::overhead.after(crate::kart::show).after(chase),
                ),
            )
            .add_systems(
                PostUpdate,
                (crate::exhaust::update, crate::exhaust::update_sparks)
                    .before(TransformSystems::Propagate),
            );
    }
}

pub(crate) fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        SplitHud,
        Text::new(""),
        TextFont {
            font_size: 30.0.into(),
            ..default()
        },
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(70.0),
            width: Val::Percent(100.0),
            ..default()
        },
    ));
    // The counter: a small white "x" and the count in the big orange digits, level with the
    // middle of the fruit, right of it.
    let height = Font::Large.line_height() as f32;
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            margin: UiRect::left(hud::x(COUNTER_X)),
            top: hud::y(MIDDLE_Y - height / 2.0),
            align_items: AlignItems::FlexEnd,
            column_gap: hud::x(COUNTER_GAP as f32),
            ..default()
        },
        children![
            (
                CtrText::new("x", Font::Middle, Gradient::WHITE).fixed(hud::PIXEL),
                Node {
                    margin: UiRect::bottom(hud::y(3.0)),
                    ..default()
                },
            ),
            (
                WumpaHud,
                CtrText::new("0", Font::Large, Gradient::ORANGE).fixed(hud::PIXEL),
            ),
        ],
    ));
    commands.spawn((
        ReservesHud,
        Text::new(""),
        TextFont {
            font_size: 24.0.into(),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            right: Val::Px(20.0),
            ..default()
        },
    ));
    // The readouts for development go where the HUD of a race leaves room: the bottom left is
    // the place's and the top right the lap's. These are left of the minimap and the slide meter
    // (the widest map reaches 100 pixels of retail's screen in from the right edge), and under
    // the lap.
    commands.spawn((
        Hud,
        Text::new(""),
        TextLayout::justify(Justify::Right),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(64.0),
            right: hud::x(108.0),
            ..default()
        },
    ));
    commands.spawn((
        DemoHud,
        Text::new(""),
        TextLayout::justify(Justify::Right),
        Node {
            position_type: PositionType::Absolute,
            top: hud::y(32.0),
            right: Val::Px(8.0),
            ..default()
        },
    ));
}

fn read_input(keys: &ButtonInput<KeyCode>, pads: &Query<&Gamepad>, previous_held: u32) -> Input {
    let down = |codes: &[KeyCode]| codes.iter().any(|&k| keys.pressed(k));
    let pad_down =
        |codes: &[GamepadButton]| pads.iter().any(|p| p.any_pressed(codes.iter().copied()));
    // Analog left stick of the first pad that has it pushed, deadzone already applied.
    let stick = pads
        .iter()
        .map(|p| p.left_stick())
        .find(|s| *s != Vec2::ZERO)
        .unwrap_or(Vec2::ZERO);

    let mut held = 0;
    if down(&[KeyCode::ArrowUp]) || pad_down(&[GamepadButton::South]) {
        held |= buttons::CROSS;
    }
    // Square brakes. Reversing is the stick (or D-pad) pulled down (`stickLY`, see
    // `gas_and_brake`): alone, it reverses when the gas is not held, and with Square it
    // brakes into reverse. On the D-pad it leaves Square alone, so it does not end a
    // powerslide. The keyboard's S and Down are both, like Square plus Down on a pad.
    let key_reverse = down(&[KeyCode::ArrowDown]);
    if key_reverse || pad_down(&[GamepadButton::West]) {
        held |= buttons::SQUARE;
    }
    // Up and Down aim a thrown weapon: a bomb or a potion goes backward with Down, a potion
    // forward with Up, and a potion is dropped with neither. The keyboard's S is Down, as it is
    // for the brake; W is the gas, so Up is E.
    if down(&[KeyCode::KeyE]) || pad_down(&[GamepadButton::DPadUp]) {
        held |= buttons::UP;
    }
    if key_reverse || pad_down(&[GamepadButton::DPadDown]) {
        held |= buttons::DOWN;
    }
    if down(&[KeyCode::Space, KeyCode::ShiftLeft, KeyCode::ShiftRight])
        || pad_down(&[GamepadButton::RightTrigger, GamepadButton::RightTrigger2])
    {
        held |= buttons::R1;
    }
    if down(&[KeyCode::KeyQ, KeyCode::ControlLeft])
        || pad_down(&[GamepadButton::LeftTrigger, GamepadButton::LeftTrigger2])
    {
        held |= buttons::L1;
    }
    // The mask item is Circle, so a demo has it.
    if down(&[KeyCode::KeyD]) || pad_down(&[GamepadButton::East]) {
        held |= buttons::CIRCLE;
    }
    let left = down(&[KeyCode::ArrowLeft]) || pad_down(&[GamepadButton::DPadLeft]);
    let right = down(&[KeyCode::ArrowRight]) || pad_down(&[GamepadButton::DPadRight]);
    // Sticks: 0 is left, 0x80 the middle, 0xff right (and back for LY).
    let axis = |v: f32| (0x80 as f32 + v * 127.0).round().clamp(0.0, 255.0) as u8;
    let stick_lx = match (left, right) {
        (true, false) => 0,
        (false, true) => 0xff,
        _ => axis(stick.x),
    };
    // 0 is up and 0xff down. The D-pad pushes the stick all the way (retail steps it there).
    let stick_ly = if key_reverse || pad_down(&[GamepadButton::DPadDown]) {
        0xff
    } else {
        axis(-stick.y)
    };
    Input {
        buttons_held: held,
        buttons_tapped: held & !previous_held,
        stick_lx,
        stick_ly,
        stick_ry: 0x80,
    }
}

/// The pad of the race for a pad of the network.
pub(crate) fn pad_of(pad: &net::Pad) -> Pad {
    Pad {
        held: pad.held,
        lx: pad.lx,
        ly: pad.ly,
        ry: pad.ry,
    }
}

/// One game tick of the sim.
fn drive_tick(
    mut particles: ResMut<crate::exhaust::Particles>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    track: Res<Track>,
    mut sim: ResMut<crate::props::Sim>,
    mut drive: ResMut<Drive>,
    mut menu: ResMut<crate::pause::PauseMenu>,
    mut was_paused: Local<bool>,
    mut keys_held: Local<[bool; 4]>,
    mut demo_label: Query<&mut Text, With<DemoHud>>,
    mut lobby: ResMut<crate::lobby::Lobby>,
    mut motions: Query<(&mut KartMotion, &mut KartSlot, Has<Kart>)>,
    smoothing: Res<crate::kart::Smoothing>,
    mut cues: Cues,
    mut waited: Local<u32>,
) {
    let drive = &mut *drive;
    // In an online race the sim is everyone's: no pause, no restarts, no demos.
    let online = lobby.online.is_some();
    // The level of the race is not up yet, nor the karts of the others.
    if lobby
        .online
        .as_ref()
        .is_some_and(|net| net.track != track.1.0 || !net.built)
    {
        return;
    }

    // The pause menu stops the race, unless the race is everyone's.
    let paused = menu.open && !online;
    if paused && !*was_paused {
        // Nothing moves between ticks now, so the interpolation must not either.
        drive.previous_view = drive.current_view;
        for (mut motion, _, _) in &mut motions {
            motion.freeze();
        }
    }
    *was_paused = paused;
    if paused {
        return;
    }

    // F5 records a demo from the grid (and saves it when pressed again), F6 plays the saved
    // one. R (or the menu's RESTART) is the grid without either. A demo is the inputs alone, so
    // every start puts everything, the objects too, back as it was at tick 0.
    let picked = std::mem::take(&mut menu.restart);
    let mut restart = (keys.pressed(KeyCode::KeyR) || picked) && !online;
    // Edges are found here, not with `just_pressed`: that is cleared every rendered frame, and
    // a frame can pass without a tick.
    let demo_down = [
        !online && keys.any_pressed([KeyCode::F5]) || !online && keys.pressed(KeyCode::Digit1),
        !online && keys.any_pressed([KeyCode::F6]) || !online && keys.pressed(KeyCode::Digit2),
    ];
    let [record_pressed, play_pressed] = std::array::from_fn(|i| demo_down[i] && !keys_held[i]);
    keys_held[..2].copy_from_slice(&demo_down);
    if record_pressed {
        if let DemoMode::Recording(demo) = &drive.demo {
            match std::fs::write(DEMO_FILE, demo.to_text()) {
                Ok(()) => info!("saved {} ticks to {DEMO_FILE}", demo.length),
                Err(e) => error!("could not save {DEMO_FILE}: {e}"),
            }
            drive.demo = DemoMode::Off;
        } else {
            drive.demo = DemoMode::Recording(Demo::new(drive.character));
            restart = true;
        }
    } else if play_pressed {
        match std::fs::read_to_string(DEMO_FILE)
            .map_err(|e| e.to_string())
            .and_then(|text| Demo::from_text(&text))
        {
            Ok(demo) => {
                // The demo was driven by this character.
                drive.character = demo.character;
                drive.demo = DemoMode::Playing(demo);
                restart = true;
            }
            Err(e) => error!("could not play {DEMO_FILE}: {e}"),
        }
    } else if restart {
        drive.demo = DemoMode::Off;
    }
    // The lobby closed, or the race failed: back to the grid with our kart alone.
    if !online && sim.race.karts.len() != 1 {
        restart = true;
    }
    // The race starts: tick 0 for everyone.
    if let Some(net) = lobby.online.as_mut().filter(|net| net.fresh) {
        net.fresh = false;
        drive.demo = DemoMode::Off;
        restart = true;
    }
    if restart {
        let (specs, own) = match &lobby.online {
            Some(net) => (net.specs.clone(), net.slot),
            None => (vec![drive.spec()], 0),
        };
        sim.restart(&track.0, &specs, own, lobby.online.is_some());
        particles.list.clear();
        drive.camera = camera::CameraDc::default();
        each_motion(&sim, &mut motions, |motion, kart| motion.reset(kart));
    }

    // F7 and F8 freeze this client for a moment, to see the race put right after a hitch: the
    // clock does not catch up (the frame is cut short), so the others run ahead and we wait.
    let stall_down = [KeyCode::F7, KeyCode::F8].map(|key| online && keys.pressed(key));
    for (i, millis) in STALLS_MS.into_iter().enumerate() {
        if stall_down[i] && !keys_held[2 + i] {
            info!("(debug) stalling for {millis} ms");
            std::thread::sleep(std::time::Duration::from_millis(millis));
        }
    }
    keys_held[2..].copy_from_slice(&stall_down);

    // Tick 0 has to come first: the others must be ready as well.
    if lobby.online.as_ref().is_some_and(|net| !net.go) {
        for mut text in &mut demo_label {
            text.0 = lobby
                .online
                .as_ref()
                .map_or_else(String::new, |net| net.summary(0, 0));
        }
        return;
    }

    let tick = sim.race.tick();
    let own = sim.own;
    let mut reports = Vec::new();
    if let Some(net) = lobby.online.as_mut() {
        // The pads of the others that came in since the last tick.
        for (slot, first, pads) in net.inbox.drain(..) {
            let pads: Vec<Pad> = pads.iter().map(pad_of).collect();
            if let Err(hole) = sim.race.confirm_all(slot, first, &pads) {
                error!(
                    "pads of player {slot} from tick {first}: missing from {}",
                    hole.expected
                );
            }
        }
        // A player left: say what we hold of its pads, which is all we take from now on.
        for slot in std::mem::take(&mut net.asked) {
            if slot < sim.race.players() {
                net.frozen[slot] = true;
                let (first, pads) = sim.race.recent(slot);
                reports.push((slot, first, pads.iter().map(|p| net::Pad {
                    held: p.held,
                    lx: p.lx,
                    ly: p.ly,
                    ry: p.ry,
                }).collect::<Vec<_>>()));
            }
        }
        // The host settled where the pads of a player that left end: everyone ends them there.
        for (slot, first, pads) in std::mem::take(&mut net.lefts) {
            if slot >= sim.race.players() {
                continue;
            }
            let pads: Vec<Pad> = pads.iter().map(pad_of).collect();
            if let Err(hole) = sim.race.confirm_all(slot, first, &pads) {
                // More than the pads kept were missing.
                error!("the pads of player {slot} that left: missing from {}", hole.expected);
                net.found([race::desync::Mismatch { tick: hole.expected, slot }]);
            }
            sim.race.leave(slot);
            net.frozen[slot] = true;
            net.gone[slot] = true;
            let name = net.labels.get(slot).map_or("?", String::as_str);
            cues.toasts.write(
                Toast::new(format!("Player {name} left the race"), 3000)
                    .color(Color::srgb(1.0, 0.6, 0.3)),
            );
        }
    }
    for (slot, first, pads) in reports {
        lobby.have(slot, first, pads);
    }
    // Each client runs its own 30 Hz clock, so one that is ahead would stay ahead (up to the
    // rollback window) and predict the others further and further. It waits for them instead.
    let late: Vec<usize> = if online {
        (0..sim.race.players())
            .filter(|&slot| slot != own)
            .filter(|&slot| tick > sim.race.confirmed_until(slot).saturating_add(MAX_LEAD))
            .collect()
    } else {
        Vec::new()
    };
    if !late.is_empty() {
        // (The host removes a player that stays silent: see `net::KICK_AFTER`.)
        if *waited % WAIT_TOAST_TICKS == 0
            && let Some(net) = lobby.online.as_ref()
        {
            let names: Vec<&str> = late
                .iter()
                .map(|&slot| net.labels.get(slot).map_or("?", String::as_str))
                .collect();
            let text = format!("Waiting for {}...", names.join(", "));
            cues.toasts.write(
                Toast::new(text, WAIT_TOAST_TICKS as u64 * 1000 / crate::GAME_TICK_HZ as u64)
                    .color(Color::srgb(1.0, 0.8, 0.2)),
            );
        }
        *waited += 1;
        return;
    }
    *waited = 0;
    // Which way R1 and L1 were tapped this tick, to change the kart the camera follows.
    let mut cycle = 0;
    // Our own pad for this tick, from the controller as it is now. It is played at once, and the
    // others get it as well. (A tick that waits plays the pad it already has.)
    if sim.race.confirmed_until(own) <= tick {
        let held = sim.kart().held;
        let pad = if online && menu.open {
            // The menu has the controls: the kart coasts.
            Pad::default()
        } else if online {
            Pad::of_input(&read_input(&keys, &pads, held))
        } else {
            let mut input = match &drive.demo {
                DemoMode::Playing(demo) => demo.input(tick, held),
                _ => None,
            };
            if input.is_none() {
                if matches!(drive.demo, DemoMode::Playing(_)) {
                    info!("demo over after {tick} ticks");
                    drive.demo = DemoMode::Off;
                }
                input = Some(read_input(&keys, &pads, held));
            }
            let input = input.unwrap();
            if let DemoMode::Recording(demo) = &mut drive.demo {
                demo.record(tick, &input);
            }
            Pad::of_input(&input)
        };
        let taps = pad.held & !held;
        cycle = (taps & buttons::R1 != 0) as i32 - (taps & buttons::L1 != 0) as i32;
        sim.race
            .confirm(own, tick, pad)
            .expect("our pads come in order");
        if online {
            lobby.send(net::Pad {
                held: pad.held,
                lx: pad.lx,
                ly: pad.ly,
                ry: pad.ry,
            });
        }
    }
    for mut text in &mut demo_label {
        text.0 = match &drive.demo {
            _ if online => lobby
                .online
                .as_ref()
                .map_or_else(String::new, |net| net.summary(tick, sim.race.unsettled())),
            DemoMode::Off => String::new(),
            DemoMode::Recording(_) => format!("REC {tick}"),
            DemoMode::Playing(demo) => format!("DEMO {tick}/{}", demo.length),
        };
    }

    // Everyone's karts play together, by slot, ours among them. The pads that are not in yet
    // are predicted, and put right when they come.
    // A late pad puts the karts of the others right, which would show as a jump: the drawing
    // eases from where the kart was predicted to be to where it is now.
    let mut replayed = 0;
    if sim.race.dirty() {
        let predicted: Vec<Pose> = sim.race.karts.iter().map(|k| pose_of(&k.driver)).collect();
        replayed = sim.race.correct(&track.0);
        for (mut motion, slot, ours) in &mut motions {
            if let (false, Some(before), Some(kart)) =
                (ours, predicted.get(slot.0), sim.race.karts.get(slot.0))
            {
                motion.absorb(before, kart, replayed);
            }
        }
    }
    let advance = sim.race.advance(&track.0);
    replayed += advance.replayed;
    if replayed > 0
        && let Some(net) = lobby.online.as_mut()
    {
        net.corrections += 1;
        net.replayed += replayed;
    }
    // The hashes of the ticks that are final now: the others get ours, and ours are held up
    // against theirs.
    for (tick, hash) in sim.race.take_checksums() {
        lobby.checksum(tick, hash);
        if let Some(net) = lobby.online.as_mut() {
            let found = net.checks.mine(tick, hash);
            net.found(found);
        }
    }
    let Step::Played(ticks) = advance.step else {
        return;
    };
    // A beep every second of ticks, higher for each player: the clients' beeps line up when
    // their ticks do. (Off.)
    if online && tick.is_multiple_of(TICKS_PER_BEEP) {
        // let frequency = BEEP_HZ * 1.25_f32.powi(sim.own as i32);
        // cues.beep(frequency, 100);
    }
    let mine = &ticks[sim.own];
    // Everyone's exhaust and sparks, ours and the others' (the others' as the sim has them).
    for (slot, kart) in ticks.iter().enumerate() {
        particles.sparks(slot, kart.landing, kart.wall);
        particles.puffs(slot, &kart.puffs);
    }
    particles.tick();

    // The mask grab put the kart back on the track: the camera swings round behind it.
    if mine.result.respawned {
        drive.camera.flags |= camera::flags::DIRECTION_CHANGED;
    }

    // A boost pulls the camera back.
    if mine
        .effects
        .iter()
        .any(|e| matches!(e, Effect::CameraFireSpeedZoom { .. }))
    {
        drive.camera.flags |= camera::flags::FIRE_SPEED_ZOOM;
    }
    let camera_input = camera::Input {
        elapsed_time_ms: vehicle::Env::default().elapsed_time_ms,
        ..Default::default()
    };
    // Once we have finished, the camera can follow the others (R1 and L1), the ones still here.
    let finished = sim.driver().actions_flag_set & vehicle::actions::RACE_FINISHED != 0;
    if let Some(net) = lobby.online.as_mut() {
        let players = sim.race.players();
        let before = net.watching;
        if !finished || net.watching >= players || net.gone[net.watching] {
            net.watching = own;
        }
        if finished && cycle != 0 {
            for _ in 0..players {
                net.watching = (net.watching as i32 + cycle).rem_euclid(players as i32) as usize;
                if !net.gone[net.watching] {
                    break;
                }
            }
        }
        if net.watching != before {
            drive.camera.flags |= camera::flags::DIRECTION_CHANGED;
        }
    }
    let watching = lobby.online.as_ref().map_or(own, |net| net.watching);
    let kart = &sim.race.karts[watching];
    drive
        .camera
        .follow_driver(&kart.driver, &track.0, &camera_input);
    // The start-line fly-in, over the chase camera, for as long as the race says.
    let fly_in_frame = camera::TRANSITION_FRAMES - tick as i32;
    if sim.race.intro_ticks > 0 && fly_in_frame >= 0 {
        drive.camera.fly_in(&track.0, fly_in_frame);
    }

    // The offsets of the others' karts fade as far as their players' pings call for.
    for (mut motion, slot, ours) in &mut motions {
        if !ours {
            let ping = lobby
                .online
                .as_ref()
                .and_then(|net| net.names.get(slot.0))
                .and_then(|name| lobby.average_pings.get(name));
            let ease = smoothing.ease(ping.copied(), motion.replayed_average());
            motion.set_ease(ease);
        }
    }
    each_motion(&sim, &mut motions, |motion, kart| motion.push(kart));
    drive.previous_view = drive.current_view;
    drive.current_view = view_of(&drive.camera);
}

/// The debug commands that change our kart: `/debug set weapon <N>` drops what we hold and rolls
/// item `N`, as the roulette would (5, the spring, turns into a turbo), and `/debug set wumpa <N>`
/// sets the wumpa count. Not online, and not in a demo, which is the inputs alone.
fn debug_actions(
    mut actions: MessageReader<Action>,
    mut sim: ResMut<crate::props::Sim>,
    drive: Res<Drive>,
    lobby: Res<crate::lobby::Lobby>,
    mut toasts: MessageWriter<Toast>,
) {
    for action in actions.read() {
        // Like F7 and F8, for as long as asked: the frame is cut short, so the others run ahead.
        if let Action::DebugStall(time) = action {
            if lobby.online.is_some() {
                info!("(debug) stalling for {time:?}");
                std::thread::sleep(*time);
            } else {
                toasts.write(Toast::new("Only online", 2500).color(Color::srgb(1.0, 0.4, 0.4)));
            }
            continue;
        }
        if !matches!(action, Action::DebugAddWeapon(_) | Action::DebugSetWumpa(_)) {
            continue;
        }
        if lobby.online.is_some() || !matches!(drive.demo, DemoMode::Off) {
            toasts.write(
                Toast::new("Not online or in a demo", 2500).color(Color::srgb(1.0, 0.4, 0.4)),
            );
            continue;
        }
        let own = sim.own;
        let driver = &mut sim.race.state_mut().karts[own].driver;
        let said = match *action {
            Action::DebugAddWeapon(item) => {
                vehicle::item::debug_roll(driver, item);
                format!("(debug) rolled item {item}")
            }
            Action::DebugSetWumpa(count) => {
                driver.num_wumpas = count as i8;
                format!("(debug) wumpa set to {count}")
            }
            _ => continue,
        };
        toasts.write(Toast::new(said, 2000));
    }
}

/// Runs `f` on the motion of each kart's drawing, with the kart of the sim that it follows.
/// (The kart we drive follows our slot, which is another one in each race.)
fn each_motion(
    sim: &crate::props::Sim,
    motions: &mut Query<(&mut KartMotion, &mut KartSlot, Has<Kart>)>,
    mut f: impl FnMut(&mut KartMotion, &race::Kart),
) {
    for (mut motion, mut slot, ours) in motions.iter_mut() {
        if ours {
            slot.0 = sim.own;
        }
        if let Some(kart) = sim.race.karts.get(slot.0) {
            f(&mut motion, kart);
        }
    }
}

/// How far the camera turns per pixel the mouse moves, in radians.
const ORBIT_SPEED: f32 = 0.004;

/// How far up or down the camera may be orbited from where the sim puts it, in radians.
const ORBIT_PITCH_MAX: f32 = 1.2;

/// The longest time between the two clicks of a double click, in seconds.
const DOUBLE_CLICK_SECS: f32 = 0.35;

/// How much the camera's distance from the kart changes per scroll line, as a factor, and the
/// nearest and farthest it can be, as factors of the distance the sim gives it.
const ZOOM_PER_LINE: f32 = 0.9;
const ZOOM_RANGE: (f32, f32) = (0.3, 5.0);

/// How the player has turned and pulled the camera away from the sim's.
struct Orbit {
    /// Yaw and pitch, in radians.
    angles: Vec2,
    /// The distance from the kart, as a factor.
    zoom: f32,
    /// When the left button last went down, in seconds of the app's time.
    last_click: f32,
}

impl Default for Orbit {
    fn default() -> Orbit {
        Orbit {
            angles: Vec2::ZERO,
            zoom: 1.0,
            last_click: f32::NEG_INFINITY,
        }
    }
}

/// The camera of the sim, between the last two ticks, turned around the kart as the mouse drags
/// it (left button held and move) and pulled away or closer by the scroll wheel. A double click
/// or Home puts it back. This is the view only, the sim's camera (and the rollback) do not know
/// of it.
fn chase(
    drive: Res<Drive>,
    fixed: Res<Time<Fixed>>,
    time: Res<Time<Real>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: Local<Orbit>,
    karts: Query<&Transform, (With<Kart>, Without<ChaseCamera>)>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    let now = time.elapsed_secs();
    if buttons.just_pressed(MouseButton::Left) {
        if now - orbit.last_click <= DOUBLE_CLICK_SECS {
            *orbit = Orbit::default();
        } else {
            orbit.last_click = now;
        }
    }
    if buttons.pressed(MouseButton::Left) {
        orbit.angles.x -= motion.delta.x * ORBIT_SPEED;
        orbit.angles.y = (orbit.angles.y - motion.delta.y * ORBIT_SPEED)
            .clamp(-ORBIT_PITCH_MAX, ORBIT_PITCH_MAX);
    }
    if keys.just_pressed(KeyCode::Home) {
        *orbit = Orbit::default();
    }
    let lines = match scroll.unit {
        MouseScrollUnit::Line => -scroll.delta.y,
        MouseScrollUnit::Pixel => -scroll.delta.y / 40.0,
    };
    if lines != 0.0 {
        orbit.zoom = (orbit.zoom * ZOOM_PER_LINE.powf(lines)).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
    }
    let t = fixed.overstep_fraction();
    let (a, b) = (drive.previous_view, drive.current_view);
    let mut eye = a.eye.lerp(b.eye, t);
    let mut target = a.target.lerp(b.target, t);
    if (orbit.angles != Vec2::ZERO || orbit.zoom != 1.0)
        && let Ok(kart) = karts.single()
    {
        let pivot = kart.translation;
        let right = (target - eye).cross(Vec3::Y).normalize_or_zero();
        let turn = Quat::from_axis_angle(Vec3::Y, orbit.angles.x)
            * Quat::from_axis_angle(right, orbit.angles.y);
        eye = pivot + turn * (eye - pivot) * orbit.zoom;
        target = pivot + turn * (target - pivot);
    }
    // The roll's sign (which way the horizon tilts) is a guess.
    let roll = (a.roll + (b.roll - a.roll) * t) * std::f32::consts::TAU;
    for mut transform in &mut cameras {
        transform.translation = eye;
        transform.look_at(target, Vec3::Y);
        transform.rotate_local_z(roll);
    }
}

/// The wumpa count.
fn wumpa_hud(sim: Res<crate::props::Sim>, mut counts: Query<&mut CtrText, With<WumpaHud>>) {
    let count = sim.driver().num_wumpas.to_string();
    for mut text in &mut counts {
        // A changed text puts its glyphs up again: only a new count changes it.
        if text.text != count {
            text.text = count.clone();
        }
    }
}

/// `m:ss.cc`, as a checkpoint time is told.
fn split_time(ms: i32) -> String {
    let ms = ms.max(0);
    format!("{}:{:02}.{:02}", ms / 60000, ms / 1000 % 60, ms % 1000 / 10)
}

/// Shows the time of the checkpoint we passed last (see `race::Race::last_split`) against the
/// first kart to pass it.
fn split_hud(
    sim: Res<crate::props::Sim>,
    lobby: Res<crate::lobby::Lobby>,
    mut text: Query<(&mut Text, &mut TextColor), With<SplitHud>>,
) {
    let race = &*sim.race;
    let finished = sim.driver().actions_flag_set & vehicle::actions::RACE_FINISHED != 0;
    let shown = lobby
        .online
        .as_ref()
        .and(race.last_split(sim.own))
        .filter(|&(_, time, _)| finished || race.event_ms - time < SPLIT_SHOW_MS);
    let (line, color) = match shown {
        Some((_, time, 0)) => (split_time(time), Color::srgb(0.4, 1.0, 0.4)),
        Some((_, _, behind)) => (
            format!("+{}", split_time(behind)),
            Color::srgb(1.0, 0.3, 0.3),
        ),
        None => (String::new(), Color::WHITE),
    };
    for (mut text, mut text_color) in &mut text {
        if text.0 != line {
            text.0 = line.clone();
        }
        text_color.0 = color;
    }
}

/// The boost reserves, always, up to 10000.
fn reserves_hud(sim: Res<crate::props::Sim>, mut text: Query<&mut Text, With<ReservesHud>>) {
    for mut text in &mut text {
        text.0 = format!("{}", sim.driver().reserves);
    }
}

/// The bottom-left lines: the frame rate and the time a frame takes (smoothed over the last few
/// frames), then where the kart is and what it does.
fn hud(
    sim: Res<crate::props::Sim>,
    diagnostics: Res<DiagnosticsStore>,
    mut text: Query<&mut Text, With<Hud>>,
) {
    let d = sim.driver();
    let grounded = d.actions_flag_set & vehicle::actions::TOUCH_GROUND != 0;
    let read = |path| diagnostics.get(path).and_then(|d| d.smoothed());
    let fps = match (
        read(&FrameTimeDiagnosticsPlugin::FPS),
        read(&FrameTimeDiagnosticsPlugin::FRAME_TIME),
    ) {
        (Some(fps), Some(ms)) => format!("{fps:5.1} fps  {ms:5.2} ms"),
        _ => String::new(),
    };
    for mut text in &mut text {
        text.0 = format!(
            "{fps}\npos {} {} {}  (level units)\nspeed {:5}  {:?}{}{}{}{}",
            d.pos_curr.x >> 8,
            d.pos_curr.y >> 8,
            d.pos_curr.z >> 8,
            d.speed_approx,
            d.phys_state,
            if grounded { "" } else { "  airborne" },
            if d.wall_rub_timer != 0 { "  wall" } else { "" },
            if d.phys_state == vehicle::PhysState::PowerSlide {
                format!(
                    "  meter {} boosts {}",
                    d.turbo_meter_room_left, d.num_boosts_success
                )
            } else {
                String::new()
            },
            match d.last_drift_end {
                Some(why) => format!("  last drift ended: {why:?}"),
                None => String::new(),
            },
        );
    }
}
