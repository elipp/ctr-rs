//! The racer menu of each local player: Select on a pad (or the "Racer menu" key, C for the first
//! player and B for the keyboard's second set) opens a grid of portraits (`icon_grid`, Pick mode)
//! in that player's part of the window, with the pick's profile beside it. The portraits shrink
//! to fit the part ([`fit`]): with 2 or 4 players a part is only 120 of the 240 lines tall.
//!
//! While it is open the player's kart gets a neutral pad (`drive_tick`): it coasts, as the first
//! player's does online while the pause menu is open.
//!
//! The first player's pick goes the way `/character` does (the lobby may refuse it, and says so).
//! The others' karts are swapped here, offline only (the change is not in the rollback's
//! snapshots). In a lobby that is not racing a pick goes to the session instead, and the kart
//! changes when the host's roster says so (`sync`): the host may have fixed the racers. The picks
//! are kept ([`Chosen`]), so that a new level or `/players` gives the players the racers they
//! picked.

use bevy::prelude::*;
use race::KartSpec;
use render::font::{Font, Gradient};
use vehicle::stats::{CHARACTER_ENGINES, Stats};

use super::input::Controller;
use super::{Drive, HudCamera};
use crate::command::Action;
use crate::font::{CtrText, Fonts, TexelSize};
use crate::hud_view::HudView;
use crate::icon_grid::{self, HasIconGrid, Icons, Mode, SheetId, Steered};
use crate::lobby::Lobby;
use crate::pause::{BORDER, PANEL, PauseMenu, RacerMenu};
use crate::props::Sim;
use crate::sound::{MENU_BACK_SFX, MENU_MOVE_SFX, MENU_TAKE_SFX, Sounds};
use crate::CHARACTERS;

/// The panel's padding, in texels.
const PADDING: usize = 6;
const COLUMNS: usize = 4;
/// Between the grid and the profile.
const GAP: usize = 8;
/// Largest first; [`fit`] takes the first that fits. The sheet is drawn stretched, so the smaller
/// ones are only blurrier, not cropped.
const PORTRAIT_SIZES: [UVec2; 3] = [UVec2::new(43, 25), UVec2::new(32, 19), UVec2::new(22, 13)];
const BAR: usize = 40;
const BAR_HEIGHT: usize = 4;
/// The fill of the weakest class: an empty bar would read as "none at all".
const BAR_FLOOR: f32 = 0.3;
const BAR_FILL: Color = Color::srgb(1.0, 0.6, 0.1);
const BAR_TRACK: Color = Color::srgba(0.0, 0.0, 0.0, 0.6);

/// What one local player's menu keeps.
#[derive(Default)]
pub(crate) struct Chooser {
    pub open: bool,
    /// The racer under the cursor; a grid index too, as the grid's cells are the racers in order.
    pick: usize,
    /// The racer driven when it opened, framed as Pick mode's pick.
    current: usize,
    /// Which way the left stick was pushed the last frame, to tap with it.
    stick: IVec2,
}

impl Chooser {
    fn open_on(&mut self, current: usize) {
        self.open = true;
        self.pick = current;
        self.current = current;
    }

    /// `way` in screen order (+y down). Off the top or the bottom it wraps to the other end of the
    /// column: the grid is all there is in this menu.
    fn step(&mut self, way: IVec2) -> bool {
        let len = CHARACTERS.len();
        match icon_grid::steer(&mut self.pick, len, COLUMNS, way) {
            Steered::Moved => true,
            Steered::Stayed => false,
            Steered::Off(from) => {
                let column = self.pick % COLUMNS;
                let top = icon_grid::entered(len, COLUMNS, from);
                self.pick = (top + column).min(len - 1);
                true
            }
        }
    }
}

/// Pointer picks, from the grids' observers (which can't reach `Drive`'s views); `input` takes
/// them. A grid's id is its view.
#[derive(Resource, Default)]
pub(super) struct ChooserPointer {
    pointed: Option<(usize, usize)>,
    taken: Option<(usize, usize)>,
}

impl HasIconGrid for ChooserPointer {
    fn point_icon(&mut self, grid: usize, icon: usize) {
        self.pointed = Some((grid, icon));
    }

    fn take_icon(&mut self, grid: usize, icon: usize) {
        self.taken = Some((grid, icon));
    }
}

/// The racers the local players picked, by view (the first is picked by `/character`, and is
/// [`Drive::character`]). A player that never picked has the one that follows the first's.
#[derive(Resource, Default)]
pub struct Chosen(Vec<Option<usize>>, Vec<Option<usize>>);

