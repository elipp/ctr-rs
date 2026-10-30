//! "FREE DRIVE MODE" in the top left corner, in the small font and a friendly yellow, while no
//! race is on. In a lobby the portraits of the players (`lobby_list.rs`) are in that corner, and
//! the text moves to their right.

use bevy::prelude::*;
use render::font::{Font, Gradient};
use render::portraits::WIDTH;

use crate::font::CtrText;
use crate::hud;
use crate::leaderboard::ICON_X;

/// Where the text is, in pixels of retail's screen: from the left edge, or in a lobby from the
/// right of the portraits, and down from the top.
const LEFT: f32 = 8.0;
const LEFT_IN_LOBBY: f32 = ICON_X + WIDTH as f32 + 4.0;
const TOP: f32 = 6.0;

/// A texel of the text, in logical pixels.
const TEXEL: Vec2 = Vec2::new(0.5 * hud::PIXEL.x, 0.5 * hud::PIXEL.y);

/// A friendly yellow: pale at the top, golden at the bottom.
const GRADIENT: Gradient = Gradient {
    top: [0xff, 0xf4, 0x90],
    bottom: [0xf0, 0xc0, 0x30],
};

#[derive(Component)]
struct FreeDrive;

pub struct FreeDrivePlugin;

impl Plugin for FreeDrivePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show.run_if(crate::drive::driving));
    }
}

fn show(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    menu: Res<crate::pause::PauseMenu>,
    lobby: Res<crate::lobby::Lobby>,
    mut text: Query<(&mut Node, &mut Visibility), With<FreeDrive>>,
) {
    let Ok((mut node, mut visibility)) = text.single_mut() else {
        // The first frame, or of a new level (which took the nodes away).
        commands.spawn((
            FreeDrive,
            Node {
                position_type: PositionType::Absolute,
                left: hud::x(LEFT),
                top: hud::y(TOP),
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            CtrText::new("FREE DRIVE", Font::Middle, GRADIENT).fixed(TEXEL),
        ));
        return;
    };
    let left = hud::x(if lobby.members().next().is_some() {
        LEFT_IN_LOBBY
    } else {
        LEFT
    });
    if node.left != left {
        node.left = left;
    }
    let shown = sim.race.laps == race::FREE_DRIVE_LAPS && !menu.open;
    visibility.set_if_neq(if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
}
