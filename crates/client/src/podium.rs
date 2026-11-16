//! The podium of a cup's end, as retail shows it (`CS_Podium_FullScene_Init`): Gem Stone Valley
//! without its objects (`NO_LEV_INSTANCE`), the podium at the level's second recorded path, the
//! top three dancing on it and the trophy girl beside them, and the camera along the level's
//! camera path (`CS_Camera_ThTick_Podium`), which then stays at its end.
//!
//! The dancers follow the scripts of overlay 233 (`cutscene`), ticked at the game's 30 Hz, and
//! play the sound effects those ask for. Confetti falls over it all (`confetti.rs`), and the
//! winner's music plays (an XA track, as `CS_Podium_FullScene_Init` picks it).
//!
//! Every client of a lobby shows it when a cup ends: the host's final standings say who is on it
//! (`lobby::cup::podium`), and each shows it once its own race is over. Any key or button goes
//! back to the level and the mode it was shown from ("PRESS ANY KEY TO CONTINUE" once the camera
//! stops, where retail has "PRESS X TO CONTINUE"), and so does the start of the next race.
//! `/debug podium [first] [second] [third]` shows it outside a lobby (us and the next two
//! characters if none are named), and takes it away anywhere.
//!
//! The dancers whose scripts set a particle id give off the particles of overlay 233's tables
//! (`CS_Thread_Particles`): N. Gin's sparks, Joe's and N. Tropy's.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use cutscene::{Dancer, Event, Scripts};
use loader::dram::DramFile;
use loader::lev::{CameraPath, PosRot, SVec3};
use loader::level::LevelId;
use loader::model::Model;
use loader::source::{Need, Source};
use loader::vram::parse_vram_file;
use render::mesh::{self, PARTS, SCALE, to_world};
use render::particles::{DrawMode, Entry, Particle, Rng};

use crate::command::Action;
use crate::subtract::SubtractMaterial;
use crate::{LevelData, LevelNumber, Options, Textures, Track, toast};

/// The level the podium stands in.
const LEVEL: LevelId = LevelId::GEM_STONE_VALLEY;

/// `ptrSpawnType2_PosRot[1]`: the recorded path whose first point the podium stands on.
const STAND_PATH: usize = 1;

/// `PODIUM_SCENE_SPAWN_Y_OFFSET`: how far above that point the podium is.
const STAND_LIFT: i16 = 0x80;

/// How many points of the camera path pass a second: one a tick, as `elapsedTimeMS` (32 a tick)
/// over `CS_PODIUM_PATH_FRAME_UNIT` (32) makes it.
const PATH_POINTS_PER_SECOND: f32 = 30.0;

/// The game's ticks a second, and the milliseconds of one (`elapsedTimeMS`).
const TICKS_PER_SECOND: f32 = 30.0;
const TICK_MS: i32 = 32;

/// The most ticks one frame catches up on, after a stall.
const MAX_TICKS_PER_FRAME: u32 = 30;

/// `CS_Thread_Init` in an adventure hub: the dancers are drawn 4 slots of the ordering table nearer
/// than their origin (`depthBiasNormal -= 4`), over the podium's walls that they reach into.
const DANCER_DEPTH_BIAS: i32 = -4;

/// Where the dancers' particles' generator starts (any start will do).
const PARTICLE_SEED: u32 = 0x2545_f491;

/// Most particles one mesh of them draws.
const MAX_PARTICLES: usize = 256;

/// The character with no dance for the top step.
const NITROS_OXIDE: u8 = 15;

/// `sdata->randomNumber` as the scripts start (any start will do; every client has this one).
const RANDOM_START: i32 = 100;

/// How long the podium is up before a key takes it away: a key still being mashed at the end of
/// the race is not one to leave it with.
const KEYS_AFTER_SECONDS: f32 = 1.0;

/// The keys that do something else on the podium, which do not take it away: the command line,
/// the effects panel, fullscreen, and the modifiers (of an app switch, say).
const NOT_TO_CONTINUE: [KeyCode; 12] = [
    KeyCode::Enter,
    KeyCode::NumpadEnter,
    KeyCode::F3,
    KeyCode::F11,
    KeyCode::ShiftLeft,
    KeyCode::ShiftRight,
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
    KeyCode::AltLeft,
    KeyCode::AltRight,
    KeyCode::SuperLeft,
    KeyCode::SuperRight,
];

/// `CS_PODIUM_CONTINUE_TEXT_Y`: how far down the 216 lines of the screen the prompt is.
const CONTINUE_TOP: f32 = 0xbe as f32 / 216.0 * 100.0;

pub struct PodiumPlugin;

impl Plugin for PodiumPlugin {
    fn build(&self, app: &mut App) {
        // The scene changes before `Update`, whose systems were told by their run conditions
        // that the race (`Drive`) is there (as `scene::switch_level`).
        app.add_systems(
            PreUpdate,
            (toggle, continue_on_key, from_lobby)
                .chain()
                .after(crate::command::capture)
                .before(crate::scene::switch_level),
        )
        .add_systems(
            Update,
            (follow_path, dance, prompt, confetti, particles).chain(),
        );
    }
}

/// Who stands where: the top step, the two lower ones, and the trophy girl.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    First,
    Second,
    Third,
    Tawna,
}

