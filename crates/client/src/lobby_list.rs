//! The players in the lobby, down the left of the screen while no race is on, under a title in the
//! game's font ([`TITLE`]): each one's portrait
//! (`render::portraits`) and name, which is the nickname or, without one, the short key. (The
//! names are for Bevy's `Text`: a nickname may have anything in it.) Ours come first, then the
//! others in the lobby's order (the host's first). The host has a crown on its portrait's corner; in quick play, a player ready for the next race a tick on the other, and our row the button to say we are, or not, on its right
//! (`quick_ready.rs`). The other players at a machine (split screen) follow
//! the players, each named "name@2", "name@3", as in the race. Our names are in [`HIGHLIGHT`], as in the race's
//! leaderboard. (The rows are `portrait_row`'s, which the leaderboard's are too.) After the
//! players, the bots the next race fills the grid with ("+3 bots", and under it "medium, brain"),
//! if any, and then those who only watch. In a quick play match's lobby, between races and
//! before the first, the grid's free places follow the players instead of the bots, each a framed question mark
//! named "Open" (the 8 places fill the column).

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;
use crate::lobby::{Lobby, Row};
use crate::portrait_row::{
    DIM, HIGHLIGHT, PING_FONT_SIZE, TAG_PADDING, Portraits, portraits, spawn_empty_portrait,
    spawn_host_badge, spawn_name, spawn_portrait, spawn_ready_badge, tag,
};

/// The name of a free place of a quick play match's grid.
const OPEN: &str = "Open";

/// The column of rows, with the players it was built for, and which of them is us, the bots
/// (how many, and what kind), the ready hint, and the free places shown.
#[derive(Component)]
struct List(Vec<Row>, Vec<(String, String)>, (usize, String), Option<String>, usize);

/// How far the whole list (the title, the portraits and the spectators) is moved down, in lines
/// of retail's screen, to be clear of the toasts and the chat at the top.
const TABLE_Y: f32 = 12.0;

/// What the list is headed with. (The fonts have capitals, and this is how they show.)
const TITLE: &str = "PLAYERS IN LOBBY:";

/// Where the title is, in lines of retail's screen. It is [`Font::Middle`], 8 lines tall.
const TITLE_Y: f32 = 2.0;

/// Where the first portrait is, in lines of retail's screen, under the title.
const FIRST_Y: f32 = 12.0;

/// The portraits' size, of retail's (the race's leaderboard keeps that), and the lines from one
/// row to the next: small enough for 8 rows to fit the 240 lines with room under them
/// ([`TABLE_Y`] + [`FIRST_Y`] + 7 rows + a portrait's 20 make 198). The 2 lines between rows are
/// what keeps them apart: the names are inside the portraits.
const SCALE: f32 = 0.8;
const SLOT: f32 = 22.0;

/// Where the list is across, in pixels of retail's screen: as the race's leaderboard.
const X: f32 = crate::leaderboard::ICON_X;

/// The lines of retail's screen between the last portrait and the line of the bots, or the first
/// who only watches (more than a row's own gap), and from one of those lines to the next.
const WATCHER_GAP: f32 = 12.0;
const WATCHER_LINE: f32 = 9.0;

/// The lines of retail's screen from each of the bots' two lines ([`Font::Middle`], 8 lines
/// tall) to the next.
const BOTS_LINE: f32 = 12.0;

pub struct LobbyListPlugin;

impl Plugin for LobbyListPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, build.run_if(crate::drive::driving));
    }
}

