//! Drive mode (`--drive`): the kart on the keyboard, simulated by the
//! `vehicle` crate once per game tick (30 Hz), drawn between the last two
//! ticks, with a chase camera.
//!
//! * W / Up: gas (Cross), S / Down: brake and reverse (Square, stick back)
//! * A / D, Left / Right: steer
//! * Space / Shift: hop (R1), Q / Ctrl: hop (L1). Land while steering hard
//!   with one still held to drift; tap the other when the meter is nearly
//!   empty for the boost
//! * M: use the mask item (longer with 10 wumpa fruit)
//! * R: back to the starting grid
//! * F5 / 1: start recording a demo from the grid, again to save it to `demo.ctr`
//! * F6 / 2: play `demo.ctr` back from the grid (any F5 or R ends it, the kart stays where it is)
//!
//! Not in the sim yet: turbo pads (they are applied in kart-against-kart), items, the
//! start-of-race countdown, other karts. Falling off the track is a mask grab: Aku Aku
//! or Uka Uka (`mask.rs`) carries the kart back to the track.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use coll::Collision;
use vehicle::demo::Demo;
use vehicle::shadow::{GroundShadow, ground_shadow};
use vehicle::sim::spawn;
use vehicle::{Driver, Effect, Env, Input, buttons};

use crate::Track;
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

/// The wumpa counter.
#[derive(Component)]
struct WumpaHud;

/// The bar under the wumpa counter.
#[derive(Component)]
struct WumpaBar;

/// The drift's boost timing bar: it fills as the boost window nears.
#[derive(Component)]
struct TurboBar;

/// The boost reserves, as a number under the boost timing bar.
#[derive(Component)]
struct ReservesHud;

/// The frame rate and the time a frame takes.
#[derive(Component)]
struct FpsHud;

/// The "[paused]" label.
#[derive(Component)]
struct PausedHud;

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

#[derive(Resource)]
pub struct Drive {
    pub(crate) driver: Driver,
    coll: Collision,
    pub(crate) character: usize,
    anim_num_frames: [u16; 8],
    pub(crate) tick: u32,
    held: u32,
    demo: DemoMode,
    previous: Pose,
    current: Pose,
    shadow_previous: Option<GroundShadow>,
    shadow_current: Option<GroundShadow>,
    camera: camera::CameraDc,
    previous_view: View,
    current_view: View,
    /// When the last bump of karts was heard.
    feedback: vehicle::collide::Feedback,
}

impl Drive {
    pub fn new(track: &Track, character: usize, anim_num_frames: [u16; 8]) -> Drive {
        let (driver, coll) = Drive::birth(track, character, anim_num_frames, 0);
        let pose = pose_of(&driver);
        let mut camera = camera::CameraDc::default();
        camera.follow_driver(
            &driver,
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
            driver,
            coll,
            character,
            anim_num_frames,
            tick: 0,
            held: 0,
            demo: DemoMode::Off,
            previous: pose,
            current: pose,
            shadow_previous: None,
            shadow_current: None,
            feedback: Default::default(),
        }
    }

    /// The ground shadow between the last two ticks (`t` of the way to the
    /// latest): its centre, orientation and the height that scales it.
    pub fn shadow(&self, t: f32) -> Option<(Vec3, Mat3, f32)> {
        let now = self.shadow_current?;
        let before = self.shadow_previous.unwrap_or(now);
        let place = |s: &GroundShadow| Vec3::new(s.x as f32, s.y as f32, s.z as f32) * SCALE;
        let orient = |s: &GroundShadow| {
            let column = |c: usize| {
                Vec3::new(
                    s.axes.m[0][c] as f32,
                    s.axes.m[1][c] as f32,
                    s.axes.m[2][c] as f32,
                ) / 4096.0
            };
            let (up, forward) = (column(1).normalize_or_zero(), column(2).normalize_or_zero());
            Quat::from_mat3(&Mat3::from_cols(up.cross(forward), up, forward))
        };
        Some((
            place(&before).lerp(place(&now), t),
            Mat3::from_quat(orient(&before).slerp(orient(&now), t)),
            before.height as f32 + (now.height - before.height) as f32 * t,
        ))
    }