impl Place {
    /// `PODIUM_*_POS` and `PODIUM_*_YAW_OFFSET`: where on the podium, in its own frame, and how
    /// far turned from it.
    fn spot(self) -> ([i16; 3], i16) {
        match self {
            Place::First => ([0, 0, 0], 0),
            Place::Second => ([-299, -42, 0], 0x200),
            Place::Third => ([299, -85, 0], 0x600),
            Place::Tawna => ([0x1a8, -128, 0x140], -0x2aa),
        }
    }
}

/// What a particle id makes each tick (`struct CsParticleConfig`, read).
struct Spawn {
    set: Vec<Entry>,
    /// The vertex of the dancer's pose they come from.
    vertex: usize,
    count: u8,
}

/// A dancer on the podium: its script, its model, and the meshes of the pose it shows.
struct Figure {
    place: Place,
    model: Model,
    dancer: Dancer,
    /// Where it stands (`Instance::matrix.t`), in game units.
    origin: [i32; 3],
    /// The same, in world units.
    at: Vec3,
    root: Option<Entity>,
    parts: [Option<Handle<Mesh>>; PARTS],
    /// The level of detail its meshes were built of.
    built: Option<usize>,
    /// Its poses of the last two ticks, the older first, which it is drawn between.
    poses: [Option<Pose>; 2],
}

/// A dancer's pose at a tick: what its script has it show, and its vertices for that.
#[derive(Clone)]
struct Pose {
    lod: usize,
    anim: usize,
    frame: i32,
    yaw: i16,
    /// The positions of the meshes' vertices (`mesh::model_positions`), made when first drawn.
    positions: Option<[Vec<[f32; 3]>; PARTS]>,
}

/// How far a dancer may turn in a tick and be turned smoothly between: a script that turns it
/// round at once (a range of another heading) is not swung through the turn.
const MOST_TURN_BLENDED: i32 = 0x100;

impl Pose {
    /// Whether `next` comes from this pose by playing on: the same animation, a frame on or back
    /// (or the same), and a small turn. A jump of the script, or a loop back to the start, is not
    /// blended across.
    fn plays_on_to(&self, next: &Pose) -> bool {
        let turn = ((next.yaw as i32 - self.yaw as i32 + 0x800) & 0xfff) - 0x800;
        self.lod == next.lod
            && self.anim == next.anim
            && (next.frame - self.frame).abs() <= 1
            && turn.abs() <= MOST_TURN_BLENDED
    }
}

/// The podium being shown.
#[derive(Resource)]
pub(crate) struct Podium {
    path: CameraPath,
    stand: Model,
    /// The point the podium stands on, and how it is turned.
    stand_at: PosRot,
    scripts: Scripts,
    figures: Vec<Figure>,
    /// `sdata->randomNumber`, as the scripts draw from it.
    rng: i32,
    /// How long the camera has been on its path, in seconds of the game's time (which stops while
    /// the level loads).
    elapsed: f32,
    /// The ticks the dancers have danced.
    ticks: u32,
    /// What each particle id of the scripts makes, and the particles there are.
    spawns: Vec<Vec<Spawn>>,
    particles: Vec<Particle>,
    particle_rng: Rng,
    confetti: crate::confetti::ConfettiView,
    /// The level and the mode to go back to.
    before: (u32, bool),
}

/// Where the podium is, `PODIUM_SCENE_SPAWN_Y_OFFSET` above its point.
fn stand_pos(at: PosRot) -> SVec3 {
    SVec3 {
        y: at.pos.y + STAND_LIFT,
        ..at.pos
    }
}

impl Podium {
    /// `CS_PODIUM_CAMERA_FRAME`: the point of its path the camera is at.
    fn camera_frame(&self) -> i32 {
        (self.ticks as i32).min(self.path.num_points() as i32 - 1)
    }
}

/// The camera that the podium's path moves.
#[derive(Component)]
pub(crate) struct PodiumCamera;

/// "PRESS ANY KEY TO CONTINUE".
#[derive(Component)]
struct Prompt;

/// `/debug podium`: shows the podium, or goes back from it. It is not shown in a lobby or a race,
/// whose level and racers are the race's.
fn toggle(
    mut actions: MessageReader<Action>,
    lobby: Res<crate::lobby::Lobby>,
    race_on: Res<crate::race_on::RaceOn>,
    podium: Option<Res<Podium>>,
    mut toasts: MessageWriter<toast::Toast>,
    mut commands: Commands,
) {
    let shown = podium.is_some();
    for action in actions.read() {
        let Action::DebugPodium(named) = action else {
            continue;
        };
        if !shown && (lobby.is_open() || race_on.is_on()) {
            toasts.write(
                toast::Toast::new("Not in a lobby or a race (/lobby close)", 2500)
                    .color(Color::srgb(1.0, 0.4, 0.4)),
            );
            continue;
        }
        let named = named.clone();
        commands.queue(move |world: &mut World| {
            // A scene that was just built is still settling (`loading.rs`).
            if world.contains_resource::<crate::loading::Loading>() {
                return;
            }
            if world.contains_resource::<Podium>() {
                leave(world);
                return;
            }
            // Us on top and the next two characters, where none are named.
            let racer = world.resource::<Options>().racer as u8;
            let n = crate::CHARACTERS.len() as u8;
            let mut places = [Some(racer), Some((racer + 1) % n), Some((racer + 2) % n)];
            if !named.is_empty() {
                places = [0, 1, 2].map(|i| named.get(i).copied());
            }
            if let Err(why) = enter(world, places) {
                world.write_message(
                    toast::Toast::new(format!("No podium: {why}"), 3000)
                        .color(Color::srgb(1.0, 0.4, 0.4)),
                );
            }
        });
    }
}

