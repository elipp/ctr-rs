//! The lobby's scoreboard: a table in the middle of the screen while [`Control::Scoreboard`] (Tab
//! by default) is held, and for [`STANDINGS_SECONDS`] after a cup's standings come in. A row
//! for each kart of the race (or each player of the lobby between races) and each driver the cup
//! has scored who is gone since: the place, the character's portrait, the name, the points of
//! the last race and of the cup, and the ping ("BOT" for a bot). In a cup the rows go by points.
//! (Not retail.)

use bevy::prelude::*;
use render::portraits::{HEIGHT, WIDTH};

use crate::controls::Control;
use crate::lobby::Lobby;
use crate::lobby::cup::Who;
use crate::portrait_row::{HIGHLIGHT, Portraits, portrait_rect, portraits};

/// How long the scoreboard shows by itself when a cup's standings come in.
const STANDINGS_SECONDS: f32 = 10.0;

/// The sizes of the title and of the rest.
const TITLE_SIZE: f32 = 24.0;
const FONT_SIZE: f32 = 18.0;

/// How wide each column is, in pixels: the place, the portrait, the name, the points of the last
/// race and of the cup, and the ping.
const COLUMNS: [f32; 6] = [32.0, 48.0, 240.0, 56.0, 64.0, 72.0];

/// The colour of the headings, the points of the last race, the pings and the footer.
const DIM: Color = Color::srgb(0.6, 0.6, 0.6);

/// What the table is on: nearly opaque, as the race and the HUDs of a split screen are under it.
const BACKGROUND: Color = Color::srgba(0.05, 0.05, 0.05, 0.97);

/// The table, which is built anew whenever what it shows changes.
#[derive(Component)]
struct Scoreboard;

/// What the table shows.
#[derive(Clone, PartialEq)]
struct Table {
    title: String,
    /// Whether there are points to show: in a cup.
    cup: bool,
    lines: Vec<Line>,
    /// What comes next, under the table.
    footer: Option<String>,
}

/// A row of the table.
#[derive(Clone, PartialEq)]
struct Line {
    character: u8,
    name: String,
    /// The points of the last race, if it gave any.
    gained: Option<u32>,
    points: u32,
    ping: String,
    you: bool,
}

/// A driver of the race or the lobby.
struct Driver {
    who: Who,
    character: u8,
    name: String,
    /// The short key its ping is kept by; none for a bot.
    key: Option<String>,
    you: bool,
}

pub struct ScoreboardPlugin;

impl Plugin for ScoreboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, show.run_if(crate::drive::driving));
    }
}

/// What `show` keeps from one frame to the next.
#[derive(Default)]
struct Shown {
    table: Option<Table>,
    /// The standings last seen, and until when they keep the table up.
    standings: Option<net::Standings>,
    until: f32,
}

#[allow(clippy::too_many_arguments)]
fn show(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<crate::config::Config>,
    menu: Res<crate::pause::PauseMenu>,
    lobby: Res<Lobby>,
    race_on: Res<crate::race_on::RaceOn>,
    time: Res<Time<Real>>,
    textures: Res<crate::Textures>,
    mut images: ResMut<Assets<Image>>,
    loaded: Option<Res<Portraits>>,
    boards: Query<Entity, With<Scoreboard>>,
    mut shown: Local<Shown>,
) {
    let now = time.elapsed_secs();
    if lobby.standings != shown.standings {
        shown.standings = lobby.standings.clone();
        if shown.standings.is_some() {
            shown.until = now + STANDINGS_SECONDS;
        }
    }
    let wanted = config.controls.held(Control::Scoreboard, &keys) || now < shown.until;
    let table = (wanted && lobby.is_open() && !menu.open).then(|| table(&lobby, &race_on));
    if table == shown.table {
        return;
    }
    for board in &boards {
        commands.entity(board).despawn();
    }
    if let Some(table) = &table {
        let image = portraits(
            &mut commands,
            loaded.as_deref(),
            &textures.vram,
            &mut images,
        );
        spawn(&mut commands, table, &image);
    }
    shown.table = table;
}

