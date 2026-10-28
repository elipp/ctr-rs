//! The client: the race on the keyboard, see `drive.rs`, and, with `/debug viewer`, a fly-through
//! viewer for a track's geometry (textured). There are no command line options: the level is 0
//! and the racer the one driven last time (`config.rs`), and `]` / Page Up and `[` / Page Down
//! switch to the next and the previous level, `.` and `,` to the next and the previous racer. The
//! disc image is `CTR_BIN`, the `iso` of the config, or `assets/ctr-u.bin`
//! in a debug build; on Windows a file dialog asks for it (see `disc.rs`).
//!
//! The viewer: a collision probe from the camera is drawn and described on screen; see
//! `probe.rs` for its keys. Hold the right mouse button to look, WASD to move, Space/Shift
//! up/down, Ctrl to go fast.

// A release build on Windows has no console window; `crash.rs` tells of a panic instead.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Bevy systems take their parameters and `Query` filters as types, so both lints fire on them.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

mod animation;
mod assets;
mod banner;
mod command;
mod confetti;
mod crash;
mod config;
mod disc;
mod drive;
mod exhaust;
mod explosion;
mod fire;
mod flame;
mod flash;
mod fly;
mod font;
mod frame;
mod fx;
mod heat;
mod hud;
mod kart;
mod laps;
mod leaderboard;
mod lights;
mod lobby;
mod lobby_list;
mod maps;
mod mask;
mod minimap;
mod names;
mod pause;
mod paths;
mod place;
mod probe;
mod projectile;

mod props;
mod rain;
mod remote;
mod resources;
mod scene;
mod shadow;
mod shield;
mod skid;
mod sky;
mod slide_meter;
mod sound;
mod spider;
mod subtract;
mod suggest;
mod toast;
mod tracker;
mod weapon;
mod weather;
mod wheels;
mod wrong_way;
mod wumpa_icon;

use bevy::prelude::*;
use loader::level::LevelId;
use subtract::SubtractMaterial;

pub(crate) use animation::*;
pub(crate) use assets::*;
use fly::*;
use frame::*;
pub(crate) use resources::*;
pub(crate) use scene::*;

fn main() {
    crash::install();
    let track_id = 0;
    let mut config = config::Config::load();
    disc::locate(&mut config);
    // The character driven last time.
    let racer = config
        .character
        .filter(|&c| (c as usize) < CHARACTERS.len())
        .unwrap_or(0);
    let options = Options {
        racer,
        driving: true,
    };
    let LevelData { level, dram, vram } = load_level(track_id);
    let character_data = load_character(racer);
    let Shared {
        crate_explosion,
        masks,
        shields,
        projectiles,
        fruit_display,
        fonts,
        digits,
    } = load_shared();
    let loaded = Track(level, LevelId(track_id));
    let textures = Textures { dram, vram };
    let drive = new_drive(&loaded, &character_data.racer, racer as usize);

    // See the executor below: without a render thread, `CTR_EXECUTOR=multi` keeps the defaults.
    let single_threaded = std::env::var("CTR_EXECUTOR").as_deref() != Ok("multi");
    let mut app = App::new();
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!(
                "ctr-rs: {}, {}",
                maps::name(track_id),
                CHARACTERS[racer as usize]
            ),
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
    // After the plugins, which set up the log that tells of a map that cannot be read.
    maps::load();
    // Bevy's own font has no ä or ö: this one takes its place as the default.
    app.world_mut()
        .resource_mut::<Assets<Font>>()
        .insert(
            AssetId::default(),
            Font::from_bytes(include_bytes!("../../../assets/Inconsolata-Regular.ttf").to_vec()),
        )
        .expect("the default font can be replaced");
    let digits = place::BigDigits::new(digits, &mut app.world_mut().resource_mut());
    app.insert_resource(digits);
    app.insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.1)))
        .insert_resource(loaded)
        .insert_resource(textures)
        .insert_resource(explosion::CrateExplosion::new(crate_explosion))
        .insert_resource(mask::MaskModels(Some(masks)))
        .insert_resource(shield::ShieldModels(Some(shields)))
        .insert_resource(wumpa_icon::FruitDisplay(fruit_display))
        .insert_resource(projectile::ProjectileModels::new(projectiles))
        .insert_resource(fonts)
        .add_plugins(font::FontPlugin)
        .insert_resource(options)
        .init_resource::<MeshOptions>()
        .init_resource::<CrateBias>()
        .insert_resource(LevelNumber(track_id))
        .add_systems(Startup, (setup, spawn_kart).chain())
        .add_systems(Update, log_draws)
        // .add_systems(Last, limit_frame_rate)
        // After the cameras and karts have moved this frame, before their transforms propagate.
        .add_systems(
            PostUpdate,
            wheels::update.before(TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            kart::hide_parts
                .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
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
                props::set_crate_bias,
                props::place_hazards,
                fire::update,
            )
                .chain(),
        );
    character_data.insert(app.world_mut());
    app.insert_resource(config);
    let world = app.world_mut();
    let scene_systems = SceneSystems {
        common: vec![
            world.register_system(setup),
            world.register_system(spawn_kart),
        ],
        race: vec![
            world.register_system(drive::spawn_hud),
            world.register_system(lobby::spawn_waiting),
        ],
        viewer: vec![world.register_system(probe::spawn_text)],
    };
    // The systems of both modes are there all the time: the race's run with the `Drive` resource
    // (`drive::driving`), the viewer's without it, and `/debug viewer` takes it away or gives it
    // back (`switch_mode`).
    app.insert_resource(sound::Sounds::load(&bigfile_path()));
    app.insert_resource(drive).add_plugins((
        drive::DrivePlugin,
        sound::SoundPlugin,
        lobby::LobbyPlugin,
        command::CommandPlugin,
        pause::PausePlugin,
        fx::FxPlugin,
        // The race's HUD. (A tuple of plugins holds 15 at most.)
        (
            leaderboard::LeaderboardPlugin,
            lobby_list::LobbyListPlugin,
            lights::LightsPlugin,
            place::PlacePlugin,
            laps::LapsPlugin,
            weapon::WeaponPlugin,
            tracker::TrackerPlugin,
            wumpa_icon::WumpaIconPlugin,
            minimap::MinimapPlugin,
            heat::HeatPlugin,
            slide_meter::SlideMeterPlugin,
            toast::ToastPlugin,
            wrong_way::WrongWayPlugin,
        ),
        config::ConfigPlugin,
        probe::ProbePlugin,
    ));
    app.add_systems(Update, fly.run_if(not(drive::driving)))
        .add_systems(Update, switch_mode);
    app.insert_resource(scene_systems);
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