/// Any key or button, after the first moment, goes back from the podium (`CS_Camera_ThTick_Podium`
/// takes Start or Cross). Not what the command line is typing: it takes those keys first.
fn continue_on_key(
    podium: Option<Res<Podium>>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut commands: Commands,
) {
    if podium.is_none_or(|p| p.elapsed < KEYS_AFTER_SECONDS) {
        return;
    }
    let key = keys
        .get_just_pressed()
        .any(|key| !NOT_TO_CONTINUE.contains(key));
    let button = pads
        .iter()
        .any(|pad| pad.get_just_pressed().next().is_some());
    if key || button {
        commands.queue(leave);
    }
}

/// A cup's podium: shown once our race is over and the host's final standings came, and taken
/// away when the next race starts (before `scene::switch_level` goes to its track).
fn from_lobby(world: &mut World) {
    if world.contains_resource::<crate::loading::Loading>() {
        return;
    }
    let shown = world.contains_resource::<Podium>();
    if world.resource::<crate::race_on::RaceOn>().is_on() {
        if shown {
            leave(world);
        }
        return;
    }
    if shown {
        return;
    }
    let Some(places) = world.resource_mut::<crate::lobby::Lobby>().podium.take() else {
        return;
    };
    if let Err(why) = enter(world, places) {
        world.write_message(
            toast::Toast::new(format!("No podium: {why}"), 3000).color(Color::srgb(1.0, 0.4, 0.4)),
        );
    }
}

/// What the scripts' particle id `id` makes, read; none (and a warning) for what cannot be read.
fn spawns(scripts: &Scripts, id: i32) -> Vec<Spawn> {
    let read = || -> Result<Vec<Spawn>, String> {
        let bursts = scripts.particles(id).map_err(|e| e.to_string())?;
        bursts
            .into_iter()
            .map(|burst| {
                Ok(Spawn {
                    set: Entry::parse_set(scripts.overlay(), burst.emitters)?,
                    vertex: burst.frame_offset,
                    count: burst.count,
                })
            })
            .collect()
    };
    read().unwrap_or_else(|why| {
        warn!("the podium's particles {id}: {why}");
        Vec::new()
    })
}

/// `CS_Thread_Particles`: the particles of `figure`'s particle id, from its pose's vertex
/// (`CS_Instance_GetFrameData`: scaled as the model is, turned as the dancer is, at where it
/// stands), in 1/256 game units.
fn emit(figure: &Figure, tilt: SVec3, spawns: &[Spawn], rng: &mut Rng, out: &mut Vec<Particle>) {
    let Some(header) = figure.model.headers.first() else {
        return;
    };
    let (anim, frame) = (figure.dancer.anim as u8, figure.dancer.frame as i16);
    let Some(pose) = crate::drive::animation_pose(header, anim, frame) else {
        return;
    };
    let matrix = vehicle::rotmat::rot_to_matrix(SVec3 {
        y: figure.dancer.yaw(),
        ..tilt
    });
    let scale = cutscene::PODIUM_SCALE as i32;
    for spawn in spawns {
        let Some(vertex) = pose.vertices.get(spawn.vertex) else {
            continue;
        };
        let local =
            std::array::from_fn(|a| ((vertex[a] * scale) >> 12) * header.scale[a] as i32 >> 12);
        let turned = vehicle::exhaust::rotate(&matrix, local);
        for _ in 0..spawn.count {
            let mut particle = Particle::new(&spawn.set, rng);
            for a in 0..3 {
                particle.axes[a].start += (turned[a] + figure.origin[a]) << 8;
            }
            out.push(particle);
        }
    }
}

/// `CS_Podium_FullScene_Init`: the XA music track of the winner's crowd.
fn music_for(first: Option<u8>) -> u16 {
    match first {
        // Crash, Coco, Fake Crash.
        Some(0 | 3 | 14) => 10,
        // Polar, Pura.
        Some(6 | 7) => 7,
        // Cortex, N. Gin, N. Tropy.
        Some(1 | 4 | 12) => 8,
        // Papu, Roo, Penta.
        Some(9 | 10 | 13) => 9,
        // Pinstripe, Joe.
        Some(8 | 11) => 11,
        // Tiny's: Tiny, Dingo, Oxide.
        _ => 12,
    }
}

/// `Podium_InitModels`: the trophy girl that comes with the winner (`STATIC_TAWNA1` and on).
fn tawna_for(first: Option<u8>) -> u32 {
    match first {
        // Crash, Coco.
        Some(0 | 3) => 1,
        // Polar, Pura.
        Some(6 | 7) => 2,
        // Cortex, N. Gin.
        Some(1 | 4) => 3,
        _ => 0,
    }
}

