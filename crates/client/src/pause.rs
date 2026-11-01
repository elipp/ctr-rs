//! The pause menu, in the game's fonts. Escape, P or a pad's Start opens and closes it. Up and
//! Down (W and S, the D-pad, the left stick pushed most of the way) pick an item, Return or Space (Cross) takes it, and Backspace
//! (Triangle) goes back to the race. The mouse picks an item by pointing at it, and a click takes
//! it.
//!
//! Outside a lobby it also offers "Public lobbies": the realm's list (`public_lobbies.rs`), where
//! taking a lobby joins it. Backspace goes back from there to the menu, not to the race.
//!
//! "Options" has the effects', the music's and the voices' volumes as retail's options menu draws
//! them (`MainFreeze_MenuPtrOptions`): a wedge that grows to the right, with a white knob on it.
//! Left and Right (A and D, the D-pad, the left stick) held move the knob of the volume picked, as fast as
//! retail's 4 of 255 a frame, and the mouse puts it where it presses or drags. While a volume is
//! picked its sound plays to hear it by (`OptionsMenu_TestSound`); online, where the race's song
//! goes on, the music's is that song. The volumes are kept once the view is left.
//!
//! Its "Controls" lists the keyboard's driving keys (`controls.rs`). Taking one waits for a key,
//! which becomes that control's; Escape (Triangle, Start) leaves it as it was. A key the client
//! uses for something else while driving is refused, with a word on what it does. "Reset" puts
//! the defaults back.
//!
//! "About" has the version, and the licenses (`about.rs`) in an accordion (`accordion.rs`).
//!
//! Offline the race stops while the menu is open. Online it goes on, as the others are still
//! racing: the menu only takes our controls, and our kart gets no input until it closes.
//!
//! While it is open it takes the keyboard, as the command line does: its keys do not drive the
//! kart, open the command line or switch the level.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{KeyboardFocusLost, KeyboardInput};
use bevy::input::mouse::MouseWheel;
use bevy::picking::events::{PointerClick, PointerMove, PointerOver, PointerPress};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use howl::settings::Slider;
use render::font::{Font, Gradient};

use crate::about;
use crate::accordion::{self, Accordion, HasAccordion, Steer};
use crate::command::Action;
use crate::config::Config;
use crate::controls::{self, Control};
use crate::font::{CtrText, TexelSize};
use crate::lobby::Lobby;
use crate::public_lobbies::{self, PublicLobbies, Status};
use crate::sound::{MENU_BACK_SFX, MENU_MOVE_SFX, MENU_TAKE_SFX, Sounds};
use crate::toast::Toast;
use crate::version::VERSION;

/// Whether the menu is open, and what it was asked to do.
#[derive(Resource, Default)]
pub struct PauseMenu {
    pub open: bool,
    /// What the menu shows: its own items, or the realm's lobbies.
    view: View,
    /// The item picked, of those the menu shows.
    selected: usize,
    /// The item that was clicked, to take in the next update.
    clicked: Option<usize>,
    /// RESTART was taken: the next tick puts the race back on the grid.
    pub restart: bool,
    /// The way the keyboard slides a volume, -1, 0 or 1: Left or Right is held.
    slide_keys: i32,
    /// The ways the left stick was pushed the last frame (right and up 1), to tap with it.
    stick: IVec2,
    /// The part of a percent a held key has slid a volume, which is kept in whole ones.
    slid: f32,
    /// A volume or a key changed since the config was last saved.
    unsaved: bool,
    /// The control whose key is waited for.
    capturing: Option<Control>,
    /// The about page's list, once it is first shown.
    about: Option<Accordion>,
}

impl HasAccordion for PauseMenu {
    fn accordion(&mut self) -> Option<&mut Accordion> {
        self.about.as_mut()
    }
}

/// What the menu is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum View {
    #[default]
    Main,
    /// The public lobbies.
    Browser,
    /// The volumes.
    Options,
    /// The keyboard's driving keys.
    Controls,
    /// The version and the licenses.
    About,
}

impl View {
    /// The view Back goes to.
    fn parent(self) -> View {
        match self {
            View::Controls => View::Options,
            _ => View::Main,
        }
    }
}

