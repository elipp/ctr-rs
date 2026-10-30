//! The message log: a window at the bottom left with the last lines of text the game has to say,
//! and the command line (`command.rs`) as its bottom row.
//!
//! A line is made of pieces of text, each in its own colour (a player's name, then what they said).
//! It gets there in one of two ways:
//!
//! - a [`Log`] message, for systems that want the colours;
//! - `console!("version {VERSION}")`, which is `info!` with the target `console`, for anything
//!   else, including code outside the ECS and other threads. Every `warn!` and `error!` is shown
//!   as well, with the name of where it came from; the rest of what is logged is not.
//!
//! The window fades away when nothing has been said for a while, and opening the command line (or
//! a new line) brings it back. While the line is open the mouse wheel and Page Up and Down scroll
//! back through up to [`CAPACITY`] lines.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::log::tracing::field::{Field, Visit};
use bevy::log::tracing::{Event as TracingEvent, Subscriber};
use bevy::log::tracing_subscriber::Layer;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::{BoxedLayer, Level};
use bevy::prelude::*;

use crate::command::{self, CommandLine};

/// The target of what `console!` logs.
pub const TARGET: &str = "console";

/// Writes a line to the console, formatted like `info!`: `console!("{n} peers")`.
macro_rules! console {
    ($($arg:tt)+) => { bevy::log::info!(target: "console", $($arg)+) };
}
pub(crate) use console;

/// How many lines are kept.
const CAPACITY: usize = 1000;

/// How many of the newest lines are on screen at most (the window clips what does not fit).
const VISIBLE: usize = 24;

/// Lines that Page Up and Page Down scroll.
pub const PAGE: i32 = 8;

const FONT_SIZE: f32 = 18.0;
const LOG_HEIGHT: f32 = 300.0;
const BACKGROUND_ALPHA: f32 = 0.25;

/// Seconds the text stays up after the last line or scroll, and how long it then fades for. The
/// background goes first, after `BACKGROUND_HOLD_SECONDS`.
const HOLD_SECONDS: f32 = 10.0;
const BACKGROUND_HOLD_SECONDS: f32 = 2.0;
const FADE_SECONDS: f32 = 1.0;

pub const ERROR_COLOR: Color = Color::srgb(1.0, 0.4, 0.4);
pub const WARN_COLOR: Color = Color::srgb(1.0, 0.8, 0.3);
pub const DIM_COLOR: Color = Color::srgb(0.6, 0.6, 0.6);

/// A piece of a line, in one colour.
#[derive(Clone, Debug)]
pub struct Piece {
    pub text: String,
    pub color: Color,
}

/// Asks for a line to be added to the console.
///
/// ```ignore
/// logs.write(Log::colored("Penta", name_color).then(": hello", Color::WHITE));
/// ```
#[derive(Message, Clone, Debug, Default)]
pub struct Log {
    pub pieces: Vec<Piece>,
}

impl Log {
    /// A line in white.
    pub fn new(text: impl Into<String>) -> Log {
        Log::colored(text, Color::WHITE)
    }

    /// A line in that colour.
    pub fn colored(text: impl Into<String>, color: Color) -> Log {
        Log::default().then(text, color)
    }

    /// An error: a line in red.
    pub fn error(text: impl Into<String>) -> Log {
        Log::colored(text, ERROR_COLOR)
    }

    /// Adds a piece after the others.
    pub fn then(mut self, text: impl Into<String>, color: Color) -> Log {
        self.pieces.push(Piece {
            text: text.into(),
            color,
        });
        self
    }
}

/// The lines, and the part of them that is shown.
#[derive(Resource)]
pub struct Console {
    /// The oldest first.
    lines: VecDeque<Log>,
    /// How many lines back from the newest the view is.
    scroll: usize,
    /// Bumped whenever the lines change, for the window to know.
    version: u32,
    /// When (`Time<Real>`) something last happened that the window should be up for.
    active_at: f32,
    /// How opaque the window is, 0 to 1.
    alpha: f32,
}

impl Default for Console {
    fn default() -> Console {
        Console {
            lines: VecDeque::new(),
            scroll: 0,
            version: 0,
            active_at: f32::NEG_INFINITY,
            alpha: 0.0,
        }
    }
}

impl Console {
    fn push(&mut self, line: Log, now: f32) {
        if self.lines.len() == CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        // A view scrolled back stays on the lines it has.
        if self.scroll > 0 {
            self.scroll = (self.scroll + 1).min(self.lines.len() - 1);
        }
        self.version += 1;
        self.active_at = now;
    }

    /// Scrolls to older lines (positive) or newer (negative).
    pub fn scroll_by(&mut self, lines: i32, now: f32) {
        let furthest = self.lines.len().saturating_sub(1);
        self.scroll = self.scroll.saturating_add_signed(lines as isize).min(furthest);
        self.active_at = now;
    }