fn model(source: &Source, need: Need) -> Result<Model, String> {
    let bytes = source.get(need).map_err(|e| e.to_string())?;
    DramFile::parse(&bytes)
        .and_then(|dram| Model::parse(&dram))
        .map_err(|e| format!("{}: {e}", need.key()))
}

/// Shows the podium with `places` on its steps, top first (none for an empty step).
fn enter(world: &mut World, places: [Option<u8>; 3]) -> Result<(), String> {
    if crate::maps::get(LEVEL.0).is_none() {
        return Err("Gem Stone Valley has not been made from the disc".into());
    }
    let LevelData {
        mut level,
        dram,
        mut vram,
    } = crate::load_level(LEVEL.0);
    let path = CameraPath::parse(&dram)
        .map_err(|e| e.to_string())?
        .ok_or("Gem Stone Valley has no camera path")?;
    let stand_at = level
        .paths
        .get(STAND_PATH)
        .and_then(|path| path.first())
        .copied()
        .ok_or("Gem Stone Valley has no place for it")?;
    let source = crate::disc::source();
    let pages = source.get(Need::PodiumVram).map_err(|e| e.to_string())?;
    for img in parse_vram_file(&pages).map_err(|e| e.to_string())? {
        vram.upload(&img).map_err(|e| e.to_string())?;
    }
    let stand = model(&source, Need::PodiumStand)?;
    let scripts = source
        .get(Need::CutsceneOverlay)
        .and_then(Scripts::parse)
        .map_err(|e| e.to_string())?;

    // As `CS_Podium_FullScene_Init` births them: third, second, first, the trophy girl. Each
    // draws its first repeats from the generator.
    let mut rng = RANDOM_START;
    let [first, second, third] = places;
    let mut figures = Vec::new();
    let mut add = |place: Place, need: Need, script: usize| -> Result<(), String> {
        let model = model(&source, need)?;
        let (spot, yaw) = place.spot();
        let dancer = Dancer::new(&scripts, script, stand_at.rot.y + yaw, &mut rng)
            .map_err(|e| e.to_string())?;
        // `CS_Thread_Init`: the spot turned as the podium is (its matrix is the GTE's light matrix
        // then), from where the podium is.
        let turned = vehicle::exhaust::rotate(
            &vehicle::rotmat::rot_to_matrix(stand_at.rot),
            spot.map(i32::from),
        );
        let base = stand_pos(stand_at);
        let origin = [
            turned[0] + base.x as i32,
            turned[1] + base.y as i32,
            turned[2] + base.z as i32,
        ];
        figures.push(Figure {
            place,
            model,
            dancer,
            origin,
            at: Vec3::from_array(origin.map(|v| v as f32)) * SCALE,
            root: None,
            parts: Default::default(),
            built: None,
            poses: [None, None],
        });
        Ok(())
    };
    let dance = |character: u8, first: bool| -> Result<(Need, usize), String> {
        let need = Need::Dance {
            character: character as u32,
            first,
        };
        let script = scripts
            .dance(character as usize, first)
            .map_err(|e| e.to_string())?;
        Ok((need, script))
    };
    for (place, character) in [(Place::Third, third), (Place::Second, second)] {
        if let Some(character) = character {
            let (need, script) = dance(character, false)?;
            add(place, need, script)?;
        }
    }
    // (Oxide has no dance for the top step: retail puts no one there.)
    if let Some(character) = first.filter(|&c| c != NITROS_OXIDE) {
        let (need, script) = dance(character, true)?;
        add(Place::First, need, script)?;
    }
    add(Place::Tawna, Need::Tawna(tawna_for(first)), scripts.tawna())?;
    level.instances.clear();

    let spawns = (0..cutscene::PARTICLE_IDS as i32)
        .map(|id| spawns(&scripts, id))
        .collect();

    let mut options = *world.resource::<Options>();
    let before = (world.resource::<LevelNumber>().0, options.driving);
    options.driving = false;
    world.insert_resource(options);
    world.insert_resource(Podium {
        path,
        stand,
        stand_at,
        scripts,
        figures,
        rng,
        elapsed: 0.0,
        ticks: 0,
        spawns,
        particles: Vec::new(),
        particle_rng: Rng::new(PARTICLE_SEED),
        confetti: crate::confetti::ConfettiView::podium(),
        before,
    });
    show(
        world,
        LEVEL.0,
        LevelData { level, dram, vram },
        options,
        true,
    );
    world
        .run_system_cached(spawn_stand)
        .expect("the podium's system runs");
    world
        .run_system_cached(spawn_prompt)
        .expect("the podium's system runs");
    world
        .resource::<crate::sound::Sounds>()
        .podium(music_for(first));
    // The kart stays where the level put it, hidden (`CS_Podium_FullScene_Init`).
    let kart: Vec<Entity> = world
        .query_filtered::<Entity, With<crate::scene::KartScene>>()
        .iter(world)
        .collect();
    for entity in kart {
        if let Some(mut visibility) = world.get_mut::<Visibility>(entity) {
            *visibility = Visibility::Hidden;
        }
    }
    Ok(())
}

