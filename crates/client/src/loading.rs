//! The loading screen. Building a scene (`scene.rs`) is not the end of it: the HUD's pictures and
//! cameras are set up by commands over the next frames, and a second change before they land
//! despawns what they are about to touch. So after each change the screen is covered, the game
//! clock stops, and nothing goes on (the keys do not change the level again, an online race does
//! not say it is ready) until the scene is whole.
//!
//! The scene is whole when a few frames have passed and what a race needs is there: our kart
//! and the chase camera. If that does not come in [`GIVE_UP_FRAMES`] the screen goes away anyway
//! and says what was missing in the log, rather than hold the game up for good.
//!
//! Most scenes settle in a few frames, so the screen is black only, and fades out (the game runs
//! under it): the level's name comes up only if it takes longer than [`NAME_DELAY_SECONDS`].

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::{LevelNumber, drive, hud, maps, scene::KartScene};

/// The frames the scene always gets to settle, whatever else is there.
const MIN_FRAMES: u32 = 3;

/// The frames after which the screen goes away even if the scene is not whole (3 seconds at 60).
const GIVE_UP_FRAMES: u32 = 180;

/// How long (real time) the screen is black before the level's name comes up on it.
const NAME_DELAY_SECONDS: f32 = 0.4;

/// How long the screen takes to fade out once the scene is whole.
const FADE_SECONDS: f32 = 0.25;

/// A scene is being settled: present from the change until [`finish`].
#[derive(Resource)]
pub struct Loading {
    frames: u32,
    /// When (`Time<Real>`) the screen went up.
    began: f32,
}

/// The screen that covers the window.
#[derive(Component)]
struct LoadingScreen;

/// The level's name on the screen, hidden until [`NAME_DELAY_SECONDS`] have passed.
#[derive(Component)]
struct LoadingName;

/// A screen whose scene is whole, fading out since then (`Time<Real>`).
#[derive(Component)]
struct Fading(f32);

/// Covers the window and stops the game clock, until the scene that was just built has settled.
pub fn begin(world: &mut World) {
    if world.contains_resource::<Loading>() {
        return;
    }
    // A screen still fading from the scene before is covered by this one.
    let fading: Vec<Entity> = world
        .query_filtered::<Entity, With<LoadingScreen>>()
        .iter(world)
        .collect();
    for screen in fading {
        world.entity_mut(screen).despawn();
    }
    let began = world.resource::<Time<Real>>().elapsed_secs();
    world.insert_resource(Loading { frames: 0, began });
    world.resource_mut::<Time<Virtual>>().pause();
    let text = format!(
        "Loading {}...",
        maps::name(world.resource::<LevelNumber>().0)
    );
    let screen = world
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
            Pickable::IGNORE,
        ))
        .id();
    let name = (LoadingName, Visibility::Hidden, ChildOf(screen));
    // A custom track's name may have what the game's font lacks.
    if text.chars().all(|c| render::font::drawable(Font::Large, c)) {
        world.spawn((
            name,
            CtrText::new(text, Font::Large, Gradient::ORANGE).fixed(hud::PIXEL),
        ));
    } else {
        world.spawn((
            name,
            Text::new(text),
            TextFont {
                font_size: bevy::text::FontSize::Px(32.0),
                ..default()
            },
            TextColor(Color::WHITE),
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

/// Counts the frames of the loading screen, shows the level's name if it takes a while, and
/// starts the fade when the scene is whole.
fn finish(
    mut loading: ResMut<Loading>,
    driving: Option<Res<drive::Drive>>,
    karts: Query<(), With<KartScene>>,
    chase: Query<(), With<drive::ChaseCamera>>,
    screens: Query<Entity, (With<LoadingScreen>, Without<Fading>)>,
    mut names: Query<&mut Visibility, With<LoadingName>>,
    real: Res<Time<Real>>,
    mut time: ResMut<Time<Virtual>>,
    mut commands: Commands,
) {
    loading.frames += 1;
    if real.elapsed_secs() - loading.began >= NAME_DELAY_SECONDS {
        for mut visibility in &mut names {
            visibility.set_if_neq(Visibility::Inherited);
        }
    }
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
        commands
            .entity(screen)
            .insert(Fading(real.elapsed_secs()))
            .despawn_related::<Children>();
    }
    time.unpause();
    commands.remove_resource::<Loading>();
}

/// Fades the black of a screen whose scene is whole, and takes it away when it is clear.
fn fade(
    real: Res<Time<Real>>,
    mut screens: Query<(Entity, &Fading, &mut BackgroundColor)>,
    mut commands: Commands,
) {
    for (entity, Fading(since), mut background) in &mut screens {
        let left = 1.0 - (real.elapsed_secs() - since) / FADE_SECONDS;
        if left <= 0.0 {
            commands.entity(entity).try_despawn();
        } else {
            background.0 = Color::BLACK.with_alpha(left);
        }
    }
}

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (finish.run_if(resource_exists::<Loading>), fade));
    }
}
