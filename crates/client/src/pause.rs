//! The pause menu, in the game's fonts. Escape, P or a pad's Start opens it; P and Start close
//! it from any view. Up and Down (W and S, the D-pad, the left stick pushed most of the way) pick
//! an item, Return or Space (Cross) takes it, and Escape or Backspace (Triangle) go back a view,
//! closing the menu from the top one. The mouse picks an item by pointing at it, and a click takes
//! it, as a tap does on a touch screen. Items that do not fit scroll: with the mouse wheel, and
//! to keep the pick shown as the keys move it.
//!
//! Outside a lobby it also offers "Multiplayer": the realm's list (`public_lobbies.rs`), where
//! taking a lobby joins it, and creating a public or a private lobby. Private lobbies have no
//! name on the wire (`Action::CreateLobby`), so only the public one asks for one.
//! Its "Split screen" (out of a race) is `/players`: Left and Right step it.
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
//! Rows are generic: `input` handles each [`Kind`] once and only calls item-specific code
//! through `step`/`remove`/`field`/`commit`/`take_icon`. A new row type should be a `Kind`, not
//! a special case on an `Item`. The menu is respawned whenever the `Vec<Entry>` compares unequal,
//! so per-frame state (pick, grid cursor, scroll) lives in `PauseMenu`, not in entries.
//!
//! A free drive or a race alone stops while the menu is open. A lobby's race goes on, as the
//! others are still racing: the menu only takes our controls, and our kart gets no input until
//! it closes.
//!
//! While it is open it takes the keyboard, as the command line does: its keys do not drive the
//! kart, open the command line or switch the level.

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::image::ImageSampler;
use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{KeyboardFocusLost, KeyboardInput};
use bevy::input::mouse::MouseWheel;
use bevy::picking::events::{PointerClick, PointerMove, PointerPress, PointerRelease};
use bevy::picking::pointer::{PointerButton, PointerId};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use howl::settings::Slider;
use render::font::{Font, Gradient};
use std::borrow::Cow;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

use crate::LevelNumber;
use crate::about;
use crate::accordion::{self, Accordion, HasAccordion, Steer};
use crate::bots_menu::{self, BotsItem};
use crate::command::Action;
use crate::config::Config;
use crate::controls::{self, Control};
use crate::cup_wizard::{self, CupItem, Draft, Go};
use crate::font::{CtrText, Fonts, TexelSize};
use crate::icon_grid::{self, HasIconGrid, Icons, Steered};
use crate::lobby::Lobby;
use crate::maps;
use crate::public_lobbies::{self, PublicLobbies, Status};
use crate::race_on::RaceOn;
use crate::sound::{MENU_BACK_SFX, MENU_MOVE_SFX, MENU_TAKE_SFX, Sounds};
use crate::suggest;
use crate::text_field::{TextField, Typed};
use crate::toast::Toast;
use crate::track_picture::TrackPictures;
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
    /// Character select was taken: the first player's racer menu opens (`chooser::input`).
    pub racer_menu: bool,
    /// The way the keyboard slides a volume, -1, 0 or 1: Left or Right is held.
    slide_keys: i32,
    /// The ways the left stick was pushed the last frame (right and up 1), to tap with it.
    stick: IVec2,
    /// The part of a percent a held key has slid a volume, which is kept in whole ones.
    slid: f32,
    /// A volume or a key changed since the config was last saved.
    unsaved: bool,
    /// The control whose key is waited for.
    pub capturing: Option<Control>,
    /// The about page's list, once it is first shown.
    about: Option<Accordion>,
    /// How far the items are scrolled, in logical pixels, when they are more than [`LIST_TEXELS`]
    /// tall. Kept here, as the menu is built anew whenever what it shows changes.
    scroll: f32,
    /// The pick was moved by a key or a pad: scroll the list to show it, once it is laid out.
    follow: bool,
    /// `None` until first edited; see `cup_wizard::draft`.
    cup: Option<Draft>,
    /// While set, the typing branch of `input` runs first and returns early, so Backspace,
    /// Escape and P never reach the menu's own handling.
    typing: Option<(Item, TextField)>,
    /// One cursor for the whole view: assumes at most one `Kind::Grid` per view.
    grid_cursor: usize,
    /// Arrow clicks and Left/Right both funnel through here so `step` runs in one place.
    stepped: Option<(usize, i32)>,
    /// Set from observers, which lack `Config`; applied in `input`.
    icon_taken: Option<(usize, usize)>,
    /// The pool page's draft: only meaningful in `View::BotPool` (see `bots_menu.rs`).
    bot_pool: u16,
    /// Seeded with a `suggest` name on entering `View::NameLobby`, so a pad or a touch screen,
    /// which can't type, can still create with Create.
    lobby_name: String,
    private_lobby: bool,
}

impl PauseMenu {
    /// Systems with their own hotkeys (mute, `/cup pause` key, racer menu) must check this.
    pub fn takes_keys(&self) -> bool {
        self.capturing.is_some() || self.typing.is_some()
    }
}

impl HasIconGrid for PauseMenu {
    fn point_icon(&mut self, grid: usize, icon: usize) {
        self.selected = grid;
        self.grid_cursor = icon;
    }

    fn take_icon(&mut self, grid: usize, icon: usize) {
        self.point_icon(grid, icon);
        self.icon_taken = Some((grid, icon));
    }
}

impl HasAccordion for PauseMenu {
    fn accordion(&mut self) -> Option<&mut Accordion> {
        self.about.as_mut()
    }
}

/// What the menu is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum View {
    #[default]
    Main,
    PlaySolo,
    Multiplayer,
    /// The public lobbies.
    Browser,
    NameLobby,
    /// The volumes.
    Options,
    /// The keyboard's driving keys.
    Controls,
    /// The version and the licenses.
    About,
    CupWizard,
    Weapons,
    Bots,
    BotPool,
    Tracks,
    /// "Quit the game?": Quit is too easy to take by mistake.
    ConfirmQuit,
}

impl View {
    /// The view Back goes to.
    fn parent(self) -> View {
        match self {
            View::Controls => View::Options,
            View::Weapons => View::CupWizard,
            View::BotPool => View::Bots,
            View::Browser | View::NameLobby => View::Multiplayer,
            _ => View::Main,
        }
    }
}

/// Something the menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Resume,
    Restart,
    /// `/race stop`: back to the free drive.
    EndRace,
    PlaySolo,
    Solo(race::solo::Kind),
    /// `/score top` of this kind on the track we are on.
    Leaderboard(race::solo::Kind),
    /// `Config::server_records`, which Left, Right and taking it toggle.
    ServerRecords,
    /// How many local players share the screen (`/players`), which Left and Right step.
    SplitScreen,
    LeaveLobby,
    Multiplayer,
    /// Shows the realm's list.
    PublicLobbies,
    /// Creates a lobby, public or private as `PauseMenu::private_lobby`, which Left and Right
    /// toggle.
    NewLobby,
    LobbyName,
    /// Creates the public lobby named in `View::NameLobby`.
    CreateLobby,
    /// Shows the volumes.
    Options,
    /// Shows the version and the licenses.
    About,
    CupWizard,
    Cup(CupItem),
    BotsMenu,
    Bots(BotsItem),
    /// Asks first (`View::ConfirmQuit`).
    Quit,
    QuitNow,
    TrackSelect,
    /// Opens the first player's racer menu (`drive::chooser`).
    CharSelect,
    Track(u32),
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    item: Item,
    label: String,
    kind: Kind,
    section: bool,
    inline: bool,
    font: Font,
    /// What the row means, in the footer while it is picked.
    help: Option<Cow<'static, str>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Button,
    Value(String),
    Cycle(String),
    Field {
        text: String,
        editing: bool,
    },
    Slider(Slider),
    Preview(Icons),
    Grid(Icons),
    /// A `LevelId`; resolved through `TrackPictures` at spawn, as `items` has no `Assets<Image>`.
    TrackPicture(u32),
}

