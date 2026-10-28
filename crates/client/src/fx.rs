//! (Debug) The effects panel: F3 or `/debug fx` opens and closes it, at the right of the screen.
//! It shows and changes what `/debug fx` does, to try a visual effect without the race it needs:
//!
//! * `confetti auto|on|off`: the confetti of a win (`confetti.rs`), as the race says or forced
//! * `flash`: the white flash of a blast on our kart (`flash.rs`)
//! * `finish`: our kart finishes the race now, first (offline)
//! * `speed <factor>`: slow motion (offline), 1 is the game's speed
//! * `freeze on|off`, `step`: the race stops, and goes on one tick at a time (offline)
//! * `weather on|off`, `rain on|off`: the level's rain or snow, and the red beaker's rain
//!
//! A click on a line of the panel does the next of what it offers. The effects that are only drawn
//! work online too; the ones that change the race (`finish`) or its clock do not, as the race is
//! everyone's there.

use bevy::ecs::system::SystemParam;
use bevy::picking::events::{PointerClick, PointerOver};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;

use render::font::Font;

use crate::command::Action;
use crate::font::{CtrText, TexelSize};
use crate::lobby::Lobby;
use crate::toast::Toast;

/// An effect to try, or a way to watch it (`/debug fx`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fx {
    /// Open or close the panel.
    Panel,
    Confetti(Force),
    Flash,
    Finish,
    /// How fast the race's clock runs, offline.
    Speed(f32),
    Freeze(bool),
    /// One tick of a frozen race.
    Step,
    Weather(bool),
    BeakerRain(bool),
}

/// Whether an effect is as the race has it, or forced on or off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Force {
    #[default]
    Auto,
    On,
    Off,
}

impl Force {
    /// Whether the effect shows, when the race says `auto`.
    pub fn apply(self, auto: bool) -> bool {
        match self {
            Force::Auto => auto,
            Force::On => true,
            Force::Off => false,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Force::Auto => "auto",
            Force::On => "on",
            Force::Off => "off",
        }
    }

    fn next(self) -> Force {
        match self {
            Force::Auto => Force::On,
            Force::On => Force::Off,
            Force::Off => Force::Auto,
        }
    }
}

/// The slowest and fastest the clock can be set to.
pub const SPEED_RANGE: (f32, f32) = (0.05, 4.0);

/// The speeds a click on the panel goes through.
const SPEEDS: [f32; 4] = [1.0, 0.5, 0.25, 0.1];

/// What the effects are set to.
#[derive(Resource)]
pub struct Effects {
    panel: bool,
    pub confetti: Force,
    pub speed: f32,
    frozen: bool,
    /// The ticks a frozen race may still run.
    steps: u32,
    pub weather: bool,
    pub beaker_rain: bool,
}

impl Default for Effects {
    fn default() -> Effects {
        Effects {
            panel: false,
            confetti: Force::Auto,
            speed: 1.0,
            frozen: false,
            steps: 0,
            weather: true,
            beaker_rain: true,
        }
    }
}

impl Effects {
    /// Whether the race holds still this tick: it is frozen, and no step is left.
    pub fn holds(&mut self) -> bool {
        if !self.frozen {
            return false;
        }
        if self.steps > 0 {
            self.steps -= 1;
            return false;
        }
        true
    }
}

/// What stops an offline race: the pause menu, or the panel's freeze. (One parameter for the two,
/// as `drive_tick` has as many as a system can.)
#[derive(SystemParam)]
pub struct Hold<'w> {
    pub menu: ResMut<'w, crate::pause::PauseMenu>,
    pub fx: ResMut<'w, Effects>,
}

/// Whether the level's weather is drawn.
pub fn weather(fx: Res<Effects>) -> bool {
    fx.weather
}

/// Whether the red beaker's rain is drawn.
pub fn beaker_rain(fx: Res<Effects>) -> bool {
    fx.beaker_rain
}

pub struct FxPlugin;

impl Plugin for FxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Effects>()
            .init_resource::<Picked>()
            .add_systems(
                Update,
                (act, online, keys, show)
                    .chain()
                    .run_if(crate::drive::driving),
            );
    }
}

/// Does what `/debug fx` and the panel ask, but `finish`, which is `drive.rs`'s.
fn act(
    mut actions: MessageReader<Action>,
    mut fx: ResMut<Effects>,
    mut flash: ResMut<crate::flash::Flash>,
    lobby: Res<Lobby>,
    mut toasts: MessageWriter<Toast>,
) {
    for action in actions.read() {
        let Action::DebugFx(what) = *action else {
            continue;
        };
        let offline_only = matches!(what, Fx::Speed(_) | Fx::Freeze(_) | Fx::Step);
        if offline_only && lobby.online.is_some() {
            toasts.write(Toast::new("Not online", 2500).color(Color::srgb(1.0, 0.4, 0.4)));
            continue;
        }
        match what {
            Fx::Panel => fx.panel = !fx.panel,
            Fx::Confetti(force) => fx.confetti = force,
            Fx::Flash => flash.full(),
            Fx::Finish => {}
            Fx::Speed(speed) => fx.speed = speed.clamp(SPEED_RANGE.0, SPEED_RANGE.1),
            Fx::Freeze(on) => {
                fx.frozen = on;
                fx.steps = 0;
            }
            Fx::Step => {
                if fx.frozen {
                    fx.steps += 1;
                } else {
                    toasts.write(Toast::new("Not frozen", 2000));
                }
            }
            Fx::Weather(on) => fx.weather = on,
            Fx::BeakerRain(on) => fx.beaker_rain = on,
        }
    }
}

