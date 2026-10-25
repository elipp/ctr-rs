//! Fly-through viewer for a track's geometry (textured).
//!
//! `view [LEVEL_ID] [--lo] [--racer N | --no-racer]` — level id as in
//! `loader::level::LevelId` (default 0); `--racer N` puts racer model N (0 Crash,
//! 1 Cortex, ... as `GAME_CHARACTER_IDS`; default 0) at the first starting grid slot;
//! `--lo` shows the `color_lo` vertex colours; `--no-cull` draws the level's back faces, which the game does not; `--show-hidden` keeps the
//! blocks without textures and the faces with a pure red corner, which are hidden by default. The disc image is `CTR_BIN`
//! or `assets/ctr-u.bin`.
//!
//! A collision probe from the camera is drawn and described on screen; see
//! `probe.rs` for its keys.
//!
//! Controls: hold the right mouse button to look, WASD to move, Space/Shift
//! up/down, Ctrl to go fast, mouse wheel to change the base speed.
//!
//! `--drive` drives the racer instead, see `drive.rs`.
//!
//! `]` / Page Up and `[` / Page Down switch to the next and the previous level; `.` and `,` to the
//! next and the previous racer.

// Bevy systems take their parameters and `Query` filters as types, so both lints fire on them.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

mod banner;
mod bar;
mod command;
mod drive;
mod exhaust;
mod explosion;
mod fire;
mod flame;
mod flash;
mod kart;
mod leaderboard;
mod lobby;
mod maps;
mod mask;
mod probe;
mod projectile;

mod props;
mod rain;
mod remote;
mod shadow;
mod shield;
mod skid;
mod sky;
mod subtract;
mod toast;
mod weapon;
mod wheels;
mod wumpa_icon;

use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::image::ImageSampler;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::disc::open_bigfile;
use loader::dram::DramFile;
use loader::icons::{Icon, parse_groups, parse_icons};
use loader::lev::Level;
use loader::level::LevelId;
use loader::model::Model;
use loader::vram::{Vram, parse_vram_file};
use render::mesh::{self, PARTS, Part, to_world};
use subtract::SubtractMaterial;

/// `NITROS_OXIDE` in `GAME_CHARACTER_IDS` order.
const NITROS_OXIDE: u32 = 15;

/// What a level is made of: its geometry, and the textures its pointers need (the level's own
/// and `shared.vrm`, which the racers' are in).
struct LevelData {
    level: Level,
    dram: DramFile,
    vram: Vram,
}

/// What a racer is made of: everything that comes from the character's own files. It can be
/// loaded again without the level.
#[derive(Default)]
struct Character {
    racer: Option<Model>,
    /// The icons of the racer's wheel sprites (`tireAnim`); none for Oxide, who has no wheels.
    wheel_icons: Vec<Icon>,
    /// Global icons 0 and 1 of the racer's pack (`shadow1`, `shadow2`).
    shadow_icons: Option<[Icon; 2]>,
    /// The first picture of the `dustpuff` icon group: the exhaust smoke.
    smoke_icon: Option<Icon>,
    /// Global icon `tiremark` of the racer's pack: the skid marks.
    tiremark_icon: Option<Icon>,
    /// `turbo` .. `turbo7` of the racer's pack: the frames of the exhaust fire.
    flame_models: Vec<Model>,
}

impl Character {
    /// Makes this the racer on the grid: the resources the kart is spawned from.
    fn insert(self, world: &mut World) {
        world.insert_resource(Racer(self.racer));
        world.insert_resource(WheelIcons(self.wheel_icons));
        world.insert_resource(ShadowIcons(self.shadow_icons));
        world.insert_resource(SmokeIcon(self.smoke_icon));
        world.insert_resource(TiremarkIcon(self.tiremark_icon));
        world.insert_resource(FlameModels(self.flame_models));
    }
}

/// What every racer's pack holds the same of, loaded once: neither a level nor a character
/// changes it.
struct Shared {
    /// `crate_explosion`: the break-up of a crate.
    crate_explosion: Option<Model>,
    /// The Aku Aku and Uka Uka masks.
    masks: render::mask::Masks,
    /// The shield bubble.
    shields: render::shield::Shields,
    /// The bombs, missiles and their bursts.
    projectiles: render::projectile::Models,
    /// The wumpa fruit of the counter, for a level that has none of its own.
    fruit_display: Model,
}

fn bigfile_path() -> PathBuf {
    std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin"))
}

/// The model and the wheel sprites of a racer (none for Oxide, who has no wheels), for the
/// karts of the other players.
pub(crate) fn load_racer(character: usize) -> (Model, Vec<Icon>) {
    let path = bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let file = big
        .read(
            &mut disc,
            BigIndex::RACER_MODEL_HI.nth(character as u32).get(),
        )
        .expect("read racer model");
    let model =
        Model::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse racer model");
    let wheel_icons = if character as u32 == NITROS_OXIDE {
        Vec::new()
    } else {
        let pack = big
            .read(
                &mut disc,
                BigIndex::ARCADE_1P_PACK.nth(character as u32).get(),
            )
            .expect("read racer pack");
        parse_groups(&DramFile::parse(&pack).expect("parse racer pack"))
            .expect("parse racer pack icons")
            .into_iter()
            .find(|g| g.group_id == 0)
            .expect("racer pack has the tire icon group")
            .icons
    };
    (model, wheel_icons)
}

