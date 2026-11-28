//! The lobby's scoreboard: a table in the middle of the screen while [`Control::Scoreboard`] (Tab
//! by default) is held, and for [`STANDINGS_SECONDS`] after a cup's standings come in. A row
//! for each kart of the race (or each player of the lobby between races) and each driver the cup
//! has scored who is gone since: the place, the character's portrait, the name, the points of
//! the last race and of the cup, and the ping ("BOT" for a bot). In a cup the rows go by points.
//! (Not retail.)
//!
//! While a quick play match gathers for its first race, the table is up by itself with a line
//! for each place of the grid: who took it, or that a player may come (and a bot drive it if
//! none does).
//!
//! A score server board (`/score top`, [`TopBoard`]) goes up in the same table: while it loads,
//! then for [`TOP_SECONDS`] or until Back dismisses it. The last one comes back while the key is
//! held outside a lobby.

use bevy::prelude::*;
use render::font::Font;
use render::portraits::{HEIGHT, WIDTH};

use crate::controls::Control;
use crate::font::CtrText;
use crate::lobby::Lobby;
use crate::lobby::cup::Who;
use crate::portrait_row::{HIGHLIGHT, Portraits, portrait_rect, portraits};

/// How long the scoreboard shows by itself when a cup's standings come in.
const STANDINGS_SECONDS: f32 = 10.0;

/// The sizes of the title and of the rest.
const TITLE_SIZE: f32 = 24.0;
const FONT_SIZE: f32 = 18.0;
const HINT_SIZE: f32 = FONT_SIZE - 2.0;

/// The first of `dismiss`'s keys and buttons.
const DISMISS: crate::prompt::Prompt =
    crate::prompt::Prompt::new(crate::pad_glyph::PadGlyph::Circle, KeyCode::Escape, "dismiss");

/// How long a board from the score server shows by itself when it comes in.
const TOP_SECONDS: f32 = 15.0;

/// How wide the place and the portrait are, in pixels.
const PLACE_WIDTH: f32 = 32.0;
const PORTRAIT_WIDTH: f32 = 48.0;

/// The columns after the portrait, with their widths in pixels.
const NAME: Column = Column {
    heading: "NAME",
    width: 240.0,
    dim: false,
};
const RACE: Column = Column {
    heading: "RACE",
    width: 56.0,
    dim: true,
};
const POINTS: Column = Column {
    heading: "POINTS",
    width: 64.0,
    dim: false,
};
const PING: Column = Column {
    heading: "PING",
    width: 72.0,
    dim: true,
};
const TIME: Column = Column {
    heading: "TIME",
    width: 96.0,
    dim: false,
};
const DATE: Column = Column {
    heading: "DATE",
    width: 112.0,
    dim: true,
};
const NOTE: Column = Column {
    heading: "",
    width: 80.0,
    dim: true,
};

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
    columns: Vec<Column>,
    lines: Vec<Line>,
    /// Under the table.
    footer: Option<String>,
    /// How to take it down, in the bottom right corner.
    hint: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
struct Column {
    heading: &'static str,
    width: f32,
    /// Grey rather than white (or [`HIGHLIGHT`] on our line).
    dim: bool,
}

/// A row of the table: a cell per column.
#[derive(Clone, PartialEq)]
struct Line {
    place: u32,
    /// None for a place nobody has taken: no portrait.
    character: Option<u8>,
    cells: Vec<String>,
    you: bool,
}

/// The last board of the score server (`/score top`), and until when it is up by itself.
#[derive(Resource, Default)]
pub(crate) struct TopBoard {
    table: Option<Table>,
    /// Infinite while it loads; past once dismissed.
    until: f32,
}

impl TopBoard {
    pub(crate) fn loading(&mut self, title: String) {
        self.table = Some(Table {
            title,
            columns: Vec::new(),
            lines: Vec::new(),
            footer: Some("Loading leaderboard...".into()),
            hint: None,
        });
        self.until = f32::INFINITY;
    }

    /// Up for [`TOP_SECONDS`] from now, unless it was dismissed while loading.
    pub(crate) fn loaded(&mut self, title: String, entries: &[score::Entry], now: f32) {
        self.table = Some(top_table(title, entries));
        if self.until == f32::INFINITY {
            self.until = now + TOP_SECONDS;
        }
    }

    /// The error is the toast's to tell.
    pub(crate) fn failed(&mut self) {
        *self = TopBoard::default();
    }