impl Chosen {
    fn set(&mut self, view: usize, character: usize) {
        if self.0.len() <= view {
            self.0.resize(view + 1, None);
        }
        self.0[view] = Some(character);
    }

    /// Picked in a lobby and sent, but not yet the host's word (it is in the roster once it is).
    fn want(&mut self, view: usize, character: usize) {
        if self.1.len() <= view {
            self.1.resize(view + 1, None);
        }
        self.1[view] = Some(character);
    }

    fn wanted(&self, view: usize) -> Option<usize> {
        self.1.get(view).copied().flatten()
    }

    /// The racer of local player `view`, when the first drives `first`.
    pub fn character(&self, view: usize, first: usize) -> usize {
        self.0
            .get(view)
            .copied()
            .flatten()
            .unwrap_or((first + view) % CHARACTERS.len())
    }
}

/// Opens and closes the menus, moves the picks, and takes them.
#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    controller: Controller,
    mut drive: ResMut<Drive>,
    mut sim: ResMut<Sim>,
    mut chosen: ResMut<Chosen>,
    lobby: Res<Lobby>,
    race_on: Res<crate::race_on::RaceOn>,
    mut pause: ResMut<PauseMenu>,
    sounds: Option<Res<Sounds>>,
    mut pointer: ResMut<ChooserPointer>,
    mut kits: ResMut<crate::Kits>,
    mut actions: MessageWriter<Action>,
) {
    // The pause menu has the controls (and the pads' buttons are its own then). Opened over
    // its own racer menu (Escape), it takes it back.
    if pause.open && pause.racer_menu == RacerMenu::Nested {
        pause.racer_menu = RacerMenu::No;
        if let Some(view) = drive.views.first_mut() {
            view.chooser.open = false;
        }
    }
    if pause.open || pause.takes_keys() {
        *pointer = ChooserPointer::default();
        return;
    }
    let sound = |id| {
        if let Some(sounds) = &sounds {
            sounds.menu(id);
        }
    };
    let locked = race_on.is_on() || lobby.characters_locked();
    let taken: Vec<Entity> = drive.views.iter().filter_map(|v| v.device.pad()).collect();
    let pointed = pointer.pointed.take();
    let pointer_taken = pointer.taken.take();
    // (Taken even when it is open already: left set, it would open it again once closed.)
    // Closed under it some other way (the views rebuilt), it would hold the pause for good.
    if pause.racer_menu == RacerMenu::Nested && !drive.views.first().is_some_and(|v| v.chooser.open) {
        pause.racer_menu = RacerMenu::Done;
    }
    // (Done unless it opens, below: the pause must still be ended.)
    let asked = pause.racer_menu == RacerMenu::Asked;
    if asked {
        pause.racer_menu = RacerMenu::Done;
    }
    let mut picked = Vec::new();
    for view in 0..drive.views.len() {
        let device = drive.views[view].device.clone();
        let mut stick = drive.views[view].chooser.stick;
        let taps = controller.menu(&device, &taken, &mut stick);
        let current = if view == 0 {
            drive.character
        } else {
            drive.views[view].spec.map_or(0, |spec| spec.character)
        };
        let chooser = &mut drive.views[view].chooser;
        chooser.stick = stick;
        let nested = view == 0 && (asked || pause.racer_menu == RacerMenu::Nested);
        if !chooser.open {
            // (No menu in a race or a cup: the racers are fixed then.)
            if !(taps.toggle || nested) || locked {
                continue;
            }
            chooser.open_on(current);
            if nested {
                pause.racer_menu = RacerMenu::Nested;
            }
            sound(MENU_TAKE_SFX);
            continue;
        }
        if view == 0 && asked {
            pause.racer_menu = RacerMenu::Nested;
        }
        let way = IVec2::new(
            taps.right as i32 - taps.left as i32,
            taps.down as i32 - taps.up as i32,
        );
        let mut moved = way != IVec2::ZERO && chooser.step(way);
        if let Some((_, icon)) = pointed.filter(|&(v, icon)| v == view && icon != chooser.pick) {
            chooser.pick = icon;
            moved = true;
        }
        if moved {
            sound(MENU_MOVE_SFX);
        }
        let clicked = pointer_taken.filter(|&(v, _)| v == view);
        if let Some((_, icon)) = clicked {
            chooser.pick = icon;
        }
        if taps.take || clicked.is_some() {
            chooser.open = false;
            picked.push((view, chooser.pick));
            if nested {
                // (The menu sounds it as the pause ends.)
                pause.racer_menu = RacerMenu::Done;
            } else {
                sound(MENU_TAKE_SFX);
            }
        } else if taps.back || taps.toggle {
            chooser.open = false;
            if nested {
                // (The menu sounds it as it reopens.)
                pause.racer_menu = RacerMenu::Back;
            } else {
                sound(MENU_BACK_SFX);
            }
        }
    }
    for (view, character) in picked {
        if view == 0 {
            if character != drive.character {
                actions.write(Action::Character(character as u32));
            }
            continue;
        }
        if lobby.is_open() {
            // The kart changes when the host says so (`sync`): it may have fixed the racers.
            chosen.want(view, character);
            let acked = lobby.own_extras().unwrap_or_default();
            let extras: Vec<u8> = (1..drive.views.len())
                .map(|v| {
                    chosen
                        .wanted(v)
                        .or_else(|| acked.get(v - 1).map(|&c| c as usize))
                        .unwrap_or_else(|| chosen.character(v, drive.character))
                        as u8
                })
                .collect();
            lobby.set_extras(&extras);
        } else {
            chosen.set(view, character);
            swap(view, character, &mut drive, &mut sim, &mut kits);
        }
    }
}