/// A level from the disc, with the texture page file shared by every track (`shared.vrm`): the
/// racers' textures are in it.
fn load_level(id: u32) -> LevelData {
    let path = bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // TODO: a player who joins a race on a map they do not have should be sent it (see TODO.md).
    assert!(
        !map::is_map_id(id) || maps::get(id).is_some(),
        "map {id:#x} is not in assets/maps"
    );
    if let Some(map) = maps::get(id) {
        // A map brings its own level and texture pages; the shared ones are the disc's.
        let mut vram = Vram::default();
        for img in &map.vram {
            vram.upload(img).expect("upload VRAM image");
        }
        upload_shared(&mut vram, &big, &mut disc);
        return LevelData {
            level: map.level.clone(),
            dram: map.dram.clone(),
            vram,
        };
    }
    let mut read = |f| {
        let index = BigIndex::level(LevelId(id), Lod::OnePlayer, f);
        big.read(&mut disc, index.get())
            .expect("read level file from BIGFILE")
    };
    let dram = DramFile::parse(&read(LevFile::Lev)).expect("parse DRAM file");
    let level = Level::parse(&dram).expect("parse LEV");
    let mut vram = Vram::default();
    for img in parse_vram_file(&read(LevFile::Vram)).expect("parse VRAM file") {
        vram.upload(&img).expect("upload VRAM image");
    }
    upload_shared(&mut vram, &big, &mut disc);
    LevelData { level, dram, vram }
}

/// The texture pages that every track shares (`shared.vrm`).
fn upload_shared(vram: &mut Vram, big: &loader::bigfile::BigFile, disc: &mut loader::disc::Disc) {
    let shared = big
        .read(disc, BigIndex::SHARED_MPK_VRM.get())
        .expect("read shared VRAM file");
    for img in parse_vram_file(&shared).expect("parse shared VRAM file") {
        vram.upload(&img).expect("upload shared VRAM image");
    }
}

/// A racer from the disc: its model, and what its pack has of the kart's. Nothing for `None`
/// (no racer).
fn load_character(character: Option<u32>) -> Character {
    let Some(character) = character else {
        return Character::default();
    };
    let path = bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let file = big
        .read(&mut disc, BigIndex::RACER_MODEL_HI.nth(character).get())
        .expect("read racer model");
    let racer =
        Model::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse racer model");
    let pack = big
        .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
        .expect("read racer pack");
    let pack = DramFile::parse(&pack).expect("parse racer pack");
    let icons = parse_icons(&pack).expect("parse racer pack icons");
    let groups = parse_groups(&pack).expect("parse racer pack icons");
    let global = |index| icons.iter().find(|i| i.global_index == index).cloned();
    let group = |id| groups.iter().find(|g| g.group_id == id);
    Character {
        wheel_icons: if character == NITROS_OXIDE {
            Vec::new()
        } else {
            group(0)
                .expect("racer pack has the tire icon group")
                .icons
                .clone()
        },
        shadow_icons: global(0).zip(global(1)).map(|(a, b)| [a, b]),
        smoke_icon: group(1).and_then(|g| g.icons.first().cloned()),
        tiremark_icon: global(skid::TIREMARK_ICON),
        flame_models: flame::models(&pack).expect("racer pack has the turbo models"),
        racer: Some(racer),
    }
}

/// What the racers' packs share. (Character 0's pack will do.)
fn load_shared() -> Shared {
    let path = bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let pack = big
        .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(0).get())
        .expect("read racer pack");
    let pack = DramFile::parse(&pack).expect("parse racer pack");
    Shared {
        crate_explosion: render::explosion::model(&pack)
            .expect("racer pack has the crate explosion"),
        masks: render::mask::models(&pack).expect("racer pack has the masks"),
        shields: render::shield::models(&pack).expect("racer pack has the shields"),
        projectiles: render::projectile::models(&pack).expect("racer pack has the weapons"),
        fruit_display: render::fruit::display_model(&pack).expect("racer pack has the fruit"),
    }
}

#[derive(Resource)]
pub struct Track(pub Level, pub LevelId);

#[derive(Resource)]
pub(crate) struct Textures {
    dram: DramFile,
    pub(crate) vram: Vram,
}

/// The racer model shown on the grid.
#[derive(Resource)]
struct Racer(Option<Model>);

#[derive(Resource)]
struct WheelIcons(Vec<Icon>);

#[derive(Resource)]
struct ShadowIcons(Option<[Icon; 2]>);

#[derive(Resource)]
struct SmokeIcon(Option<Icon>);

#[derive(Resource)]
struct TiremarkIcon(Option<Icon>);

#[derive(Resource)]
struct FlameModels(Vec<Model>);

#[derive(Resource)]
struct LowColors(bool);

/// The viewer's switches that are not about the data: `--show-hidden` and `--no-cull`.
#[derive(Resource)]
struct ShowHidden {
    show_hidden: bool,
    /// Whether the level's back faces are culled, as the game does.
    cull: bool,
}

/// The racers in `GAME_CHARACTER_IDS` order.
const CHARACTERS: [&str; 16] = [
    "Crash",
    "Cortex",
    "Tiny",
    "Coco",
    "N. Gin",
    "Dingo",
    "Polar",
    "Pura",
    "Pinstripe",
    "Papu",
    "Roo",
    "Joe",
    "N. Tropy",
    "Penta",
    "Fake Crash",
    "Oxide",
];

