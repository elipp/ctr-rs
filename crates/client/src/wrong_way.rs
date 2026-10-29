//! "WRONG WAY!" over the middle of the screen, in the large font, orange, while our kart (or the
//! one we watch) drives against the track and has for long enough (`UI_RenderFrame_Racing`,
//! `vehicle::lap::wrong_way`). It flashes with the race's clock: on while its bit 0x80 is, a
//! little under four times a second. Not while the race is paused, nor in a free drive. What the masks say of it is
//! `howl`'s.

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;

/// How far above the middle of the screen the text's top is, in lines of retail's screen.
const ABOVE_MIDDLE: f32 = 30.0;

/// The bit of the race's clock (`elapsedEventTime`, in milliseconds) that the text is on with.
const FLASH_BIT: i32 = 0x80;

/// The line across the screen that holds the text.
#[derive(Component)]
struct WrongWay;

pub struct WrongWayPlugin;

impl Plugin for WrongWayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show.run_if(crate::drive::driving));
    }
}

fn show(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    menu: Res<crate::pause::PauseMenu>,
    mut line: Query<&mut Visibility, With<WrongWay>>,
) {
    let Ok(mut visibility) = line.single_mut() else {
        // The first frame, or of a new level (which took the nodes away).
        commands.spawn((
            WrongWay,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(50.0),
                margin: UiRect::top(-hud::y(ABOVE_MIDDLE)),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            children![(
                CtrText::new("WRONG WAY!", Font::Large, Gradient::ORANGE).fixed(hud::PIXEL),
                Pickable::IGNORE,
            )],
        ));
        return;
    };
    let race = &*sim.race;
    let shown = race.laps != race::FREE_DRIVE_LAPS
        && race
        .karts
        .get(sim.own)
        .is_some_and(|kart| vehicle::lap::wrong_way(&kart.driver))
        && race.event_ms & FLASH_BIT != 0
        && !menu.open;
    visibility.set_if_neq(if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
}