/// Something the menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Resume,
    Restart,
    LeaveLobby,
    /// Shows the realm's list.
    PublicLobbies,
    /// Shows the volumes.
    Options,
    /// Shows the version and the licenses.
    About,
    Quit,
    /// A volume, which Left and Right change.
    Volume(Slider),
    /// Shows the driving keys.
    Controls,
    /// A driving key, which taking waits for a key to bind.
    Control(Control),
    /// The driving keys back to the defaults.
    ResetControls,
    /// The public lobby at that place of [`PublicLobbies::shown`].
    Join(usize),
    Refresh,
    /// Back to the main view.
    Back,
    /// A line that says something and does nothing.
    Info,
}

/// An item and the text it has, and what it is set to, shown at its right.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    item: Item,
    label: String,
    value: Option<String>,
}

fn entry(item: Item, label: impl Into<String>) -> Entry {
    Entry {
        item,
        label: label.into(),
        value: None,
    }
}

/// The title of the menu and what it offers now. An online race cannot be restarted by one
/// player.
fn items(
    lobby: &Lobby,
    menu: &PauseMenu,
    public: &PublicLobbies,
    config: &Config,
) -> (String, Vec<Entry>) {
    match menu.view {
        View::Browser => return ("Public lobbies".into(), browser_items(public)),
        View::Options => return ("Options".into(), options_items()),
        View::Controls => return ("Controls".into(), controls_items(config, menu.capturing)),
        View::About => return ("About".into(), vec![entry(Item::Back, "Back")]),
        View::Main => {}
    }
    let mut items = vec![entry(Item::Resume, "Resume")];
    if lobby.online.is_none() {
        items.push(entry(Item::Restart, "Restart"));
    }
    if lobby.is_open() {
        items.push(entry(Item::LeaveLobby, "Leave lobby"));
    } else {
        items.push(entry(Item::PublicLobbies, "Public lobbies"));
    }
    items.push(entry(Item::Options, "Options"));
    items.push(entry(Item::About, "About"));
    // A page cannot close its tab: in a browser quitting would only stop the game, on its last
    // frame.
    if cfg!(not(target_arch = "wasm32")) {
        items.push(entry(Item::Quit, "Quit"));
    }
    // Online the race goes on.
    let title = if lobby.online.is_none() {
        "Paused"
    } else {
        "Menu"
    };
    (title.into(), items)
}

/// The volumes, as retail has them (`Options_StringIDs_Audio`).
fn options_items() -> Vec<Entry> {
    vec![
        entry(Item::Volume(Slider::Fx), "FX"),
        entry(Item::Volume(Slider::Music), "Music"),
        entry(Item::Volume(Slider::Voice), "Voice"),
        entry(Item::Controls, "Controls"),
        entry(Item::Back, "Back"),
    ]
}

/// The driving keys, and "Press a key" at the one `capturing`.
fn controls_items(config: &Config, capturing: Option<Control>) -> Vec<Entry> {
    let mut items: Vec<Entry> = Control::ALL
        .iter()
        .map(|&control| Entry {
            item: Item::Control(control),
            label: control.label().into(),
            value: Some(if capturing == Some(control) {
                "Press a key".into()
            } else {
                config.controls.describe(control)
            }),
        })
        .collect();
    items.push(entry(Item::ResetControls, "Reset"));
    items.push(entry(Item::Back, "Back"));
    items
}

/// The realm's lobbies, one to a row: the name, the players, and if it is not simply a lobby we
/// can join, why not.
fn browser_items(public: &PublicLobbies) -> Vec<Entry> {
    let mut items = Vec::new();
    match &public.status {
        Status::Idle | Status::Loading => items.push(entry(Item::Info, "Looking...")),
        Status::Failed => {
            items.push(entry(Item::Info, "Could not reach the realm"));
            items.push(entry(Item::Refresh, "Try again"));
        }
        Status::Loaded(lobbies) => {
            let (shown, more) = public.shown();
            if lobbies.is_empty() {
                items.push(entry(Item::Info, "No public lobbies"));
            }
            for (i, lobby) in shown.iter().enumerate() {
                let name: String = lobby.name.chars().take(NAME_SHOWN).collect();
                let note = if !lobby.compatible {
                    "  other version"
                } else if lobby.racing {
                    "  racing"
                } else {
                    ""
                };
                let label = format!("{name}  {}/{}{note}", lobby.players, lobby.max_players);
                items.push(entry(Item::Join(i), label));
            }
            if more > 0 {
                items.push(entry(Item::Info, format!("and {more} more")));
            }
            items.push(entry(Item::Refresh, "Refresh"));
        }
    }
    items.push(entry(Item::Back, "Back"));
    items
}