/// What the command line asked for, to load the next level the same way.
#[derive(Resource, Clone, Copy)]
struct Options {
    /// The character of the racer on the grid, none for `--no-racer`.
    racer: Option<u32>,
    driving: bool,
}

/// The level shown, a `LevelId`.
#[derive(Resource)]
struct LevelNumber(u32);

/// The systems that build the scene, to run again for another level: `setup`, then the text overlay.
#[derive(Resource)]
struct SceneSystems(Vec<bevy::ecs::system::SystemId>);

/// A drive-mode state for `track`: the kart on its first starting slot.
fn new_drive(track: &Track, racer: &Option<Model>, character: usize) -> drive::Drive {
    let model = racer.as_ref().expect("--drive needs a racer");
    let mut anim_num_frames = [0; 8];
    for (n, anim) in anim_num_frames.iter_mut().zip(&model.headers[0].animations) {
        *n = anim.num_frames;
    }
    let spec = race::KartSpec {
        character,
        anim_num_frames,
    };
    let kart = race::Kart::birth(&track.0, 0, &spec);
    drive::Drive::new(track, &kart.driver, character, anim_num_frames)
}

#[derive(Component)]
struct FlyCamera {
    yaw: f32,
    pitch: f32,
    speed: f32,
}

fn main() {
    maps::load();
    let track_id = 0;
    let mut racer = Some(0);
    let mut low = false;
    let mut show_hidden = false;
    let mut cull = true;
    let mut driving = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lo" => low = true,
            "--drive" => driving = true,
            "--show-hidden" => show_hidden = true,
            "--no-cull" => cull = false,
            "--no-racer" => racer = None,
            "--racer" => {
                racer = Some(
                    args.next()
                        .and_then(|n| n.parse().ok())
                        .filter(|&n| n < 16)
                        .expect("--racer takes a number below 16"),
                )
            }
            n => {
                panic!(
                    "usage: view [--lo] [--show-hidden] [--no-cull] [--drive] [--racer N | --no-racer], got '{n}'"
                )
            }
        }
    }

    let character = racer.unwrap_or(0) as usize;
    let options = Options { racer, driving };
    let LevelData { level, dram, vram } = load_level(track_id);
    let character_data = load_character(racer);
    let Shared {
        crate_explosion,
        masks,
        shields,
        projectiles,
        fruit_display,
    } = load_shared();
    let loaded = Track(level, LevelId(track_id));
    let textures = Textures { dram, vram };
    let drive = driving.then(|| new_drive(&loaded, &character_data.racer, character));

    // See the executor below: without a render thread, `CTR_EXECUTOR=multi` keeps the defaults.
    let single_threaded = std::env::var("CTR_EXECUTOR").as_deref() != Ok("multi");
    let mut app = App::new();
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: match options.racer {
                Some(racer) => format!(
                    "ctr-rs view: {}, {}",
                    maps::name(track_id),
                    CHARACTERS[racer as usize]
                ),
                None => format!("ctr-rs view: {}", maps::name(track_id)),
            },
            // The frame rate is `limit_frame_rate`'s.
            // present_mode: bevy::window::PresentMode::AutoNoVsync,
            ..default()
        }),
        ..default()
    });
    #[cfg(target_os = "macos")]
    let plugins = if single_threaded {
        // The render thread would run the single-threaded schedules off the main thread, which
        // Metal's window surface does not allow.
        plugins.disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
    } else {
        plugins
    };
    app.add_plugins(plugins);
    // Bevy's own font has no ä or ö: this one takes its place as the default.
    app.world_mut()
        .resource_mut::<Assets<Font>>()
        .insert(
            AssetId::default(),
            Font::from_bytes(include_bytes!("../../../assets/Inconsolata-Regular.ttf").to_vec()),
        )
        .expect("the default font can be replaced");
    app.insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.1)))
        .insert_resource(loaded)
        .insert_resource(textures)
        .insert_resource(explosion::CrateExplosion::new(crate_explosion))
        .insert_resource(mask::MaskModels(Some(masks)))
        .insert_resource(shield::ShieldModels(Some(shields)))
        .insert_resource(wumpa_icon::FruitDisplay(fruit_display))
        .insert_resource(projectile::ProjectileModels::new(projectiles))
        .insert_resource(options)
        .insert_resource(LevelNumber(track_id))
        .insert_resource(LowColors(low))
        .insert_resource(ShowHidden { show_hidden, cull })
        .add_systems(Startup, (setup, spawn_kart).chain())
        .add_systems(Update, log_draws)
        // .add_systems(Last, limit_frame_rate)
        // After the cameras and karts have moved this frame, before their transforms propagate.
        .add_systems(
            PostUpdate,
            wheels::update.before(TransformSystems::Propagate),
        )
        .add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default())
        .add_plugins(sky::SkyPlugin)
        .add_plugins(MaterialPlugin::<SubtractMaterial>::default())
        .insert_resource(Time::<Fixed>::from_hz(GAME_TICK_HZ))
        .add_systems(
            FixedUpdate,
            (
                animate,
                animate_water,
                animate_scenery,
                props::animate,
                banner::wave,
                // Driving ticks the objects with the karts (`drive_tick`).
                props::tick_objects.run_if(not(resource_exists::<drive::Drive>)),
                props::pose_hazards,
                props::hide_fruit,
                props::pose_crates,
                explosion::spawn,
                explosion::animate,
                fire::tick,
                advance_tick,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                switch_level,
                props::roll,
                props::spin_fruit,
                props::place_hazards,
                fire::update,
            )
                .chain(),
        );
    character_data.insert(app.world_mut());
    let world = app.world_mut();
    let mut scene_systems = vec![
        world.register_system(setup),
        world.register_system(spawn_kart),
    ];
    if let Some(drive) = drive {
        scene_systems.push(app.world_mut().register_system(drive::spawn_hud));
        scene_systems.push(app.world_mut().register_system(lobby::spawn_waiting));
        app.insert_resource(drive).add_plugins((
            drive::DrivePlugin,
            lobby::LobbyPlugin,
            command::CommandPlugin,
            leaderboard::LeaderboardPlugin,
            weapon::WeaponPlugin,
            wumpa_icon::WumpaIconPlugin,
            toast::ToastPlugin,
        ));
    } else {
        scene_systems.push(app.world_mut().register_system(probe::spawn_text));
        app.add_plugins(probe::ProbePlugin).add_systems(Update, fly);
    }
    app.insert_resource(SceneSystems(scene_systems));
    // The multi-threaded executor spawns and wakes a task per system; for a frame this light
    // those hand-offs cost more than the systems. `CTR_EXECUTOR=multi` brings it back.
    if single_threaded {
        use bevy::ecs::schedule::SingleThreadedExecutor;
        let single = |schedules: &mut Schedules| {
            for (_, schedule) in schedules.iter_mut() {
                schedule.set_executor(SingleThreadedExecutor::new());
            }
        };
        single(&mut app.world_mut().resource_mut::<Schedules>());
        if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
            single(&mut render.world_mut().resource_mut::<Schedules>());
        }
    }
    app.run();
}

