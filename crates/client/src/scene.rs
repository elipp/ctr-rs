//! Building the scene: the level, the kart and the cameras, and switching between them.

use crate::controls::Control;
use crate::subtract::SubtractMaterial;
use crate::Mode;
use crate::{
    AnimState, CHARACTERS, FlyCamera, GameTick, LevelData, LevelNumber, MeshOptions, Options,
    SceneSystems, SceneryState, Textures, Track, WaterState, command, config, drive, exhaust,
    fire, kart, load_level, lobby, maps, props, sky, subtract, toast,
};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use instance::lev_init::LevMode;
use loader::level::LevelId;
pub(crate) use race::track::anim_num_frames;
use render::mesh::{self, PARTS, Part};

/// Tells the page what the game is about to do (`web::stage`), in a `wasm-debug` build; otherwise
/// nothing, and `what` is not even formatted.
pub(crate) fn stage(what: impl std::fmt::Display) {
    #[cfg(all(target_arch = "wasm32", feature = "wasm-debug"))]
    crate::web::stage(&what.to_string());
    #[cfg(not(all(target_arch = "wasm32", feature = "wasm-debug")))]
    let _ = what;
}

/// A drive-mode state for `track`: `kit`'s kart on its first starting slot.
pub(crate) fn new_drive(track: &Track, kit: &crate::Kit) -> drive::Drive {
    let (character, anim_num_frames) = (kit.character, kit.anim_num_frames());
    let spec = race::KartSpec {
        character,
        anim_num_frames,
        bot: false,
        line: None,
    };
    let kart = race::Kart::birth(&track.0, 0, &spec);
    drive::Drive::new(track, &kart.driver, character, anim_num_frames)
}

/// Makes our kart in the race `character`'s, where it is and as it goes (`Sim::set_racer`), and
/// the free drive's to come.
fn swap_character(world: &mut World, character: usize) {
    let kit = world.resource_mut::<crate::Kits>().get(character);
    world
        .resource_mut::<drive::Drive>()
        .set_character(character, kit.anim_num_frames());
    let mut sim = world.resource_mut::<props::Sim>();
    let own = sim.own;
    sim.set_racer(own, &kit);
}

/// The level that was shown when an online race moved the viewer to its own.
#[derive(Resource, Default)]
pub(crate) struct LevelBeforeRace(pub(crate) Option<u32>);

/// What `/character` and `/track` asked for, until [`switch_level`] has switched to it.
#[derive(Resource, Default)]
pub(crate) struct Picked {
    racer: Option<u32>,
    track: Option<u32>,
}

/// Keeps what `/character` and `/track` ask for, for [`switch_level`].
pub(crate) fn keep_picks(
    mut actions: MessageReader<command::Action>,
    lobby: Res<lobby::Lobby>,
    race_on: Res<crate::race_on::RaceOn>,
    mut picked: ResMut<Picked>,
) {
    for action in actions.read() {
        // (The lobby refuses it, and says so.)
        if lobby::commands::refusal(action, &lobby, &race_on).is_some() {
            continue;
        }
        match action {
            command::Action::Character(character) => picked.racer = Some(*character),
            command::Action::Track(track) => picked.track = Some(*track),
            _ => {}
        }
    }
}

/// [`Control::NextRacer`] and [`Control::PreviousRacer`] (`.` and `,` at first): the next or the
/// previous racer, as `/character` would pick it (and is refused
/// alike: in a race, or a cup). On a touch screen the page's buttons step too
/// (`web/play.html`).
pub(crate) fn racer_keys(
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<crate::config::Config>,
    options: Res<Options>,
    mut actions: MessageWriter<command::Action>,
) {
    let pressed = |control| config.controls.just_pressed(control, &keys) as i32;
    #[allow(unused_mut)]
    let mut step = pressed(Control::NextRacer) - pressed(Control::PreviousRacer);
    #[cfg(target_arch = "wasm32")]
    {
        step += crate::web::take_racer_step();
    }
    if step != 0 {
        let racer = (options.racer as i32 + step).rem_euclid(CHARACTERS.len() as i32) as u32;
        actions.write(command::Action::Character(racer));
    }
}