/// An online race keeps its own clock: the slow motion and the freeze end when one begins.
fn online(lobby: Res<Lobby>, mut fx: ResMut<Effects>) {
    if lobby.online.is_some() && (fx.speed != 1.0 || fx.frozen) {
        fx.speed = 1.0;
        fx.frozen = false;
    }
}

/// F3 opens and closes the panel. (Not while the command line has the keyboard: it takes the
/// keys it reads.)
fn keys(keys: Res<ButtonInput<KeyCode>>, mut actions: MessageWriter<Action>) {
    if keys.just_pressed(KeyCode::F3) {
        actions.write(Action::DebugFx(Fx::Panel));
    }
}

/// A line of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Confetti,
    Flash,
    Finish,
    Speed,
    Freeze,
    Step,
    Weather,
    BeakerRain,
}

const ITEMS: [Item; 8] = [
    Item::Confetti,
    Item::Flash,
    Item::Finish,
    Item::Speed,
    Item::Freeze,
    Item::Step,
    Item::Weather,
    Item::BeakerRain,
];

impl Item {
    /// The line, in what the game's font has (capitals, digits, a little punctuation).
    fn label(self, fx: &Effects) -> String {
        let on_off = |on: bool| if on { "ON" } else { "OFF" };
        match self {
            Item::Confetti => format!("CONFETTI: {}", fx.confetti.name().to_uppercase()),
            Item::Flash => "FLASH".into(),
            Item::Finish => "FINISH FIRST".into(),
            Item::Speed => format!("SPEED: {}X", fx.speed),
            Item::Freeze => format!("FREEZE: {}", on_off(fx.frozen)),
            Item::Step => "STEP ONE TICK".into(),
            Item::Weather => format!("WEATHER: {}", on_off(fx.weather)),
            Item::BeakerRain => format!("BEAKER RAIN: {}", on_off(fx.beaker_rain)),
        }
    }

    /// What a click asks for: the next setting, or the effect.
    fn action(self, fx: &Effects) -> Fx {
        match self {
            Item::Confetti => Fx::Confetti(fx.confetti.next()),
            Item::Flash => Fx::Flash,
            Item::Finish => Fx::Finish,
            Item::Speed => {
                let at = SPEEDS.iter().position(|&s| s == fx.speed);
                Fx::Speed(SPEEDS[at.map_or(0, |i| (i + 1) % SPEEDS.len())])
            }
            Item::Freeze => Fx::Freeze(!fx.frozen),
            Item::Step => Fx::Step,
            Item::Weather => Fx::Weather(!fx.weather),
            Item::BeakerRain => Fx::BeakerRain(!fx.beaker_rain),
        }
    }
}

/// The panel, and the texel size it was built for.
#[derive(Component)]
struct Panel(TexelSize);

/// A row, and its place in the panel.
#[derive(Component)]
struct Row(usize);

/// The text of a row.
#[derive(Component)]
struct Line(Item);

/// The row the mouse last pointed at, which has the menu's bar.
#[derive(Resource, Default)]
struct Picked(usize);

/// Puts the panel up or takes it down (and builds it again for a new texel size), keeps its lines
/// those of the settings now, and the bar on the row picked.
#[allow(clippy::too_many_arguments)]
fn show(
    mut commands: Commands,
    fx: Res<Effects>,
    size: Res<TexelSize>,
    time: Res<Time<Real>>,
    picked: Res<Picked>,
    panels: Query<(Entity, &Panel)>,
    mut lines: Query<(&Line, &mut CtrText)>,
    mut rows: Query<(&Row, &mut BackgroundColor)>,
) {
    let panel = panels.single().ok();
    let stale = panel.is_some_and(|(_, built)| built.0 != *size);
    if let Some((entity, _)) = panel.filter(|_| stale || !fx.panel) {
        commands.entity(entity).despawn();
    }
    if fx.panel && (panel.is_none() || stale) {
        spawn(&mut commands, &fx, &size);
        return;
    }
    if fx.is_changed() {
        for (Line(item), mut text) in &mut lines {
            let label = item.label(&fx);
            if text.text != label {
                text.text = label;
            }
        }
    }
    let bar = crate::rect_menu::bar(time.elapsed_secs());
    for (Row(i), mut color) in &mut rows {
        color.0 = if *i == picked.0 { bar } else { Color::NONE };
    }
}

fn spawn(commands: &mut Commands, fx: &Effects, size: &TexelSize) {
    let (panel, rows) = crate::rect_menu::spawn(commands, size, "EFFECTS", |node| {
        node.right = size.x(24);
        node.top = size.y(40);
    });
    commands
        .entity(panel)
        .insert((Panel(*size), GlobalZIndex(i32::MAX - 2)));
    for (i, item) in ITEMS.into_iter().enumerate() {
        commands
            .spawn((
                ChildOf(rows),
                Row(i),
                Node {
                    height: size.y(crate::rect_menu::ROW),
                    padding: UiRect::left(size.x(1)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_child((
                Line(item),
                CtrText::new(item.label(fx), Font::Middle, crate::rect_menu::TEXT),
                Pickable::IGNORE,
            ))
            .observe(move |_: On<PointerOver>, mut picked: ResMut<Picked>| {
                picked.0 = i;
            })
            .observe(
                move |click: On<PointerClick>,
                      fx: Res<Effects>,
                      mut actions: MessageWriter<Action>| {
                    if click.button == PointerButton::Primary {
                        actions.write(Action::DebugFx(item.action(&fx)));
                    }
                },
            );
    }
}