/// `]` / Page Up and `[` / Page Down: shows the next or the previous level; `.` and `,`: the next or
/// the previous racer. A new level loads right here (the window stands still meanwhile) and
/// builds the scene from scratch; a new racer only replaces the kart, and starts the race again
/// on the same level.
fn switch_level(world: &mut World) {
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let pressed = |codes: &[KeyCode]| codes.iter().any(|&k| keys.just_pressed(k));
    let step =
        |next: &[KeyCode], previous: &[KeyCode]| pressed(next) as i32 - pressed(previous) as i32;
    let mut level_step = step(
        &[KeyCode::BracketRight, KeyCode::PageUp],
        &[KeyCode::BracketLeft, KeyCode::PageDown],
    );
    let mut racer_step = step(&[KeyCode::Period], &[KeyCode::Comma]);
    // An online race is on its level and with its racers: the lobby's start sets the level, and
    // the keys do nothing.
    let race_track = world
        .get_resource::<lobby::Lobby>()
        .and_then(|l| l.online.as_ref())
        .map(|o| o.track);
    if race_track.is_some() {
        (level_step, racer_step) = (0, 0);
    }
    let mut options = *world.resource::<Options>();
    let current = world.resource::<LevelNumber>().0;
    let wanted = race_track.filter(|&track| track != current);
    if options.racer.is_none() {
        racer_step = 0;
    }
    if wanted.is_none() && level_step == 0 && racer_step == 0 {
        return;
    }
    let id = wanted.unwrap_or_else(|| {
        let cycle = maps::cycle();
        let at = cycle.iter().position(|&n| n == current).unwrap_or(0) as i32;
        cycle[(at + level_step).rem_euclid(cycle.len() as i32) as usize]
    });
    options.racer = options
        .racer
        .map(|r| (r as i32 + racer_step).rem_euclid(CHARACTERS.len() as i32) as u32);
    world.insert_resource(options);
    if racer_step != 0
        && let (Some(lobby), Some(racer)) = (world.get_resource::<lobby::Lobby>(), options.racer)
    {
        lobby.set_character(racer as u8);
    }
    let character = (racer_step != 0).then(|| load_character(options.racer));

    if id != current {
        let LevelData { level, dram, vram } = load_level(id);
        let scene = world_scene(world);
        despawn(world, scene);
        world.insert_resource(Track(level, LevelId(id)));
        world.insert_resource(Textures { dram, vram });
        if let Some(character) = character {
            character.insert(world);
        }
        world.insert_resource(LevelNumber(id));
        restart_drive(world, options);
        for system in world.resource::<SceneSystems>().0.clone() {
            world.run_system(system).expect("scene system runs");
        }
    } else if let Some(character) = character {
        // Only the kart goes: the level stays, and the race starts again with the new racer.
        let kart = world
            .query_filtered::<Entity, With<KartScene>>()
            .iter(world)
            .collect();
        despawn(world, kart);
        character.insert(world);
        restart_drive(world, options);
        if options.driving {
            world
                .run_system_cached(restart_sim)
                .expect("restart system runs");
        }
        spawn_kart(world);
    }
    for mut window in world.query::<&mut Window>().iter_mut(world) {
        window.title = match options.racer {
            Some(racer) => format!(
                "ctr-rs view: {}, {}",
                maps::name(id),
                CHARACTERS[racer as usize]
            ),
            None => format!("ctr-rs view: {}", maps::name(id)),
        };
    }
}

/// Everything in the scene has a transform (the 3D parts) or a node (the text); the window and
/// the gamepads have neither.
fn world_scene(world: &mut World) -> Vec<Entity> {
    world
        .query_filtered::<Entity, Or<(With<Transform>, With<Node>)>>()
        .iter(world)
        .collect()
}

