//! The pause menu's cup wizard: `/cup` as menu pages. Holds no UI code: it builds `pause::Entry`
//! lists, and `pause.rs` calls back into `step`/`remove`/`field`/`commit`/`take`/`take_icon` by
//! item. Each `Kind` is handled generically there.
//!
//! Every effect is an [`Action`], so `lobby::commands::refusal` gates the wizard exactly as it
//! gates the command line. Don't pre-check lobby state here beyond hiding items. The exception
//! is the weapons page, which edits `Config` directly: the host reads it at each race's start
//! (`lobby::rules`).

use vehicle::item::ENABLED;

use crate::command::Action;
use crate::config::Config;
use crate::icon_grid::{self, Icons, Mode, SheetId};
use crate::lobby::Lobby;
use crate::lobby::cup::{self, CupDef, MAX_LAPS, MAX_NAME, MAX_TRACKS};
use crate::maps;
use crate::pause::{Entry, Item, Kind, View, entry};
use crate::text_field::TextField;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CupItem {
    Saved,
    Name,
    /// Index into `Draft::tracks`. Stale after a removal until the next rebuild; the
    /// handlers bounds-check.
    Track(usize),
    AddTrack,
    /// Racing tracks only (no custom maps), capped at `RACING_TRACKS` by `random_tracks`.
    RandomTracks,
    Laps,
    Weapons,
    Start,
    Save,
    Delete,
    Pause,
    Resume,
    Next,
    Previous,
    Stop,
    WeaponGrid,
    AllWeapons,
    NoWeapons,
    Odds,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Draft {
    /// Set optimistically on Save, before the `CupCreate` action has run.
    saved: Option<String>,
    name: String,
    tracks: Vec<u32>,
    laps: u8,
}

impl Draft {
    fn new(config: &Config) -> Draft {
        let name = (1..)
            .map(|n| format!("cup{n}"))
            .find(|name| config.cups.get(name).is_none())
            .unwrap_or_default();
        Draft {
            saved: None,
            name,
            tracks: maps::cycle().into_iter().take(1).collect(),
            laps: race::DEFAULT_LAPS,
        }
    }

    /// Unresolvable tracks (a deleted custom map) are dropped silently, unlike `/cup start`,
    /// which refuses the whole cup.
    fn of(def: &CupDef) -> Draft {
        Draft {
            saved: Some(def.name.clone()),
            name: def.name.clone(),
            tracks: def
                .tracks
                .iter()
                .filter_map(|track| maps::resolve(track).ok())
                .collect(),
            laps: def.laps,
        }
    }
}

pub(crate) enum Go {
    Stay,
    To(View),
    Close,
}

/// `items` shows a throwaway `Draft::new` while the slot is empty, and this makes an equal one,
/// so the first edit doesn't visibly change anything else.
fn draft<'a>(draft: &'a mut Option<Draft>, config: &Config) -> &'a mut Draft {
    draft.get_or_insert_with(|| Draft::new(config))
}

/// Editing is hidden while `lobby.series()` runs: the running cup's controls replace it.
pub(crate) fn items(draft: &Option<Draft>, lobby: &Lobby, config: &Config) -> Vec<Entry> {
    let cup = |item| Item::Cup(item);
    let mut items = Vec::new();
    if let Some(series) = lobby.series() {
        let name = series.name.as_deref().unwrap_or("Race");
        items.push(Entry::new(Item::Info, "Racing", Kind::Value(name.into())));
        items.push(Entry::new(
            Item::Info,
            "Track",
            Kind::Value(format!("{} of {}", series.race + 1, series.tracks.len())),
        ));
        if lobby.is_paused() {
            items.push(entry(cup(CupItem::Resume), "Resume"));
        } else {
            items.push(entry(cup(CupItem::Pause), "Pause"));
        }
        if series.scored && series.has_next() {
            items.push(entry(cup(CupItem::Next), "Next track"));
        }
        if series.scored && series.race > 0 {
            items.push(entry(cup(CupItem::Previous), "Previous track"));
        }
        items.push(entry(cup(CupItem::Stop), "Stop"));
        items.push(entry(Item::Back, "Back"));
        return items;
    }
    let fresh;
    let draft = match draft {
        Some(draft) => draft,
        None => {
            fresh = Draft::new(config);
            &fresh
        }
    };
    let saved = draft.saved.clone().unwrap_or_else(|| "New".into());
    items.push(Entry::new(cup(CupItem::Saved), "Cup", Kind::Cycle(saved)).inline());
    items.push(
        Entry::new(
            cup(CupItem::Name),
            "Name",
            Kind::Field {
                text: draft.name.clone(),
                editing: false,
            },
        )
        .inline(),
    );
    for (n, &track) in draft.tracks.iter().enumerate() {
        let row = Entry::new(
            cup(CupItem::Track(n)),
            "",
            Kind::Cycle(format!("{}. {}", n + 1, maps::name(track))),
        );
        items.push(if n == 0 { row.starts_section() } else { row });
    }
    if draft.tracks.len() < MAX_TRACKS {
        items.push(entry(cup(CupItem::AddTrack), "+ Add track"));
    }
    items.push(entry(cup(CupItem::RandomTracks), "Randomize"));
    items.push(
        Entry::new(cup(CupItem::Laps), "", Kind::Cycle(laps(draft.laps))).starts_section(),
    );
    items.push(if SHOW_WEAPON_PREVIEW {
        Entry::new(
            cup(CupItem::Weapons),
            "Weapons",
            Kind::Preview(weapon_icons(config, PREVIEW_SIZE, ENABLED.len())),
        )
    } else {
        entry(cup(CupItem::Weapons), "Weapons")
    });
    items.push(entry(cup(CupItem::Start), "Start"));
    items.push(entry(cup(CupItem::Save), "Save"));
    if draft.saved.is_some() {
        items.push(entry(cup(CupItem::Delete), "Delete"));
    }
    items.push(entry(Item::Back, "Back"));
    items
}