fn leave(world: &mut World) {
    let Some(podium) = world.remove_resource::<Podium>() else {
        return;
    };
    world.resource::<crate::sound::Sounds>().podium_over();
    let (level, driving) = podium.before;
    // (A custom track can have been purged since.)
    let level = if crate::maps::get(level).is_some() {
        level
    } else {
        LevelId::DINGO_CANYON.0
    };
    let mut options = *world.resource::<Options>();
    options.driving = driving;
    world.insert_resource(options);
    show(world, level, crate::load_level(level), options, false);
}

/// Builds the scene of `level` in place of the one shown. The podium's has the scene's common
/// parts only: not the viewer's text.
fn show(world: &mut World, id: u32, data: LevelData, options: Options, podium: bool) {
    let scene = crate::world_scene(world);
    crate::despawn(world, scene);
    world.insert_resource(Track(data.level, LevelId(id)));
    world.insert_resource(Textures {
        dram: data.dram,
        vram: data.vram,
    });
    world.insert_resource(LevelNumber(id));
    crate::restart_drive(world, options);
    if podium {
        for system in world.resource::<crate::SceneSystems>().common.clone() {
            world.run_system(system).expect("scene system runs");
        }
    } else {
        crate::run_scene_systems(world, options);
    }
    crate::loading::begin(world);
}

/// The material of a model of the podium, of the atlas its mesh was built with.
fn material(atlas: mesh::Atlas, images: &mut Assets<Image>) -> StandardMaterial {
    let mut image = Image::new(
        Extent3d {
            width: atlas.width as u32,
            height: atlas.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(image)),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    }
}

/// `CS_Podium_Stand_Init`: the podium, on its point and turned as it is.
fn spawn_stand(
    podium: Res<Podium>,
    textures: Res<Textures>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    let Some(header) = podium.stand.headers.first() else {
        return;
    };
    let Some(frame) = crate::props::rest_pose(header) else {
        return;
    };
    let data = mesh::model_mesh(header, frame, &textures.vram, [0x1000; 3]);
    let base = material(data.atlas, &mut images);
    let transform = Transform::from_translation(Vec3::from(to_world(stand_pos(podium.stand_at))))
        .with_rotation(crate::props::rotation_of(podium.stand_at.rot));
    crate::spawn_parts(
        data.parts,
        &base,
        transform,
        None,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut subtract_materials,
        0,
    );
}

fn spawn_prompt(mut commands: Commands) {
    commands.spawn((
        Prompt,
        Visibility::Hidden,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(CONTINUE_TOP),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Pickable::IGNORE,
        children![(
            crate::font::CtrText::new(
                "PRESS ANY KEY TO CONTINUE",
                render::font::Font::Large,
                render::font::Gradient::ORANGE,
            )
            .fixed(crate::hud::PIXEL),
            Pickable::IGNORE,
        )],
    ));
}

