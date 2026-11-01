//! The loading screen. Building a scene (`scene.rs`) is not the end of it: the HUD's pictures and
//! cameras are set up by commands over the next frames, and a second change before they land
//! despawns what they are about to touch. So after each change the screen is covered, the game
//! clock stops, and nothing goes on (the keys do not change the level again, an online race does
//! not say it is ready) until the scene is whole.
//!
//! The scene is whole when a few frames have passed and what a race needs is there: our kart
//! and the chase camera. If that does not come in [`GIVE_UP_FRAMES`] the screen goes away anyway
//! and says what was missing in the log, rather than hold the game up for good.

use bevy::prelude::*;

use crate::{LevelNumber, drive, maps, scene::KartScene};

/// The frames the scene always gets to settle, whatever else is there.
const MIN_FRAMES: u32 = 3;

/// The frames after which the screen goes away even if the scene is not whole (3 seconds at 60).
const GIVE_UP_FRAMES: u32 = 180;

/// A scene is being settled: present from the change until [`finish`].
#[derive(Resource, Default)]
pub struct Loading {
    frames: u32,
}

/// The screen that covers the window.
#[derive(Component)]
struct LoadingScreen;

/// Covers the window and stops the game clock, until the scene that was just built has settled.
pub fn begin(world: &mut World) {
    if world.contains_resource::<Loading>() {
        return;
    }
    world.insert_resource(Loading::default());
    world.resource_mut::<Time<Virtual>>().pause();
    let name = maps::name(world.resource::<LevelNumber>().0);
    world
        .spawn((
            LoadingScreen,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::BLACK),
            GlobalZIndex(i32::MAX - 3),
        ))
        .with_child((
            Text::new(format!("Loading {name}...")),
            TextFont {
                font_size: bevy::text::FontSize::Px(32.0),
                ..default()
            },
            TextColor(Color::WHITE),
        ));
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

/// Counts the frames of the loading screen, and takes it away when the scene is whole.
fn finish(
    mut loading: ResMut<Loading>,
    driving: Option<Res<drive::Drive>>,
    karts: Query<(), With<KartScene>>,
    chase: Query<(), With<drive::ChaseCamera>>,
    screens: Query<Entity, With<LoadingScreen>>,
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
    for screen in &screens {
        commands.entity(screen).try_despawn();
    }
    time.unpause();
    commands.remove_resource::<Loading>();
}

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, finish.run_if(resource_exists::<Loading>));
    }
}