impl Entry {
    pub(crate) fn new(item: Item, label: impl Into<String>, kind: Kind) -> Entry {
        Entry {
            item,
            label: label.into(),
            kind,
            section: false,
            inline: false,
            font: Font::Middle,
            help: None,
        }
    }

    pub(crate) fn help(mut self, help: impl Into<Cow<'static, str>>) -> Entry {
        self.help = Some(help.into());
        self
    }

    pub(crate) fn large(mut self) -> Entry {
        self.font = Font::Large;
        self
    }

    /// Label and value on one centred line ("Name abc"), instead of in the two columns.
    pub(crate) fn inline(mut self) -> Entry {
        self.inline = true;
        self
    }

    /// Space above this row, to set a group of rows apart. A group ends where the next starts.
    pub(crate) fn starts_section(mut self) -> Entry {
        self.section = true;
        self
    }

    /// Unpaired rows span both columns, centred. An empty label unpairs any kind, so e.g. a
    /// cycle can carry its own numbering in its value ("1. Dingo Canyon").
    fn paired(&self) -> bool {
        !self.inline && !self.label.is_empty() && !matches!(self.kind, Kind::Button | Kind::Grid(_))
    }
}

pub(crate) fn entry(item: Item, label: impl Into<String>) -> Entry {
    Entry::new(item, label, Kind::Button)
}

/// The title of the menu and what it offers now. An online race cannot be restarted by one
/// player.
fn items(
    lobby: &Lobby,
    race_on: &RaceOn,
    menu: &PauseMenu,
    public: &PublicLobbies,
    config: &Config,
    players: usize,
) -> (String, Vec<Entry>) {
    match menu.view {
        View::PlaySolo => {
            let items = vec![
                entry(Item::Solo(race::solo::Kind::Relic), "Relic race")
                    .help("Rules:\n- Time crates stop the clock\n- Collect all to get a -10 second bonus\n\nNOTE: you can submit your best times to the online leaderboards!"),
                entry(Item::Solo(race::solo::Kind::TimeTrial), "Time trial")
                    .help("Rules:\n- Drive the fastest possible 3-lap time\n\nNOTE: you can submit your best times to the online leaderboards!"),
                entry(Item::Leaderboard(race::solo::Kind::Relic), "Relic leaderboard")
                    .starts_section()
                    .help("The fastest relic races on this track, from the score server."),
                entry(Item::Leaderboard(race::solo::Kind::TimeTrial), "Time trial leaderboard")
                    .help("The fastest time trials on this track, from the score server."),
                Entry::new(
                    Item::ServerRecords,
                    SERVER_RECORDS,
                    Kind::Cycle(ON_OFF[config.server_records as usize].into()),
                )
                .inline()
                .help(
                    "On: each solo race downloads the leaderboard's #1 run from the score \
                     server, and its ghost becomes available.",
                ),
                entry(Item::Back, "Back").starts_section(),
            ];
            return ("Play solo".into(), items);
        }
        View::Multiplayer => {
            let mut items = vec![
                entry(Item::PublicLobbies, "Public lobbies")
                    .help(
                        "Lobbies that others have listed for anyone to join. Others see you by \
                         your nick: use '/config set nick <name>' to set it.",
                    ),
                Entry::new(
                    Item::NewLobby,
                    NEW_LOBBY,
                    Kind::Cycle(LOBBY_PRIVACY[menu.private_lobby as usize].into()),
                )
                .inline()
                .help(if menu.private_lobby {
                    "Private: not listed. Others join by its address (/lobby join)."
                } else {
                    "Public: listed under Public lobbies for anyone to join."
                }),
            ];
            // (`/players` is refused in a race; the view is only offered out of a lobby.)
            if !race_on.is_on() {
                items.push(
                    Entry::new(
                        Item::SplitScreen,
                        SPLIT_SCREEN,
                        Kind::Cycle(split_screen_value(players)),
                    )
                    .inline()
                    .starts_section(),
                );
            }
            items.push(entry(Item::Back, "Back"));
            return ("Multiplayer".into(), items);
        }
        View::Browser => return ("Public lobbies".into(), browser_items(public)),
        View::NameLobby => {
            let name = Kind::Field {
                text: menu.lobby_name.clone(),
                editing: false,
            };
            let items = vec![
                Entry::new(Item::LobbyName, "Name", name),
                entry(Item::CreateLobby, "Create"),
                entry(Item::Back, "Back"),
            ];
            return ("Public lobby".into(), typed_into(items, menu));
        }
        View::Options => return ("Options".into(), options_items()),
        View::Controls => return ("Controls".into(), controls_items(config, menu.capturing)),
        View::About => return ("About".into(), vec![entry(Item::Back, "Back")]),
        View::CupWizard => {
            let items = cup_wizard::items(&menu.cup, lobby, race_on, config);
            return ("Cup wizard".into(), typed_into(items, menu));
        }
        View::Weapons => return ("Weapons".into(), cup_wizard::weapon_items(config)),
        View::Bots => return ("Bots".into(), bots_menu::items(lobby)),
        View::BotPool => {
            return (
                "Character pool".into(),
                bots_menu::pool_items(menu.bot_pool),
            );
        }
        View::Tracks => return ("Track select".into(), track_items()),
        View::ConfirmQuit => {
            let items = vec![entry(Item::QuitNow, "Quit"), entry(Item::Back, "Cancel")];
            return ("Quit the game?".into(), items);
        }
        View::Main => {}
    }
    let mut items = vec![entry(Item::Resume, "Resume")];
    if !race_on.shared() {
        items.push(entry(Item::Restart, "Restart"));
    }
    // (Not a lobby's: the host ends it for everyone, which is `/race stop`'s to do.)
    if race_on.is_on() && !race_on.shared() {
        items.push(entry(Item::EndRace, "End race").help("Back to the free drive."));
    }
    items.push(entry(Item::TrackSelect, "Track select"));
    // (Same lock as the racer menu's own key: the racers are fixed in a race or a cup.)
    if !race_on.is_on() && !lobby.characters_locked() {
        let keys = config.controls.describe(Control::RacerMenu);
        items.push(
            entry(Item::CharSelect, "Character select")
                .help(format!("Shortcut: {keys} (pad: Select)")),
        );
    }
    // (A race on refuses `/race relic|time-trial`: the buttons would only toast why.)
    if !lobby.is_open() && !race_on.is_on() {
        items.push(entry(Item::PlaySolo, "Play solo"));
    }
    // (Bots only race in a lobby's series: offline there's nothing for them to fill.)
    if lobby.is_host() {
        items.push(entry(Item::CupWizard, "Cup wizard"));
        items.push(entry(Item::BotsMenu, "Bots"));
    }
    if lobby.is_open() {
        items.push(entry(Item::LeaveLobby, "Leave lobby"));
    } else {
        items.push(entry(Item::Multiplayer, "Multiplayer"));
    }
    items.push(entry(Item::Options, "Options"));
    items.push(entry(Item::About, "About"));
    // A page cannot close its tab: in a browser quitting would only stop the game, on its last
    // frame.
    if cfg!(not(target_arch = "wasm32")) {
        items.push(entry(Item::Quit, "Quit"));
    }
    // A lobby's race goes on.
    let title = if !race_on.shared() { "Paused" } else { "Menu" };
    (title.into(), items)
}