/// The most chars of a lobby's name a row shows: the font is wide, and the screen is not.
pub(crate) const NAME_SHOWN: usize = 20;

/// A volume's wedge, in texels across and down, as long as retail's when its labels are as wide
/// as these (`380 - 30` less the widest), and its knob: a white bar in a black one a texel bigger
/// all round.
const TRACK: usize = 160;
const TRACK_HEIGHT: usize = 10;
const KNOB: usize = 5;
const KNOB_HEIGHT: usize = 12;
/// The width of a volume's label.
const LABEL: usize = 56;
/// `Options_VolumeSlider_Colors`: the wedge's colour at its point, and at its end.
const WEDGE_FROM: [u8; 3] = [0, 0, 0x20];
const WEDGE_TO: [u8; 3] = [0x80, 0x80, 0xff];
/// How fast a held key slides a volume, in percent a second: retail's 4 of 255 a frame, at 30
/// frames a second.
const SLIDE_PER_SECOND: f32 = 4.0 / 255.0 * 100.0 * 30.0;

/// How far the left stick is pushed to count as the D-pad: past 0x20 or 0xe0 of retail's 0 to
/// 0xff, 0x80 being the middle (`GAMEPAD_ProcessHold`'s analog buttons).
const STICK_PUSHED: f32 = 0x60 as f32 / 127.0;

/// The about page's lines shown at once, and their height in texels of the 2D art.
const ABOUT_LINES: usize = 22;
const ABOUT_TEXELS: f32 = 5.0;

/// The panel behind the items, the line around it, and its shadow.
const PANEL: Color = Color::srgba(0.04, 0.04, 0.06, 0.75);
const BORDER: Color = Color::srgb(0.5, 0.85, 0.85);
const SHADOW: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);

/// The menu on screen, and what it was built for.
#[derive(Component)]
struct Shown {
    items: Vec<Entry>,
    title: String,
}

/// An item's text, and its place in the menu.
#[derive(Component)]
struct Row(usize);

/// The knob of a volume.
#[derive(Component)]
struct Knob(Slider);

/// The picture of a volume's wedge.
#[derive(Resource)]
struct Wedge(Handle<Image>);

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenu>()
            .init_resource::<PublicLobbies>()
            .add_systems(Startup, make_wedge)
            .add_systems(Update, public_lobbies::poll.run_if(crate::drive::driving))
            .add_systems(
                PreUpdate,
                input
                    .after(InputSystems)
                    .after(crate::command::capture)
                    .run_if(crate::drive::driving),
            )
            .add_systems(Update, show.run_if(crate::drive::driving));
    }
}