fn laps(n: u8) -> String {
    if n == 1 { "1 lap".into() } else { format!("{n} laps") }
}

const GRID_SIZE: bevy::math::UVec2 = bevy::math::UVec2::new(48, 32);
const PREVIEW_SIZE: bevy::math::UVec2 = bevy::math::UVec2::new(16, 11);
/// Off: the user found the inline icon row of little use next to the sub-menu (2026-11-15), but
/// wants the code kept. Flipping it back on is all it takes.
const SHOW_WEAPON_PREVIEW: bool = false;
const GRID_COLUMNS: usize = 4;

/// Icon order is `ENABLED` order, which `take_icon` relies on. Character 0 makes the mask Aku
/// Aku's.
fn weapon_icons(config: &Config, size: bevy::math::UVec2, columns: usize) -> Icons {
    Icons {
        sheet: SheetId::Weapons,
        cells: ENABLED
            .iter()
            .map(|&weapon| {
                render::weapons::cell(render::weapons::icon_id(weapon, false, 0)).unwrap_or(0)
            })
            .collect(),
        // Everyone knows them: no caption (user's call, 2026-11-15).
        names: Vec::new(),
        columns,
        size,
        mode: Mode::Mask(icon_grid::mask_of(&ENABLED, &config.masked_weapons)),
    }
}

pub(crate) fn weapon_items(config: &Config) -> Vec<Entry> {
    vec![
        Entry::new(
            Item::Cup(CupItem::WeaponGrid),
            "",
            Kind::Grid(weapon_icons(config, GRID_SIZE, GRID_COLUMNS)),
        ),
        entry(Item::Cup(CupItem::AllWeapons), "All on"),
        entry(Item::Cup(CupItem::NoWeapons), "All off"),
        Entry::new(
            Item::Cup(CupItem::Odds),
            "Odds",
            Kind::Cycle(if config.even_weapon_odds { "Even" } else { "Retail" }.into()),
        )
        .inline(),
        entry(Item::Back, "Back"),
    ]
}

/// `changed` is `PauseMenu::unsaved`, for the items that step `config` rather than the draft.
pub(crate) fn step(
    item: CupItem,
    way: i32,
    slot: &mut Option<Draft>,
    config: &mut Config,
    changed: &mut bool,
) -> bool {
    if item == CupItem::Odds {
        config.even_weapon_odds ^= true;
        *changed = true;
        return true;
    }
    let draft = draft(slot, config);
    match item {
        CupItem::Saved => {
            let cups = config.cups.all();
            // Index 0 is "New"; saved cups are 1-based.
            let at = draft
                .saved
                .as_ref()
                .and_then(|name| cups.iter().position(|c| &c.name == name))
                .map_or(0, |n| n + 1);
            let to = (at as i32 + way).rem_euclid(cups.len() as i32 + 1) as usize;
            *draft = match to {
                0 => Draft::new(config),
                n => Draft::of(&cups[n - 1]),
            };
            true
        }
        CupItem::Track(n) => {
            let all = maps::cycle();
            let Some(track) = draft.tracks.get_mut(n).filter(|_| !all.is_empty()) else {
                return false;
            };
            let at = all.iter().position(|t| t == track).unwrap_or(0);
            *track = all[(at as i32 + way).rem_euclid(all.len() as i32) as usize];
            true
        }
        CupItem::Laps => {
            let laps = (draft.laps as i32 + way).clamp(1, MAX_LAPS as i32) as u8;
            std::mem::replace(&mut draft.laps, laps) != laps
        }
        _ => false,
    }
}

/// Never empties `tracks`: Save and Start assume at least one track.
pub(crate) fn remove(item: CupItem, slot: &mut Option<Draft>, config: &Config) -> bool {
    let draft = draft(slot, config);
    match item {
        CupItem::Track(n) if n < draft.tracks.len() && draft.tracks.len() > 1 => {
            draft.tracks.remove(n);
            true
        }
        _ => false,
    }
}

