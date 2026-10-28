//! The traffic lights of an online race's start (`render::lights`): when the countdown starts they
//! slide down from above the screen, come on one by one (the three reds and then the green), and
//! after the green they slide back up. What is on is the race's `lights_ms`
//! (`trafficLightsTimer`), as `race::lights_on` says. The slides and where the lights stop are
//! ours, from screenshots: retail's (`DotLights_AudioAndVideo`) were not checked.

use bevy::prelude::*;
use race::{LIGHTS_FLOOR_MS, START_LIGHTS_MS, lights_on};
use render::lights::{HEIGHT, LIGHTS, WIDTH, atlas, cell};

use crate::hud;
use crate::leaderboard::{atlas_image, pack_icons};

/// Where the lights' tops stop, in lines of retail's screen: under the weapon.
const TOP: f32 = 40.0;

/// How long each slide takes, in milliseconds of the timer: the first after the countdown starts,
/// the second before the lights go out for good.
const SLIDE_MS: f32 = 480.0;

/// The timer counts down this much a tick.
const MS_PER_TICK: f32 = 32.0;

/// The lights' pictures (`render::lights::atlas`), made when the first race needs them.
#[derive(Resource)]
struct LightsImage(Handle<Image>);

/// The row of lights.
#[derive(Component)]
struct Lights;

/// A light, the reds 0 to 2 and the green 3.
#[derive(Component)]
struct Light(usize);

pub struct LightsPlugin;

impl Plugin for LightsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show.run_if(crate::drive::driving));
    }
}

/// How far down the lights are at the timer `ms`: 0 above the screen, 1 where they stop.
fn down(ms: f32) -> f32 {
    let (start, floor) = (START_LIGHTS_MS as f32, LIGHTS_FLOOR_MS as f32);
    ((start - ms) / SLIDE_MS)
        .min((ms - floor) / SLIDE_MS)
        .clamp(0.0, 1.0)
}

fn show(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    lobby: Res<crate::lobby::Lobby>,
    fixed: Res<Time<Fixed>>,
    textures: Res<crate::Textures>,
    mut images: ResMut<Assets<Image>>,
    image: Option<Res<LightsImage>>,
    mut row: Query<(&mut Node, &mut Visibility), With<Lights>>,
    mut lights: Query<(&Light, &mut ImageNode)>,
) {
    let Ok((mut node, mut visibility)) = row.single_mut() else {
        // The first frame, or of a new level (which took the nodes away).
        let image = match image {
            Some(image) => image.0.clone(),
            None => {
                let image = atlas_image(atlas(&pack_icons(), &textures.vram), &mut images);
                commands.insert_resource(LightsImage(image.clone()));
                image
            }
        };
        spawn(&mut commands, image);
        return;
    };
    let race = &*sim.race;
    // Between ticks the timer goes on, for the slides to be smooth.
    let counting = race.tick >= race.intro_ticks && race.lights_ms > LIGHTS_FLOOR_MS;
    let ms = race.lights_ms as f32
        - if counting {
            MS_PER_TICK * fixed.overstep_fraction()
        } else {
            0.0
        };
    // Only an online race has the start with the lights, once the sim has gone back to its tick 0
    // (until then it is the free drive's, which has none).
    let started = lobby
        .online
        .as_ref()
        .is_some_and(|net| net.built && !net.fresh);
    let down = if started { down(ms) } else { 0.0 };
    let shown = if down > 0.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    visibility.set_if_neq(shown);
    // Out of sight above the screen, the lights' height up from its top.
    node.top = hud::y(TOP - (1.0 - down) * (TOP + HEIGHT as f32));
    let on = lights_on(race.lights_ms);
    for (light, mut image) in &mut lights {
        let top = (cell(light.0, on) * HEIGHT) as f32;
        let rect = Some(Rect::new(0.0, top, WIDTH as f32, top + HEIGHT as f32));
        if image.rect != rect {
            image.rect = rect;
        }
    }
}

/// The row, in the middle across, hidden.
fn spawn(commands: &mut Commands, image: Handle<Image>) {
    let width = (LIGHTS * WIDTH) as f32;
    let row = commands
        .spawn((
            Lights,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                margin: UiRect::left(hud::x(-width / 2.0)),
                top: hud::y(TOP),
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .id();
    for n in 0..LIGHTS {
        commands.spawn((
            Light(n),
            // Stretch, as for every picture of the HUD (see `hud.rs`).
            ImageNode::new(image.clone()).with_mode(NodeImageMode::Stretch),
            Node {
                width: hud::x(WIDTH as f32),
                height: hud::y(HEIGHT as f32),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(row),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lights_slide_in_and_out() {
        let start = START_LIGHTS_MS as f32;
        assert_eq!(down(start), 0.0);
        assert_eq!(down(start - SLIDE_MS / 2.0), 0.5);
        assert_eq!(down(start - SLIDE_MS), 1.0);
        assert_eq!(down(0.0), 1.0);
        assert_eq!(down(LIGHTS_FLOOR_MS as f32 + SLIDE_MS / 2.0), 0.5);
        assert_eq!(down(LIGHTS_FLOOR_MS as f32), 0.0);
    }
}