/// Shows the prompt once the camera is at the end of its path (`CS_WAIT_INPUT`).
fn prompt(podium: Option<Res<Podium>>, mut prompts: Query<&mut Visibility, With<Prompt>>) {
    let Some(podium) = podium else {
        return;
    };
    let stopped = podium.elapsed * PATH_POINTS_PER_SECOND >= podium.path.num_points() as f32 - 1.0;
    for mut visibility in &mut prompts {
        *visibility = if stopped {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// A dancer not drawn: its script ended, or the camera is passing it (`CS_Thread_UseOpcode`).
fn hidden(figure: &Figure, camera_frame: i32) -> bool {
    figure.dancer.ended
        || match figure.place {
            Place::First => cutscene::FIRST_HIDDEN.contains(&camera_frame),
            Place::Second => cutscene::SECOND_HIDDEN.contains(&camera_frame),
            _ => false,
        }
}

/// The mesh of the dancers' particles of one way of blending, put up the first time.
#[derive(Component)]
struct Sparks {
    mode: DrawMode,
    width: i32,
    height: i32,
}

/// Lays the dancers' particles out as quads that face the view, between their last two ticks.
#[allow(clippy::too_many_arguments)]
fn particles(
    podium: Option<Res<Podium>>,
    textures: Res<Textures>,
    smoke: Res<crate::SmokeIcon>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut billboard: ResMut<Assets<crate::billboard::BillboardMaterial>>,
    mut subtract: ResMut<Assets<crate::billboard::BillboardSubtractMaterial>>,
    sparks: Query<(&Sparks, &Mesh3d, &Transform)>,
) {
    use crate::billboard::{ATTRIBUTE_CORNER, Billboard, BillboardSubtract};
    use crate::exhaust::{SCREEN_PIXEL_ASPECT, icon_image, icon_image_blended};
    let Some(podium) = podium else {
        return;
    };
    let Some(icon) = &smoke.0 else {
        return;
    };
    if sparks.is_empty() {
        let mut mesh = || {
            let quads = MAX_PARTICLES;
            let mut mesh = Mesh::new(
                bevy::mesh::PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4 * quads]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4 * quads]);
            mesh.insert_attribute(ATTRIBUTE_CORNER, vec![[0.0f32; 2]; 4 * quads]);
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_UV_0,
                (0..quads)
                    .flat_map(|_| [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]])
                    .collect::<Vec<_>>(),
            );
            mesh.insert_indices(bevy::mesh::Indices::U32(
                (0..quads as u32)
                    .flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i))
                    .collect(),
            ));
            meshes.add(mesh)
        };
        let (image, width, height) = icon_image(icon, &textures.vram);
        let image = images.add(image);
        let average = images.add(icon_image_blended(icon, &textures.vram, 128).0);
        let base = |texture: &Handle<Image>, alpha_mode| StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(texture.clone()),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            alpha_mode,
            ..default()
        };
        // (They sort as one mesh, by the podium's middle.)
        let at = Transform::from_translation(Vec3::from(to_world(stand_pos(podium.stand_at))));
        let sparks = |mode| Sparks {
            mode,
            width,
            height,
        };
        commands.spawn((
            sparks(DrawMode::Add),
            Mesh3d(mesh()),
            MeshMaterial3d(billboard.add(crate::billboard::BillboardMaterial {
                base: base(&image, AlphaMode::Add),
                extension: Billboard {},
            })),
            at,
            bevy::camera::visibility::NoFrustumCulling,
        ));
        commands.spawn((
            sparks(DrawMode::Average),
            Mesh3d(mesh()),
            MeshMaterial3d(billboard.add(crate::billboard::BillboardMaterial {
                base: base(&average, AlphaMode::Blend),
                extension: Billboard {},
            })),
            at,
            bevy::camera::visibility::NoFrustumCulling,
        ));
        commands.spawn((
            sparks(DrawMode::Subtract),
            Mesh3d(mesh()),
            MeshMaterial3d(subtract.add(crate::billboard::BillboardSubtractMaterial {
                base: base(&image, AlphaMode::Blend),
                extension: BillboardSubtract {},
            })),
            at,
            bevy::camera::visibility::NoFrustumCulling,
        ));
        return;
    }
    let t = (podium.elapsed * TICKS_PER_SECOND - podium.ticks as f32).clamp(0.0, 1.0);
    for (sparks, mesh, transform) in &sparks {
        let quads = MAX_PARTICLES;
        let mut positions = vec![[0.0f32; 3]; 4 * quads];
        let mut colors = vec![[0.0f32; 4]; 4 * quads];
        let mut corners_out = vec![[0.0f32; 2]; 4 * quads];
        let wanted = (podium.particles.iter())
            .filter(|p| !p.is_line() && p.draw_mode() == sparks.mode)
            .take(quads);
        for (n, p) in wanted.enumerate() {
            let Some(corners) = p.corners(sparks.width, sparks.height) else {
                continue;
            };
            let (position, velocity) = (p.position(), p.velocity());
            let at = Vec3::from_array(std::array::from_fn(|a| {
                position[a] as f32 + velocity[a] as f32 * t
            }));
            let centre = at / 256.0 * SCALE - transform.translation;
            let gain = p.rgb().map(|c| c as f32 / 128.0);
            for (k, [x, y]) in corners.into_iter().enumerate() {
                positions[n * 4 + k] = centre.to_array();
                corners_out[n * 4 + k] = [
                    render::particles::quarter_units_to_world(x as f32),
                    -(render::particles::quarter_units_to_world(y as f32) * SCREEN_PIXEL_ASPECT),
                ];
                colors[n * 4 + k] = [gain[0], gain[1], gain[2], 1.0];
            }
        }
        crate::mesh_write::set_attributes(
            &mut meshes,
            &mesh.0,
            [
                (Mesh::ATTRIBUTE_POSITION, positions.into()),
                (Mesh::ATTRIBUTE_COLOR, colors.into()),
                (ATTRIBUTE_CORNER, corners_out.into()),
            ],
        );
    }
}

/// The confetti between its last two ticks.
fn confetti(
    podium: Option<Res<Podium>>,
    camera: Query<&Transform, With<PodiumCamera>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut confetti: Query<(&Mesh3d, &mut Visibility), With<crate::confetti::ConfettiMesh>>,
) {
    let Some(podium) = podium else {
        return;
    };
    let t = (podium.elapsed * TICKS_PER_SECOND - podium.ticks as f32).clamp(0.0, 1.0);
    crate::confetti::show(
        &podium.confetti,
        t,
        camera.iter().next(),
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut confetti,
    );
}

/// Moves the camera along the path, between its points as the frame falls between two ticks, and
/// holds it at the end. A level shown since (`/track`, `[`) ends the podium.
fn follow_path(
    mut commands: Commands,
    podium: Option<ResMut<Podium>>,
    level: Res<LevelNumber>,
    time: Res<Time>,
    mut cameras: Query<&mut Transform, With<PodiumCamera>>,
) {
    let Some(mut podium) = podium else {
        return;
    };
    if level.0 != LEVEL.0 {
        commands.remove_resource::<Podium>();
        return;
    }
    podium.elapsed += time.delta_secs();
    let Some(transform) = camera_at(&podium.path, podium.elapsed * PATH_POINTS_PER_SECOND) else {
        return;
    };
    for mut camera in &mut cameras {
        *camera = transform;
    }
}

/// What a dancer's model has, that its script goes by.
struct Frames<'a>(&'a Model);

impl cutscene::Frames for Frames<'_> {
    fn num_frames(&self, lod: usize, anim: usize) -> i32 {
        self.0
            .headers
            .get(lod)
            .and_then(|h| h.animations.get(anim))
            .map_or(0, |a| a.num_frames as i32)
    }

    fn lods(&self) -> usize {
        self.0.headers.len()
    }
}