fn despawn(world: &mut World, entities: Vec<Entity>) {
    for entity in entities {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
}

/// The kart on the grid of the level shown, and what it is driven with, when driving.
fn restart_drive(world: &mut World, options: Options) {
    if options.driving {
        let drive = new_drive(
            world.resource::<Track>(),
            &world.resource::<Racer>().0,
            options.racer.unwrap_or(0) as usize,
        );
        world.insert_resource(drive);
        world.insert_resource(exhaust::Particles::new());
    }
}

/// A new `Sim` of the level shown, with the karts on the grid. Its objects start again, and the
/// sparks of the fireballs that were left are gone.
fn restart_sim(
    mut commands: Commands,
    track: Res<Track>,
    textures: Res<Textures>,
    driving: Option<Res<drive::Drive>>,
    fire: Option<ResMut<fire::Fire>>,
) {
    let objects = instance::Objects::new(&track.0, &textures.dram);
    let specs: Vec<race::KartSpec> = driving.iter().map(|d| d.spec()).collect();
    commands.insert_resource(props::Sim::new(&track.0, objects, &specs));
    if let Some(mut fire) = fire {
        fire.clear();
    }
}

/// Spawns one entity per non-empty part, with the material its blend mode needs.
/// The render layer of everything of the kart: its body, wheels, fire, smoke and sparks. A second
/// camera draws them over the rest with a fresh depth buffer, which is the closest thing to retail
/// giving them a slot of their own in the ordering table (it has no depth buffer): they are
/// ordered among themselves by depth, and never hidden by the track.
const KART_LAYER: usize = 1;

/// The render layer of our own kart's shield, which a camera draws over the kart.
const SHIELD_LAYER: usize = 4;

/// The cameras that draw over the picture of the main one: the one for [`KART_LAYER`], and the
/// HUD's (`wumpa_icon`). The systems that follow "the camera" (the wheels' view, the smoke, the
/// skid marks) look for the main camera, which has not got this.
#[derive(Component)]
struct OverlayCamera;

/// What one step of the game's draw order bias (a face filed that many ordering table slots
/// farther back) is worth in `wgpu` constant depth bias units. Those are one ulp of the depth
/// there, so a flush decal needs a good few of them to win against its surface.
const DEPTH_BIAS_PER_ORDER: f32 = 8.0;

/// Spawns the four parts of a model that is not layered by draw order.
pub(crate) fn spawn_parts(
    parts: [mesh::MeshPart; PARTS],
    base: &StandardMaterial,
    transform: Transform,
    parent: Option<Entity>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    layer: usize,
) -> [Option<Handle<Mesh>>; PARTS] {
    spawn_layers(
        Vec::from(parts),
        &[],
        base,
        transform,
        parent,
        commands,
        meshes,
        materials,
        subtract_materials,
        layer,
    )
    .try_into()
    .unwrap()
}

/// Spawns `parts` (slot `i` is blended as `i % PARTS`). `biases` holds the draw order bias of each
/// group of [`PARTS`] slots, 0 for those beyond it: a face with a higher one draws behind.
fn spawn_layers(
    parts: Vec<mesh::MeshPart>,
    biases: &[i32],
    base: &StandardMaterial,
    transform: Transform,
    parent: Option<Entity>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    layer: usize,
) -> Vec<Option<Handle<Mesh>>> {
    let mut handles = vec![None; parts.len()];
    for (slot, part) in parts.into_iter().enumerate() {
        let i = slot % PARTS;
        let order = biases.get(slot / PARTS).copied().unwrap_or(0);
        if part.indices.is_empty() {
            continue;
        }
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, part.positions);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, part.colors);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, part.uvs);
        m.insert_indices(Indices::U32(part.indices));
        let handle = meshes.add(m);
        handles[slot] = Some(handle.clone());
        let with = |alpha_mode| StandardMaterial {
            alpha_mode,
            depth_bias: base.depth_bias - (order as f32) * DEPTH_BIAS_PER_ORDER,
            ..base.clone()
        };
        let mut entity = commands.spawn((Mesh3d(handle), transform));
        if let Some(parent) = parent {
            entity.insert(ChildOf(parent));
        }
        entity.insert(RenderLayers::layer(layer));
        match i {
            i if i == Part::Opaque as usize => {
                entity.insert(MeshMaterial3d(materials.add(with(AlphaMode::Mask(0.5)))))
            }
            i if i == Part::Average as usize => {
                entity.insert(MeshMaterial3d(materials.add(with(AlphaMode::Blend))))
            }
            i if i == Part::Add as usize => {
                entity.insert(MeshMaterial3d(materials.add(with(AlphaMode::Add))))
            }
            _ => entity.insert(MeshMaterial3d(subtract_materials.add(SubtractMaterial {
                base: with(AlphaMode::Blend),
                extension: subtract::Subtract {},
            }))),
        };
    }
    handles
}