/// Item builders don't know about typing; the live text is overlaid here so they stay pure.
fn typed_into(mut items: Vec<Entry>, menu: &PauseMenu) -> Vec<Entry> {
    if let Some((typing, field)) = &menu.typing
        && let Some(entry) = items.iter_mut().find(|e| e.item == *typing)
        && let Kind::Field { .. } = entry.kind
    {
        entry.kind = Kind::Field {
            text: field.text.clone(),
            editing: true,
        };
    }
    items
}

fn track_items() -> Vec<Entry> {
    let mut items: Vec<Entry> = maps::cycle()
        .into_iter()
        .map(|id| Entry::new(Item::Track(id), maps::name(id), Kind::TrackPicture(id)).large())
        .collect();
    items.push(entry(Item::Back, "Back"));
    items
}

/// The volumes, as retail has them (`Options_StringIDs_Audio`).
fn options_items() -> Vec<Entry> {
    let volume = |slider, label| Entry::new(Item::Volume(slider), label, Kind::Slider(slider));
    vec![
        volume(Slider::Fx, "FX"),
        volume(Slider::Music, "Music"),
        volume(Slider::Voice, "Voice"),
        entry(Item::Controls, "Controls"),
        entry(Item::Back, "Back"),
    ]
}

/// The driving keys, and "Press a key" at the one `capturing`.
fn controls_items(config: &Config, capturing: Option<Control>) -> Vec<Entry> {
    let mut items: Vec<Entry> = Control::ALL
        .iter()
        .map(|&control| {
            let value = if capturing == Some(control) {
                "Press a key".into()
            } else {
                config.controls.describe(control)
            };
            Entry::new(Item::Control(control), control.label(), Kind::Value(value))
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
        Status::Outdated => {
            items.push(entry(Item::Info, "Please update"));
            items.push(entry(Item::Info, "to see public lobbies"));
        }
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
/// Between the panel and its footer, in texels.
const FOOTER_GAP: usize = 3;
const FOOTER_TEXELS: f32 = 5.0;
/// [`PANEL`] nearly opaque: the race behind washed the text out.
const FOOTER_BACKGROUND: Color = Color::srgba(0.04, 0.04, 0.06, 0.92);
/// White with a little blue.
const FOOTER_TEXT: Color = Color::srgb(0.78, 0.88, 1.0);

/// The most the items take down, in texels of the 2D art (of its 240): taller, they scroll.
const LIST_TEXELS: usize = 180;
/// An item's height: a line of the middle font and its padding.
const ROW_TEXELS: usize = Font::Middle.line_height() + 4;
const ROW_PADDING: usize = 2;
const SECTION_GAP: usize = 10;
/// Between an inline row's label and its value.
const INLINE_GAP: usize = 8;

/// The panel behind the items, the line around it, and its shadow.
pub(crate) const PANEL: Color = Color::srgba(0.04, 0.04, 0.06, 0.75);
pub(crate) const BORDER: Color = Color::srgb(0.5, 0.85, 0.85);
const SHADOW: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);

/// The menu on screen, and what it was built for.
#[derive(Component)]
struct Shown {
    items: Vec<Entry>,
    title: String,
}

/// The items, which scroll when they do not fit.
#[derive(Component)]
struct List;

/// The picked row's [`Entry::help`], under the panel. Only there when some row of the view has
/// one.
#[derive(Component)]
struct Footer;

/// A cell of an item, and the item's place in the menu.
#[derive(Component)]
struct Row(usize);

/// The knob of a volume.
#[derive(Component)]
struct Knob(Slider);

/// The picture of a volume's wedge.
#[derive(Resource)]
struct Wedge(Handle<Image>);

/// Item index, for re-tinting with the row's pick.
#[derive(Component)]
struct Arrow(usize);

/// Item index, for re-tinting the border with the row's pick.
#[derive(Component)]
struct PictureFrame(usize);

/// White so that `ImageNode::color` can tint them like the row's text.
#[derive(Resource)]
struct Arrows {
    left: Handle<Image>,
    right: Handle<Image>,
}

#[derive(SystemParam)]
struct Art<'w> {
    wedge: Res<'w, Wedge>,
    arrows: Res<'w, Arrows>,
    sheets: ResMut<'w, icon_grid::Sheets>,
    textures: Res<'w, crate::Textures>,
    images: ResMut<'w, Assets<Image>>,
    pictures: ResMut<'w, TrackPictures>,
}

const ARROW: usize = 4;

const NEW_LOBBY: &str = "Create lobby";
const SPLIT_SCREEN: &str = "Split screen";
const SERVER_RECORDS: &str = "Server record ghost";
/// Indexed by `Config::server_records`.
const ON_OFF: [&str; 2] = ["Off", "On"];

fn split_screen_value(players: usize) -> String {
    match players {
        1 => "Off".into(),
        n => format!("{n} players"),
    }
}
/// `Item::NewLobby`'s options, by `PauseMenu::private_lobby`.
const LOBBY_PRIVACY: [&str; 2] = ["Public", "Private"];
const ARROW_HEIGHT: usize = 7;
const ARROW_GAP: usize = 4;

const CARET: usize = 2;

/// A track's picture is fitted into this, in texels: the disc's pictures (169x71) at two thirds,
/// three rows to the page. Maps' pictures (captures, minimaps) have other shapes.
const TRACK_PICTURE: UVec2 = UVec2::new(112, 47);
/// The picture's box with its border, and the row's padding; taller than the large font's line.
const TRACK_ROW_TEXELS: usize = TRACK_PICTURE.y as usize + 2 + 2 * ROW_PADDING;

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenu>()
            .init_resource::<PublicLobbies>()
            .init_resource::<icon_grid::Sheets>()
            .add_systems(Startup, (make_wedge, make_arrows))
            .add_systems(Update, public_lobbies::poll.run_if(crate::drive::driving))
            .add_systems(
                PreUpdate,
                input
                    .after(InputSystems)
                    .after(crate::command::capture)
                    // (Its keys reset must come before `scene::switch_level`, in `LevelChange`,
                    // or `]`/`[` switch the level while typing.)
                    .run_if(crate::drive::driving),
            )
            .add_systems(Update, show.run_if(crate::drive::driving));
    }
}

