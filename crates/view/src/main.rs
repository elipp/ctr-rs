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

mod banner;
mod bar;
mod drive;
mod exhaust;
mod explosion;
mod fire;
mod flame;
mod kart;
mod leaderboard;
mod lobby;
mod mask;
mod probe;
mod props;
mod remote;
mod shadow;
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

/// A level's geometry plus what its texture pointers need.
struct Loaded {
    level: Level,
    dram: DramFile,
    vram: Vram,
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
    /// `crate_explosion` of the racer pack: the break-up of a crate.
    crate_explosion: Option<Model>,
    /// The Aku Aku and Uka Uka masks of the racer pack.
    masks: render::mask::Masks,
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

/// The sprite and texture page file shared by every track (`shared.vrm`):
/// the racers' textures are in it.
fn load_level(id: u32, racer: Option<u32>) -> Loaded {
    let path = bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
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
    let shared = big
        .read(&mut disc, BigIndex::SHARED_MPK_VRM.get())
        .expect("read shared VRAM file");
    for img in parse_vram_file(&shared).expect("parse shared VRAM file") {
        vram.upload(&img).expect("upload shared VRAM image");
    }
    let racer_character = racer;
    let racer = racer.map(|character| {
        let file = big
            .read(&mut disc, BigIndex::RACER_MODEL_HI.nth(character).get())
            .expect("read racer model");
        Model::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse racer model")
    });
    let shadow_icons = racer_character.and_then(|character| {
        let pack = big
            .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
            .expect("read racer pack");
        let icons = parse_icons(&DramFile::parse(&pack).expect("parse racer pack"))
            .expect("parse racer pack icons");
        let find = |index| icons.iter().find(|i| i.global_index == index).cloned();
        Some([find(0)?, find(1)?])
    });
    let tiremark_icon = racer_character.and_then(|character| {
        let pack = big
            .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
            .expect("read racer pack");
        let icons = parse_icons(&DramFile::parse(&pack).expect("parse racer pack"))
            .expect("parse racer pack icons");
        icons
            .into_iter()
            .find(|i| i.global_index == skid::TIREMARK_ICON)
    });
    let smoke_icon = racer_character.and_then(|character| {
        let pack = big
            .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
            .expect("read racer pack");
        let groups = parse_groups(&DramFile::parse(&pack).expect("parse racer pack"))
            .expect("parse racer pack icons");
        groups
            .into_iter()
            .find(|g| g.group_id == 1)?
            .icons
            .into_iter()
            .next()
    });
    let flame_models = racer_character.map_or_else(Vec::new, |character| {
        let pack = big
            .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
            .expect("read racer pack");
        flame::models(&DramFile::parse(&pack).expect("parse racer pack"))
            .expect("racer pack has the turbo models")
    });
    let crate_explosion = {
        // Every racer's pack has the same shared models; character 0's will do without a racer.
        let pack = big
            .read(
                &mut disc,
                BigIndex::ARCADE_1P_PACK
                    .nth(racer_character.unwrap_or(0))
                    .get(),
            )
            .expect("read racer pack");
        render::explosion::model(&DramFile::parse(&pack).expect("parse racer pack"))
            .expect("racer pack has the crate explosion")
    };
    let masks = {
        let pack = big
            .read(
                &mut disc,
                BigIndex::ARCADE_1P_PACK
                    .nth(racer_character.unwrap_or(0))
                    .get(),
            )
            .expect("read racer pack");
        render::mask::models(&DramFile::parse(&pack).expect("parse racer pack"))
            .expect("racer pack has the masks")
    };
    let wheel_icons = match racer_character {
        Some(character) if character != NITROS_OXIDE => {
            let pack = big
                .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
                .expect("read racer pack");
            parse_groups(&DramFile::parse(&pack).expect("parse racer pack"))
                .expect("parse racer pack icons")
                .into_iter()
                .find(|g| g.group_id == 0)
                .expect("racer pack has the tire icon group")
                .icons
        }
        _ => Vec::new(),
    };
    Loaded {
        level,
        dram,
        vram,
        racer,
        wheel_icons,
        shadow_icons,
        smoke_icon,
        tiremark_icon,
        flame_models,
        crate_explosion,
        masks,
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
    let track_id = 0;
    let mut racer = Some(0);
    let mut low = false;
    let mut show_hidden = false;
    let mut cull = true;
    let mut driving = false;
    let mut join = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lo" => low = true,
            "--drive" => driving = true,
            "--join" => {
                join = Some(
                    args.next()
                        .and_then(|t| t.parse::<net::Ticket>().map_err(|e| eprintln!("{e}")).ok())
                        .expect("--join takes the ticket a lobby printed"),
                )
            }
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
                    "usage: view [--lo] [--show-hidden] [--no-cull] [--drive [--join TICKET]] [--racer N | --no-racer], got '{n}'"
                )
            }
        }
    }

    let character = racer.unwrap_or(0) as usize;
    let options = Options { racer, driving };
    let Loaded {
        level,
        dram,
        vram,
        racer,
        wheel_icons,
        shadow_icons,
        smoke_icon,
        tiremark_icon,
        flame_models,
        crate_explosion,
        masks,
    } = load_level(track_id, racer);
    let loaded = Track(level, LevelId(track_id));
    let textures = Textures { dram, vram };
    let drive = driving.then(|| new_drive(&loaded, &racer, character));

    // See the executor below: without a render thread, `CTR_EXECUTOR=multi` keeps the defaults.
    let single_threaded = std::env::var("CTR_EXECUTOR").as_deref() != Ok("multi");
    let mut app = App::new();
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: match options.racer {
                Some(racer) => format!(
                    "ctr-rs view: level {track_id}, {}",
                    CHARACTERS[racer as usize]
                ),
                None => format!("ctr-rs view: level {track_id}"),
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
    app.add_plugins(plugins)
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.1)))
        .insert_resource(loaded)
        .insert_resource(textures)
        .insert_resource(Racer(racer))
        .insert_resource(WheelIcons(wheel_icons))
        .insert_resource(ShadowIcons(shadow_icons))
        .insert_resource(SmokeIcon(smoke_icon))
        .insert_resource(TiremarkIcon(tiremark_icon))
        .insert_resource(FlameModels(flame_models))
        .insert_resource(explosion::CrateExplosion::new(crate_explosion))
        .insert_resource(mask::MaskModels(Some(masks)))
        .insert_resource(options)
        .insert_resource(LevelNumber(track_id))
        .insert_resource(LowColors(low))
        .insert_resource(ShowHidden { show_hidden, cull })
        .add_systems(Startup, setup)
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
    let world = app.world_mut();
    let mut scene_systems = vec![world.register_system(setup)];
    if let Some(drive) = drive {
        scene_systems.push(app.world_mut().register_system(drive::spawn_hud));
        scene_systems.push(app.world_mut().register_system(mask::spawn));
        scene_systems.push(app.world_mut().register_system(lobby::spawn_waiting));
        let character = drive.character as u8;
        app.insert_resource(drive).add_plugins((
            drive::DrivePlugin,
            lobby::LobbyPlugin,
            leaderboard::LeaderboardPlugin,
            weapon::WeaponPlugin,
            wumpa_icon::WumpaIconPlugin,
            toast::ToastPlugin,
        ));
        if let Some(ticket) = join {
            app.insert_resource(lobby::Lobby::joining(ticket, character));
        }
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
/// the previous racer. It loads right here (the window stands still meanwhile) and builds the scene
/// from scratch.
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
    if wanted.is_none() && level_step == 0 && (racer_step == 0 || options.racer.is_none()) {
        return;
    }
    let levels = (LevelId::TURBO_TRACK.0 - LevelId::DINGO_CANYON.0 + 1) as i32;
    let id = wanted.unwrap_or_else(|| {
        ((current as i32 - LevelId::DINGO_CANYON.0 as i32 + level_step).rem_euclid(levels) as u32)
            + LevelId::DINGO_CANYON.0
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
    let Loaded {
        level,
        dram,
        vram,
        racer,
        wheel_icons,
        shadow_icons,
        smoke_icon,
        tiremark_icon,
        flame_models,
        crate_explosion,
        masks,
    } = load_level(id, options.racer);

    // Everything in the scene has a transform (the 3D parts) or a node (the text); the window
    // and the gamepads have neither.
    let scene: Vec<Entity> = world
        .query_filtered::<Entity, Or<(With<Transform>, With<Node>)>>()
        .iter(world)
        .collect();
    for entity in scene {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }

    world.insert_resource(Track(level, LevelId(id)));
    world.insert_resource(Textures { dram, vram });
    if options.driving {
        let drive = new_drive(
            world.resource::<Track>(),
            &racer,
            options.racer.unwrap_or(0) as usize,
        );
        world.insert_resource(drive);
        world.insert_resource(exhaust::Particles::new());
    }
    world.insert_resource(Racer(racer));
    world.insert_resource(WheelIcons(wheel_icons));
    world.insert_resource(ShadowIcons(shadow_icons));
    world.insert_resource(SmokeIcon(smoke_icon));
    world.insert_resource(TiremarkIcon(tiremark_icon));
    world.insert_resource(FlameModels(flame_models));
    world.insert_resource(explosion::CrateExplosion::new(crate_explosion));
    world.insert_resource(mask::MaskModels(Some(masks)));
    world.insert_resource(LevelNumber(id));

    for system in world.resource::<SceneSystems>().0.clone() {
        world.run_system(system).expect("scene system runs");
    }
    for mut window in world.query::<&mut Window>().iter_mut(world) {
        window.title = match options.racer {
            Some(racer) => format!("ctr-rs view: level {id}, {}", CHARACTERS[racer as usize]),
            None => format!("ctr-rs view: level {id}"),
        };
    }
}

/// Spawns one entity per non-empty part, with the material its blend mode needs.
/// The render layer of everything of the kart: its body, wheels, fire, smoke and sparks. A second
/// camera draws them over the rest with a fresh depth buffer, which is the closest thing to retail
/// giving them a slot of their own in the ordering table (it has no depth buffer): they are
/// ordered among themselves by depth, and never hidden by the track.
const KART_LAYER: usize = 1;

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
    racer: Res<Racer>,
    wheel_icons: Res<WheelIcons>,
    shadow_icons: Res<ShadowIcons>,
    smoke_icon: Res<SmokeIcon>,
    tiremark_icon: Res<TiremarkIcon>,
    flame_models: Res<FlameModels>,
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

    if let Some(model) = &racer.0 {
        let kart = spawn_racer(
            model,
            level.driver_spawns[0],
            driving
                .as_ref()
                .map(|_| kart::KartMotion::new(sim.driver())),
            &textures.vram,
            &wheel_icons.0,
            &mut commands,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
        );
        if let Some(kart) = kart {
            flame::spawn(
                &mut commands,
                &mut images,
                &mut meshes,
                &mut materials,
                &mut subtract_materials,
                &flame_models.0,
                &textures.vram,
                kart,
            );
        }
    }
    commands.insert_resource(sim);

    if let (Some(icons), true) = (&shadow_icons.0, driving.is_some()) {
        shadow::spawn(
            &mut commands,
            &mut images,
            &mut meshes,
            &mut subtract_materials,
            [&icons[0], &icons[1]],
            &textures.vram,
        );
    }

    if let (Some(icon), true) = (&tiremark_icon.0, driving.is_some()) {
        skid::spawn(
            &mut commands,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
            icon,
            &textures.vram,
        );
    }

    if let (Some(icon), true) = (&smoke_icon.0, driving.is_some()) {
        exhaust::spawn(
            &mut commands,
            &mut images,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
            icon,
            &textures.vram,
        );
    }

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
                projection,
                Transform::default(),
                RenderLayers::layer(KART_LAYER),
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

/// Puts the model's neutral pose (the middle frame of its first animation,
/// which is the steering one) at a spawn point. With a `motion` the sim moves the kart from
/// there (the kart that is returned), and it is drawn over everything in the overlay layer.
fn spawn_racer(
    model: &Model,
    spawn: loader::lev::SpawnPoint,
    motion: Option<kart::KartMotion>,
    vram: &Vram,
    wheel_icons: &[Icon],
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
) -> Option<Entity> {
    let [x, y, z] = mesh::to_world(spawn.pos);
    let yaw = spawn.rot.y as f32 / 4096.0 * std::f32::consts::TAU;
    // The model faces along +X; a driver's heading is along +Z. A driven kart is posed by its
    // sim instead.
    let placement = Transform::from_xyz(x, y, z)
        .with_rotation(Quat::from_rotation_y(yaw) * Quat::from_rotation_y(MODEL_FORWARD_YAW));
    let driven = motion.is_some();
    let kart = kart::spawn(
        model,
        wheel_icons,
        vram,
        placement,
        KART_LAYER,
        motion,
        commands,
        images,
        meshes,
        materials,
        subtract_materials,
    );
    driven.then(|| {
        commands.entity(kart).insert(drive::Kart);
        kart
    })
}

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