    /// Back to the newest line.
    pub fn scroll_to_end(&mut self) {
        self.scroll = 0;
    }
}

/// Where what the tracing layer picks up waits for `collect`.
#[derive(Resource)]
struct Inbox(Mutex<Receiver<Log>>);

/// Sends the events that are for the console to [`Inbox`]; they may come from any thread.
struct ConsoleLayer(Sender<Log>);

/// `LogPlugin::custom_layer`: the layer that feeds the console.
pub fn layer(app: &mut App) -> Option<BoxedLayer> {
    let (sender, receiver) = channel();
    app.insert_resource(Inbox(Mutex::new(receiver)));
    Some(Box::new(ConsoleLayer(sender)))
}

/// The `message` of an event.
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for ConsoleLayer {
    fn on_event(&self, event: &TracingEvent<'_>, _: Context<'_, S>) {
        let meta = event.metadata();
        let ours = meta.target() == TARGET;
        // (More verbose levels are the greater ones.)
        if !ours && *meta.level() > Level::WARN {
            return;
        }
        let color = match *meta.level() {
            Level::ERROR => ERROR_COLOR,
            Level::WARN => WARN_COLOR,
            _ => Color::WHITE,
        };
        let mut message = Message(String::new());
        event.record(&mut message);
        let source = meta.target().rsplit("::").next().unwrap_or_default();
        for text in message.0.lines() {
            let mut line = Log::default();
            if !ours {
                line = line.then(format!("{source}: "), DIM_COLOR);
            }
            let _ = self.0.send(line.then(text, color));
        }
    }
}

pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Console>()
            .add_message::<Log>()
            .add_systems(Startup, spawn)
            .add_systems(Update, (collect, scroll, fade, rebuild).chain());
    }
}

/// The window.
#[derive(Component)]
struct ConsoleWindow;

/// Where the lines are, above the command line.
#[derive(Component)]
struct LogArea;

/// The colour a piece of text has at full opacity.
#[derive(Component)]
struct Ink(Color);

fn spawn(mut commands: Commands) {
    commands.spawn((
        ConsoleWindow,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(40.0),
            left: Val::Px(20.0),
            width: Val::Percent(40.0),
            min_width: Val::Px(480.0),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        // Over every other piece of the HUD.
        GlobalZIndex(i32::MAX),
        Visibility::Hidden,
        children![
            (
                LogArea,
                Node {
                    height: Val::Px(LOG_HEIGHT),
                    flex_direction: FlexDirection::Column,
                    // The newest line is at the bottom, and the oldest are clipped off the top.
                    justify_content: JustifyContent::FlexEnd,
                    overflow: Overflow::clip(),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::BLACK.with_alpha(BACKGROUND_ALPHA)),
            ),
            command::input_row(),
        ],
    ));
}

/// Takes in the lines that were sent.
fn collect(
    inbox: Res<Inbox>,
    mut logs: MessageReader<Log>,
    mut console: ResMut<Console>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs();
    let from_tracing: Vec<Log> = inbox.0.lock().map_or(Vec::new(), |r| r.try_iter().collect());
    for line in from_tracing.into_iter().chain(logs.read().cloned()) {
        if !line.pieces.is_empty() {
            console.push(line, now);
        }
    }
}

/// The mouse wheel scrolls while the command line is open.
fn scroll(
    mut wheel: MessageReader<MouseWheel>,
    line: Res<CommandLine>,
    mut console: ResMut<Console>,
    time: Res<Time<Real>>,
    // Fractions of a line that a trackpad has scrolled so far.
    mut carry: Local<f32>,
) {
    for event in wheel.read() {
        if !line.is_open() {
            continue;
        }
        *carry += match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y / (FONT_SIZE * 1.5),
        };
        let whole = carry.trunc();
        *carry -= whole;
        if whole != 0.0 {
            console.scroll_by(whole as i32, time.elapsed_secs());
        }
    }
}

/// How opaque something is that holds for `hold` seconds, `idle` seconds after something last
/// happened to the window.
fn opacity(idle: f32, hold: f32) -> f32 {
    (1.0 - (idle - hold) / FADE_SECONDS).clamp(0.0, 1.0)
}