/// Opens and closes the menu, moves the pick and takes an item, with the menu's sounds. Offline
/// it pauses the race's sounds too (`MainFreeze_IfPressStart`, `MainFrame_TogglePauseAudio`).
#[allow(clippy::too_many_arguments)]
fn input(
    mut menu: ResMut<PauseMenu>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut key_events: MessageReader<KeyboardInput>,
    mut focus_lost: MessageReader<KeyboardFocusLost>,
    mut wheel: MessageReader<MouseWheel>,
    size: Res<TexelSize>,
    pads: crate::drive::Pads,
    lobby: Res<Lobby>,
    mut public: ResMut<PublicLobbies>,
    mut config: ResMut<Config>,
    time: Res<Time<Real>>,
    sounds: Option<Res<Sounds>>,
    mut actions: MessageWriter<Action>,
    mut toasts: MessageWriter<Toast>,
    mut exit: MessageWriter<AppExit>,
) {
    // (The menu resets the keys every frame, so they no longer tell which are held.)
    let mut first_press = None;
    // With the repeats, for the about page.
    let mut list_keys = 0i32;
    for event in key_events.read() {
        if event.state == ButtonState::Pressed && !event.repeat {
            first_press = first_press.or(Some(event.key_code));
        }
        if event.state == ButtonState::Pressed {
            list_keys = list_keys.saturating_add(accordion::key_lines(event.key_code, ABOUT_LINES));
        }
        let way = match event.key_code {
            KeyCode::ArrowLeft | KeyCode::KeyA => -1,
            KeyCode::ArrowRight | KeyCode::KeyD => 1,
            _ => continue,
        };
        match event.state {
            ButtonState::Pressed => menu.slide_keys = way,
            ButtonState::Released if menu.slide_keys == way => menu.slide_keys = 0,
            ButtonState::Released => {}
        }
    }
    if focus_lost.read().count() > 0 {
        menu.slide_keys = 0;
    }
    let font_size = size.0.y * ABOUT_TEXELS;
    let wheeled: f32 = wheel
        .read()
        .map(|event| accordion::wheel_lines(event, font_size))
        .sum();
    let was_open = menu.open;
    let offline = lobby.online.is_none();
    let sound = |id| {
        if let Some(sounds) = &sounds {
            sounds.menu(id);
        }
    };
    let key = |codes: &[KeyCode]| codes.iter().any(|&k| keys.just_pressed(k));
    let pad = |button: GamepadButton| pads.iter().any(|p| p.just_pressed(button));
    // The left stick of the first pad that has it pushed, as the D-pad: tapped as it is pushed
    // past the line, and held as long as it is.
    let stick = pads
        .iter()
        .map(|p| p.left_stick())
        .find(|s| *s != Vec2::ZERO)
        .unwrap_or(Vec2::ZERO);
    let way = |v: f32| (v > STICK_PUSHED) as i32 - (v < -STICK_PUSHED) as i32;
    let pushed = IVec2::new(way(stick.x), way(stick.y));
    let stick_tap = |y: i32| pushed.y == y && menu.stick.y != y;
    let (stick_up, stick_down) = (stick_tap(1), stick_tap(-1));
    menu.stick = pushed;
    if let Some(control) = menu.capturing.filter(|_| menu.open) {
        menu.clicked = None;
        if first_press == Some(KeyCode::Escape)
            || pad(GamepadButton::North)
            || pad(GamepadButton::Start)
        {
            menu.capturing = None;
            sound(MENU_BACK_SFX);
        } else if let Some(key) = first_press {
            let refuse = |why: String| Toast::new(why, 3000).color(Color::srgb(1.0, 0.4, 0.4));
            if let Some(what) = controls::taken(key) {
                toasts.write(refuse(format!("{} {what}", controls::label(key))));
            } else if !controls::bindable(key) {
                toasts.write(refuse("That key cannot be bound".into()));
            } else {
                config.controls.bind(control, key);
                menu.capturing = None;
                menu.unsaved = true;
                sound(MENU_TAKE_SFX);
            }
        }
        save(&mut menu, &config, &mut toasts);
        return finish(was_open, &menu, &mut keys);
    }
    if key(&[KeyCode::Escape, KeyCode::KeyP]) || pad(GamepadButton::Start) {
        menu.open = !menu.open;
        menu.view = View::Main;
        menu.selected = 0;
        menu.clicked = None;
        menu.capturing = None;
        test_sound(&menu, &lobby, &config, &time, sounds.as_deref());
        if let Some(sounds) = sounds.as_ref().filter(|_| offline) {
            if menu.open {
                sounds.pause();
            } else {
                sounds.unpause();
            }
        }
        sound(MENU_TAKE_SFX);
        save(&mut menu, &config, &mut toasts);
        return finish(was_open, &menu, &mut keys);
    } else if menu.open {
        let (_, items) = items(&lobby, &menu, &public, &config);
        let count = items.len();
        let enter = key(&[KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space])
            || pad(GamepadButton::South);
        let about = menu.view == View::About;
        if let Some(list) = menu.about.as_mut().filter(|_| about) {
            let held = |button| pads.iter().any(|p| p.pressed(button)) as i32;
            let taps = (pad(GamepadButton::DPadDown) || stick_down) as i32
                - (pad(GamepadButton::DPadUp) || stick_up) as i32;
            let pages =
                pad(GamepadButton::RightTrigger) as i32 - pad(GamepadButton::LeftTrigger) as i32;
            let steer = Steer {
                lines: list_keys.saturating_add(taps + pages * ABOUT_LINES as i32),
                held: held(GamepadButton::DPadDown) - held(GamepadButton::DPadUp) - pushed.y,
                wheel: wheeled,
                take: enter,
            };
            if list.steer(steer, time.delta_secs()) {
                sound(MENU_TAKE_SFX);
            }
        } else {
            if key(&[KeyCode::ArrowUp, KeyCode::KeyW]) || pad(GamepadButton::DPadUp) || stick_up {
                menu.selected = (menu.selected + count - 1) % count;
                sound(MENU_MOVE_SFX);
            }
            if key(&[KeyCode::ArrowDown, KeyCode::KeyS])
                || pad(GamepadButton::DPadDown)
                || stick_down
            {
                menu.selected = (menu.selected + 1) % count;
                sound(MENU_MOVE_SFX);
            }
        }
        // (On the about page taking is for the list, and Back is clicked.)
        let taken = if enter && menu.view != View::About {
            Some(menu.selected)
        } else {
            menu.clicked.take()
        };
        let taken_item = taken.and_then(|i| items.get(i)).map(|e| e.item);
        if taken_item.is_some_and(|item| !matches!(item, Item::Volume(_))) {
            sound(MENU_TAKE_SFX);
        }
        if let Some(&Entry {
            item: Item::Volume(slider),
            ..
        }) = items.get(menu.selected)
        {
            let pad_way = |button| pads.iter().any(|p| p.pressed(button)) as i32;
            let way = menu.slide_keys + pad_way(GamepadButton::DPadRight)
                - pad_way(GamepadButton::DPadLeft)
                + pushed.x;
            slide(
                &mut menu,
                &mut config,
                slider,
                way.signum(),
                time.delta_secs(),
            );
        }
        if key(&[KeyCode::Backspace]) || pad(GamepadButton::North) {
            sound(MENU_BACK_SFX);
            if menu.view != View::Main {
                menu.view = menu.view.parent();
                menu.selected = 0;
            } else {
                menu.open = false;
            }
        }
        match taken_item {
            Some(Item::Resume) => menu.open = false,
            Some(Item::Restart) => {
                menu.open = false;
                menu.restart = true;
            }
            Some(Item::LeaveLobby) => {
                menu.open = false;
                actions.write(Action::CloseLobby);
            }
            Some(Item::PublicLobbies) => {
                menu.view = View::Browser;
                menu.selected = 0;
                public.refresh();
            }
            Some(Item::Options) => {
                menu.view = View::Options;
                menu.selected = 0;
            }
            Some(Item::About) => {
                menu.view = View::About;
                menu.selected = 0;
                menu.about.get_or_insert_with(|| about::list(ABOUT_LINES));
            }
            Some(Item::Refresh) => public.refresh(),
            Some(Item::Back) => {
                menu.view = menu.view.parent();
                menu.selected = 0;
            }
            Some(Item::Controls) => {
                menu.view = View::Controls;
                menu.selected = 0;
            }
            Some(Item::Control(control)) => menu.capturing = Some(control),
            Some(Item::ResetControls) => {
                config.controls = controls::Controls::default();
                menu.unsaved = true;
            }
            Some(Item::Join(i)) => {
                if let Some(found) = public.shown().0.get(i) {
                    actions.write(Action::JoinLobby(net::Ticket {
                        id: found.id,
                        relay: None,
                    }));
                    menu.open = false;
                }
            }
            Some(Item::Info | Item::Volume(_)) | None => {}
            Some(Item::Quit) => {
                exit.write(AppExit::Success);
            }
        }
        test_sound(&menu, &lobby, &config, &time, sounds.as_deref());
        // Leaving the pause cuts the sound of what left it (`MainFrame_TogglePauseAudio`).
        if !menu.open
            && let Some(sounds) = sounds.as_ref().filter(|_| offline)
        {
            sounds.unpause();
        }
    }
    save(&mut menu, &config, &mut toasts);
    finish(was_open, &menu, &mut keys);
}