    fn up(&self, now: f32) -> bool {
        self.table.is_some() && now < self.until
    }
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
        app.init_resource::<TopBoard>()
            .add_systems(
                PreUpdate,
                dismiss
                    .after(bevy::input::InputSystems)
                    .before(crate::pause::input)
                    .run_if(crate::drive::driving),
            )
            .add_systems(Update, show.run_if(crate::drive::driving));
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

/// Back (Escape, Backspace, Circle or Triangle) takes a board that is up down, and is used up:
/// it doesn't open the menu or fire an item too.
fn dismiss(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut pads: Query<&mut Gamepad>,
    menu: Res<crate::pause::PauseMenu>,
    time: Res<Time<Real>>,
    sounds: Option<Res<crate::sound::Sounds>>,
    mut top: ResMut<TopBoard>,
) {
    const KEYS: [KeyCode; 2] = [KeyCode::Escape, KeyCode::Backspace];
    const BUTTONS: [GamepadButton; 2] = [GamepadButton::East, GamepadButton::North];
    if menu.open || !top.up(time.elapsed_secs()) {
        return;
    }
    let key = KEYS.iter().any(|k| keys.just_pressed(*k));
    let pad = (pads.iter()).any(|p| BUTTONS.iter().any(|b| p.just_pressed(*b)));
    if !key && !pad {
        return;
    }
    top.until = 0.0;
    for k in KEYS {
        keys.clear_just_pressed(k);
    }
    for mut p in &mut pads {
        for b in BUTTONS {
            p.digital_mut().clear_just_pressed(b);
        }
    }
    if let Some(sounds) = sounds {
        sounds.menu(crate::sound::MENU_BACK_SFX);
    }
}

#[allow(clippy::too_many_arguments)]
fn show(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    config: Res<crate::config::Config>,
    menu: Res<crate::pause::PauseMenu>,
    top: Res<TopBoard>,
    last: Res<crate::last_input::LastInput>,
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
    let held = config.controls.held(Control::Scoreboard, &keys);
    // Holding the key in a lobby asks for the lobby's table, so it wins over a board.
    let table = if menu.open {
        None
    } else if lobby.gathering(&race_on) {
        Some(gathering(&lobby))
    } else if held && lobby.is_open() {
        Some(table(&lobby, &race_on))
    } else if held || top.up(now) {
        // Back only dismisses a board that is up by itself.
        let hint = top.up(now).then(|| DISMISS.text(last.pad()));
        (top.table.clone()).map(|table| Table { hint, ..table })
    } else if now < shown.until && lobby.is_open() {
        Some(table(&lobby, &race_on))
    } else {
        None
    };
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
    let mut lines: Vec<Driven> = (drivers.iter())
        .map(|driver| Driven {
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
            lines.push(Driven {
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
        Some(s) if s.stopped || s.next.is_none() => {
            (format!("Cup '{}': Final standings", s.cup), None)
        }
        Some(s) => (
            format!("{}: race {} of {}", s.cup, s.race as usize + 1, s.races),
            s.next.as_ref().map(|(_, track)| format!("Next: {track}")),
        ),
        None if cup => ("Cup".into(), None),
        None => ("Lobby".into(), None),
    };
    let columns = if cup {
        vec![NAME, RACE, POINTS, PING]
    } else {
        vec![NAME, PING]
    };
    let lines = (lines.into_iter().enumerate())
        .map(|(place, line)| {
            let gained = line
                .gained
                .map_or(String::new(), |points| format!("+{points}"));
            Line {
                place: place as u32 + 1,
                character: Some(line.character),
                cells: if cup {
                    vec![line.name, gained, line.points.to_string(), line.ping]
                } else {
                    vec![line.name, line.ping]
                },
                you: line.you,
            }
        })
        .collect();
    Table {
        title,
        columns,
        lines,
        footer,
        hint: None,
    }
}

/// A line of the lobby's table, before it is put in cells.
struct Driven {
    character: u8,
    name: String,
    /// The points of the last race, if it gave any.
    gained: Option<u32>,
    points: u32,
    ping: String,
    you: bool,
}

/// A quick play match gathering: a line for each place of the grid, the players there first,
/// then the free places, the last of which the host's bots take if no player does.
fn gathering(lobby: &Lobby) -> Table {
    let rows = lobby.rows();
    let free = race::MAX_KARTS.saturating_sub(rows.len());
    let bots = lobby.next_bots();
    let players = rows.into_iter().map(|row| (Some(row.character), row.name, row.key, row.you));
    let waiting = (0..free).map(|n| {
        let note = if n >= free - bots { "BOT" } else { "" };
        (None, "Waiting for player...".to_string(), note.to_string(), false)
    });
    let lines = (players.chain(waiting).enumerate())
        .map(|(place, (character, name, key, you))| {
            let ping = match character {
                None => key,
                Some(_) if you => "you".into(),
                Some(_) => lobby
                    .pings
                    .get(&key)
                    .map_or("...".to_string(), |ping| format!("{} ms", ping.as_millis())),
            };
            Line {
                place: place as u32 + 1,
                character,
                cells: vec![name, ping],
                you,
            }
        })
        .collect();
    Table {
        title: "Quick play".into(),
        columns: vec![NAME, PING],
        lines,
        footer: Some("Waiting for the players of the match".into()),
        hint: None,
    }
}

/// A board of the score server as a table: its own ranks, as the server ranks.
fn top_table(title: String, entries: &[score::Entry]) -> Table {
    let lines = (entries.iter())
        .map(|entry| {
            let note = match (entry.perfect, entry.suspect) {
                (_, true) => "suspect",
                (true, false) => "perfect",
                (false, false) => "",
            };
            Line {
                place: entry.rank,
                character: Some(entry.character),
                cells: vec![
                    crate::world_record::who(entry),
                    race::clock::time(entry.final_ms),
                    crate::world_record::date(entry.at),
                    note.to_string(),
                ],
                you: crate::world_record::is_ours(entry),
            }
        })
        .collect();
    Table {
        title,
        columns: vec![NAME, TIME, DATE, NOTE],
        lines,
        footer: entries
            .is_empty()
            .then(|| "Nobody on the board yet".to_string()),
        hint: None,
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
                    key: (!race.is_any_bot(slot))
                        .then(|| race.keys().get(slot).cloned())
                        .flatten(),
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
    // (Headings over nothing would only say what is missing.)
    if !table.lines.is_empty() {
        let heading = row(commands);
        text(commands, heading, "#", FONT_SIZE, DIM, Some(PLACE_WIDTH));
        text(commands, heading, "", FONT_SIZE, DIM, Some(PORTRAIT_WIDTH));
        for column in &table.columns {
            text(
                commands,
                heading,
                column.heading,
                FONT_SIZE,
                DIM,
                Some(column.width),
            );
        }
    }
    for line in &table.lines {
        let parent = row(commands);
        let color = match line.character {
            None => DIM,
            Some(_) if line.you => HIGHLIGHT,
            Some(_) => Color::WHITE,
        };
        text(
            commands,
            parent,
            &line.place.to_string(),
            FONT_SIZE,
            color,
            Some(PLACE_WIDTH),
        );
        let portrait = Node {
            width: Val::Px(PORTRAIT_WIDTH),
            height: Val::Px(
                PORTRAIT_WIDTH * HEIGHT as f32 * crate::hud::PIXEL.y
                    / (WIDTH as f32 * crate::hud::PIXEL.x),
            ),
            ..default()
        };
        match line.character {
            Some(character) => commands.spawn((
                // Stretch: `Auto` would keep the picture's square texels, not the HUD's tall ones.
                ImageNode::new(image.clone())
                    .with_rect(portrait_rect(character as usize))
                    .with_mode(NodeImageMode::Stretch),
                portrait,
                Pickable::IGNORE,
                ChildOf(parent),
            )),
            // (The line keeps the height of the others.)
            None => commands.spawn((portrait, Pickable::IGNORE, ChildOf(parent))),
        };
        for (column, cell) in table.columns.iter().zip(&line.cells) {
            let color = if column.dim { DIM } else { color };
            text(commands, parent, cell, FONT_SIZE, color, Some(column.width));
        }
    }
    if let Some(footer) = &table.footer {
        text(commands, root, footer, FONT_SIZE, DIM, None);
    }
    if let Some(hint) = &table.hint {
        let corner = commands
            .spawn((
                Node {
                    align_self: AlignSelf::FlexEnd,
                    padding: UiRect::top(Val::Px(8.0)),
                    ..default()
                },
                Pickable::IGNORE,
                ChildOf(root),
            ))
            .id();
        commands.spawn((
            CtrText::new(hint, Font::Middle, crate::toast::gradient(DIM))
                .fixed(crate::toast::texel_size(HINT_SIZE)),
            Pickable::IGNORE,
            ChildOf(corner),
        ));
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
        TextFont::from_font_size(size),
        TextColor(color),
        Node {
            width: width.map_or(Val::Auto, Val::Px),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}