/// Fades the window out when it has been idle, and in when it is not.
fn fade(
    mut console: ResMut<Console>,
    line: Res<CommandLine>,
    time: Res<Time<Real>>,
    mut windows: Query<&mut Visibility, With<ConsoleWindow>>,
    mut areas: Query<&mut BackgroundColor, With<LogArea>>,
    mut texts: Query<(&Ink, &mut TextColor)>,
) {
    let now = time.elapsed_secs();
    if line.is_open() {
        console.active_at = now;
    }
    let idle = now - console.active_at;
    let alpha = opacity(idle, HOLD_SECONDS);
    console.alpha = alpha;
    let shown = if alpha > 0.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut windows {
        if *visibility != shown {
            *visibility = shown;
        }
    }
    for mut background in &mut areas {
        let faded = Color::BLACK.with_alpha(BACKGROUND_ALPHA * opacity(idle, BACKGROUND_HOLD_SECONDS));
        if background.0 != faded {
            background.0 = faded;
        }
    }
    for (ink, mut color) in &mut texts {
        let faded = faded(ink.0, alpha);
        if color.0 != faded {
            color.0 = faded;
        }
    }
}

fn faded(color: Color, alpha: f32) -> Color {
    color.with_alpha(color.alpha() * alpha)
}

/// Puts the lines in view on screen, when they or the view have changed.
fn rebuild(
    console: Res<Console>,
    areas: Query<Entity, With<LogArea>>,
    mut commands: Commands,
    mut shown: Local<Option<(u32, usize)>>,
) {
    let key = (console.version, console.scroll);
    if *shown == Some(key) {
        return;
    }
    let Ok(area) = areas.single() else {
        return;
    };
    *shown = Some(key);
    commands.entity(area).despawn_children();
    let end = console.lines.len() - console.scroll;
    let start = end.saturating_sub(VISIBLE);
    for line in console.lines.range(start..end) {
        spawn_line(&mut commands, area, line, console.alpha);
    }
    if console.scroll > 0 {
        let footer = Log::colored(format!("-- {} newer --", console.scroll), DIM_COLOR);
        spawn_line(&mut commands, area, &footer, console.alpha);
    }
}

/// A line of text in the area: the first piece is the text, and the others spans of it.
fn spawn_line(commands: &mut Commands, area: Entity, line: &Log, alpha: f32) {
    let font = TextFont {
        font_size: FONT_SIZE.into(),
        ..default()
    };
    let mut pieces = line.pieces.iter();
    let Some(first) = pieces.next() else {
        return;
    };
    commands
        .spawn((
            Text::new(first.text.clone()),
            font.clone(),
            TextColor(faded(first.color, alpha)),
            Ink(first.color),
            // The area is clipped, not squeezed.
            Node {
                flex_shrink: 0.0,
                ..default()
            },
            ChildOf(area),
        ))
        .with_children(|parent| {
            for piece in pieces {
                parent.spawn((
                    TextSpan::new(piece.text.clone()),
                    font.clone(),
                    TextColor(faded(piece.color, alpha)),
                    Ink(piece.color),
                ));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console_of(n: usize) -> Console {
        let mut console = Console::default();
        for i in 0..n {
            console.push(Log::new(i.to_string()), 0.0);
        }
        console
    }

    #[test]
    fn a_view_scrolled_back_stays_on_its_lines() {
        let mut console = console_of(10);
        console.scroll_by(3, 0.0);
        assert_eq!(console.scroll, 3);
        console.push(Log::new("new"), 0.0);
        assert_eq!(console.scroll, 4);
        console.scroll_to_end();
        console.push(Log::new("newer"), 0.0);
        assert_eq!(console.scroll, 0);
    }

    #[test]
    fn scrolling_stops_at_the_ends() {
        let mut console = console_of(5);
        console.scroll_by(100, 0.0);
        assert_eq!(console.scroll, 4);
        console.scroll_by(-100, 0.0);
        assert_eq!(console.scroll, 0);
        // (Nothing to scroll in.)
        let mut empty = Console::default();
        empty.scroll_by(5, 0.0);
        assert_eq!(empty.scroll, 0);
    }

    #[test]
    fn the_oldest_lines_go_first() {
        let mut console = console_of(CAPACITY + 5);
        assert_eq!(console.lines.len(), CAPACITY);
        assert_eq!(console.lines[0].pieces[0].text, "5");
        console.scroll_by(i32::MAX, 0.0);
        assert_eq!(console.scroll, CAPACITY - 1);
    }

    #[test]
    fn the_window_holds_then_fades() {
        let text = |idle| opacity(idle, HOLD_SECONDS);
        assert_eq!(text(0.0), 1.0);
        assert_eq!(text(HOLD_SECONDS), 1.0);
        assert!((text(HOLD_SECONDS + FADE_SECONDS / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(text(HOLD_SECONDS + FADE_SECONDS), 0.0);
        assert_eq!(text(f32::INFINITY), 0.0);
        // The background is gone while the text is still up.
        assert_eq!(opacity(BACKGROUND_HOLD_SECONDS + FADE_SECONDS, BACKGROUND_HOLD_SECONDS), 0.0);
    }
}