/// Opens and closes the menu, moves the pick and takes an item, with the menu's sounds. Offline
/// it pauses the race's sounds too (`MainFreeze_IfPressStart`, `MainFrame_TogglePauseAudio`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn input(
    mut menu: ResMut<PauseMenu>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut key_events: MessageReader<KeyboardInput>,
    mut focus_lost: MessageReader<KeyboardFocusLost>,
    mut wheel: MessageReader<MouseWheel>,
    size: Res<TexelSize>,
    mut pads: crate::drive::Pads,
    lobby: Res<Lobby>,
    (race_on, level, players): (Res<RaceOn>, Res<LevelNumber>, Res<crate::drive::LocalPlayers>),
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
    let key_events: Vec<KeyboardInput> = key_events.read().cloned().collect();
    for event in &key_events {
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
    let wheel: Vec<MouseWheel> = wheel.read().cloned().collect();
    let font_size = size.0.y * ABOUT_TEXELS;
    let wheeled: f32 = wheel
        .iter()
        .map(|event| accordion::wheel_lines(event, font_size))
        .sum();
    // The items' list scrolls by rows.
    let row = size.0.y * PageLayout::of(menu.view).row as f32;
    let wheeled_px: f32 = wheel
        .iter()
        .map(|event| accordion::wheel_lines(event, row) * row)
        .sum();
    let view = menu.view;
    let was_open = menu.open;
    let offline = !race_on.shared();
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
    let stick_tap_x = |x: i32| pushed.x == x && menu.stick.x != x;
    let (stick_left, stick_right) = (stick_tap_x(-1), stick_tap_x(1));
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
        return finish(was_open, &menu, &mut keys, &mut pads);
    }
    if let Some((item, mut field)) = menu.typing.take().filter(|_| menu.open) {
        menu.clicked = None;
        let mut typed = key_events
            .iter()
            .map(|event| field.key(event))
            .find(|typed| *typed != Typed::Typing);
        if pad(GamepadButton::North) || pad(GamepadButton::Start) {
            typed = Some(Typed::Cancelled(String::new()));
        }
        let pad_way = (pad(GamepadButton::DPadDown) || stick_down) as i32
            - (pad(GamepadButton::DPadUp) || stick_up) as i32;
        if typed.is_none() && pad_way != 0 {
            typed = Some(Typed::Moved(field.text.clone(), pad_way));
        }
        match typed {
            None | Some(Typed::Typing) => menu.typing = Some((item, field)),
            Some(Typed::Done(text)) => {
                commit(item, text, true, &mut menu, &config, &mut actions);
                sound(MENU_TAKE_SFX);
            }
            Some(Typed::Moved(text, way)) => {
                commit(item, text, false, &mut menu, &config, &mut actions);
                let (_, items) = items(&lobby, &race_on, &menu, &public, &config, players.0);
                navigate(&mut menu, &items, IVec2::new(0, way));
                menu.follow = true;
                sound(MENU_MOVE_SFX);
            }
            Some(Typed::Cancelled(_)) => {
                cancel(item, &mut menu);
                sound(MENU_BACK_SFX);
            }
        }
        return finish(was_open, &menu, &mut keys, &mut pads);
    }
    // In a submenu Escape is Back (below); P and Start still close the menu from anywhere.
    let in_submenu = menu.open && menu.view != View::Main;
    if (key(&[KeyCode::Escape]) && !in_submenu)
        || key(&[KeyCode::KeyP])
        || pad(GamepadButton::Start)
    {
        if menu.open && !leave(&menu, &lobby, &mut actions, &mut toasts) {
            sound(MENU_BACK_SFX);
            return finish(was_open, &menu, &mut keys, &mut pads);
        }
        menu.open = !menu.open;
        menu.view = View::Main;
        menu.selected = 0;
        menu.scroll = 0.0;
        menu.clicked = None;
        menu.capturing = None;
        menu.typing = None;
        menu.grid_cursor = 0;
        test_sound(&menu, &race_on, &config, &time, sounds.as_deref());
        if let Some(sounds) = sounds.as_ref().filter(|_| offline) {
            if menu.open {
                sounds.pause();
            } else {
                sounds.unpause();
            }
        }
        sound(MENU_TAKE_SFX);
        save(&mut menu, &config, &mut toasts);
        return finish(was_open, &menu, &mut keys, &mut pads);
    } else if menu.open {
        let (_, items) = items(&lobby, &race_on, &menu, &public, &config, players.0);
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
            let up =
                key(&[KeyCode::ArrowUp, KeyCode::KeyW]) || pad(GamepadButton::DPadUp) || stick_up;
            let down = key(&[KeyCode::ArrowDown, KeyCode::KeyS])
                || pad(GamepadButton::DPadDown)
                || stick_down;
            let left = key(&[KeyCode::ArrowLeft, KeyCode::KeyA])
                || pad(GamepadButton::DPadLeft)
                || stick_left;
            let right = key(&[KeyCode::ArrowRight, KeyCode::KeyD])
                || pad(GamepadButton::DPadRight)
                || stick_right;
            let way = IVec2::new(right as i32 - left as i32, down as i32 - up as i32);
            if way != IVec2::ZERO && navigate(&mut menu, &items, way) {
                menu.follow = true;
                sound(MENU_MOVE_SFX);
            }
            let cycle = matches!(
                items.get(menu.selected),
                Some(Entry {
                    kind: Kind::Cycle(_),
                    ..
                })
            );
            if way.x != 0 && cycle && menu.stepped.is_none() {
                menu.stepped = Some((menu.selected, way.x));
            }
            // (`show` keeps it within the list.)
            menu.scroll += wheeled_px;
        }
        if let Some((i, way)) = menu.stepped.take()
            && let Some(entry) = items.get(i)
            && step(
                entry.item,
                way,
                &mut menu,
                &mut config,
                (&lobby, players.0),
                &mut actions,
            )
        {
            sound(MENU_MOVE_SFX);
        }
        if (key(&[KeyCode::Delete]) || pad(GamepadButton::West))
            && let Some(entry) = items.get(menu.selected)
            && remove(entry.item, &mut menu, &config)
        {
            sound(MENU_BACK_SFX);
        }
        if let Some((grid, icon)) = menu.icon_taken.take()
            && let Some(entry) = items.get(grid)
            && take_icon(entry.item, icon, &mut menu, &mut config)
        {
            sound(MENU_TAKE_SFX);
        }
        // (On the about page taking is for the list, and Back is clicked.)
        let taken = if enter && menu.view != View::About {
            Some(menu.selected)
        } else {
            menu.clicked.take()
        };
        let taken_entry = taken.and_then(|i| items.get(i));
        let taken_item = match taken_entry {
            // The one cycle that taking doesn't step: it's the action, the cycle its option.
            Some(Entry {
                item: Item::NewLobby,
                ..
            }) => {
                sound(MENU_TAKE_SFX);
                Some(Item::NewLobby)
            }
            Some(Entry {
                item,
                kind: Kind::Cycle(_),
                ..
            }) => {
                if step(*item, 1, &mut menu, &mut config, (&lobby, players.0), &mut actions) {
                    sound(MENU_MOVE_SFX);
                }
                None
            }
            Some(Entry {
                item,
                kind: Kind::Field { .. },
                ..
            }) => {
                menu.typing = field(*item, &mut menu, &config).map(|field| (*item, field));
                sound(MENU_TAKE_SFX);
                None
            }
            Some(Entry {
                item,
                kind: Kind::Grid(_),
                ..
            }) => {
                let icon = menu.grid_cursor;
                if take_icon(*item, icon, &mut menu, &mut config) {
                    sound(MENU_TAKE_SFX);
                }
                None
            }
            Some(Entry {
                kind: Kind::Slider(_),
                ..
            }) => None,
            Some(entry) => {
                sound(MENU_TAKE_SFX);
                Some(entry.item)
            }
            None => None,
        };
        if let Some(&Entry {
            kind: Kind::Slider(slider),
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
        if key(&[KeyCode::Backspace, KeyCode::Escape]) || pad(GamepadButton::North) {
            sound(MENU_BACK_SFX);
            if menu.view != View::Main {
                if leave(&menu, &lobby, &mut actions, &mut toasts) {
                    menu.view = menu.view.parent();
                    menu.selected = 0;
                }
            } else {
                menu.open = false;
            }
        }
        match taken_item {
            Some(Item::Resume) => menu.open = false,
            // A race alone starts over as a race, with its lights (the lobby's); a free drive
            // only puts the kart back on the grid (`drive_tick`).
            Some(Item::Restart) if race_on.is_on() => {
                menu.open = false;
                actions.write(Action::RestartRace);
            }
            Some(Item::Restart) => {
                menu.open = false;
                menu.restart = true;
            }
            Some(Item::CharSelect) => {
                menu.open = false;
                menu.racer_menu = true;
            }
            Some(Item::EndRace) => {
                menu.open = false;
                actions.write(Action::StopRace);
            }
            Some(Item::Solo(kind)) => {
                menu.open = false;
                actions.write(Action::StartSolo(kind));
            }
            Some(Item::ServerRecords) => toggle_server_records(&mut menu, &mut config),
            // Closed, as the menu hides the table.
            Some(Item::Leaderboard(kind)) => {
                menu.open = false;
                actions.write(Action::ScoreTop { kind, track: None });
            }
            Some(Item::LeaveLobby) => {
                menu.open = false;
                actions.write(Action::CloseLobby);
            }
            Some(Item::PlaySolo) => {
                menu.view = View::PlaySolo;
                menu.selected = 0;
            }
            Some(Item::Multiplayer) => {
                menu.view = View::Multiplayer;
                menu.selected = 0;
            }
            Some(Item::NewLobby) if menu.private_lobby => {
                menu.open = false;
                actions.write(Action::CreateLobby { public: None });
            }
            Some(Item::NewLobby) => {
                // (A browser's std hasher has fixed keys: the time is what differs there.)
                let seed = RandomState::new().hash_one(time.elapsed().as_nanos());
                menu.lobby_name = suggest::lobby_name_at(seed % suggest::LOBBY_NAME_COUNT);
                menu.view = View::NameLobby;
                menu.selected = 0;
                menu.typing =
                    field(Item::LobbyName, &mut menu, &config).map(|f| (Item::LobbyName, f));
            }
            Some(Item::CreateLobby) => create_public(&mut menu, &mut actions),
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
                if leave(&menu, &lobby, &mut actions, &mut toasts) {
                    menu.view = menu.view.parent();
                    menu.selected = 0;
                }
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
            Some(Item::CupWizard) => {
                menu.view = View::CupWizard;
                menu.selected = 0;
            }
            Some(Item::Cup(item)) => {
                let menu = &mut *menu;
                let go = cup_wizard::take(
                    item,
                    cup_wizard::Take {
                        slot: &mut menu.cup,
                        config: &mut config,
                        actions: &mut actions,
                        now: time.elapsed_secs(),
                        changed: &mut menu.unsaved,
                    },
                );
                match go {
                    Go::Stay => {}
                    Go::To(view) => {
                        menu.view = view;
                        menu.selected = 0;
                    }
                    Go::Close => menu.open = false,
                }
            }
            Some(Item::BotsMenu) => {
                menu.view = View::Bots;
                menu.selected = 0;
            }
            Some(Item::Bots(item)) => {
                if let Some(view) = bots_menu::take(item, lobby.bots, &mut menu.bot_pool) {
                    menu.view = view;
                    menu.selected = 0;
                }
            }
            Some(Item::TrackSelect) => {
                menu.view = View::Tracks;
                menu.selected = track_items()
                    .iter()
                    .position(|e| e.item == Item::Track(level.0))
                    .unwrap_or(0);
                menu.follow = true;
            }
            Some(Item::Track(id)) => {
                menu.open = false;
                actions.write(Action::Track(id));
            }
            Some(Item::Info | Item::Volume(_) | Item::LobbyName | Item::SplitScreen)
            | None => {}
            Some(Item::Quit) => {
                menu.view = View::ConfirmQuit;
                // (On Cancel: taking Quit twice by habit must not quit.)
                menu.selected = 1;
            }
            Some(Item::QuitNow) => {
                exit.write(AppExit::Success);
            }
        }
        // Another view starts at its top.
        if menu.view != view {
            menu.scroll = 0.0;
            menu.grid_cursor = 0;
        }
        test_sound(&menu, &race_on, &config, &time, sounds.as_deref());
        // Leaving the pause cuts the sound of what left it (`MainFrame_TogglePauseAudio`).
        if !menu.open
            && let Some(sounds) = sounds.as_ref().filter(|_| offline)
        {
            sounds.unpause();
            // A leaderboard goes up over the race: taking it is not a way out of the pause.
            if matches!(taken_item, Some(Item::Leaderboard(_))) {
                sound(MENU_TAKE_SFX);
            }
        }
    }
    save(&mut menu, &config, &mut toasts);
    finish(was_open, &menu, &mut keys, &mut pads);
}