/// What the table shows now.
fn table(lobby: &Lobby, race_on: &crate::race_on::RaceOn) -> Table {
    let drivers = drivers(lobby, race_on);
    let standings = lobby.standings.as_ref();
    let ping = |driver: &Driver| match &driver.key {
        _ if driver.you => "you".to_string(),
        None => "BOT".to_string(),
        Some(key) => lobby
            .pings
            .get(key)
            .map_or("...".to_string(), |ping| format!("{} ms", ping.as_millis())),
    };
    let score =
        |scores: &[net::Score], who: &Who| scores.iter().find(|s| who.matches(s)).map(|s| s.points);
    let mut lines: Vec<Line> = (drivers.iter())
        .map(|driver| Line {
            character: driver.character,
            name: driver.name.clone(),
            gained: standings.and_then(|s| score(&s.gained, &driver.who)),
            points: standings
                .and_then(|s| score(&s.totals, &driver.who))
                .unwrap_or(0),
            ping: ping(driver),
            you: driver.you,
        })
        .collect();
    // The ones the cup scored who are not in the race or the lobby any more.
    for total in standings.map_or(&[][..], |s| &s.totals) {
        if drivers.iter().all(|d| !d.who.matches(total)) {
            let gained = (standings.into_iter())
                .flat_map(|s| &s.gained)
                .find(|g| (g.id, g.seat) == (total.id, total.seat));
            lines.push(Line {
                character: total.character,
                name: total.name.clone(),
                gained: gained.map(|g| g.points),
                points: total.points,
                // (Between races only the players are listed: a bot is still in the cup.)
                ping: if Who::is_bot_id(&total.id) {
                    "BOT"
                } else {
                    "left"
                }
                .into(),
                you: false,
            });
        }
    }
    let cup = standings.is_some() || race_on.get().is_some_and(|race| race.retail.cup);
    if cup {
        lines.sort_by_key(|line| std::cmp::Reverse(line.points));
    }
    let (title, footer) = match standings {
        Some(s) if s.stopped || s.next.is_none() => (format!("Cup '{}': Final standings", s.cup), None),
        Some(s) => (
            format!("{}: race {} of {}", s.cup, s.race as usize + 1, s.races),
            s.next.as_ref().map(|(_, track)| format!("Next: {track}")),
        ),
        None if cup => ("Cup".into(), None),
        None => ("Lobby".into(), None),
    };
    Table {
        title,
        cup,
        lines,
        footer,
    }
}

/// The karts of the race on, or the players of the lobby between races.
fn drivers(lobby: &Lobby, race_on: &crate::race_on::RaceOn) -> Vec<Driver> {
    if let Some(race) = race_on.get() {
        let characters = race.kart_characters();
        return (0..race.karts())
            .filter_map(|slot| {
                Some(Driver {
                    who: race.who(slot)?,
                    character: characters[slot],
                    name: race.kart_label(slot).to_string(),
                    key: (!race.is_any_bot(slot)).then(|| race.keys().get(slot).cloned()).flatten(),
                    you: race.local(slot),
                })
            })
            .collect();
    }
    (lobby.rows().into_iter())
        .map(|row| Driver {
            who: Who::Player(row.id, row.seat),
            character: row.character,
            name: row.name,
            key: Some(row.key),
            you: row.you,
        })
        .collect()
}

fn spawn(commands: &mut Commands, table: &Table, image: &Handle<Image>) {
    let root = commands
        .spawn((
            Scoreboard,
            // Centered, whatever its size.
            UiTransform::from_translation(Val2::percent(-50.0, -50.0)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(16.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(BACKGROUND),
            // Over the HUD, under the console.
            GlobalZIndex(i32::MAX - 4),
            Pickable::IGNORE,
        ))
        .id();
    text(commands, root, &table.title, TITLE_SIZE, Color::WHITE, None);
    let row = |commands: &mut Commands| {
        commands
            .spawn((
                Node {
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(root),
            ))
            .id()
    };
    let heading = row(commands);
    let headings = ["#", "", "NAME", "RACE", "POINTS", "PING"];
    for (column, label) in headings.into_iter().enumerate() {
        if table.cup || !matches!(column, 3 | 4) {
            text(
                commands,
                heading,
                label,
                FONT_SIZE,
                DIM,
                Some(COLUMNS[column]),
            );
        }
    }
    for (place, line) in table.lines.iter().enumerate() {
        let parent = row(commands);
        let color = if line.you { HIGHLIGHT } else { Color::WHITE };
        text(
            commands,
            parent,
            &(place + 1).to_string(),
            FONT_SIZE,
            color,
            Some(COLUMNS[0]),
        );
        commands.spawn((
            // Stretch: `Auto` would keep the picture's square texels, not the HUD's tall ones.
            ImageNode::new(image.clone())
                .with_rect(portrait_rect(line.character as usize))
                .with_mode(NodeImageMode::Stretch),
            Node {
                width: Val::Px(COLUMNS[1]),
                height: Val::Px(
                    COLUMNS[1] * HEIGHT as f32 * crate::hud::PIXEL.y
                        / (WIDTH as f32 * crate::hud::PIXEL.x),
                ),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(parent),
        ));
        text(
            commands,
            parent,
            &line.name,
            FONT_SIZE,
            color,
            Some(COLUMNS[2]),
        );
        if table.cup {
            let gained = line
                .gained
                .map_or(String::new(), |points| format!("+{points}"));
            text(commands, parent, &gained, FONT_SIZE, DIM, Some(COLUMNS[3]));
            text(
                commands,
                parent,
                &line.points.to_string(),
                FONT_SIZE,
                color,
                Some(COLUMNS[4]),
            );
        }
        text(
            commands,
            parent,
            &line.ping,
            FONT_SIZE,
            DIM,
            Some(COLUMNS[5]),
        );
    }
    if let Some(footer) = &table.footer {
        text(commands, root, footer, FONT_SIZE, DIM, None);
    }
}

/// A piece of text in `parent`, as wide as `width` if given.
fn text(
    commands: &mut Commands,
    parent: Entity,
    text: &str,
    size: f32,
    color: Color,
    width: Option<f32>,
) {
    commands.spawn((
        Text::new(text),
        TextFont {
            font_size: size.into(),
            ..default()
        },
        TextColor(color),
        Node {
            width: width.map_or(Val::Auto, Val::Px),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}
