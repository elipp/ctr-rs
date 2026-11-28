//! The heights of the lines centred across the screen (or across a split screen's view, for a
//! line under a `HudView`), all in one place: a line that goes up elsewhere has to fit between
//! these. Top to bottom.
//!
//! Mixed units, as retail's lines are in its screen's (`hud::y`) and ours in the window's. On a
//! short window the toasts' column, which grows down, reaches [`Band::Status`]: the countdown is
//! under it for that, in what is otherwise the status label's place (`lobby/hud.rs` never shows
//! both).

use bevy::prelude::*;

use crate::font::CtrText;
use crate::hud;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Band {
    /// The toasts' column (`toast.rs`): under the weapon slot.
    Toasts,
    /// Solo's end panel (`solo_end.rs`).
    SoloEnd,
    /// The lobby's waiting, paused and quick play labels (`lobby/hud.rs`).
    Status,
    /// The countdown to a lobby's race.
    Countdown,
    /// "You came in 2nd" (`lobby/placing.rs`).
    Placing,
    /// Retail's "WRONG WAY!": 30 of its lines above the middle.
    WrongWay,
    /// The podium's "press any key": retail's line 0xbe of 216 (`CS_PODIUM_CONTINUE_TEXT_Y`).
    PodiumContinue,
    /// From the bottom: who the camera follows (`names.rs`), or the replay's banner, which are
    /// never up together.
    Watching,
}

/// Between the weapon slot's bottom and the first toast, in pixels.
const BELOW_WEAPON: f32 = 8.0;

impl Band {
    /// A node across its parent at the band's height, its children centred in a column.
    pub(crate) fn node(self) -> Node {
        let mut node = Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            ..default()
        };
        match self {
            Band::Toasts => node.top = Val::Px(crate::weapon::WEAPON_BOTTOM + BELOW_WEAPON),
            Band::SoloEnd => node.top = hud::y(40.0),
            Band::Status => node.top = Val::Percent(20.0),
            Band::Countdown => node.top = Val::Percent(24.0),
            Band::Placing => node.top = Val::Percent(30.0),
            Band::WrongWay => {
                node.top = Val::Percent(50.0);
                node.margin = UiRect::top(-hud::y(30.0));
            }
            Band::PodiumContinue => node.top = Val::Percent(0xbe as f32 / 216.0 * 100.0),
            Band::Watching => node.bottom = Val::Px(60.0),
        }
        node
    }
}

/// A line of `text` alone in `band`.
pub(crate) fn line(band: Band, text: CtrText) -> impl Bundle {
    (
        band.node(),
        Pickable::IGNORE,
        children![(text, Pickable::IGNORE)],
    )
}