/// `way` is screen order (+y down). A grid gets first claim on the move; only `Steered::Off`
/// moves the selection out of it. Across moves on non-grid rows return false: cycles step
/// separately, via `stepped`.
fn navigate(menu: &mut PauseMenu, items: &[Entry], way: IVec2) -> bool {
    let grid = |entry: Option<&Entry>| match entry {
        Some(Entry {
            kind: Kind::Grid(icons),
            ..
        }) => Some((icons.cells.len(), icons.columns)),
        _ => None,
    };
    if let Some((len, columns)) = grid(items.get(menu.selected)) {
        match icon_grid::steer(&mut menu.grid_cursor, len, columns, way) {
            Steered::Moved => return true,
            Steered::Stayed => return false,
            Steered::Off(_) => {}
        }
    }
    if way.y == 0 || items.is_empty() {
        return false;
    }
    let count = items.len() as i32;
    menu.selected = (menu.selected as i32 + way.y.signum()).rem_euclid(count) as usize;
    if let Some((len, columns)) = grid(items.get(menu.selected)) {
        menu.grid_cursor = icon_grid::entered(len, columns, way.y.signum());
    }
    true
}

/// Returns whether anything changed, for the sound. `actions`: for rows whose state isn't the
/// menu's or the config's (the bots, in `Lobby`), which change once the action has run.
fn step(
    item: Item,
    way: i32,
    menu: &mut PauseMenu,
    config: &mut Config,
    (lobby, players): (&Lobby, usize),
    actions: &mut MessageWriter<Action>,
) -> bool {
    match item {
        Item::SplitScreen => {
            let n = (players as i32 - 1 + way).rem_euclid(crate::drive::MAX_PLAYERS as i32) + 1;
            actions.write(Action::Players(Some(n as u8)));
            true
        }
        Item::Cup(item) => cup_wizard::step(item, way, &mut menu.cup, config, &mut menu.unsaved),
        Item::Bots(item) => write(actions, bots_menu::step(item, way, lobby.bots)),
        Item::NewLobby => {
            menu.private_lobby ^= true;
            true
        }
        Item::ServerRecords => {
            toggle_server_records(menu, config);
            true
        }
        _ => false,
    }
}

fn toggle_server_records(menu: &mut PauseMenu, config: &mut Config) {
    config.server_records ^= true;
    menu.unsaved = true;
}