/// Where [`switch_level`] runs: between `StateTransition` and `Update` (`main.rs`).
#[derive(bevy::ecs::schedule::ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LevelChange;

/// [`Control::NextLevel`] and [`Control::PreviousLevel`] (`]` / Page Up and `[` / Page Down at
/// first): shows the next or the previous level; and the racer
/// `/character` (or [`racer_keys`]) picked. A new level loads right here (the window stands still meanwhile) and
/// builds the scene from scratch; a new racer only replaces the kart, which drives on from where
/// it was (the race and its music go on). On a touch screen the page's buttons step to the next
/// (`web/play.html`).
pub(crate) fn switch_level(world: &mut World) {
    // A scene that was just built is still settling (`loading.rs`): another change would despawn
    // what its commands are about to touch.
    if world.contains_resource::<crate::loading::Loading>() {
        return;
    }
    // The flag is coming in before a race: the level loads once it covers the window.
    if crate::loading::entering(world) {
        return;
    }
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let controls = &world.resource::<crate::config::Config>().controls;
    let pressed = |control| controls.just_pressed(control, keys) as i32;
    #[allow(unused_mut)]
    let mut level_step = pressed(Control::NextLevel) - pressed(Control::PreviousLevel);
    #[cfg(target_arch = "wasm32")]
    {
        level_step += crate::web::take_level_step();
    }
    // A race has started: the loading screen comes in over what was shown first, and the race is
    // on once it covers the window (`RaceOn::start`).
    let started = world
        .get_resource::<crate::race_on::RaceOn>()
        .and_then(|race_on| race_on.newest())
        .map(|race| race.track);
    if let Some(track) = started
        && let Some(mut lobby) = world.get_resource_mut::<lobby::Lobby>()
        && lobby.race_next == Some(track)
    {
        lobby.race_next = None;
        crate::loading::bring_in(world);
        return;
    }
    if let Some(mut race_on) = world.get_resource_mut::<crate::race_on::RaceOn>() {
        race_on.take_over();
    }
    // An online race is on its level and with its racers: the lobby's start sets the level, and
    // the keys do nothing.
    // (A map we lack is waited for, see `lobby.rs`.)
    let race_track = world
        .get_resource::<crate::race_on::RaceOn>()
        .and_then(|race_on| race_on.get())
        .map(|o| (o.track, o.hash, o.lev_mode()));
    let missing = race_track.is_some_and(|(track, hash, _)| !maps::has(track, hash));
    // A race of another mode on the shown track loads it again: its instances differ
    // (`lev_init`), and a relic race is on another LEV.
    let mode = race_track.map_or(LevMode::Arcade, |(_, _, mode)| mode);
    let mode_changed = mode != world.resource::<Track>().2;
    let race_track = race_track.map(|(track, ..)| track);
    // The level shown before the race took the viewer to its own: back to it when the race is
    // over (the lobby closed, or failed).
    let mut before = world
        .remove_resource::<LevelBeforeRace>()
        .unwrap_or_default();
    let picked = std::mem::take(&mut *world.resource_mut::<Picked>());
    // (A race that has started since it was asked for goes first.)
    let picked_track = picked.track.filter(|_| race_track.is_none());
    if picked_track.is_some() {
        // A track of our choosing is where we stay when we leave the lobby.
        before.0 = None;
    }
    let in_lobby = world
        .get_resource::<lobby::Lobby>()
        .is_some_and(|lobby| lobby.is_open());
    let mut back = None;
    match race_track {
        Some(track) if track != world.resource::<LevelNumber>().0 => {
            before.0.get_or_insert(world.resource::<LevelNumber>().0);
        }
        Some(_) => {}
        // In a lobby the tracks are the races': we stay on the last until the next (a cup goes
        // on to its next straight from it), and go back when we leave.
        None if in_lobby => {}
        None => back = before.0.take(),
    }
    if in_lobby {
        level_step = 0;
    }
    world.insert_resource(before);
    if race_track.is_some() {
        level_step = 0;
    }
    let mut options = *world.resource::<Options>();
    let racer_step = picked
        .racer
        .filter(|_| race_track.is_none())
        .map_or(0, |racer| racer as i32 - options.racer as i32);
    let current = world.resource::<LevelNumber>().0;
    let wanted = race_track
        .filter(|&track| (track != current || mode_changed) && !missing)
        .or(back.filter(|&track| track != current))
        .or(picked_track.filter(|&track| track != current))
        .or((race_track.is_none() && mode_changed).then_some(current));
    if wanted.is_none() && level_step == 0 && racer_step == 0 {
        return;
    }
    let id = wanted.unwrap_or_else(|| {
        let cycle = maps::cycle();
        let at = cycle.iter().position(|&n| n == current).unwrap_or(0) as i32;
        cycle[(at + level_step).rem_euclid(cycle.len() as i32) as usize]
    });
    options.racer = (options.racer as i32 + racer_step).rem_euclid(CHARACTERS.len() as i32) as u32;
    world.insert_resource(options);
    if racer_step != 0 {
        if let Some(lobby) = world.get_resource::<lobby::Lobby>() {
            lobby.set_character(options.racer as u8);
        }
        if let Some(mut config) = world.get_resource_mut::<config::Config>() {
            config.set_character(options.racer);
        }
    }

    if id != current || mode_changed {
        stage(format_args!("loading level {id:#x}"));
        let LevelData { level, dram, vram } = load_level(id, mode);
        stage("level loaded, old scene still up");
        let scene = world_scene(world);
        despawn(world, scene);
        world.insert_resource(Track(level, LevelId(id), mode));
        world.insert_resource(Textures { dram, vram });
        world.insert_resource(LevelNumber(id));
        stage("building the scene");
        match *world.resource::<State<Mode>>().get() {
            // Another level is not the podium's: the race on it takes over, which `OnEnter`
            // builds. Right away, as `Update` would run the podium's systems on its figures,
            // despawned.
            Mode::Podium => {
                world.resource_mut::<crate::podium::Podium>().back = None;
                world.resource_mut::<NextState<Mode>>().set(Mode::Drive);
                world.run_schedule(StateTransition);
            }
            Mode::Drive => {
                restart_drive(world);
                run_scene_systems(world, Mode::Drive);
            }
            Mode::Viewer => run_scene_systems(world, Mode::Viewer),
        }
        stage("scene built");
        world.write_message(toast::Toast::new(
            format!("Track: {}", maps::name(id)),
            2500,
        ));
        crate::loading::begin(world);
    } else if racer_step != 0 {
        // The level and the race stay, and the new racer drives on from where the old one was.
        // (Online the racers are the race's, and do not change here.)
        if world.contains_resource::<drive::Drive>() {
            swap_character(world, options.racer as usize);
        } else {
            // The viewer's kart is no sim's, for its body to follow: it is the pick's.
            let karts: Vec<Entity> = (world.query_filtered::<Entity, With<drive::Kart>>())
                .iter(world)
                .collect();
            despawn(world, karts);
            world.run_system_cached(spawn_kart).expect("kart system runs");
        }
    }
    for mut window in world.query::<&mut Window>().iter_mut(world) {
        window.title = format!(
            "ctr-rs: {}, {}",
            maps::name(id),
            CHARACTERS[options.racer as usize]
        );
    }
}

