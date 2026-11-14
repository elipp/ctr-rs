//! The players in the lobby, down the left of the screen while no race is on, under a title in the
//! game's font ([`TITLE`]): each one's portrait
//! (`render::portraits`) and name, which is the nickname or, without one, the short key. (The
//! names are for Bevy's `Text`: a nickname may have anything in it.) The host, always the first,
//! has an "H" on its portrait's corner. The other players at a machine (split screen) follow
//! the players, each named "name@2", "name@3", as in the race. Our names are in [`HIGHLIGHT`], as in the race's
//! leaderboard. (The rows are `portrait_row`'s, which the leaderboard's are too.) After the
//! players, the bots the next race fills the grid with ("+3 bots", and under it "medium, brain"),
//! if any, and then those who only watch.

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;
use crate::leaderboard::ICON_X;
use crate::lobby::{Lobby, Row};
use crate::portrait_row::{
    BACKGROUND, HIGHLIGHT, PING_FONT_SIZE, Portraits, portraits, spawn_host_badge,
    spawn_portrait, spawn_tags,
};

/// The column of rows, with the players it was built for, and which of them is us, and the bots
/// (how many, and what kind).
#[derive(Component)]
struct List(Vec<Row>, Vec<(String, String)>, (usize, String));

/// How far the whole list (the title, the portraits and the spectators) is moved down, in lines
/// of retail's screen, to be clear of the toasts and the chat at the top.
const TABLE_Y: f32 = 12.0;

/// What the list is headed with. (The fonts have capitals, and this is how they show.)
const TITLE: &str = "PLAYERS IN LOBBY:";

/// Where the title is, in lines of retail's screen. It is [`Font::Middle`], 8 lines tall.
const TITLE_Y: f32 = 2.0;

/// Where the first portrait is, in lines of retail's screen: under the title, and high enough
/// that 8 players fit (8 rows of [`SLOT`] lines make 233 of the 240).
const FIRST_Y: f32 = 12.0;

/// The lines from one row to the next: a little more than the leaderboard's, to make room for
/// the names, which hang under the portraits.
const SLOT: f32 = 28.0;

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
    textures: Res<crate::Textures>,
    mut images: ResMut<Assets<Image>>,
    loaded: Option<Res<Portraits>>,
    lists: Query<(Entity, &List)>,
) {
    let players: Vec<Row> = if lobby.is_waiting() {
        lobby.rows()
    } else {
        Vec::new()
    };
    let watchers: Vec<(String, String)> = if lobby.is_waiting() {
        lobby
            .watchers()
            .map(|(n, k)| (n.to_string(), k.to_string()))
            .collect()
    } else {
        Vec::new()
    };
    let bots = if players.is_empty() {
        (0, String::new())
    } else {
        (lobby.next_bots(), lobby.next_bots_kind())
    };
    for (entity, list) in &lists {
        if list.0 == players && list.1 == watchers && list.2 == bots {
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
            left: hud::x(ICON_X),
            top: hud::y(TITLE_Y),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(column),
    ));
    for (n, row) in players.iter().enumerate() {
        let y = FIRST_Y + n as f32 * SLOT;
        let portrait = spawn_portrait(&mut commands, column, &image, row.character, y);
        if row.host {
            spawn_host_badge(&mut commands, portrait, true);
        }
        let tag = spawn_tags(&mut commands, portrait, &row.name, &row.key);
        if row.you {
            commands.entity(tag).insert(TextColor(HIGHLIGHT));
        }
    }
    // The bots and those who only watch have no portrait: a line each, a little further down,
    // the bots' in the game's font as the title is: how many, and under it what kind.
    let mut below = FIRST_Y + players.len() as f32 * SLOT + WATCHER_GAP;
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
                    left: hud::x(ICON_X),
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
            Text::new(format!("{name} (spectator)")),
            TextFont {
                font_size: PING_FONT_SIZE.into(),
                ..default()
            },
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
            BackgroundColor(BACKGROUND),
            Node {
                position_type: PositionType::Absolute,
                left: hud::x(ICON_X),
                top: hud::y(below + n as f32 * WATCHER_LINE),
                padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(column),
        ));
    }
    commands.entity(column).insert(List(players, watchers, bots));
}