/// Runs the dancers' scripts for the ticks that have passed (`CS_Thread_ThTick`), plays the
/// sounds they ask for, and shows each in its pose: turned as its script turns it, and hidden
/// once its script ended or while the camera passes it (`CS_Thread_UseOpcode`).
#[allow(clippy::too_many_arguments)]
fn dance(
    podium: Option<ResMut<Podium>>,
    textures: Res<Textures>,
    sounds: Res<crate::sound::Sounds>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
    mut roots: Query<(&mut Transform, &mut Visibility)>,
) {
    let Some(mut podium) = podium else {
        return;
    };
    let podium = &mut *podium;
    let due = (podium.elapsed * TICKS_PER_SECOND) as u32;
    let mut events = Vec::new();
    let tilt = podium.stand_at.rot;
    for _ in 0..due.saturating_sub(podium.ticks).min(MAX_TICKS_PER_FRAME) {
        let camera_frame = podium.camera_frame();
        // The last born ticks first, as the thread list has them.
        for figure in podium.figures.iter_mut().rev() {
            let frames = Frames(&figure.model);
            let ticked = figure.dancer.tick(
                &podium.scripts,
                &frames,
                TICK_MS,
                &mut podium.rng,
                &mut events,
            );
            if let Err(why) = ticked {
                warn!("the podium's {:?} stops: {why}", figure.place);
                figure.dancer.ended = true;
            }
            let lod = figure
                .dancer
                .lod()
                .min(figure.model.headers.len().saturating_sub(1));
            let pose = Pose {
                lod,
                anim: figure.dancer.anim,
                frame: figure.dancer.frame,
                yaw: figure.dancer.yaw(),
                positions: None,
            };
            figure.poses = [figure.poses[1].take(), Some(pose)];
            let id = figure.dancer.particle_id;
            if let Some(spawns) = podium.spawns.get(id as u8 as usize)
                && !hidden(figure, camera_frame)
            {
                emit(
                    figure,
                    tilt,
                    spawns,
                    &mut podium.particle_rng,
                    &mut podium.particles,
                );
            }
        }
        // (After the threads, as retail updates them: a new particle starts where it was made.)
        podium.particles.retain_mut(|p| p.update());
        podium.ticks += 1;
        // The confetti is where the camera is (`DrawConfetti` goes by the push buffer).
        if let Some((camera, _)) = podium.path.at(podium.camera_frame()) {
            podium.confetti.step_at(camera.pos, camera.rot.y);
        }
    }
    podium.ticks = podium.ticks.max(due.saturating_sub(MAX_TICKS_PER_FRAME));
    for event in events {
        match event {
            Event::Play(id) => sounds.play_fx(id),
            Event::Stop(id) => sounds.stop_fx(id),
        }
    }

    let camera_frame = podium.camera_frame();
    // How far between the last two ticks the frame is.
    let t = (podium.elapsed * TICKS_PER_SECOND - podium.ticks as f32).clamp(0.0, 1.0);
    for figure in &mut podium.figures {
        // (Before its first tick it is not drawn.)
        let Some(lod) = figure.poses[1].as_ref().map(|pose| pose.lod) else {
            continue;
        };
        let Some(header) = figure.model.headers.get(lod) else {
            continue;
        };
        if figure.built != Some(lod) {
            if let Some(root) = figure.root.take() {
                commands.entity(root).despawn();
            }
            let Some(pose) = crate::drive::animation_pose(header, 0, 0) else {
                continue;
            };
            let scale = [cutscene::PODIUM_SCALE; 3];
            let data = mesh::model_mesh(header, &pose, &textures.vram, scale);
            let base = material(data.atlas, &mut images);
            let root = commands
                .spawn((
                    Transform::from_translation(figure.at),
                    Visibility::Hidden,
                    crate::ordered::OrderedAsOne {
                        bias: DANCER_DEPTH_BIAS,
                    },
                ))
                .id();
            figure.parts = crate::spawn_parts(
                data.parts,
                &base,
                Transform::default(),
                Some(root),
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut subtract_materials,
                0,
            );
            figure.root = Some(root);
            figure.built = Some(lod);
        }
        let Some(root) = figure.root else {
            continue;
        };
        let hidden = hidden(figure, camera_frame);
        // Its vertices at both ticks, where it played on from the older pose; at the newer only
        // where it did not.
        let [older, newer] = &mut figure.poses;
        let Some(newer) = newer.as_mut() else {
            continue;
        };
        let older = older.as_mut().filter(|older| older.plays_on_to(newer));
        let positions_of = |pose: &mut Pose| -> Option<[Vec<[f32; 3]>; PARTS]> {
            if pose.positions.is_none() {
                let frame =
                    crate::drive::animation_pose(header, pose.anim as u8, pose.frame as i16)?;
                let scale = [cutscene::PODIUM_SCALE; 3];
                pose.positions = Some(mesh::model_positions(header, &frame, scale));
            }
            pose.positions.clone()
        };
        let Some(to) = positions_of(newer) else {
            continue;
        };
        let to_yaw = newer.yaw;
        let (from, from_yaw) = match older {
            Some(older) => (positions_of(older), older.yaw),
            None => (None, to_yaw),
        };
        let positions = match from {
            Some(from) if t < 1.0 => std::array::from_fn(|part| {
                (from[part].iter().zip(&to[part]))
                    .map(|(a, b)| std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t))
                    .collect::<Vec<_>>()
            }),
            _ => to,
        };
        for (handle, positions) in figure.parts.iter().zip(positions) {
            if let Some(mut mesh) = handle.as_ref().and_then(|h| meshes.get_mut(h)) {
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            }
        }
        // (A root spawned this frame is not in the world yet: it is shown from the next.)
        if let Ok((mut transform, mut visibility)) = roots.get_mut(root) {
            *visibility = if hidden {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            let turned = |yaw| crate::props::rotation_of(SVec3 { y: yaw, ..tilt });
            transform.rotation = turned(from_yaw).slerp(turned(to_yaw), t);
        }
    }
}