/// Builds the scene of the level shown, in `mode`.
pub(crate) fn run_scene_systems(world: &mut World, mode: Mode) {
    let systems = world.resource::<SceneSystems>().of(mode);
    let count = systems.len();
    for (n, system) in systems.into_iter().enumerate() {
        stage(format_args!("scene system {} of {count}", n + 1));
        world.run_system(system).expect("scene system runs");
    }
    stage("scene systems done");
}

/// Builds the scene of the level shown again, in `mode` (the race starts from the grid).
pub(crate) fn rebuild_scene(world: &mut World, mode: Mode) {
    let scene = world_scene(world);
    despawn(world, scene);
    if mode == Mode::Drive {
        restart_drive(world);
    }
    run_scene_systems(world, mode);
    crate::loading::begin(world);
}

/// `OnEnter` of the race and the viewer, the first `Drive` at startup included (which is how
/// the first scene is built: before `PreStartup`). The podium's is `podium::enter`.
pub(crate) fn enter_mode(world: &mut World) {
    let mode = *world.resource::<State<Mode>>().get();
    rebuild_scene(world, mode);
}

pub(crate) fn exit_drive(mut commands: Commands) {
    commands.remove_resource::<drive::Drive>();
}

/// The viewer starts where the race's camera was. (Between `OnExit` and `OnEnter`: the chase
/// camera is still there.)
pub(crate) fn keep_viewer_start(
    cameras: Query<&GlobalTransform, With<drive::ChaseCamera>>,
    mut commands: Commands,
) {
    if let Some(at) = cameras.iter().next() {
        commands.insert_resource(ViewerStart(at.compute_transform()));
    }
}