/// Moves volume `slider` the `way` a key is held (-1, 0 or 1) for `seconds`, in whole percents.
fn slide(menu: &mut PauseMenu, config: &mut Config, slider: Slider, way: i32, seconds: f32) {
    if way == 0 {
        menu.slid = 0.0;
        return;
    }
    menu.slid += way as f32 * SLIDE_PER_SECOND * seconds;
    let whole = menu.slid.trunc();
    menu.slid -= whole;
    if whole != 0.0 {
        let to = (volume(config, slider) as f32 + whole).clamp(0.0, 100.0) as u8;
        set_volume(config, menu, slider, to);
    }
}

/// The config's volume `slider`, 0 to 100.
fn volume(config: &Config, slider: Slider) -> u8 {
    match slider {
        Slider::Fx => config.volume_fx,
        Slider::Music => config.volume_music,
        Slider::Voice => config.volume_voice,
    }
}

/// Sets the config's volume `slider` to `percent`, if it is another.
fn set_volume(config: &mut Config, menu: &mut PauseMenu, slider: Slider, percent: u8) {
    if volume(config, slider) != percent {
        *volume_mut(config, slider) = percent;
        menu.unsaved = true;
    }
}

fn volume_mut(config: &mut Config, slider: Slider) -> &mut u8 {
    match slider {
        Slider::Fx => &mut config.volume_fx,
        Slider::Music => &mut config.volume_music,
        Slider::Voice => &mut config.volume_voice,
    }
}