/// Takes the host's word on our extras' racers: in a lobby, their karts follow the roster (our
/// entry of it), not the picks, which the host may have turned down.
pub(super) fn sync(
    mut drive: ResMut<Drive>,
    mut sim: ResMut<Sim>,
    mut chosen: ResMut<Chosen>,
    lobby: Res<Lobby>,
    race_on: Res<crate::race_on::RaceOn>,
    mut kits: ResMut<crate::Kits>,
) {
    if !lobby.is_open() || lobby.characters_locked() {
        // (A pick that never came back is dropped.)
        chosen.1.clear();
        return;
    }
    // (The race has its own karts, from the start.)
    if race_on.is_on() {
        return;
    }
    let Some(acked) = lobby.own_extras() else {
        return;
    };
    for view in 1..drive.views.len() {
        let Some(&character) = acked.get(view - 1) else {
            continue;
        };
        let character = character as usize;
        if chosen.wanted(view) == Some(character) {
            chosen.1[view] = None;
        }
        if drive.views[view].spec.is_some_and(|s| s.character != character) {
            chosen.set(view, character);
            swap(view, character, &mut drive, &mut sim, &mut kits);
        }
    }
}

/// The kart of `view` is `character` from now on.
fn swap(view: usize, character: usize, drive: &mut Drive, sim: &mut Sim, kits: &mut crate::Kits) {
    let slot = drive.views[view].slot;
    if drive.views[view].spec.is_some_and(|s| s.character == character)
        || slot >= sim.race.karts.len()
    {
        return;
    }
    let kit = kits.get(character);
    sim.set_racer(slot, &kit);
    drive.views[view].spec = Some(KartSpec {
        character,
        anim_num_frames: kit.anim_num_frames(),
        bot: false,
        line: None,
    });
}


/// The profile's stats: the engine class's numbers that differ by class and that a player feels.
const STATS: [(&str, fn(&Stats) -> i32); 3] = [
    ("Speed", |s| s.speed_class_stat as i32),
    ("Accel", |s| s.accel_class_stat as i32),
    ("Turn", |s| s.turn_rate as i32),
];

/// `character`'s bars, each from [`BAR_FLOOR`] (the weakest class) to 1 (the strongest).
fn profile(character: usize) -> [f32; 3] {
    let stats = |c: usize| Stats::for_class(CHARACTER_ENGINES[c]);
    let ours = stats(character);
    STATS.map(|(_, stat)| {
        let all = (0..CHARACTER_ENGINES.len()).map(|c| stat(&stats(c)));
        let (low, high) = all.fold((i32::MAX, i32::MIN), |(l, h), v| (l.min(v), h.max(v)));
        let at = if high > low {
            (stat(&ours) - low) as f32 / (high - low) as f32
        } else {
            1.0
        };
        BAR_FLOOR + (1.0 - BAR_FLOOR) * at
    })
}

/// The top node of a player's menu.
#[derive(Component)]
pub(super) struct ChooserUi;

#[derive(Component)]
pub(super) struct ChooserName(usize);

/// The fill of stat `stat` (of [`STATS`]) of view `view`'s profile.
#[derive(Component)]
pub(super) struct ChooserBar {
    view: usize,
    stat: usize,
}

/// How the menu is laid out, in texels.
struct Layout {
    portrait: UVec2,
    profile: usize,
}

