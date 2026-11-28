//! The lobby's roster while no race is on. The rows are `portrait_row`'s, shared with the race's
//! leaderboard. Names are Bevy `Text`, not the game's font: a nickname may have anything in it.
use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;
use crate::lobby::{Lobby, Row};
use crate::portrait_row::{
    BACKGROUND, DIM, HIGHLIGHT, HOST_COLOR, PING_FONT_SIZE, READY_COLOR, TAG_PADDING, Portraits,
    portraits, spawn_empty_portrait, spawn_portrait, tag,
};

const OPEN: &str = "Open";

/// Everything the column was built from: any change rebuilds it.
#[derive(Component)]
struct List(Vec<Row>, Vec<(String, String)>, (usize, String), Option<String>, usize);

/// Clear of the toasts at the top.
const TABLE_Y: f32 = 12.0;

/// Capitals: the game's fonts have no lowercase.
const TITLE: &str = "PLAYERS IN LOBBY:";

const TITLE_Y: f32 = 2.0;

const FIRST_Y: f32 = 12.0;

/// At 0.8 a full grid ran into the chat. At this size the name and badges don't fit on the
/// portrait, hence [`spawn_beside`].
const SCALE: f32 = 0.55;
const SLOT: f32 = 15.0;

const NAME_SIZE: f32 = 16.0;
const BADGE_SIZE: f32 = 16.0;

/// Lined up with the race's leaderboard.
const X: f32 = crate::leaderboard::ICON_X;

const WATCHER_GAP: f32 = 12.0;
const WATCHER_LINE: f32 = 9.0;

const BOTS_LINE: f32 = 12.0;

pub struct LobbyListPlugin;

impl Plugin for LobbyListPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, build.run_if(crate::drive::driving));
    }
}

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
    // Stable: split screen players keep their order.
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
        let hint = hint.0.as_deref().filter(|_| ours == Some(n));
        let color = if row.you { HIGHLIGHT } else { Color::WHITE };
        spawn_beside(&mut commands, portrait, &row.name, color, row.host, row.ready, hint);
    }
    for n in players.len()..players.len() + free {
        let y = FIRST_Y + n as f32 * SLOT;
        let place = spawn_empty_portrait(&mut commands, column, Vec2::new(X, y), SCALE);
        spawn_beside(&mut commands, place, OPEN, DIM, false, false, None);
    }
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

fn spawn_beside(
    commands: &mut Commands,
    portrait: Entity,
    name: &str,
    color: Color,
    host: bool,
    ready: bool,
    hint: Option<&str>,
) {
    let line = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(100.0),
                top: Val::ZERO,
                height: Val::Percent(100.0),
                margin: UiRect::left(hud::x(2.0)),
                align_items: AlignItems::Center,
                column_gap: hud::x(2.0),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(portrait),
        ))
        .id();
    commands.spawn((
        Text::new(name),
        TextFont::from_font_size(NAME_SIZE),
        TextColor(color),
        BackgroundColor(BACKGROUND),
        TextLayout::linebreak(LineBreak::NoWrap),
        Node {
            padding: TAG_PADDING,
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(line),
    ));
    let badges = [
        (host, crate::icons::CROWN_SIMPLE, HOST_COLOR),
        (ready, crate::icons::CHECK_FAT, READY_COLOR),
    ];
    for (_, icon, tint) in badges.into_iter().filter(|badge| badge.0) {
        commands.spawn((
            crate::icons::outlined(icon, BADGE_SIZE, tint, Node::default()),
            Pickable::IGNORE,
            ChildOf(line),
        ));
    }
    if let Some(hint) = hint {
        commands.spawn((
            CtrText::new(hint, Font::Middle, Gradient::WHITE).fixed(hud::PIXEL),
            Pickable::IGNORE,
            ChildOf(line),
        ));
    }
}