/// The camera at `frame` points along `path` (a fraction is between two), held at its last.
fn camera_at(path: &CameraPath, frame: f32) -> Option<Transform> {
    let last = path.num_points() as i32 - 1;
    let frame = frame.clamp(0.0, last.max(0) as f32);
    let i = frame.floor() as i32;
    let t = frame - i as f32;
    let (a, _) = path.at(i)?;
    let (b, _) = path.at((i + 1).min(last))?;
    let v = |p: SVec3| Vec3::new(p.x as f32, p.y as f32, p.z as f32);
    // An angle goes the short way round, through the wrap at a full turn.
    let turn = |a: f32, b: f32| a + ((b - a + 2048.0).rem_euclid(4096.0) - 2048.0) * t;
    let (ra, rb) = (v(a.rot), v(b.rot));
    let rot = Vec3::new(turn(ra.x, rb.x), turn(ra.y, rb.y), turn(ra.z, rb.z));
    Some(crate::drive::push_buffer_view(v(a.pos).lerp(v(b.pos), t), rot).transform())
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::lev::CameraRun;

    fn path(rots: &[i16]) -> CameraPath {
        CameraPath {
            runs: vec![CameraRun {
                flags: 0,
                points: rots
                    .iter()
                    .enumerate()
                    .map(|(i, &y)| PosRot {
                        pos: SVec3 {
                            x: i as i16 * 100,
                            y: 0,
                            z: 0,
                        },
                        // Stored as `CAM_Path_Move` reads it: sixteen times the angle.
                        rot: SVec3 {
                            x: 0,
                            y: y << 4,
                            z: 0,
                        },
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn the_camera_is_between_two_points_and_held_at_the_last() {
        let path = path(&[0, 0, 0]);
        let x = |frame| camera_at(&path, frame).unwrap().translation.x;
        assert_eq!(x(0.5), x(0.0) + (x(1.0) - x(0.0)) / 2.0);
        assert_eq!(x(2.0), x(10.0));
        assert_eq!(x(-1.0), x(0.0));
    }

    #[test]
    fn an_angle_turns_the_short_way_through_the_wrap() {
        // From just short of a full turn to just past it: through 0, not back round.
        let path = path(&[0xff0, 0x010]);
        let middle = camera_at(&path, 0.5).unwrap().rotation;
        let zero = camera_at(&self::path(&[0, 0]), 0.0).unwrap().rotation;
        assert!(middle.angle_between(zero) < 1e-3, "{middle:?}");
    }

    #[test]
    fn a_pose_is_blended_into_the_next_only_where_it_plays_on() {
        let pose = |anim, frame, yaw| Pose {
            lod: 0,
            anim,
            frame,
            yaw,
            positions: None,
        };
        let at = pose(0, 10, 0x800);
        assert!(at.plays_on_to(&pose(0, 11, 0x810)));
        assert!(at.plays_on_to(&pose(0, 9, 0x800)));
        assert!(at.plays_on_to(&pose(0, 10, 0x800)));
        // Another animation, a jump, a loop back to the start, a turn round, another level of
        // detail: each snaps.
        assert!(!at.plays_on_to(&pose(1, 11, 0x800)));
        assert!(!at.plays_on_to(&pose(0, 40, 0x800)));
        assert!(!at.plays_on_to(&pose(0, 0, 0x800)));
        assert!(!at.plays_on_to(&pose(0, 11, 0)));
        assert!(!at.plays_on_to(&Pose {
            lod: 1,
            ..pose(0, 11, 0x800)
        }));
        // A small turn through the wrap of a full turn is blended.
        assert!(pose(0, 10, 0xff0).plays_on_to(&pose(0, 11, 0x010)));
    }

    #[test]
    fn the_winner_picks_the_music() {
        assert_eq!(music_for(Some(0)), 10);
        assert_eq!(music_for(Some(7)), 7);
        assert_eq!(music_for(Some(12)), 8);
        assert_eq!(music_for(Some(13)), 9);
        assert_eq!(music_for(Some(11)), 11);
        assert_eq!(music_for(Some(5)), 12);
        assert_eq!(music_for(Some(15)), 12);
    }

    #[test]
    fn the_winner_brings_the_trophy_girl() {
        assert_eq!(tawna_for(Some(0)), 1);
        assert_eq!(tawna_for(Some(6)), 2);
        assert_eq!(tawna_for(Some(4)), 3);
        assert_eq!(tawna_for(Some(2)), 0);
        assert_eq!(tawna_for(None), 0);
    }
}