/// Whether the view may be left (Back, Backspace, or closing the menu), with what leaving it
/// sends. Every way out of a view must ask: a view's draft is only sent from here.
fn leave(
    menu: &PauseMenu,
    lobby: &Lobby,
    actions: &mut MessageWriter<Action>,
    toasts: &mut MessageWriter<Toast>,
) -> bool {
    let left = match menu.view {
        View::BotPool => bots_menu::leave_pool(menu.bot_pool, lobby.bots),
        _ => Ok(None),
    };
    match left {
        Ok(action) => {
            write(actions, action);
            true
        }
        Err(why) => {
            toasts.write(Toast::new(why, 2000).color(Color::srgb(1.0, 0.4, 0.4)));
            false
        }
    }
}

fn write(actions: &mut MessageWriter<Action>, action: Option<Action>) -> bool {
    action.map(|action| actions.write(action)).is_some()
}

fn remove(item: Item, menu: &mut PauseMenu, config: &Config) -> bool {
    match item {
        Item::Cup(item) => cup_wizard::remove(item, &mut menu.cup, config),
        _ => false,
    }
}

fn field(item: Item, menu: &mut PauseMenu, config: &Config) -> Option<TextField> {
    match item {
        Item::Cup(item) => cup_wizard::field(item, &mut menu.cup, config),
        Item::LobbyName => Some(TextField::new(
            menu.lobby_name.clone(),
            net::MAX_LOBBY_NAME,
            net::is_lobby_name_char,
        )),
        _ => None,
    }
}

/// `submit`: Enter, rather than moving off the field.
fn commit(
    item: Item,
    text: String,
    submit: bool,
    menu: &mut PauseMenu,
    config: &Config,
    actions: &mut MessageWriter<Action>,
) {
    match item {
        Item::Cup(item) => cup_wizard::commit(item, text, &mut menu.cup, config),
        Item::LobbyName => {
            menu.lobby_name = text;
            if submit {
                create_public(menu, actions);
            }
        }
        _ => {}
    }
}

/// Escape on a field. The field is the name dialog's whole point, so there it backs out of the
/// dialog; elsewhere it only gives the text back (which `commit` never saw).
fn cancel(item: Item, menu: &mut PauseMenu) {
    if item == Item::LobbyName {
        menu.view = menu.view.parent();
        menu.selected = 0;
        menu.scroll = 0.0;
        menu.grid_cursor = 0;
    }
}

/// Can't fail: the field only admits `net::is_lobby_name_char`s up to `MAX_LOBBY_NAME`, which is
/// all `net::lobby_name` checks; it still collapses the spaces. An empty name is the realm's
/// default (`Lobby::host`).
fn create_public(menu: &mut PauseMenu, actions: &mut MessageWriter<Action>) {
    let Ok(name) = net::lobby_name(&menu.lobby_name) else {
        return;
    };
    menu.open = false;
    actions.write(Action::CreateLobby { public: Some(name) });
}