/// The sound of the volume picked plays, and none once none is (`OptionsMenu_TestSound`). Online
/// the race's song plays on, and is the music's.
fn test_sound(
    menu: &PauseMenu,
    lobby: &Lobby,
    config: &Config,
    time: &Time<Real>,
    sounds: Option<&Sounds>,
) {
    let Some(sounds) = sounds else {
        return;
    };
    let picked = (menu.open && menu.view == View::Options)
        .then(options_items)
        .and_then(|items| match items.get(menu.selected)?.item {
            Item::Volume(Slider::Music) if lobby.online.is_some() => None,
            Item::Volume(slider) => Some(slider),
            _ => None,
        });
    // The game's frames, 30 a second.
    let frame = (time.elapsed_secs_f64() * 30.0) as u32;
    let character = config.character.unwrap_or(0) as u8;
    sounds.test_sound(picked, character, frame);
}

/// Keeps the volumes once the options are left, and the keys at once.
fn save(menu: &mut PauseMenu, config: &Config, toasts: &mut MessageWriter<Toast>) {
    if menu.unsaved && !(menu.open && menu.view == View::Options) {
        menu.unsaved = false;
        menu.slide_keys = 0;
        if let Err(why) = config.save() {
            toasts.write(Toast::new(why, 3000).color(Color::srgb(1.0, 0.4, 0.4)));
        }
    }
}

/// The keys that opened, worked or closed the menu are not for anything else.
fn finish(was_open: bool, menu: &PauseMenu, keys: &mut ButtonInput<KeyCode>) {
    if was_open || menu.open {
        keys.reset_all();
    }
}