/// `/debug viewer`: the race becomes the viewer with the probe, or the viewer the race again
/// (`enter_mode`); and `/debug level`: how the level is built, which builds the scene again in
/// the mode it is in.
pub(crate) fn switch_mode(
    mut actions: MessageReader<command::Action>,
    lobby: Res<lobby::Lobby>,
    race_on: Res<crate::race_on::RaceOn>,
    mesh_options: Res<MeshOptions>,
    mode: Res<State<Mode>>,
    mut next: ResMut<NextState<Mode>>,
    mut toasts: MessageWriter<toast::Toast>,
    mut commands: Commands,
) {
    use command::{Action, MeshSetting};
    let refuse = |toasts: &mut MessageWriter<toast::Toast>, why: &str| {
        toasts.write(toast::Toast::error(why, 2500));
    };
    for action in actions.read() {
        let setting = match action {
            Action::DebugViewer => None,
            Action::DebugLevel(None) => {
                let on = |b| if b { "on" } else { "off" };
                toasts.write(toast::Toast::new(
                    format!(
                        "Level: hidden {}, cull {}, colors {}, crate fixup {}",
                        on(mesh_options.hidden),
                        on(mesh_options.cull),
                        if mesh_options.low { "lo" } else { "hi" },
                        on(mesh_options.crate_fixup),
                    ),
                    3000,
                ));
                continue;
            }
            Action::DebugLevel(setting) => *setting,
            _ => continue,
        };
        // The scene is rebuilt: the race would start over.
        if lobby.is_open() || race_on.is_on() {
            refuse(&mut toasts, "Not in a lobby or a race (/lobby close)");
            continue;
        }
        // (The podium's scene is its own, which the common scene systems alone do not build.)
        if *mode.get() == Mode::Podium {
            refuse(&mut toasts, "Not on the podium");
            continue;
        }
        let Some(setting) = setting else {
            let to = match mode.get() {
                Mode::Drive => Mode::Viewer,
                _ => Mode::Drive,
            };
            next.set_if_different(to);
            toasts.write(toast::Toast::new(
                if to == Mode::Drive { "Race" } else { "Viewer" },
                2000,
            ));
            continue;
        };
        commands.queue(move |world: &mut World| {
            let mut mesh = world.resource_mut::<MeshOptions>();
            match setting {
                MeshSetting::Hidden(on) => mesh.hidden = on,
                MeshSetting::Cull(on) => mesh.cull = on,
                MeshSetting::LowColors(low) => mesh.low = low,
                MeshSetting::CrateFixup(on) => mesh.crate_fixup = on,
            }
            let mode = *world.resource::<State<Mode>>().get();
            rebuild_scene(world, mode);
            world.write_message(toast::Toast::new("Level rebuilt", 2000));
        });
    }
}

/// Everything in the scene has a transform (the 3D parts) or a node (the text); the window and
/// the gamepads have neither.
pub(crate) fn world_scene(world: &mut World) -> Vec<Entity> {
    world
        .query_filtered::<Entity, (
            Or<(With<Transform>, With<Node>)>,
            Without<crate::pixel::Present>,
            Without<OutlivesScene>,
        )>()
        .iter(world)
        .collect()
}