pub(crate) fn field(item: CupItem, slot: &mut Option<Draft>, config: &Config) -> Option<TextField> {
    match item {
        CupItem::Name => Some(TextField::new(
            draft(slot, config).name.clone(),
            MAX_NAME,
            |c| cup::valid_name(&c.to_string()),
        )),
        _ => None,
    }
}

pub(crate) fn commit(item: CupItem, text: String, slot: &mut Option<Draft>, config: &Config) {
    if item == CupItem::Name {
        draft(slot, config).name = text;
    }
}

pub(crate) struct Take<'a, 'w> {
    pub slot: &'a mut Option<Draft>,
    pub config: &'a mut Config,
    pub actions: &'a mut bevy::prelude::MessageWriter<'w, Action>,
    pub now: f32,
    /// `PauseMenu::unsaved`: config edits persist via the menu's own save, not here.
    pub changed: &'a mut bool,
}

pub(crate) fn take(item: CupItem, take: Take) -> Go {
    let Take {
        slot,
        config,
        actions,
        now,
        changed,
    } = take;
    let mut write = |action| {
        actions.write(action);
    };
    match item {
        // Handled by kind in pause.rs (Cycle, Field, Grid).
        CupItem::Saved
        | CupItem::Track(_)
        | CupItem::Laps
        | CupItem::Name
        | CupItem::WeaponGrid
        | CupItem::Odds => {}
        CupItem::AddTrack => {
            let draft = draft(slot, config);
            let all = maps::cycle();
            let next = draft
                .tracks
                .last()
                .and_then(|last| all.iter().position(|t| t == last))
                .map_or(0, |at| (at + 1) % all.len().max(1));
            if let Some(&track) = all.get(next) {
                draft.tracks.push(track);
            }
        }
        CupItem::RandomTracks => {
            let draft = draft(slot, config);
            let count = draft.tracks.len().clamp(1, race::RACING_TRACKS as usize);
            draft.tracks = cup::random_tracks(count, cup::random_seed(now));
        }
        CupItem::Weapons => return Go::To(View::Weapons),
        CupItem::Start => {
            let draft = draft(slot, config);
            let name = if cup::valid_name(&draft.name) {
                draft.name.clone()
            } else {
                "Cup".into()
            };
            write(Action::CupDrive {
                name,
                tracks: draft.tracks.clone(),
                laps: draft.laps,
            });
            return Go::Close;
        }
        CupItem::Save => {
            let fallback = Draft::new(config).name;
            let draft = draft(slot, config);
            if !cup::valid_name(&draft.name) {
                draft.name = fallback;
            }
            draft.saved = Some(draft.name.clone());
            write(Action::CupCreate {
                name: draft.name.clone(),
                tracks: draft.tracks.clone(),
                laps: draft.laps,
            });
        }
        CupItem::Delete => {
            if let Some(name) = draft(slot, config).saved.take() {
                write(Action::CupDelete(name));
            }
        }
        CupItem::Pause => write(Action::CupPause),
        CupItem::Resume => write(Action::CupResume),
        CupItem::Next => {
            write(Action::CupNext);
            return Go::Close;
        }
        CupItem::Previous => {
            write(Action::CupPrevious);
            return Go::Close;
        }
        CupItem::Stop => {
            write(Action::StopRace);
            return Go::Close;
        }
        CupItem::AllWeapons => {
            config.masked_weapons.clear();
            *changed = true;
        }
        CupItem::NoWeapons => {
            config.masked_weapons = ENABLED.to_vec();
            *changed = true;
        }
    }
    Go::Stay
}

pub(crate) fn take_icon(item: CupItem, icon: usize, config: &mut Config) -> bool {
    match (item, ENABLED.get(icon)) {
        (CupItem::WeaponGrid, Some(&weapon)) => {
            icon_grid::toggle(&mut config.masked_weapons, weapon);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_laps_stay_in_range() {
        let mut config = Config::default();
        let mut slot = None;
        for _ in 0..50 {
            step(CupItem::Laps, 1, &mut slot, &mut config, &mut false);
        }
        assert_eq!(slot.as_ref().unwrap().laps, MAX_LAPS);
        for _ in 0..50 {
            step(CupItem::Laps, -1, &mut slot, &mut config, &mut false);
        }
        assert_eq!(slot.as_ref().unwrap().laps, 1);
    }

    #[test]
    fn the_last_track_stays() {
        let config = Config::default();
        let mut slot = None;
        assert!(!remove(CupItem::Track(0), &mut slot, &config));
        slot.as_mut().unwrap().tracks.push(3);
        assert!(remove(CupItem::Track(0), &mut slot, &config));
        assert_eq!(slot.unwrap().tracks, vec![3]);
    }

    #[test]
    fn a_new_cup_has_a_free_name() {
        let mut config = Config::default();
        config.cups.set(CupDef {
            name: "cup1".into(),
            tracks: vec!["dingo_canyon".into()],
            laps: 3,
        });
        assert_eq!(Draft::new(&config).name, "cup2");
    }
}
