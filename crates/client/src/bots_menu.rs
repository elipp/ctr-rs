//! The pause menu's bots page: `/bots` as menu rows, built like `cup_wizard.rs`.
//!
//! Rows are read from `Lobby::bots`, and every edit is an `Action::Bots`, so
//! `lobby::commands::refusal` gates it as it gates the command line. The menu (PreUpdate) reads
//! `Lobby::bots` before the action is applied (Update), so two edits in one frame would build on
//! the same stale settings; input gives at most one per frame.
//!
//! The exception is the pool page: it edits a draft (`PauseMenu::bot_pool`), sent on leaving,
//! because it may pass through empty ("All off", then pick some) and the lobby must never hold an
//! empty pool (a race could start meanwhile). `leave` refuses to go while it is empty.

use vehicle::bot::difficulty_level::{EASY, HARD, MEDIUM};

use crate::command::Action;
use crate::icon_grid::{Icons, Mode, SheetId};
use crate::lobby::{ALL_CHARACTERS, BotKind, BotPick, BotSettings, Lobby, difficulty_name};
use crate::pause::{Entry, Item, Kind, View, entry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BotsItem {
    Kind,
    Difficulty,
    Count,
    Pick,
    Pool,
    PoolGrid,
    AllCharacters,
    NoCharacters,
}

const DIFFICULTIES: [u16; 3] = [EASY as u16, MEDIUM as u16, HARD as u16];
/// Rows of 4: 16 portraits make a square, and 43 texels wide, 8 of them would not fit the 320.
const GRID_COLUMNS: usize = 4;

pub(crate) fn items(lobby: &Lobby) -> Vec<Entry> {
    let bots = lobby.bots;
    let kind = match bots.kind {
        BotKind::Brain => "Brain",
        BotKind::Retail => "Retail",
    };
    let pick = match bots.pick {
        BotPick::Ordered => "Ordered",
        BotPick::Random => "Random",
    };
    let mut difficulty = difficulty_name(bots.difficulty).to_string();
    difficulty[..1].make_ascii_uppercase();
    vec![
        Entry::new(Item::Bots(BotsItem::Kind), "AI", Kind::Cycle(kind.into())),
        Entry::new(Item::Bots(BotsItem::Difficulty), "Difficulty", Kind::Cycle(difficulty)),
        Entry::new(Item::Bots(BotsItem::Count), "Max bots", Kind::Cycle(bots.count.to_string())),
        Entry::new(Item::Bots(BotsItem::Pick), "Pick", Kind::Cycle(pick.into())),
        entry(Item::Bots(BotsItem::Pool), "Character pool"),
        entry(Item::Back, "Back"),
    ]
}

pub(crate) fn pool_items(pool: u16) -> Vec<Entry> {
    let characters = crate::CHARACTERS.len();
    let icons = Icons {
        sheet: SheetId::Portraits,
        cells: (0..characters).collect(),
        names: crate::CHARACTERS.iter().map(|name| name.to_string()).collect(),
        columns: GRID_COLUMNS,
        size: bevy::math::UVec2::new(
            render::portraits::WIDTH as u32,
            render::portraits::HEIGHT as u32,
        ),
        mode: Mode::Mask((0..characters).map(|c| pool & 1 << c == 0).collect()),
    };
    vec![
        Entry::new(Item::Bots(BotsItem::PoolGrid), "", Kind::Grid(icons)),
        entry(Item::Bots(BotsItem::AllCharacters), "All on"),
        entry(Item::Bots(BotsItem::NoCharacters), "All off"),
        entry(Item::Back, "Back"),
    ]
}

fn set(bots: BotSettings, change: impl FnOnce(&mut BotSettings)) -> Option<Action> {
    let mut to = bots;
    change(&mut to);
    (to != bots).then_some(Action::Bots {
        count: (to.count != bots.count).then_some(to.count),
        difficulty: (to.difficulty != bots.difficulty).then_some(to.difficulty),
        kind: (to.kind != bots.kind).then_some(to.kind),
        pool: (to.pool != bots.pool).then_some(to.pool),
        pick: (to.pick != bots.pick).then_some(to.pick),
        quiet: true,
    })
}

/// The count clamps, as the cup's laps do; the two-or-three-way choices wrap.
pub(crate) fn step(item: BotsItem, way: i32, bots: BotSettings) -> Option<Action> {
    set(bots, |to| match item {
        BotsItem::Kind => {
            to.kind = match to.kind {
                BotKind::Brain => BotKind::Retail,
                BotKind::Retail => BotKind::Brain,
            }
        }
        BotsItem::Pick => {
            to.pick = match to.pick {
                BotPick::Ordered => BotPick::Random,
                BotPick::Random => BotPick::Ordered,
            }
        }
        BotsItem::Difficulty => {
            // An odd difficulty (none `/bots` sets) counts as medium, as `difficulty_name` has it.
            let at = DIFFICULTIES.iter().position(|&d| d == to.difficulty).unwrap_or(1);
            let len = DIFFICULTIES.len() as i32;
            to.difficulty = DIFFICULTIES[(at as i32 + way).rem_euclid(len) as usize];
        }
        BotsItem::Count => {
            let most = race::MAX_KARTS as i32 - 1;
            to.count = (to.count as i32 + way).clamp(0, most) as u8;
        }
        _ => {}
    })
}

/// `draft`: `PauseMenu::bot_pool`, started from `bots` on entering the pool page.
pub(crate) fn take(item: BotsItem, bots: BotSettings, draft: &mut u16) -> Option<View> {
    match item {
        BotsItem::Pool => {
            *draft = bots.pool;
            return Some(View::BotPool);
        }
        BotsItem::AllCharacters => *draft = ALL_CHARACTERS,
        BotsItem::NoCharacters => *draft = 0,
        // Handled by kind in pause.rs (Cycle, Grid).
        BotsItem::Kind
        | BotsItem::Difficulty
        | BotsItem::Count
        | BotsItem::Pick
        | BotsItem::PoolGrid => {}
    }
    None
}

pub(crate) fn take_icon(item: BotsItem, icon: usize, draft: &mut u16) -> bool {
    match item {
        BotsItem::PoolGrid if icon < crate::CHARACTERS.len() => {
            *draft ^= 1 << icon;
            true
        }
        _ => false,
    }
}

/// Leaving the pool page: the draft's action, or why it can't be left yet.
pub(crate) fn leave_pool(draft: u16, bots: BotSettings) -> Result<Option<Action>, &'static str> {
    if draft == 0 {
        return Err("Pick at least one character");
    }
    Ok(set(bots, |to| to.pool = draft))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_clamps_and_the_difficulty_wraps() {
        let bots = BotSettings::default();
        assert!(step(BotsItem::Count, -1, bots).is_none());
        let hard = BotSettings {
            difficulty: HARD as u16,
            ..bots
        };
        assert!(matches!(
            step(BotsItem::Difficulty, 1, hard),
            Some(Action::Bots { difficulty: Some(d), count: None, kind: None, pool: None, .. })
                if d == EASY as u16
        ));
    }

    #[test]
    fn the_pool_page_cannot_be_left_empty() {
        let bots = BotSettings::default();
        let mut draft = 0;
        take(BotsItem::Pool, bots, &mut draft);
        assert_eq!(draft, ALL_CHARACTERS);
        take(BotsItem::NoCharacters, bots, &mut draft);
        assert!(leave_pool(draft, bots).is_err());
        assert!(take_icon(BotsItem::PoolGrid, 3, &mut draft));
        assert!(!take_icon(BotsItem::PoolGrid, 16, &mut draft));
        assert!(matches!(
            leave_pool(draft, bots),
            Ok(Some(Action::Bots { pool: Some(8), quiet: true, .. }))
        ));
        assert!(matches!(leave_pool(ALL_CHARACTERS, bots), Ok(None)));
    }
}