/// The largest portraits whose panel fits `area`, or the smallest. `label`: the widest stat
/// label; `name`: the widest racer name.
fn fit(area: Vec2, label: usize, name: usize) -> Layout {
    let profile = name.max(label + 4 + BAR);
    let line = Font::Middle.line_height();
    let profile_height = line + 3 + STATS.len() * (line + 3);
    let layout = |portrait| Layout { portrait, profile };
    for portrait in PORTRAIT_SIZES {
        let grid = icon_grid::extent(CHARACTERS.len(), COLUMNS, portrait, true);
        let wide = 2 * PADDING + 2 + grid.x as usize + GAP + profile;
        let tall = 2 * PADDING + 2 + line + 3 + (grid.y as usize).max(profile_height);
        if wide as f32 <= area.x && tall as f32 <= area.y {
            return layout(portrait);
        }
    }
    layout(PORTRAIT_SIZES[PORTRAIT_SIZES.len() - 1])
}

/// Puts each open menu up (in the player's part of the window, which `hud_view::target` sees to)
/// and takes it down when it closes, and has the cursor, the name and the bars follow the pick.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    mut commands: Commands,
    drive: Res<Drive>,
    size: Res<TexelSize>,
    fonts: Res<Fonts>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut sheets: ResMut<icon_grid::Sheets>,
    textures: Res<crate::Textures>,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<(Entity, &HudCamera)>,
    roots: Query<(Entity, &HudView), With<ChooserUi>>,
    mut names: Query<
        (&ChooserName, &mut CtrText),
        Without<icon_grid::GridCaption<ChooserPointer>>,
    >,
    mut bars: Query<(&ChooserBar, &mut Node)>,
    mut frames: icon_grid::Frames<ChooserPointer>,
    mut captions: icon_grid::Captions<ChooserPointer>,
) {
    let views = drive.views();
    let mut have = Vec::new();
    for (entity, &HudView(view)) in &roots {
        let open = views.get(view).is_some_and(|v| v.chooser.open);
        if !open || size.is_changed() {
            commands.entity(entity).despawn();
        } else {
            have.push(view);
        }
    }
    if !size.is_changed()
        && let Ok(window) = windows.single()
    {
        for (view, local) in views.iter().enumerate() {
            if !local.chooser.open || have.contains(&view) {
                continue;
            }
            // Where the player's part of the window is, from the first frame: with several
            // players the camera is looked up here, not left to `hud_view::target`, which would
            // show the menu in the middle of the window for a frame.
            let camera = if views.len() > 1 {
                match cameras.iter().find(|(_, c)| c.0 == view) {
                    Some((camera, _)) => Some(camera),
                    // (Not up yet: the next frame.)
                    None => continue,
                }
            } else {
                None
            };
            let atlas = fonts.atlas(Font::Middle);
            let widest = |texts: &mut dyn Iterator<Item = &str>| {
                texts.map(|t| atlas.layout(t).width).max().unwrap_or(0)
            };
            let area = super::window_rect(view, views.len(), window).size() / size.0;
            let layout = fit(
                area,
                widest(&mut STATS.iter().map(|(label, _)| *label)),
                widest(&mut CHARACTERS.iter().copied()),
            );
            let sheet = sheets.get(SheetId::Portraits, &textures.vram, &mut images);
            let root = spawn(&mut commands, view, &local.chooser, &size, &layout, sheet);
            if let Some(camera) = camera {
                commands.entity(root).insert(UiTargetCamera(camera));
            }
        }
    }
    let pick = |view: usize| views.get(view).filter(|v| v.chooser.open).map(|v| v.chooser.pick);
    for (name, mut text) in &mut names {
        let Some(pick) = pick(name.0) else {
            continue;
        };
        if text.text != CHARACTERS[pick] {
            text.text = CHARACTERS[pick].to_string();
        }
    }
    for (bar, mut node) in &mut bars {
        let Some(pick) = pick(bar.view) else {
            continue;
        };
        let width = Val::Percent(100.0 * profile(pick)[bar.stat]);
        if node.width != width {
            node.width = width;
        }
    }
    icon_grid::update(pick, &mut frames, &mut captions);
}