/// Returns whether anything changed, for the sound; marks the config unsaved itself.
fn take_icon(item: Item, icon: usize, menu: &mut PauseMenu, config: &mut Config) -> bool {
    match item {
        Item::Cup(item) => {
            let changed = cup_wizard::take_icon(item, icon, config);
            menu.unsaved |= changed;
            changed
        }
        Item::Bots(item) => bots_menu::take_icon(item, icon, &mut menu.bot_pool),
        _ => false,
    }
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
    race_on: &RaceOn,
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
            Item::Volume(Slider::Music) if race_on.shared() => None,
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

/// The keys and buttons that opened, worked or closed the menu are not for anything else.
fn finish(
    was_open: bool,
    menu: &PauseMenu,
    keys: &mut ButtonInput<KeyCode>,
    pads: &mut crate::drive::Pads,
) {
    if was_open || menu.open {
        keys.reset_all();
        pads.consume();
    }
}

/// Builds the menu when it opens (and again when what it offers or the texel size changes), takes
/// it down when it closes, and shows the pick in white.
#[allow(clippy::too_many_arguments)]
fn show(
    mut commands: Commands,
    mut menu: ResMut<PauseMenu>,
    (lobby, race_on, players): (Res<Lobby>, Res<RaceOn>, Res<crate::drive::LocalPlayers>),
    public: Res<PublicLobbies>,
    config: Res<Config>,
    (size, fonts): (Res<TexelSize>, Res<Fonts>),
    mut art: Art,
    shown: Query<(Entity, &Shown)>,
    rows: Query<(&Row, &Children, &ComputedNode, &UiGlobalTransform)>,
    mut list: Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition), With<List>>,
    mut texts: Query<&mut CtrText, Without<icon_grid::GridCaption<PauseMenu>>>,
    (mut knobs, mut footer): (
        Query<(&Knob, &mut Node), Without<Footer>>,
        Query<(&mut Text, &mut Node), (With<Footer>, Without<accordion::AccordionLine<PauseMenu>>)>,
    ),
    (mut arrows, mut picture_frames): (
        Query<(&Arrow, &mut ImageNode)>,
        Query<(&PictureFrame, &mut BorderColor), Without<icon_grid::GridIcon<PauseMenu>>>,
    ),
    mut about_lines: accordion::Lines<PauseMenu>,
    mut frames: icon_grid::Frames<PauseMenu>,
    mut captions: icon_grid::Captions<PauseMenu>,
) {
    let wanted = menu.open.then(|| {
        let (title, items) = items(&lobby, &race_on, &menu, &public, &config, players.0);
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
            let scroll = menu.scroll;
            let mut layout = PageLayout::of(menu.view);
            // As wide as its widest option, so the centred panel holds still while it toggles.
            if menu.view == View::Multiplayer {
                let privacy = LOBBY_PRIVACY
                    .iter()
                    .map(|value| inline_cycle_width(&fonts, Font::Middle, NEW_LOBBY, value));
                let players = (1..=crate::drive::MAX_PLAYERS).map(|n| {
                    let value = split_screen_value(n);
                    inline_cycle_width(&fonts, Font::Middle, SPLIT_SCREEN, &value)
                });
                layout.min_width = privacy.chain(players).max().unwrap_or(0);
            }
            if menu.view == View::PlaySolo {
                let widest = ON_OFF
                    .iter()
                    .map(|value| inline_cycle_width(&fonts, Font::Middle, SERVER_RECORDS, value));
                layout.min_width = widest.chain([layout.min_width]).max().unwrap_or(0);
            }
            spawn(
                &mut commands,
                wanted,
                menu.selected,
                scroll,
                about,
                layout,
                &size,
                &mut art,
            );
        }
        return;
    }
    // Once laid out: scrolled to show the pick if it was moved to, and no further than the end.
    if let Ok((node, at, mut scroll)) = list.single_mut()
        && node.size.y > 0.0
    {
        let logical = node.inverse_scale_factor;
        let shown = node.size.y * logical;
        if menu.follow {
            let picked = rows.iter().find(|(row, ..)| row.0 == menu.selected);
            if let Some((_, _, row, row_at)) = picked {
                // The row's top and bottom down the whole list, as it was scrolled when laid out.
                let top = (row_at.translation.y - row.size.y / 2.0)
                    - (at.translation.y - node.size.y / 2.0)
                    + node.scroll_position.y;
                let top = top * logical;
                let bottom = top + row.size.y * logical;
                menu.scroll = menu.scroll.max(bottom - shown).min(top);
            }
            menu.follow = false;
        }
        let end = (node.content_size.y * logical - shown).max(0.0);
        menu.scroll = menu.scroll.clamp(0.0, end);
        if scroll.y != menu.scroll {
            scroll.y = menu.scroll;
        }
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
    if let Ok((mut text, mut node)) = footer.single_mut()
        && let Some((_, shown)) = shown.single().ok()
    {
        let help = shown.items.get(menu.selected).and_then(|item| item.help.as_deref());
        let display = if help.is_some() { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        let help = help.unwrap_or_default();
        if text.0 != help {
            text.0 = help.into();
        }
    }
    for (row, children, ..) in &rows {
        let gradient = gradient(row.0 == menu.selected);
        for &child in children {
            if let Ok(mut text) = texts.get_mut(child)
                && text.gradient != gradient
            {
                text.gradient = gradient;
            }
        }
    }
    for (arrow, mut image) in &mut arrows {
        let tint = tint(arrow.0 == menu.selected);
        if image.color != tint {
            image.color = tint;
        }
    }
    for (frame, mut border) in &mut picture_frames {
        let tint = BorderColor::all(tint(frame.0 == menu.selected));
        if *border != tint {
            *border = tint;
        }
    }
    let on_grid = shown.single().ok().and_then(|(_, shown)| {
        let entry = shown.items.get(menu.selected)?;
        matches!(entry.kind, Kind::Grid(_)).then_some((menu.selected, menu.grid_cursor))
    });
    icon_grid::update(
        |grid| on_grid.filter(|&(g, _)| g == grid).map(|(_, icon)| icon),
        &mut frames,
        &mut captions,
    );
}

/// Per-page geometry, in texels.
#[derive(Clone, Copy)]
struct PageLayout {
    /// The cup wizard's rows change width as tracks are stepped through, and the panel is
    /// centred, so without a minimum it jumps around.
    min_width: usize,
    /// Left padding in both columns of paired rows, on top of the panel's own.
    indent: usize,
    /// The list's most height: a whole number of `row`s, so no row shows cut off. Every row is
    /// at least `row` tall (Back too), so scrolling stays on whole rows; the wheel steps by it.
    list: usize,
    row: usize,
}

impl PageLayout {
    fn of(view: View) -> PageLayout {
        match view {
            View::CupWizard | View::Weapons => PageLayout {
                // Fits a typical `cup::MAX_NAME` name without the panel growing (and, centred,
                // shifting) as it is typed. All-W names still overflow it.
                min_width: 280,
                // No paired rows on these pages (all inline or full-width).
                indent: 0,
                list: LIST_TEXELS,
                row: ROW_TEXELS,
            },
            // Room for the footer's rule texts, which would otherwise wrap to a narrow column.
            View::PlaySolo => PageLayout {
                min_width: 200,
                indent: 8,
                list: LIST_TEXELS,
                row: ROW_TEXELS,
            },
            View::Tracks => PageLayout {
                min_width: 0,
                indent: 8,
                list: 3 * TRACK_ROW_TEXELS,
                row: TRACK_ROW_TEXELS,
            },
            _ => PageLayout {
                min_width: 0,
                indent: 8,
                list: LIST_TEXELS,
                row: ROW_TEXELS,
            },
        }
    }
}

fn tint(picked: bool) -> Color {
    let [r, g, b] = gradient(picked).top;
    Color::srgb_u8(r, g, b)
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
#[allow(clippy::too_many_arguments)]
fn spawn(
    commands: &mut Commands,
    shown: Shown,
    selected: usize,
    scroll: f32,
    about: Option<&Accordion>,
    layout: PageLayout,
    size: &TexelSize,
    art: &mut Art,
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
    // Absolute, so the panel is no wider for it (the text wraps to the panel's width instead)
    // and does not change height as the pick moves between rows with and without help.
    if items.iter().any(|item| item.help.is_some()) {
        let help = items.get(selected).and_then(|item| item.help.as_deref());
        commands.spawn((
            ChildOf(panel),
            Footer,
            Text::new(help.unwrap_or_default()),
            crate::font::bold(TextFont {
                font_size: (size.0.y * FOOTER_TEXELS).into(),
                ..default()
            }),
            TextColor(FOOTER_TEXT),
            Node {
                display: if help.is_some() { Display::Flex } else { Display::None },
                position_type: PositionType::Absolute,
                top: Val::Percent(100.0),
                // (Out to the panel's border.)
                left: Val::Px(-size.0.x),
                right: Val::Px(-size.0.x),
                margin: UiRect::top(size.y(FOOTER_GAP)),
                padding: UiRect::axes(size.x(6), size.y(3)),
                border: UiRect::axes(size.x(1), size.y(1)),
                ..default()
            },
            BackgroundColor(FOOTER_BACKGROUND),
            BorderColor::all(BORDER),
            Pickable::IGNORE,
        ));
    }
    // Second column left-aligned (user's choice; centre and right were tried). Centred, typed
    // text shifted left with every char.
    let list = commands
        .spawn((
            ChildOf(panel),
            List,
            Node {
                display: Display::Grid,
                grid_template_columns: RepeatedGridTrack::auto(2),
                align_self: AlignSelf::Stretch,
                max_height: size.y(layout.list),
                min_width: size.x(layout.min_width),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            ScrollPosition(Vec2::new(0.0, scroll)),
        ))
        .id();
    for (i, item) in items.into_iter().enumerate() {
        let picked = i == selected;
        // Padding, not margin: a margin would be a dead strip for pointer picking.
        let top = size.y(ROW_PADDING + if item.section { SECTION_GAP } else { 0 });
        let font = item.font;
        let text = |text: String| CtrText::new(text, font, gradient(picked));
        let value_cell = if item.paired() {
            // (The label's padding is the gap, so that pointing between the two still picks.)
            let label = cell(commands, list, i, size, true, |node| {
                node.padding.left = size.x(layout.indent);
                node.padding.right = size.x(16);
                node.padding.top = top;
                node.min_height = size.y(layout.row);
            });
            commands.spawn((ChildOf(label), text(item.label)));
            cell(commands, list, i, size, true, |node| {
                node.padding.left = size.x(layout.indent);
                node.padding.top = top;
                node.min_height = size.y(layout.row);
            })
        } else {
            // A grid's cell doesn't take: a click in the gaps between icons would take the icon
            // at the keyboard cursor.
            let takes = !matches!(item.kind, Kind::Grid(_));
            let only = cell(commands, list, i, size, takes, |node| {
                node.grid_column = GridPlacement::span(2);
                node.justify_content = JustifyContent::Center;
                node.padding.top = top;
                node.min_height = size.y(layout.row);
            });
            if !item.label.is_empty() {
                let gap = if item.kind == Kind::Button {
                    0
                } else {
                    INLINE_GAP
                };
                commands.spawn((
                    ChildOf(only),
                    text(item.label),
                    Node {
                        margin: UiRect::right(size.x(gap)),
                        ..default()
                    },
                ));
            }
            only
        };
        match item.kind {
            Kind::Value(value) => {
                commands.spawn((ChildOf(value_cell), text(value)));
            }
            Kind::Cycle(value) => {
                spawn_arrow(commands, value_cell, i, -1, picked, size, &art.arrows);
                commands.spawn((
                    ChildOf(value_cell),
                    text(value),
                    Node {
                        margin: UiRect::horizontal(size.x(ARROW_GAP)),
                        ..default()
                    },
                ));
                spawn_arrow(commands, value_cell, i, 1, picked, size, &art.arrows);
            }
            Kind::Field {
                text: typed,
                editing,
            } => {
                commands.spawn((ChildOf(value_cell), text(typed)));
                if editing {
                    commands.spawn((
                        ChildOf(value_cell),
                        Node {
                            width: size.x(CARET),
                            height: size.y(font.line_height()),
                            margin: UiRect::left(size.x(1)),
                            ..default()
                        },
                        BackgroundColor(Color::WHITE),
                        Pickable::IGNORE,
                    ));
                }
            }
            Kind::Slider(slider) => spawn_track(commands, value_cell, slider, size, &art.wedge),
            Kind::Preview(icons) => {
                let sheet = art
                    .sheets
                    .get(icons.sheet, &art.textures.vram, &mut art.images);
                icon_grid::spawn::<PauseMenu>(commands, value_cell, i, &icons, sheet, size, false);
            }
            Kind::Grid(icons) => {
                let sheet = art
                    .sheets
                    .get(icons.sheet, &art.textures.vram, &mut art.images);
                icon_grid::spawn::<PauseMenu>(commands, value_cell, i, &icons, sheet, size, true);
            }
            Kind::TrackPicture(id) => {
                spawn_track_picture(commands, value_cell, i, id, picked, size, art);
            }
            Kind::Button => {}
        }
    }
}

/// An inline [`Kind::Cycle`] row's width in texels with `value` showing, as `spawn` lays it out.
fn inline_cycle_width(fonts: &Fonts, font: Font, label: &str, value: &str) -> usize {
    let width = |text| fonts.atlas(font).layout(text).width;
    width(label) + INLINE_GAP + 2 * (ARROW + ARROW_GAP) + width(value)
}

/// Centred in a [`TRACK_PICTURE`] box whatever its shape, so the column stays aligned; the box
/// stays empty if the track has no picture. The border hugs the picture, outside the box.
#[allow(clippy::too_many_arguments)]
fn spawn_track_picture(
    commands: &mut Commands,
    parent: Entity,
    i: usize,
    id: u32,
    picked: bool,
    size: &TexelSize,
    art: &mut Art,
) {
    let holder = commands
        .spawn((
            ChildOf(parent),
            Node {
                width: size.x(TRACK_PICTURE.x as usize + 2),
                height: size.y(TRACK_PICTURE.y as usize + 2),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .id();
    let Some(picture) = art.pictures.of(id, &mut art.images) else {
        return;
    };
    let box_size = TRACK_PICTURE.as_vec2();
    let fitted = picture.rect.size() * (box_size / picture.rect.size()).min_element();
    let mut image = ImageNode::new(picture.image).with_mode(NodeImageMode::Stretch);
    image.rect = Some(picture.rect);
    // (Bevy's border is inside the node's size and the image draws inside the border.)
    commands.spawn((
        ChildOf(holder),
        PictureFrame(i),
        image,
        Node {
            width: Val::Px((fitted.x + 2.0) * size.0.x),
            height: Val::Px((fitted.y + 2.0) * size.0.y),
            border: UiRect::axes(size.x(1), size.y(1)),
            ..default()
        },
        BorderColor::all(tint(picked)),
        Pickable::IGNORE,
    ));
}

/// `way`: -1 left, 1 right.
fn spawn_arrow(
    commands: &mut Commands,
    parent: Entity,
    i: usize,
    way: i32,
    picked: bool,
    size: &TexelSize,
    arrows: &Arrows,
) {
    let image = if way < 0 { &arrows.left } else { &arrows.right };
    let step = move |menu: &mut PauseMenu| {
        menu.selected = i;
        menu.stepped = Some((i, way));
    };
    commands
        .spawn((
            ChildOf(parent),
            Arrow(i),
            ImageNode::new(image.clone())
                .with_color(tint(picked))
                .with_mode(NodeImageMode::Stretch),
            Node {
                width: size.x(ARROW),
                height: size.y(ARROW_HEIGHT),
                ..default()
            },
        ))
        // Stop propagation: the row cell takes a click on a cycle as a step +1, which would
        // step twice.
        .observe(
            move |mut click: On<PointerClick>, mut menu: ResMut<PauseMenu>| {
                click.propagate(false);
                if click.button == PointerButton::Primary {
                    step(&mut menu);
                }
            },
        )
        .observe(
            move |mut release: On<PointerRelease>, mut menu: ResMut<PauseMenu>| {
                if matches!(release.pointer.id, PointerId::Touch(_)) {
                    release.propagate(false);
                    step(&mut menu);
                }
            },
        );
}

/// A cell of item `i` in `list`, laid out as `style` changes it: pointing at it picks the item,
/// and clicking or tapping it takes it if it `takes`.
fn cell(
    commands: &mut Commands,
    list: Entity,
    i: usize,
    size: &TexelSize,
    takes: bool,
    style: impl FnOnce(&mut Node),
) -> Entity {
    let mut node = Node {
        align_items: AlignItems::Center,
        padding: UiRect::vertical(size.y(ROW_PADDING)),
        ..default()
    };
    style(&mut node);
    commands
        .spawn((ChildOf(list), Row(i), node))
        // Not PointerOver: the menu respawns on every change, centred, so rows land under a
        // still mouse, fire Over, and steal the pick.
        .observe(move |_: On<PointerMove>, mut menu: ResMut<PauseMenu>| {
            if menu.selected != i {
                menu.selected = i;
            }
        })
        .observe(
            move |click: On<PointerClick>, mut menu: ResMut<PauseMenu>| {
                if takes && click.button == PointerButton::Primary {
                    menu.clicked = Some(i);
                }
            },
        )
        // A touch's pointer is new in the frame it presses, when it is over nothing yet: Bevy has
        // no press on the cell, and so no click.
        .observe(
            move |release: On<PointerRelease>, mut menu: ResMut<PauseMenu>| {
                if takes && matches!(release.pointer.id, PointerId::Touch(_)) {
                    menu.clicked = Some(i);
                }
            },
        )
        .id()
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
                if mouse.pressed(MouseButton::Left)
                    || matches!(moved.pointer.id, PointerId::Touch(_))
                {
                    put_knob(&mut config, &mut menu, slider, moved.hit.position);
                }
            },
        )
        // (A touch has no press on it, as on a row.)
        .observe(
            move |release: On<PointerRelease>,
                  mut config: ResMut<Config>,
                  mut menu: ResMut<PauseMenu>| {
                if matches!(release.pointer.id, PointerId::Touch(_)) {
                    put_knob(&mut config, &mut menu, slider, release.hit.position);
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

fn make_arrows(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut arrow = |left: bool| {
        let mut rgba = vec![0; ARROW * ARROW_HEIGHT * 4];
        let middle = ARROW_HEIGHT / 2;
        for y in 0..ARROW_HEIGHT {
            let wide = ARROW - (y.abs_diff(middle) * ARROW).div_ceil(middle + 1);
            for x in 0..wide {
                let x = if left { ARROW - 1 - x } else { x };
                rgba[(y * ARROW + x) * 4..][..4].fill(0xff);
            }
        }
        let mut image = Image::new(
            Extent3d {
                width: ARROW as u32,
                height: ARROW_HEIGHT as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::nearest();
        images.add(image)
    };
    let arrows = Arrows {
        left: arrow(true),
        right: arrow(false),
    };
    commands.insert_resource(arrows);
}