/// Puts the list up when the roster changes, and takes it down when there is no lobby, or a race.
fn build(
    mut commands: Commands,
    lobby: Res<Lobby>,
    race_on: Res<crate::race_on::RaceOn>,
    textures: Res<crate::Textures>,
    mut images: ResMut<Assets<Image>>,
    loaded: Option<Res<Portraits>>,
    hint: Res<crate::quick_ready::ReadyHint>,
    lists: Query<(Entity, &List)>,
) {
    let mut players: Vec<Row> = if lobby.is_waiting(&race_on) {
        lobby.rows()
    } else {
        Vec::new()
    };
    // (Stable: our split screen's players keep their order, and so do the others.)
    players.sort_by_key(|row| !row.you);
    let watchers: Vec<(String, String)> = if lobby.is_waiting(&race_on) {
        lobby
            .watchers()
            .map(|(n, k)| (n.to_string(), k.to_string()))
            .collect()
    } else {
        Vec::new()
    };
    let free = if !players.is_empty() && lobby.matched_waiting(&race_on) {
        race::MAX_KARTS.saturating_sub(players.len())
    } else {
        0
    };
    // The free places stand for the bots too: a bot drives any no player takes.
    let bots = if players.is_empty() || lobby.matched_waiting(&race_on) {
        (0, String::new())
    } else {
        (lobby.next_bots(), lobby.next_bots_kind())
    };
    for (entity, list) in &lists {
        if list.0 == players
            && list.1 == watchers
            && list.2 == bots
            && list.3 == hint.0
            && list.4 == free
        {
            return;
        }
        commands.entity(entity).despawn();
    }
    if players.is_empty() && watchers.is_empty() {
        return;
    }
    let image = portraits(
        &mut commands,
        loaded.as_deref(),
        &textures.vram,
        &mut images,
    );
    let column = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::ZERO,
                top: hud::y(TABLE_Y),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    commands.spawn((
        CtrText::new(TITLE, Font::Middle, Gradient::ORANGE).fixed(hud::PIXEL),
        Node {
            position_type: PositionType::Absolute,
            left: hud::x(X),
            top: hud::y(TITLE_Y),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(column),
    ));
    // (On our first row only: the others at our machine are ready with it.)
    let ours = players.iter().position(|row| row.you);
    for (n, row) in players.iter().enumerate() {
        let y = FIRST_Y + n as f32 * SLOT;
        let at = Vec2::new(X, y);
        let portrait = spawn_portrait(&mut commands, column, &image, row.character, at, SCALE);
        if row.host {
            spawn_host_badge(&mut commands, portrait, true);
        }
        if row.ready {
            spawn_ready_badge(&mut commands, portrait);
        }
        if let Some(hint) = hint.0.as_ref().filter(|_| ours == Some(n)) {
            spawn_ready_hint(&mut commands, portrait, hint);
        }
        let tag = spawn_name(&mut commands, portrait, &row.name);
        if row.you {
            commands.entity(tag).insert(TextColor(HIGHLIGHT));
        }
    }
    for n in players.len()..players.len() + free {
        let y = FIRST_Y + n as f32 * SLOT;
        let place = spawn_empty_portrait(&mut commands, column, Vec2::new(X, y), SCALE);
        let name = spawn_name(&mut commands, place, OPEN);
        commands.entity(name).insert(TextColor(DIM));
    }
    // The bots and those who only watch have no portrait: a line each, a little further down,
    // the bots' in the game's font as the title is: how many, and under it what kind.
    let mut below = FIRST_Y + (players.len() + free) as f32 * SLOT + WATCHER_GAP;
    if bots.0 > 0 {
        let count = match bots.0 {
            1 => "+1 bot".to_string(),
            n => format!("+{n} bots"),
        };
        for (text, gradient) in [(count, Gradient::WHITE), (bots.1.clone(), Gradient::YELLOW)] {
            commands.spawn((
                CtrText::new(text, Font::Middle, gradient).fixed(hud::PIXEL),
                Node {
                    position_type: PositionType::Absolute,
                    left: hud::x(X),
                    top: hud::y(below),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(column),
            ));
            below += BOTS_LINE;
        }
    }
    for (n, (name, _)) in watchers.iter().enumerate() {
        commands.spawn((
            tag(format!("{name} (spectator)"), PING_FONT_SIZE, DIM),
            Node {
                position_type: PositionType::Absolute,
                left: hud::x(X),
                top: hud::y(below + n as f32 * WATCHER_LINE),
                padding: TAG_PADDING,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(column),
        ));
    }
    commands.entity(column).insert(List(players, watchers, bots, hint.0.clone(), free));
}

/// The ready hint on the right of our portrait, level with its top: our name, on the portrait's
/// bottom, is usually wider than it and runs under the hint (`portrait_row::spawn_name`).
fn spawn_ready_hint(commands: &mut Commands, portrait: Entity, hint: &str) {
    commands.spawn((
        CtrText::new(hint, Font::Middle, Gradient::WHITE).fixed(hud::PIXEL),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(100.0),
            top: Val::ZERO,
            margin: UiRect::left(hud::x(2.0)),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(portrait),
    ));
}
