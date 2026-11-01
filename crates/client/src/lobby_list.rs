//! The players in the lobby, down the left of the screen while no race is on, under a title in the
//! game's font ([`TITLE`]): each one's portrait
//! (`render::portraits`) and name, which is the nickname or, without one, the short key. (The
//! names are for Bevy's `Text`: a nickname may have anything in it.)

use bevy::prelude::*;
use render::portraits::{HEIGHT, ICON_IDS, WIDTH};

use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::hud;
use crate::leaderboard::{ICON_X, Portraits, atlas_image};
use crate::lobby::Lobby;

/// The column of rows, with the players it was built for.
#[derive(Component)]
struct List(Vec<(u8, String, String)>, Vec<(String, String)>);

const FONT_SIZE: f32 = 16.0;
const PING_FONT_SIZE: f32 = 12.0;

/// What the name and the ping are on: the toasts' colour, more opaque so that it reads over the
/// portrait.
const BACKGROUND: Color = Color::srgba(0.1, 0.1, 0.1, 0.9);

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

/// The lines of retail's screen between the last portrait and the first who only watches (more
/// than a row's own gap), and from one to the next.
const WATCHER_GAP: f32 = 12.0;
const WATCHER_LINE: f32 = 9.0;

/// The ping of the player with a short key.
#[derive(Component)]
struct PingLabel(String);

pub struct LobbyListPlugin;

impl Plugin for LobbyListPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (build, pings).chain().run_if(crate::drive::driving));
    }
}

/// Puts the list up when the roster changes, and takes it down when there is no lobby, or a race.
fn build(
    mut commands: Commands,
    lobby: Res<Lobby>,
    textures: Res<crate::Textures>,
    mut images: ResMut<Assets<Image>>,
    portraits: Option<Res<Portraits>>,
    lists: Query<(Entity, &List)>,
) {
    let players: Vec<(u8, String, String)> = if lobby.is_waiting() {
        lobby
            .members()
            .map(|(c, n, k)| (c, n.to_string(), k.to_string()))
            .collect()
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
    for (entity, list) in &lists {
        if list.0 == players && list.1 == watchers {
            return;
        }
        commands.entity(entity).despawn();
    }
    if players.is_empty() && watchers.is_empty() {
        return;
    }
    let portraits = match portraits {
        Some(portraits) => portraits.0.clone(),
        None => {
            let atlas = render::portraits::portrait_atlas(
                &crate::leaderboard::pack_icons(),
                &textures.vram,
            );
            let handle = atlas_image(atlas, &mut images);
            commands.insert_resource(Portraits(handle.clone()));
            handle
        }
    };
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
    for (n, (character, name, key)) in players.iter().enumerate() {
        let character = (*character as usize).min(ICON_IDS.len() - 1);
        let y = n as f32 * SLOT;
        let portrait = commands
            .spawn((
                ImageNode::new(portraits.clone())
                    .with_rect(Rect::new(
                        0.0,
                        (character * HEIGHT) as f32,
                        WIDTH as f32,
                        ((character + 1) * HEIGHT) as f32,
                    ))
                    .with_mode(NodeImageMode::Stretch),
                Node {
                    position_type: PositionType::Absolute,
                    left: hud::x(ICON_X),
                    top: hud::y(FIRST_Y + y),
                    width: hud::x(WIDTH as f32),
                    height: hud::y(HEIGHT as f32),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(column),
            ))
            .id();
        spawn_tags(&mut commands, portrait, name, key);
    }
    // Those who only watch have no portrait: a line each, a little further down.
    let below = FIRST_Y + players.len() as f32 * SLOT + WATCHER_GAP;
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
    commands.entity(column).insert(List(players, watchers));
}

/// Puts a player's name on a portrait, at the bottom and left-aligned, and the ping after it, on its
/// line (which [`pings`] fills in). Gives the name's entity back.
pub(crate) fn spawn_tags(
    commands: &mut Commands,
    portrait: Entity,
    name: &str,
    key: &str,
) -> Entity {
    // The name and the ping are a line, from the portrait's left edge.
    let wrapper = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::ZERO,
                // A little below the portrait's edge.
                bottom: Val::Px(-6.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(portrait),
        ))
        .id();
    let tag = commands
        .spawn((
            Text::new(name),
            TextFont {
                font_size: FONT_SIZE.into(),
                ..default()
            },
            BackgroundColor(BACKGROUND),
            Node {
                padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(wrapper),
        ))
        .id();
    commands.spawn((
        PingLabel(key.to_string()),
        Text::new(""),
        TextFont {
            font_size: FONT_SIZE.into(),
            ..default()
        },
        TextColor(Color::srgb(0.8, 0.8, 0.8)),
        BackgroundColor(BACKGROUND),
        Node {
            padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
            display: Display::None,
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(wrapper),
    ));
    tag
}

/// Writes each player's ping, and shows the label once there is one (none for us, nor before
/// the first pong).
fn pings(lobby: Res<Lobby>, mut labels: Query<(&PingLabel, &mut Text, &mut Node)>) {
    for (PingLabel(key), mut text, mut node) in &mut labels {
        let ping = lobby.pings.get(key);
        // "d" for a direct path, "r" for a relay.
        let route = match lobby.relayed.get(key) {
            Some(true) => "r/",
            Some(false) => "d/",
            None => "",
        };
        let line = ping.map_or(String::new(), |ping| {
            format!("{route}{} ms", ping.as_millis())
        });
        if text.0 != line {
            text.0 = line;
        }
        node.display = if ping.is_some() {
            Display::DEFAULT
        } else {
            Display::None
        };
    }
}