/// The menu of player `view`: a panel in the middle of the player's part, with its title, the
/// grid and the profile.
fn spawn(
    commands: &mut Commands,
    view: usize,
    chooser: &Chooser,
    size: &TexelSize,
    layout: &Layout,
    sheet: &icon_grid::Sheet,
) -> Entity {
    // Under the pause menu.
    let root = commands
        .spawn((
            ChooserUi,
            HudView(view),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(i32::MAX - 2),
            Pickable::IGNORE,
        ))
        .id();
    let panel = commands
        .spawn((
            ChildOf(root),
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(size.x(PADDING), size.y(PADDING)),
                border: UiRect::axes(size.x(1), size.y(1)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(BORDER),
        ))
        .id();
    commands.spawn((
        ChildOf(panel),
        CtrText::new(format!("Player {}", view + 1), Font::Middle, Gradient::ORANGE),
        Node {
            margin: UiRect::bottom(size.y(3)),
            ..default()
        },
    ));
    let body = commands
        .spawn((
            ChildOf(panel),
            Node {
                column_gap: size.x(GAP),
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .id();
    let icons = Icons {
        sheet: SheetId::Portraits,
        cells: (0..CHARACTERS.len()).collect(),
        // The profile has the name.
        names: Vec::new(),
        columns: COLUMNS,
        size: layout.portrait,
        mode: Mode::Pick(Some(chooser.current)),
    };
    icon_grid::spawn::<ChooserPointer>(commands, body, view, &icons, sheet, size, true);
    let column = commands
        .spawn((
            ChildOf(body),
            Node {
                flex_direction: FlexDirection::Column,
                // As wide as the widest name, so that it does not jump as the names go by.
                width: size.x(layout.profile),
                row_gap: size.y(3),
                ..default()
            },
        ))
        .id();
    commands.spawn((
        ChildOf(column),
        ChooserName(view),
        CtrText::new(CHARACTERS[chooser.pick], Font::Middle, Gradient::WHITE),
    ));
    let bars = profile(chooser.pick);
    // A grid, not rows with a sized label: `font::lay_out` overwrites a `CtrText`'s width.
    let table = commands
        .spawn((
            ChildOf(column),
            Node {
                display: Display::Grid,
                grid_template_columns: vec![RepeatedGridTrack::auto(2)],
                column_gap: size.x(4),
                row_gap: size.y(3),
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .id();
    for (stat, (label, _)) in STATS.iter().enumerate() {
        commands.spawn((
            ChildOf(table),
            CtrText::new(*label, Font::Middle, Gradient::ORANGE),
        ));
        let track = commands
            .spawn((
                ChildOf(table),
                Node {
                    width: size.x(BAR),
                    height: size.y(BAR_HEIGHT),
                    ..default()
                },
                BackgroundColor(BAR_TRACK),
            ))
            .id();
        commands.spawn((
            ChildOf(track),
            ChooserBar { view, stat },
            Node {
                width: Val::Percent(100.0 * bars[stat]),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(BAR_FILL),
        ));
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use vehicle::stats::EngineClass;

    #[test]
    fn the_cursor_wraps_round_the_columns() {
        let mut chooser = Chooser::default();
        chooser.open_on(1);
        assert!(chooser.step(IVec2::new(0, -1)));
        assert_eq!(chooser.pick, 13);
        assert!(chooser.step(IVec2::new(0, 1)));
        assert_eq!(chooser.pick, 1);
        assert!(chooser.step(IVec2::new(-1, 0)));
        assert!(!chooser.step(IVec2::new(-1, 0)));
        assert_eq!(chooser.pick, 0);
    }

    #[test]
    fn the_portraits_shrink_to_fit_a_part_of_the_window() {
        // 240 lines; 16:9 is 640 texels across, 4:3 is 480 (`hud::PIXEL` is 2:3).
        let (label, name) = (35, 60);
        assert_eq!(fit(Vec2::new(480.0, 240.0), label, name).portrait, PORTRAIT_SIZES[0]);
        assert_eq!(fit(Vec2::new(640.0, 120.0), label, name).portrait, PORTRAIT_SIZES[1]);
        assert_eq!(fit(Vec2::new(240.0, 120.0), label, name).portrait, PORTRAIT_SIZES[2]);
    }

    #[test]
    fn a_class_has_the_fullest_bar_of_its_own_stat() {
        let of = |class| CHARACTER_ENGINES.iter().position(|&c| c == class).unwrap();
        assert_eq!(profile(of(EngineClass::Speed))[0], 1.0);
        assert_eq!(profile(of(EngineClass::Accel))[1], 1.0);
        assert_eq!(profile(of(EngineClass::Turn))[2], 1.0);
        for bar in (0..CHARACTERS.len()).flat_map(profile) {
            assert!((BAR_FLOOR..=1.0).contains(&bar));
        }
    }
}