fn setup(
    mut commands: Commands,
    track: Res<Track>,
    textures: Res<Textures>,
    mut images: ResMut<Assets<Image>>,
    low: Res<LowColors>,
    show_hidden: Res<ShowHidden>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
    driving: Option<Res<drive::Drive>>,
) {
    let level = &track.0;
    let data = mesh::level_mesh(
        level,
        &textures.dram,
        &textures.vram,
        low.0,
        !show_hidden.show_hidden,
        show_hidden.cull,
    );
    info!(
        "{} turbo pad collision blocks, {} blocks that show them (those get the extra depth bias)",
        level
            .quad_blocks
            .iter()
            .filter(|q| mesh::is_turbo_pad(q))
            .count(),
        mesh::turbo_pad_visuals(level).len()
    );
    info!(
        "{} quad blocks, vertices per part {:?}, {} animated faces",
        level.quad_blocks.len(),
        data.parts
            .iter()
            .map(|p| p.positions.len())
            .collect::<Vec<_>>(),
        data.animated_faces.len()
    );

    let mut atlas = Image::new(
        Extent3d {
            width: data.atlas.width as u32,
            height: data.atlas.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data.atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    atlas.sampler = ImageSampler::nearest();
    let atlas = images.add(atlas);

    let base = StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(atlas),
        unlit: true,
        // The level winds its faces as the game's `NCLIP` test draws them (see `level_mesh`).
        cull_mode: show_hidden
            .cull
            .then_some(bevy::render::render_resource::Face::Back),
        double_sided: !show_hidden.cull,
        ..default()
    };
    let uvs = data.parts.iter().map(|p| p.uvs.clone()).collect();
    let scenery_positions = data.parts.iter().map(|p| p.positions.clone()).collect();
    let scenery_colors = data.parts.iter().map(|p| p.colors.clone()).collect();
    let handles = spawn_layers(
        data.parts,
        &data.biases,
        &base,
        Transform::default(),
        None,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut subtract_materials,
        0,
    );
    commands.insert_resource(SceneryState {
        scenery: level.scenery.clone(),
        corners: data.scenery_corners,
        positions: scenery_positions,
        colors: scenery_colors,
        handles: handles.clone(),
    });
    let water_state = WaterState {
        water: level.water.clone(),
        corners: data.water_corners,
        atlas_size: data.atlas_size,
        colors: data.water.each_ref().map(|p| p.colors.clone()),
        uvs: data.water.each_ref().map(|p| p.uvs.clone()),
        handles: spawn_parts(
            data.water,
            &base,
            Transform::default(),
            None,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
            0,
        ),
    };
    commands.insert_resource(water_state);
    commands.insert_resource(GameTick(0));
    commands.insert_resource(AnimState {
        shown: vec![usize::MAX; data.animations.len()],
        animations: data.animations,
        faces: data.animated_faces,
        uvs,
        handles,
    });

    let objects = instance::Objects::new(level, &textures.dram);
    props::spawn(
        level,
        &objects,
        &textures.dram,
        &textures.vram,
        &mut commands,
        &mut images,
        &mut meshes,
        &mut materials,
        &mut subtract_materials,
    );
    // The previous level's fire, if the level has none.
    commands.remove_resource::<fire::Fire>();
    fire::spawn(
        objects.fireballs.len(),
        &textures.dram,
        &textures.vram,
        &mut commands,
        &mut images,
        &mut meshes,
        &mut materials,
    );
    let specs: Vec<race::KartSpec> = driving.iter().map(|d| d.spec()).collect();
    let sim = props::Sim::new(level, objects, &specs);

    commands.insert_resource(sim);

    // The sky camera clears the picture; the main camera draws over it.
    let sky_clear = if level.skybox.is_some() {
        ClearColorConfig::None
    } else {
        ClearColorConfig::Default
    };
    let spawn = level.driver_spawns[0].pos;
    let [x, y, z] = mesh::to_world(spawn);
    if driving.is_some() {
        // The game projects with a screen distance of 256 for a 512 pixel wide
        // screen: 90 degrees across, 73.7 degrees up at 4:3.
        let perspective = PerspectiveProjection {
            fov: 2.0 * (0.75f32).atan(),
            ..default()
        };
        if let Some(skybox) = &level.skybox {
            sky::spawn(
                &mut commands,
                &mut meshes,
                &mut materials,
                skybox,
                perspective.clone(),
            );
        }
        let projection = Projection::Perspective(perspective);
        commands
            .spawn((
                Camera3d::default(),
                Camera {
                    clear_color: sky_clear,
                    ..default()
                },
                projection.clone(),
                Transform::default(),
                drive::ChaseCamera::new(),
            ))
            .with_child((
                OverlayCamera,
                Camera3d::default(),
                // After the main camera, on top of its picture, with only the depth buffer cleared.
                Camera {
                    order: 1,
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                projection.clone(),
                Transform::default(),
                RenderLayers::layer(KART_LAYER),
            ))
            .with_child((
                OverlayCamera,
                Camera3d::default(),
                // And the shield over the kart, likewise.
                Camera {
                    order: 2,
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                projection,
                Transform::default(),
                RenderLayers::layer(SHIELD_LAYER),
            ));
    } else {
        if let Some(skybox) = &level.skybox {
            sky::spawn(
                &mut commands,
                &mut meshes,
                &mut materials,
                skybox,
                default(),
            );
        }
        commands.spawn((
            Camera3d::default(),
            Camera {
                clear_color: sky_clear,
                ..default()
            },
            Transform::from_xyz(x, y + 3.0, z + 6.0),
            FlyCamera {
                yaw: 0.0,
                pitch: -0.3,
                speed: 10.0,
            },
        ));
    }
}

/// Marks what belongs to the kart on the grid, to despawn it for another racer (and not the level).
#[derive(Component)]
struct KartScene;

/// Spawns the kart and what goes with it (its flames, shadow, skid marks and exhaust), and marks
/// it all as [`KartScene`]. Those spawn each into their own entities, so the scene entities
/// that are new after they ran are the kart's.
fn spawn_kart(world: &mut World) {
    let before: std::collections::HashSet<Entity> = world_scene(world).into_iter().collect();
    world
        .run_system_cached(spawn_kart_parts)
        .expect("kart system runs");
    // (Only the scene: resources and registered systems are entities too, and must stay.)
    for entity in world_scene(world) {
        if !before.contains(&entity) {
            world.entity_mut(entity).insert(KartScene);
        }
    }
}

fn spawn_kart_parts(
    track: Res<Track>,
    sim: Res<props::Sim>,
    racer: Res<Racer>,
    wheel_icons: Res<WheelIcons>,
    driving: Option<Res<drive::Drive>>,
    mut builder: kart::KartBuilder,
) {
    let Some(model) = &racer.0 else { return };
    let spawn = track.0.driver_spawns[0];
    let [x, y, z] = mesh::to_world(spawn.pos);
    let yaw = spawn.rot.y as f32 / 4096.0 * std::f32::consts::TAU;
    // The model faces along +X; a driver's heading is along +Z. A driven kart is posed by its
    // sim instead.
    let placement = Transform::from_xyz(x, y, z)
        .with_rotation(Quat::from_rotation_y(yaw) * Quat::from_rotation_y(MODEL_FORWARD_YAW));
    let driven = driving.map(|_| (sim.own, sim.kart()));
    builder.spawn(model, &wheel_icons.0, placement, true, driven);
}

/// The material of a kart model's parts: its texture atlas, unlit.
pub(crate) fn kart_material(atlas: mesh::Atlas, images: &mut Assets<Image>) -> StandardMaterial {
    let mut atlas = Image::new(
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
    atlas.sampler = ImageSampler::nearest();
    StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(images.add(atlas)),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    }
}

/// Racer models face along +X, while a driver's heading (`spawn.rot.y`) points along
/// +Z: a quarter turn clockwise seen from above (Bevy's +Y is up, so negative).
const MODEL_FORWARD_YAW: f32 = std::f32::consts::FRAC_PI_2;

/// The game logic and its texture animation run at this rate; rendering may
/// run at any multiple of it.
const GAME_TICK_HZ: f64 = 30.0;

/// Animated level textures: which frame each shows, and the UVs to patch into
/// the part meshes when it changes.
#[derive(Resource)]
struct AnimState {
    animations: Vec<mesh::Animation>,
    faces: Vec<mesh::AnimatedFace>,
    uvs: Vec<Vec<[f32; 2]>>,
    handles: Vec<Option<Handle<Mesh>>>,
    shown: Vec<usize>,
}

/// Game ticks elapsed (`gGT->timer`).
#[derive(Resource)]
struct GameTick(u32);

/// The water faces: their meshes, and what the per-tick water animation patches into them.
#[derive(Resource)]
struct WaterState {
    water: loader::water::Water,
    corners: std::collections::HashMap<usize, Vec<mesh::WaterCorner>>,
    atlas_size: [f32; 2],
    colors: [Vec<[f32; 4]>; PARTS],
    uvs: [Vec<[f32; 2]>; PARTS],
    handles: [Option<Handle<Mesh>>; PARTS],
}

/// The swaying scenery (`AnimateQuad`): the vertices' mesh corners, and the meshes' arrays that
/// each tick patches.
#[derive(Resource)]
struct SceneryState {
    scenery: loader::scenery::Scenery,
    corners: std::collections::HashMap<usize, Vec<mesh::SceneryCorner>>,
    positions: Vec<Vec<[f32; 3]>>,
    colors: Vec<Vec<[f32; 4]>>,
    handles: Vec<Option<Handle<Mesh>>>,
}

/// The frame rate the viewer is held to, in Hz; `CTR_FPS` overrides it, 0 for no limit.
const FRAME_RATE: f64 = 120.0;

// Holds the loop to [`FRAME_RATE`] now that the display does not (no vsync): sleeps most of
// what is left of the frame, then spins the last stretch, as a sleep wakes up late.
fn limit_frame_rate(
    mut previous: Local<Option<std::time::Instant>>,
    mut configured: Local<Option<f64>>,
) {
    use std::time::{Duration, Instant};
    const SPIN: Duration = Duration::from_micros(1500);
    let rate = *configured.get_or_insert_with(|| {
        std::env::var("CTR_FPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(FRAME_RATE)
    });
    let start = previous.unwrap_or_else(Instant::now);
    if rate > 0.0 {
        let target = start + Duration::from_secs_f64(1.0 / rate);
        if let Some(rest) = target.checked_duration_since(Instant::now()) {
            std::thread::sleep(rest.saturating_sub(SPIN));
        }
        while Instant::now() < target {
            std::hint::spin_loop();
        }
    }
    // From the target, not from now, so that one late frame does not slow the ones after it;
    // but never fall behind by more than a frame, or a stall would be followed by a burst.
    let now = Instant::now();
    *previous = Some(if rate > 0.0 {
        let step = Duration::from_secs_f64(1.0 / rate);
        (start + step).max(now.checked_sub(step).unwrap_or(now))
    } else {
        now
    });
}

/// Logs how many meshes the scene draws, a few frames in (once everything is spawned).
fn log_draws(
    mut frame: Local<u32>,
    meshes: Query<(&Mesh3d, Option<&Visibility>, Option<&InheritedVisibility>)>,
    assets: Res<Assets<Mesh>>,
) {
    *frame += 1;
    if *frame != 30 {
        return;
    }
    let (mut entities, mut visible, mut vertices) = (0, 0, 0);
    for (mesh, _, inherited) in &meshes {
        entities += 1;
        if inherited.is_none_or(|v| v.get()) {
            visible += 1;
            vertices += assets.get(&mesh.0).map_or(0, |m| m.count_vertices());
        }
    }
    info!("{entities} mesh entities, {visible} visible, {vertices} vertices");
}

/// `AnimateQuad`, once per game tick.
fn animate_scenery(
    mut state: ResMut<SceneryState>,
    tick: Res<GameTick>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let state = &mut *state;
    if state.scenery.vertices.is_empty() {
        return;
    }
    // Only the meshes whose arrays changed are uploaded again.
    let mut moved = vec![false; state.handles.len()];
    let mut recolored = vec![false; state.handles.len()];
    for update in state.scenery.animate(tick.0, |_| true) {
        let Some(corners) = state.corners.get(&update.vertex) else {
            continue;
        };
        for corner in corners {
            if let Some(pos) = update.pos {
                let at = &mut state.positions[corner.part][corner.slot];
                let pos = to_world(pos);
                moved[corner.part] |= std::mem::replace(at, pos) != pos;
            }
            if let Some([r, g, b, _]) = update.color {
                let [r, g, b] = [r, g, b].map(|c| mesh::vertex_channel(c as f32 * corner.gain));
                let at = &mut state.colors[corner.part][corner.slot];
                recolored[corner.part] |= std::mem::replace(at, [r, g, b, 1.0]) != [r, g, b, 1.0];
            }
        }
    }
    for part in 0..state.handles.len() {
        if !(moved[part] || recolored[part]) {
            continue;
        }
        if let Some(mut mesh) = state.handles[part].as_ref().and_then(|h| meshes.get_mut(h)) {
            if moved[part] {
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, state.positions[part].clone());
            }
            if recolored[part] {
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, state.colors[part].clone());
            }
        }
    }
}

/// `CTR_CycleTex_LEV`, once per game tick.
fn animate(mut state: ResMut<AnimState>, tick: Res<GameTick>, mut meshes: ResMut<Assets<Mesh>>) {
    let state = &mut *state;
    let mut dirty = vec![false; state.handles.len()];
    for (i, anim) in state.animations.iter().enumerate() {
        let frame = anim.texture.frame_at(tick.0 as i32);
        if std::mem::replace(&mut state.shown[i], frame) == frame {
            continue;
        }
        for face in state.faces.iter().filter(|f| f.animation == i) {
            for (k, &from) in face.uv_order.iter().enumerate() {
                state.uvs[face.part][face.first_vertex + k] = anim.uvs[frame][from];
            }
            dirty[face.part] = true;
        }
    }
    for (part, _) in dirty.iter().enumerate().filter(|(_, d)| **d) {
        if let Some(mut mesh) = state.handles[part].as_ref().and_then(|h| meshes.get_mut(h)) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, state.uvs[part].clone());
        }
    }
}

