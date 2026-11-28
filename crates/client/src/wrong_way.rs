//! "WRONG WAY!" over the middle of the screen, in the large font, orange, while our kart (or the
//! one we watch) drives against the track and has for long enough (`UI_RenderFrame_Racing`,
//! `vehicle::lap::wrong_way`). It flashes with the race's clock: on while its bit 0x80 is, a
//! little under four times a second. Not while the race is paused, nor in a free drive. What the masks say of it is
//! `howl`'s.

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;
use crate::hud_view::{HudView, HudViews};

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
    hud: HudViews,
    menu: Res<crate::pause::PauseMenu>,
    mut lines: Query<(&HudView, &mut Visibility), With<WrongWay>>,
) {
    for view in hud.missing(lines.iter().map(|(v, _)| *v)) {
        // The first frame, or of a new level (which took the nodes away).
        commands.spawn((
            WrongWay,
            HudView(view),
            Visibility::Hidden,
            crate::bands::line(
                crate::bands::Band::WrongWay,
                CtrText::new("WRONG WAY!", Font::Large, Gradient::ORANGE).fixed(hud::PIXEL),
            ),
        ));
    }
    let race = &*sim.race;
    for (&HudView(view), mut visibility) in &mut lines {
        let Some(kart) = hud.get(view) else {
            continue;
        };
        let shown = race.mode.is_race()
            && vehicle::lap::wrong_way(kart.driver)
            && race.event_ms & FLASH_BIT != 0
            && !menu.open;
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}