    /// `driver_id` is the player's slot in an online race (0 offline), which picks its place on
    /// the grid.
    pub(crate) fn birth(
        track: &Track,
        character: usize,
        anim_num_frames: [u16; 8],
        driver_id: usize,
    ) -> (Driver, Collision) {
        let mut coll = Collision::default();
        let mut driver = Driver::new(driver_id as u8, character);
        driver.anim_num_frames = anim_num_frames;
        let order = vehicle::grid::spawn_order(
            vehicle::grid::GridMode::Arcade,
            1,
            [0, 1, 2, 3, 4, 5, 6, 7],
        );
        spawn(&mut driver, &mut coll, &track.0, order[driver_id] as usize);
        (driver, coll)
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
            .add_systems(Startup, (spawn_hud, crate::mask::spawn))
            .add_systems(FixedUpdate, drive_tick)
            .add_systems(
                Update,
                (
                    crate::kart::animate,
                    crate::kart::show,
                    chase,
                    hud,
                    wumpa_hud,
                    wumpa_bar,
                    turbo_bar,
                    reserves_hud,
                    fps_hud,
                    crate::bar::update,
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
        WumpaHud,
        Text::new(""),
        TextFont {
            font_size: 34.0.into(),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.65, 0.1)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            right: Val::Px(20.0),
            ..default()
        },
    ));
    let node = Node {
        position_type: PositionType::Absolute,
        bottom: Val::Px(40.0),
        right: Val::Px(20.0),
        width: Val::Px(220.0),
        height: Val::Px(16.0),
        ..default()
    };
    let turbo = crate::bar::spawn(
        &mut commands,
        node,
        crate::bar::Bar::new(0.0, crate::bar::Bar::traffic_light()),
    );
    commands.entity(turbo).insert(TurboBar);
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
    commands.spawn((
        FpsHud,
        Text::new(""),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));
    commands.spawn((
        Hud,
        Text::new(""),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));
    // Above the three lines of `Hud`.
    commands.spawn((
        PausedHud,
        Text::new("[paused]"),
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(80.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));
    commands.spawn((
        DemoHud,
        Text::new(""),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
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
    if down(&[KeyCode::KeyW, KeyCode::ArrowUp]) || pad_down(&[GamepadButton::South]) {
        held |= buttons::CROSS;
    }
    // Square brakes. Reversing is the stick (or D-pad) pulled down (`stickLY`, see
    // `gas_and_brake`): alone, it reverses when the gas is not held, and with Square it
    // brakes into reverse. On the D-pad it leaves Square alone, so it does not end a
    // powerslide. The keyboard's S and Down are both, like Square plus Down on a pad.
    let key_reverse = down(&[KeyCode::KeyS, KeyCode::ArrowDown]);
    if key_reverse || pad_down(&[GamepadButton::West]) {
        held |= buttons::SQUARE;
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
    if down(&[KeyCode::KeyM]) || pad_down(&[GamepadButton::East]) {
        held |= buttons::CIRCLE;
    }
    let left = down(&[KeyCode::KeyA, KeyCode::ArrowLeft]) || pad_down(&[GamepadButton::DPadLeft]);
    let right =
        down(&[KeyCode::KeyD, KeyCode::ArrowRight]) || pad_down(&[GamepadButton::DPadRight]);
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

/// A pad as the input the sim takes; the taps follow from the buttons held a tick before.
pub(crate) fn input_of_pad(pad: &net::Pad, previous_held: u32) -> Input {
    Input {
        buttons_held: pad.held,
        buttons_tapped: pad.held & !previous_held,
        stick_lx: pad.lx,
        stick_ly: pad.ly,
        stick_ry: pad.ry,
    }
}

/// What a kart does before the stages of a tick: the same for ours and for every other player's,
/// so that all clients play every kart the same way.
pub(crate) fn before_tick(
    driver: &mut Driver,
    input: &Input,
    sim: &crate::props::Sim,
    effects: &mut Vec<Effect>,
) {
    // M (or East on a pad) uses the mask item, once per press. (Not while a mask grab has the
    // kart: it has the mask already.)
    if input.buttons_tapped & buttons::CIRCLE != 0
        && !matches!(
            driver.kart_state,
            vehicle::kart_state::MASK_GRABBED | vehicle::kart_state::ENGINE_REVVING
        )
    {
        vehicle::stuck::shoot_mask(driver, effects);
    }
    // The hazards that reached the kart's instance in their last tick hurt it
    // (`RB_CheckHazardCollisions`): the spin-out starts on this tick.
    let (_, at) = vehicle::baked::inst_matrix(driver);
    for contact in sim.last.contacts.iter().filter(|c| c.hits(at)) {
        let was_spinning = driver.kart_state == vehicle::kart_state::SPINNING;
        if vehicle::pick::new_state(driver, contact.damage as u8, effects)
            && !was_spinning
            && contact.sound != 0
        {
            let echo = driver.actions_flag_set & vehicle::actions::ENGINE_ECHO != 0;
            effects.push(Effect::Sound {
                id: contact.sound as u8,
                echo,
            });
        }
    }
}

/// What a kart does after the stages of a tick. Ours also gives its sparks and exhaust to
/// `particles`.
pub(crate) fn after_tick(
    driver: &mut Driver,
    env: &Env,
    particles: Option<&mut crate::exhaust::Particles>,
    effects: &mut Vec<Effect>,
) {
    // The pickup HUD gives the fruit that were picked up to the driver.
    vehicle::wumpa::hud_tick(driver, true, effects);

    // The exhaust and the skid marks keep their state in the driver, so they run for every kart
    // even when nobody sees the particles.
    let mut particles = particles;
    if let Some(particles) = particles.as_deref_mut() {
        particles.sparks(driver);
    }
    let puffs = vehicle::exhaust::exhaust(driver);
    if let Some(particles) = particles {
        particles.tick(&puffs);
    }
    vehicle::skid::skidmarks(driver);
    vehicle::turbo::tick(driver, env.elapsed_time_ms as i16);
}

/// One game tick of the sim.
fn drive_tick(
    mut particles: ResMut<crate::exhaust::Particles>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    track: Res<Track>,
    mut sim: ResMut<crate::props::Sim>,
    mut drive: ResMut<Drive>,
    mut enter_held: Local<bool>,
    mut paused: Local<bool>,
    mut demo_keys_held: Local<[bool; 2]>,
    mut label: Query<&mut Visibility, With<PausedHud>>,
    mut demo_label: Query<&mut Text, With<DemoHud>>,
    mut lobby: ResMut<crate::lobby::Lobby>,
    mut remotes: ResMut<crate::remote::Remotes>,
    mut motions: Query<(&mut crate::kart::KartMotion, Has<Kart>)>,
) {
    let drive = &mut *drive;
    // In an online race the sim is everyone's: no pause, no restarts, no demos.
    let online = lobby.online.is_some();
    // The level of the race is not up yet.
    if lobby
        .online
        .as_ref()
        .is_some_and(|net| net.track != track.1.0)
    {
        return;
    }

    // Enter toggles the pause, once per press.
    let enter_down = keys.pressed(KeyCode::Enter);
    if enter_down && !*enter_held && !online {
        *paused = !*paused;
        for mut visibility in &mut label {
            *visibility = if *paused {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
        if *paused {
            // Nothing moves between ticks now, so the interpolation must not either.
            drive.previous = drive.current;
            drive.previous_view = drive.current_view;
            drive.shadow_previous = drive.shadow_current;
        }
    }
    *enter_held = enter_down;
    if *paused && !online {
        return;
    }

    // F5 records a demo from the grid (and saves it when pressed again), F6 plays the saved
    // one. R is the grid without either. A demo is the inputs alone, so every start puts
    // everything, the objects too, back as it was at tick 0.
    let mut restart = keys.pressed(KeyCode::KeyR) && !online;
    // Edges are found here, not with `just_pressed`: that is cleared every rendered frame, and
    // a frame can pass without a tick.
    let demo_down = [
        !online && keys.any_pressed([KeyCode::F5, KeyCode::Digit1]),
        !online && keys.any_pressed([KeyCode::F6, KeyCode::Digit2]),
    ];
    let [record_pressed, play_pressed] =
        std::array::from_fn(|i| demo_down[i] && !demo_keys_held[i]);
    *demo_keys_held = demo_down;
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
    // The race starts: tick 0 for everyone.
    if let Some(net) = lobby.online.as_mut().filter(|net| net.fresh) {
        net.fresh = false;
        drive.demo = DemoMode::Off;
        restart = true;
    }
    if restart {
        let (driver, coll) = Drive::birth(
            &track,
            drive.character,
            drive.anim_num_frames,
            lobby.online.as_ref().map_or(0, |net| net.slot),
        );
        (drive.driver, drive.coll, drive.tick) = (driver, coll, 0);
        drive.held = 0;
        sim.restart();
        particles.list.clear();
        drive.camera = camera::CameraDc::default();
        drive.feedback = Default::default();
        for (mut motion, ours) in &mut motions {
            if ours {
                motion.reset(&drive.driver);
            }
        }
    }

    let mut input = match &drive.demo {
        DemoMode::Playing(demo) => demo.input(drive.tick, drive.held),
        _ => None,
    };
    if let Some(net) = &lobby.online {
        // Everyone's pad for this tick, or the sim waits for it.
        let Some(frame) = net.frames.get(drive.tick as usize) else {
            return;
        };
        input = Some(input_of_pad(&frame[net.slot], drive.held));
        // Ours for the tick INPUT_DELAY on.
        let ours = read_input(&keys, &pads, 0);
        lobby.send(net::Pad {
            held: ours.buttons_held,
            lx: ours.stick_lx,
            ly: ours.stick_ly,
            ry: ours.stick_ry,
        });
    }
    if input.is_none() {
        if matches!(drive.demo, DemoMode::Playing(_)) {
            info!("demo over after {} ticks", drive.tick);
            drive.demo = DemoMode::Off;
        }
        input = Some(read_input(&keys, &pads, drive.held));
    }
    let input = input.unwrap();
    if let DemoMode::Recording(demo) = &mut drive.demo {
        demo.record(drive.tick, &input);
    }
    drive.held = input.buttons_held;
    for mut text in &mut demo_label {
        text.0 = match &drive.demo {
            _ if online => lobby
                .online
                .as_ref()
                .map_or_else(String::new, |net| net.summary(drive.tick)),
            DemoMode::Off => String::new(),
            DemoMode::Recording(_) => format!("REC {}", drive.tick),
            DemoMode::Playing(demo) => format!("DEMO {}/{}", drive.tick, demo.length),
        };
    }
    let env = Env {
        elapsed_event_time: drive.tick as i32 * 32,
        timer: drive.tick,
        kart_collisions: crate::remote::kart_collisions(),
        ..Env::default()
    };
    // Everyone's karts play together, by slot, ours among them: each stage of the tick over all
    // the karts before the next, as the original runs its drivers.
    let frame = lobby
        .online
        .as_ref()
        .map(|net| &net.frames[drive.tick as usize][..]);
    let own = lobby.online.as_ref().map_or(0, |net| net.slot);
    let (result, effects) = remotes.play(
        crate::remote::Own {
            driver: &mut drive.driver,
            coll: &mut drive.coll,
            input: &input,
            slot: own,
        },
        frame,
        &env,
        &track,
        &mut sim,
        &mut particles,
        &mut drive.feedback,
        &mut motions,
    );
    drive.tick += 1;

    // The mask grab put the kart back on the track: the camera swings round behind it.
    if result.respawned {
        drive.camera.flags |= camera::flags::DIRECTION_CHANGED;
    }

    // A boost pulls the camera back.
    if effects
        .iter()
        .any(|e| matches!(e, Effect::CameraFireSpeedZoom { .. }))
    {
        drive.camera.flags |= camera::flags::FIRE_SPEED_ZOOM;
    }
    let camera_input = camera::Input {
        elapsed_time_ms: env.elapsed_time_ms,
        ..Default::default()
    };
    drive
        .camera
        .follow_driver(&drive.driver, &track.0, &camera_input);

    drive.shadow_previous = drive.shadow_current;
    drive.shadow_current = ground_shadow(&drive.driver, &mut drive.coll.gte.clone());
    drive.previous = drive.current;
    drive.current = pose_of(&drive.driver);
    for (mut motion, ours) in &mut motions {
        if ours {
            motion.push(&drive.driver);
        }
    }
    drive.previous_view = drive.current_view;
    drive.current_view = view_of(&drive.camera);
}

/// The kart between the last two ticks.
pub(crate) fn interpolated(drive: &Drive, fixed: &Time<Fixed>) -> Pose {
    let t = fixed.overstep_fraction();
    Pose {
        translation: drive
            .previous
            .translation
            .lerp(drive.current.translation, t),
        rotation: drive.previous.rotation.slerp(drive.current.rotation, t),
        scale: drive.previous.scale.lerp(drive.current.scale, t),
    }
}

/// The camera of the sim, between the last two ticks.
fn chase(
    drive: Res<Drive>,
    fixed: Res<Time<Fixed>>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    let t = fixed.overstep_fraction();
    let (a, b) = (drive.previous_view, drive.current_view);
    let eye = a.eye.lerp(b.eye, t);
    let target = a.target.lerp(b.target, t);
    // The roll's sign (which way the horizon tilts) is a guess.
    let roll = (a.roll + (b.roll - a.roll) * t) * std::f32::consts::TAU;
    for mut transform in &mut cameras {
        transform.translation = eye;
        transform.look_at(target, Vec3::Y);
        transform.rotate_local_z(roll);
    }
}

/// The wumpa count.
fn wumpa_hud(drive: Res<Drive>, mut text: Query<&mut Text, With<WumpaHud>>) {
    for mut text in &mut text {
        text.0 = format!("{}/10", drive.driver.num_wumpas);
    }
}

/// Frames per second and milliseconds per frame, smoothed over the last few frames.
fn fps_hud(diagnostics: Res<DiagnosticsStore>, mut text: Query<&mut Text, With<FpsHud>>) {
    let read = |path| diagnostics.get(path).and_then(|d| d.smoothed());
    let (Some(fps), Some(ms)) = (
        read(&FrameTimeDiagnosticsPlugin::FPS),
        read(&FrameTimeDiagnosticsPlugin::FRAME_TIME),
    ) else {
        return;
    };
    for mut text in &mut text {
        text.0 = format!("{fps:5.1} fps  {ms:5.2} ms");
    }
}

fn wumpa_bar(drive: Res<Drive>, mut bars: Query<&mut crate::bar::Bar, With<WumpaBar>>) {
    for mut bar in &mut bars {
        let value = drive.driver.num_wumpas as f32 / 10.0;
        // Only touch the bar when it changes, so its fill is not redone every frame.
        if bar.value != value {
            bar.value = value;
        }
    }
}

/// The boost timing bar, `MAX - current` of the drift's meter: empty outside a drift, full when
/// the window of the boost is reached.
fn turbo_bar(drive: Res<Drive>, mut bars: Query<&mut crate::bar::Bar, With<TurboBar>>) {
    let d = &drive.driver;
    let max = (d.stats.turbo_max_room as i32) << 5;
    let value = if d.phys_state == vehicle::PhysState::PowerSlide && max > 0 {
        ((max - d.turbo_meter_room_left as i32) as f32 / max as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    for mut bar in &mut bars {
        if bar.value != value {
            bar.value = value;
        }
    }
}

/// The boost reserves, always, up to 10000.
fn reserves_hud(drive: Res<Drive>, mut text: Query<&mut Text, With<ReservesHud>>) {
    for mut text in &mut text {
        text.0 = format!("{}", drive.driver.reserves);
    }
}

fn hud(drive: Res<Drive>, mut text: Query<&mut Text, With<Hud>>) {
    let d = &drive.driver;
    let grounded = d.actions_flag_set & vehicle::actions::TOUCH_GROUND != 0;
    for mut text in &mut text {
        text.0 = format!(
            "pos {} {} {}  (level units)\nspeed {:5}  {:?}{}{}{}{}\nW/S gas/brake, A/D steer, Space/Q hop+drift, M mask, R restart",
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