/// Builds the menu when it opens (and again when what it offers or the texel size changes), takes
/// it down when it closes, and shows the pick in white.
#[allow(clippy::too_many_arguments)]
fn show(
    mut commands: Commands,
    mut menu: ResMut<PauseMenu>,
    lobby: Res<Lobby>,
    public: Res<PublicLobbies>,
    config: Res<Config>,
    size: Res<TexelSize>,
    wedge: Res<Wedge>,
    shown: Query<(Entity, &Shown)>,
    rows: Query<(&Row, &Children)>,
    mut texts: Query<&mut CtrText>,
    mut knobs: Query<(&Knob, &mut Node)>,
    mut about_lines: accordion::Lines<PauseMenu>,
) {
    let wanted = menu.open.then(|| {
        let (title, items) = items(&lobby, &menu, &public, &config);
        Shown { items, title }
    });
    let current = shown.single().ok();
    let stale = match (&wanted, current) {
        (Some(wanted), Some((_, current))) => {
            size.is_changed() || wanted.items != current.items || wanted.title != current.title
        }
        (None, None) => false,
        _ => true,
    };
    if stale {
        if let Some((entity, _)) = current {
            commands.entity(entity).despawn();
        }
        if let Some(wanted) = wanted {
            menu.selected = menu.selected.min(wanted.items.len() - 1);
            let about = menu.about.as_ref().filter(|_| menu.view == View::About);
            spawn(&mut commands, wanted, menu.selected, about, &size, &wedge);
        }
        return;
    }
    if let Some(list) = &menu.about {
        accordion::update(list, &mut about_lines);
    }
    for (knob, mut node) in &mut knobs {
        let at = size.x(volume(&config, knob.0) as usize * (TRACK - KNOB) / 100);
        if node.left != at {
            node.left = at;
        }
    }
    for (row, children) in &rows {
        let gradient = gradient(row.0 == menu.selected);
        for &child in children {
            if let Ok(mut text) = texts.get_mut(child)
                && text.gradient != gradient
            {
                text.gradient = gradient;
            }
        }
    }
}

/// The picked item is white, the others orange.
fn gradient(picked: bool) -> Gradient {
    if picked {
        Gradient::WHITE
    } else {
        Gradient::ORANGE
    }
}