/// Not of the scene, so a level change leaves it be: the loading screen, which a load happens
/// under.
#[derive(Component)]
pub(crate) struct OutlivesScene;

pub(crate) fn despawn(world: &mut World, entities: Vec<Entity>) {
    for entity in entities {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
}

/// The kart on the grid of the level shown, and what it is driven with. `Mode::Drive` only.
pub(crate) fn restart_drive(world: &mut World) {
    stage("starting the drive");
    let racer = world.resource::<Options>().racer as usize;
    let kit = world.resource_mut::<crate::Kits>().get(racer);
    let drive = new_drive(world.resource::<Track>(), &kit);
    world.insert_resource(drive);
    world.insert_resource(exhaust::Particles::new());
    stage("drive started");
}

/// Spawns one entity per non-empty part, with the material its blend mode needs.
/// The render layer of everything of the kart: its body, wheels, fire, smoke and sparks. A second
/// camera draws them over the rest with a fresh depth buffer, which is the closest thing to retail
/// giving them a slot of their own in the ordering table (it has no depth buffer): they are
/// ordered among themselves by depth, and never hidden by the track.
pub(crate) const KART_LAYER: usize = 1;

/// The render layer of the kart of local player `slot` (0 is ours), and what goes with it: the
/// layer that the camera over that player's picture draws, and the other players' main cameras
/// draw as well, so that each sees the others' karts in the world.
pub(crate) fn player_layer(slot: usize) -> usize {
    if slot == 0 { KART_LAYER } else { 20 + slot }
}

/// The cameras that draw over the picture of the main one: the one for [`KART_LAYER`], and the
/// HUD's (`wumpa_icon` and `tracker`), and the cameras of the local players after the first. The
/// systems that follow "the camera" (the wheels' view, the smoke, the skid marks) look for the
/// main camera, which has not got this.
#[derive(Component)]
pub(crate) struct OverlayCamera;

/// What one step of the game's draw order bias (a face filed that many ordering table slots
/// farther back) is worth in `wgpu` constant depth bias units. Those are one ulp of the depth
/// there, so a flush decal needs a good few of them to win against its surface.
pub(crate) const DEPTH_BIAS_PER_ORDER: f32 = 8.0;

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
    spawn_faded_parts(
        parts,
        base,
        transform,
        parent,
        commands,
        meshes,
        materials,
        subtract_materials,
        layer,
        None,
    )
}

/// [`spawn_parts`], see-through at `alpha` if given: every part blends then, the opaque ones too.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_faded_parts(
    parts: [mesh::MeshPart; PARTS],
    base: &StandardMaterial,
    transform: Transform,
    parent: Option<Entity>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    subtract_materials: &mut Assets<SubtractMaterial>,
    layer: usize,
    alpha: Option<f32>,
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
        alpha,
    )
    .try_into()
    .unwrap()
}