/// `AnimateWater1P`, once per game tick, for every water vertex.
fn animate_water(
    mut state: ResMut<WaterState>,
    tick: Res<GameTick>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let state = &mut *state;
    let [width, height] = state.atlas_size;
    // Only the meshes whose arrays changed are uploaded again.
    let mut recolored = [false; PARTS];
    let mut moved = [false; PARTS];
    for c in state.water.animate(tick.0, |_| true) {
        // Textured primitives get their colour doubled, as everywhere else.
        let gray = mesh::vertex_channel(c.gray as f32 * 2.0 / 255.0);
        for corner in state.corners.get(&c.vertex).into_iter().flatten() {
            let color = [gray, gray, gray, 1.0];
            let uv = [
                (corner.origin[0] + c.uv[0] as f32) / width,
                (corner.origin[1] + c.uv[1] as f32) / height,
            ];
            recolored[corner.part] |=
                std::mem::replace(&mut state.colors[corner.part][corner.slot], color) != color;
            moved[corner.part] |=
                std::mem::replace(&mut state.uvs[corner.part][corner.slot], uv) != uv;
        }
    }
    for part in 0..PARTS {
        if !(recolored[part] || moved[part]) {
            continue;
        }
        if let Some(mut mesh) = state.handles[part].as_ref().and_then(|h| meshes.get_mut(h)) {
            if recolored[part] {
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, state.colors[part].clone());
            }
            if moved[part] {
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, state.uvs[part].clone());
            }
        }
    }
}

fn advance_tick(mut tick: ResMut<GameTick>) {
    tick.0 += 1;
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut cameras: Query<(&mut Transform, &mut FlyCamera)>,
) {
    for (mut transform, mut cam) in &mut cameras {
        if buttons.pressed(MouseButton::Right) {
            cam.yaw -= motion.delta.x * 0.003;
            cam.pitch = (cam.pitch - motion.delta.y * 0.003).clamp(-1.55, 1.55);
        }
        transform.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);

        let axis = |pos: KeyCode, neg: KeyCode| {
            keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32
        };
        let wish = transform.forward() * axis(KeyCode::KeyW, KeyCode::KeyS)
            + transform.right() * axis(KeyCode::KeyD, KeyCode::KeyA)
            + Vec3::Y * axis(KeyCode::Space, KeyCode::ShiftLeft);
        let boost = if keys.pressed(KeyCode::ControlLeft) {
            4.0
        } else {
            1.0
        };
        transform.translation += wish.normalize_or_zero() * cam.speed * boost * time.delta_secs();
    }
}