/// The menu, and the about page's list if it shows it.
fn spawn(
    commands: &mut Commands,
    shown: Shown,
    selected: usize,
    about: Option<&Accordion>,
    size: &TexelSize,
    wedge: &Wedge,
) {
    let items = shown.items.clone();
    let title = shown.title.clone();
    // The whole window, to put the panel in its middle. Under the command line.
    let root = commands
        .spawn((
            shown,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(i32::MAX - 1),
        ))
        .id();
    let panel = commands
        .spawn((
            ChildOf(root),
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(size.x(12), size.y(6)),
                border: UiRect::axes(size.x(1), size.y(1)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(BORDER),
            BoxShadow::new(SHADOW, size.x(4), size.y(4), Val::ZERO, Val::ZERO),
        ))
        .id();
    commands.spawn((
        ChildOf(panel),
        CtrText::new(title, Font::Large, Gradient::ORANGE),
        Node {
            margin: UiRect::bottom(size.y(4)),
            ..default()
        },
    ));
    if let Some(list) = about {
        spawn_about(commands, panel, list, size);
    }
    for (i, item) in items.into_iter().enumerate() {
        let text = CtrText::new(item.label, Font::Middle, gradient(i == selected));
        // A setting has its label at the left and its value at the right.
        let justify = if item.value.is_some() {
            JustifyContent::SpaceBetween
        } else {
            JustifyContent::Center
        };
        let row = commands
            .spawn((
                ChildOf(panel),
                Row(i),
                Node {
                    width: Val::Percent(100.0),
                    justify_content: justify,
                    align_items: AlignItems::Center,
                    column_gap: size.x(16),
                    padding: UiRect::vertical(size.y(2)),
                    ..default()
                },
            ))
            .id();
        if let Some(value) = item.value {
            commands.spawn((ChildOf(row), text));
            commands.spawn((
                ChildOf(row),
                CtrText::new(value, Font::Middle, gradient(i == selected)),
            ));
        } else if let Item::Volume(slider) = item.item {
            commands.spawn((
                ChildOf(row),
                text,
                Node {
                    width: size.x(LABEL),
                    ..default()
                },
            ));
            spawn_track(commands, row, slider, size, wedge);
        } else {
            commands.spawn((ChildOf(row), text));
        }
        commands
            .entity(row)
            .observe(move |_: On<PointerOver>, mut menu: ResMut<PauseMenu>| {
                menu.selected = i;
            })
            .observe(
                move |click: On<PointerClick>, mut menu: ResMut<PauseMenu>| {
                    if click.button == PointerButton::Primary {
                        menu.clicked = Some(i);
                    }
                },
            );
    }
}

/// The about page in `panel`: the version, and the list.
fn spawn_about(commands: &mut Commands, panel: Entity, list: &Accordion, size: &TexelSize) {
    let font_size = size.0.y * ABOUT_TEXELS;
    commands.spawn((
        ChildOf(panel),
        Text::new(format!(
            "ctr-rs {VERSION}\nFree software under the GNU GPL 3. Source: https://github.com/elipp/ctr-rs"
        )),
        TextFont {
            font_size: font_size.into(),
            ..default()
        },
        TextColor(accordion::TEXT),
        Node {
            margin: UiRect::bottom(size.y(4)),
            ..default()
        },
    ));
    let holder = commands
        .spawn((
            ChildOf(panel),
            Node {
                margin: UiRect::bottom(size.y(4)),
                ..default()
            },
        ))
        .id();
    accordion::spawn::<PauseMenu>(commands, holder, list, font_size);
}

/// Volume `slider`'s wedge and knob, in `row`. Pressing the wedge, or dragging over it, puts the
/// knob there.
fn spawn_track(
    commands: &mut Commands,
    row: Entity,
    slider: Slider,
    size: &TexelSize,
    wedge: &Wedge,
) {
    let track = commands
        .spawn((
            ChildOf(row),
            Node {
                width: size.x(TRACK),
                height: size.y(TRACK_HEIGHT),
                ..default()
            },
            ImageNode::new(wedge.0.clone()).with_mode(NodeImageMode::Stretch),
        ))
        .observe(
            move |press: On<PointerPress>,
                  mut config: ResMut<Config>,
                  mut menu: ResMut<PauseMenu>| {
                if press.button == PointerButton::Primary {
                    put_knob(&mut config, &mut menu, slider, press.hit.position);
                }
            },
        )
        .observe(
            move |moved: On<PointerMove>,
                  mouse: Res<ButtonInput<MouseButton>>,
                  mut config: ResMut<Config>,
                  mut menu: ResMut<PauseMenu>| {
                if mouse.pressed(MouseButton::Left) {
                    put_knob(&mut config, &mut menu, slider, moved.hit.position);
                }
            },
        )
        .id();
    // The knob's black, a texel over the wedge, and its white inside.
    commands.spawn((
        ChildOf(track),
        Knob(slider),
        Node {
            position_type: PositionType::Absolute,
            top: -size.y((KNOB_HEIGHT - TRACK_HEIGHT) / 2),
            width: size.x(KNOB),
            height: size.y(KNOB_HEIGHT),
            border: UiRect::axes(size.x(1), size.y(1)),
            ..default()
        },
        BackgroundColor(Color::WHITE),
        BorderColor::all(Color::BLACK),
        Pickable::IGNORE,
    ));
}

/// Volume `slider` where the pointer is on its wedge (`at`: -0.5 at its left, 0.5 at its right),
/// the knob's middle under the pointer.
fn put_knob(config: &mut Config, menu: &mut PauseMenu, slider: Slider, at: Option<Vec3>) {
    let Some(at) = at else {
        return;
    };
    let middle = (at.x + 0.5) * TRACK as f32 - KNOB as f32 / 2.0;
    let percent = (middle / (TRACK - KNOB) as f32 * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8;
    set_volume(config, menu, slider, percent);
}

/// The picture of a volume's wedge (`RECTMENU_DrawRwdTriangle`): a texel high at its left and
/// [`TRACK_HEIGHT`] at its right, its colour shading from [`WEDGE_FROM`] to [`WEDGE_TO`] across.
fn make_wedge(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut rgba = vec![0; TRACK * TRACK_HEIGHT * 4];
    for x in 0..TRACK {
        let t = x as f32 / (TRACK - 1) as f32;
        let height = 1 + ((TRACK_HEIGHT - 1) as f32 * t).round() as usize;
        let mut colour = [0xff; 4];
        for (c, (from, to)) in colour.iter_mut().zip(WEDGE_FROM.iter().zip(WEDGE_TO)) {
            *c = (*from as f32 + (to as f32 - *from as f32) * t).round() as u8;
        }
        for y in TRACK_HEIGHT - height..TRACK_HEIGHT {
            rgba[(y * TRACK + x) * 4..][..4].copy_from_slice(&colour);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: TRACK as u32,
            height: TRACK_HEIGHT as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    commands.insert_resource(Wedge(images.add(image)));
}