/// Spawns `parts` (slot `i` is blended as `i % PARTS`). `biases` holds the draw order bias of each
/// group of [`PARTS`] slots, 0 for those beyond it: a face with a higher one draws behind.
pub(crate) fn spawn_layers(
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
    alpha: Option<f32>,
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
            base_color: alpha.map_or(base.base_color, |a| base.base_color.with_alpha(a)),
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
                let mode = alpha.map_or(AlphaMode::Mask(0.5), |_| AlphaMode::Blend);
                entity.insert(MeshMaterial3d(materials.add(with(mode))))
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

pub(crate) fn setup(
    mut commands: Commands,
    track: Res<Track>,
    textures: Res<Textures>,
    mut images: ResMut<Assets<Image>>,
    mesh_options: Res<MeshOptions>,
    crate_bias: Res<crate::CrateBias>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
    mut billboard_materials: ResMut<Assets<crate::billboard::BillboardMaterial>>,
    driving: Option<Res<drive::Drive>>,
    kart_parts: Res<crate::KartParts>,
    start: Option<Res<ViewerStart>>,
    mode: Res<State<Mode>>,
) {
    let mode = *mode.get();
    let level = &track.0;
    let data = mesh::level_mesh(
        level,
        &textures.dram,
        &textures.vram,
        mesh_options.low,
        !mesh_options.hidden,
        mesh_options.cull,
    );
    debug!(
        "{} turbo pad collision blocks, {} blocks that show them (those get the extra depth bias)",
        level
            .quad_blocks
            .iter()
            .filter(|q| mesh::is_turbo_pad(q))
            .count(),
        mesh::turbo_pad_visuals(level).len()
    );
    debug!(
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
        cull_mode: mesh_options
            .cull
            .then_some(bevy::render::render_resource::Face::Back),
        double_sided: !mesh_options.cull,
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
        None,
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

    stage("setup: props");
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
        crate_bias.0,
        track.1,
        // (Its instance numbers are the 1P LEVs'.)
        mesh_options.crate_fixup && track.2 == LevMode::Arcade,
    );
    // The previous level's fire, if the level has none.
    commands.remove_resource::<fire::Fire>();
    stage("setup: fire");
    fire::spawn(
        fire::Sources {
            emitters: objects.fireballs.len() + objects.flame_jets.len(),
            plants: objects.plants.len(),
        },
        &kart_parts.wheels,
        kart_parts.smoke.as_ref(),
        &textures.dram,
        &textures.vram,
        &mut commands,
        &mut images,
        &mut meshes,
        &mut billboard_materials,
    );
    let specs: Vec<race::KartSpec> = driving.iter().map(|d| d.spec()).collect();
    stage("setup: sim");
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
    // The game projects with a screen distance of 256 for a 512 pixel wide
    // screen: 90 degrees across, 73.7 degrees up at 4:3.
    let game_perspective = PerspectiveProjection {
        fov: 2.0 * (0.75f32).atan(),
        ..default()
    };
    stage("setup: cameras");
    if mode == Mode::Drive {
        let perspective = game_perspective;
        if let Some(skybox) = &level.skybox {
            sky::spawn(
                &mut commands,
                &mut meshes,
                &mut materials,
                skybox,
                level.clear_colors[0],
                perspective.clone(),
            );
        }
        let projection = Projection::Perspective(perspective);
        let mut main = commands.spawn((
            Camera3d::default(),
            Camera {
                clear_color: sky_clear,
                ..default()
            },
            projection.clone(),
            Transform::default(),
            drive::ChaseCamera::new(),
            drive::MainView(0),
            drive::ViewOf(0),
        ));
        // The heat shimmer, a pass over the whole picture: Tiger Temple's flame jets have it, and
        // the warpball's glow on every track.
        main.insert(crate::heat::HeatWarp::default());
        main.with_child((
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
                drive::ViewOf(0),
                RenderLayers::layer(KART_LAYER),
                // The clock's trail is over the world and our kart, which this camera draws last.
                crate::clock::ClockTrail::default(),
            ))
;
        // The HUD's figures (the fruit and the tracker) share one camera, over all the others'
        // pictures with only the depth buffer cleared. It looks at the middle of the window, with
        // y up, and the figures hang `DEPTH` pixels in front of it.
        commands.spawn((
            OverlayCamera,
            // Not pixelated with the scene (`crate::pixel`).
            crate::pixel::OnWindow,
            Camera3d::default(),
            Camera {
                order: crate::pixel::FIGURES_ORDER,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::WindowSize,
                ..OrthographicProjection::default_3d()
            }),
            Transform::default(),
            RenderLayers::from_layers(&[crate::wumpa_icon::LAYER, crate::tracker::LAYER]),
        ));
    } else if mode == Mode::Podium {
        if let Some(skybox) = &level.skybox {
            sky::spawn(
                &mut commands,
                &mut meshes,
                &mut materials,
                skybox,
                level.clear_colors[0],
                game_perspective.clone(),
            );
        }
        commands.spawn((
            Camera3d::default(),
            Camera {
                clear_color: sky_clear,
                ..default()
            },
            Projection::Perspective(game_perspective),
            Transform::default(),
            crate::podium::PodiumCamera,
            drive::MainView(0),
        ));
    } else {
        if let Some(skybox) = &level.skybox {
            sky::spawn(
                &mut commands,
                &mut meshes,
                &mut materials,
                skybox,
                level.clear_colors[0],
                default(),
            );
        }
        let (transform, yaw, pitch) = match start {
            Some(start) => {
                let (yaw, pitch, _) = start.0.rotation.to_euler(EulerRot::YXZ);
                commands.remove_resource::<ViewerStart>();
                (start.0, yaw, pitch)
            }
            None => (Transform::from_xyz(x, y + 3.0, z + 6.0), 0.0, -0.3),
        };
        commands.spawn((
            Camera3d::default(),
            Camera {
                clear_color: sky_clear,
                ..default()
            },
            transform,
            FlyCamera {
                yaw,
                pitch,
                speed: 10.0,
            },
        ));
    }
}

/// Where the viewer's camera starts, when `/debug viewer` leaves a race: where the chase camera
/// was. Taken by `setup`.
#[derive(Resource)]
pub(crate) struct ViewerStart(Transform);

/// Spawns our kart, and what goes with it (its flames, shadow, skid marks and exhaust). Driven,
/// it is its sim's racer (a race's may not be our pick); else the pick.
pub(crate) fn spawn_kart(
    track: Res<Track>,
    sim: Res<props::Sim>,
    options: Res<Options>,
    mode: Res<State<Mode>>,
    mut builder: kart::KartBuilder,
) {
    let spawn = track.0.driver_spawns[0];
    let [x, y, z] = mesh::to_world(spawn.pos);
    let yaw = spawn.rot.y as f32 / 4096.0 * std::f32::consts::TAU;
    // The model faces along +X; a driver's heading is along +Z. A driven kart is posed by its
    // sim instead.
    let placement = Transform::from_xyz(x, y, z)
        .with_rotation(Quat::from_rotation_y(yaw) * Quat::from_rotation_y(MODEL_FORWARD_YAW));
    let driven = (*mode.get() == Mode::Drive).then(|| (sim.own, sim.kart()));
    let character = driven.map_or(options.racer as usize, |(_, kart)| {
        kart.driver.character as usize
    });
    let kit = builder.kit(character);
    builder.spawn(&kit, placement, true, driven);
}

/// The images made so far, by what is in them: the id of one that still lives.
static SHARED_IMAGES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<u64, AssetId<Image>>>,
> = std::sync::LazyLock::new(Default::default);

/// Adds `image`, unless an image of the very same texels is there already: then that one. A kart
/// has dozens of pictures (the flames, masks, shields, wheels, shadow), and every kart on the grid
/// has the same ones; each in its own copy was a gigabyte on a phone. An image that nothing
/// holds is dropped by the assets, and then it is made again.
pub(crate) fn shared_image(images: &mut Assets<Image>, image: Image) -> Handle<Image> {
    use std::hash::{BuildHasher, Hash, Hasher};
    let mut hasher = bevy::platform::hash::FixedHasher.build_hasher();
    image.texture_descriptor.size.hash(&mut hasher);
    image.texture_descriptor.format.hash(&mut hasher);
    image.data.hash(&mut hasher);
    let key = hasher.finish();
    let mut shared = SHARED_IMAGES.lock().unwrap();
    if let Some(handle) = shared.get(&key).and_then(|&id| images.get_strong_handle(id)) {
        return handle;
    }
    let handle = images.add(image);
    shared.insert(key, handle.id());
    handle
}

/// The material of a kart model's parts: its texture atlas, unlit.
pub(crate) fn kart_material(atlas: mesh::Atlas, images: &mut Assets<Image>) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(crate::hud::atlas_image(atlas, images)),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    }
}

/// Racer models face along +X, while a driver's heading (`spawn.rot.y`) points along
/// +Z: a quarter turn clockwise seen from above (Bevy's +Y is up, so negative).
pub(crate) const MODEL_FORWARD_YAW: f32 = std::f32::consts::FRAC_PI_2;
