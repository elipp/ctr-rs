//! The race on, if any ([`RaceOn`]): what the race is ([`CurrentRace`]) and, for a lobby's race,
//! the netcode under it ([`NetRace`]).
//!
//! Invariants others rely on:
//! - a race is on exactly while [`RaceOn`] holds one; a lobby's start (`lobby::report`) or
//!   `/race start` alone (`Lobby::race_alone`) puts it there, the end of the race takes it away;
//! - [`CurrentRace::net`] is there exactly when the race is a lobby's: everyone's sim is the
//!   same, so no pause, restart or debug change of our own. A race alone has none.

use bevy::prelude::*;
use net::{EndpointId, Pad};

use crate::lobby::cup::{Finisher, Who};

/// The race on, if any. (No `Deref` to the `Option`: `Res` has `as_ref`/`as_mut` of its own,
/// which would shadow the `Option`'s.)
#[derive(Resource, Default)]
pub struct RaceOn(pub Option<CurrentRace>);

impl RaceOn {
    pub fn get(&self) -> Option<&CurrentRace> {
        self.0.as_ref()
    }

    pub fn get_mut(&mut self) -> Option<&mut CurrentRace> {
        self.0.as_mut()
    }

    pub fn is_on(&self) -> bool {
        self.0.is_some()
    }

    /// Whether the race on is a relic race, whose HUD has the time crates and no wumpa fruit.
    pub fn relic(&self) -> bool {
        self.get().is_some_and(|race| race.relic)
    }

    /// Whether the race on is a lobby's (see [`CurrentRace::net`]).
    pub fn shared(&self) -> bool {
        self.net().is_some()
    }

    /// The netcode of the race on, if it is a lobby's.
    pub fn net(&self) -> Option<&NetRace> {
        self.get()?.net.as_ref()
    }
}

/// How far our part of the race has got. Each step is taken by one system, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// The level may not be up yet.
    Starting,
    /// `remote::build` has the karts' specs and has put the others' karts on the grid.
    Spawned,
    /// `drive_tick` has put the sim back to tick 0 with them.
    OnGrid,
    /// We told the others we can start (`loading::tell_ready`).
    Ready,
}

/// A race that has started.
pub struct CurrentRace {
    /// Our place in the roster: our player, or, past the last of them, a spectator's (see
    /// [`CurrentRace::spectating`]).
    pub slot: usize,
    /// The slots that are ours: `slot` first, then our other local players' (split screen), each
    /// on a view of its own. The host's bots are not among them.
    pub locals: Vec<usize>,
    /// The character each slot drives, which sets its kart's stats.
    pub characters: Vec<u8>,
    /// How many of the players are the host's bots, the last of them (see
    /// [`CurrentRace::is_bot`]).
    pub bots: usize,
    /// What each slot is called on screen: in a lobby's race the nickname if there is one,
    /// following the nicknames (`Lobby::refresh_labels`), which can change in a race.
    pub labels: Vec<String>,
    /// The `LevelId` it is on, which the viewer shows before anything is played.
    pub track: u32,
    /// The hash of the contents of the map, if it is a map (0 for the game's tracks).
    pub hash: u64,
    /// The rules of the race (laps, weapons, seed): the host's in a lobby.
    pub rules: net::Rules,
    /// A relic race (`race::Mode::Relic`): only ever alone, so it is not one of the rules the
    /// host sends.
    pub relic: bool,
    /// The relic to beat and its time in ms (`race::relic::target`), from the personal best as it
    /// was when the race went on the grid.
    pub relic_target: Option<(race::relic::Tier, i32)>,
    pub phase: Phase,
    /// Tick 0 has come: the host heard every player ready (alone, we are). Not ordered with
    /// [`CurrentRace::phase`]: a spectator is not waited for, and can hear it before its own race
    /// is built.
    pub go: bool,
    /// The kart of each slot, as the race takes them: set by `remote::build`, which has their
    /// models.
    pub specs: Vec<race::KartSpec>,
    /// The retail bots the host added, in the karts' slots after the players' (`remote::build`
    /// drops them on a track with no paths for them), and their karts. (Not the brain bots: those
    /// are players, see [`CurrentRace::bots`].)
    pub retail: net::RetailBots,
    pub retail_specs: Vec<race::KartSpec>,
    /// How many of the finishers (by place) have been announced.
    pub announced: usize,
    /// The players whose kart has crossed the line (by slot), as the sim has it now: set by
    /// `drive_tick`. The sim takes no more of their pads, so a client that watches does not wait
    /// for them, and `pace` does not keep our clock with theirs.
    pub finished: Vec<bool>,
    /// The kart the camera follows: ours, or another's once we have finished (R1 and L1 change
    /// it). A spectator has none of its own: it is whichever it chose.
    pub watching: usize,
    pub net: Option<NetRace>,
}

/// The netcode of a lobby's race.
pub struct NetRace {
    /// The endpoint of each slot.
    pub ids: Vec<EndpointId>,
    /// The short form of each slot's iroh key (`fmt_short`), which keys the pings.
    pub names: Vec<String>,
    /// The pads of the others that have come in and are not in the sim yet: (slot, first tick,
    /// the pads).
    pub inbox: Vec<(usize, u32, Vec<Pad>)>,
    /// The pads of the others that `/debug lag` holds back before they go in the inbox.
    pub delayed: crate::lobby::lag::Delayed,
    /// How many times a late pad differed from its prediction, and the ticks played again for it.
    pub corrections: u32,
    pub replayed: u32,
    /// The hashes of the sims, ours and the others'.
    pub checks: race::desync::Checks,
    /// How many ticks our clock is ahead of each player's (smoothed; ours is 0), from `net`.
    pub ahead: Vec<f32>,
    /// A watching client's ticks (see [`CurrentRace::watches`]) that have the pad of every
    /// player still racing and are not played yet (smoothed), which `pace` keeps near
    /// `SPECTATE_BUFFER`. It plays only those (see `drive_tick`).
    pub buffered: f32,
    /// The players whose sim differs from ours, with the first tick it was seen at.
    pub desync: Vec<Desync>,
    /// The players that left the race (by slot): they have a neutral pad from their last on, and
    /// the race does not wait for them to finish.
    pub gone: Vec<bool>,
    /// The players that left whose pads we still have to report (`Event::Gone`): `drive_tick`
    /// answers once the pads that came in are taken.
    pub asked: Vec<usize>,
    /// The slots whose pads we stopped taking, having reported what we hold of them.
    pub frozen: Vec<bool>,
    /// Where the pads of the players that left end (`Event::Left`), for `drive_tick` to apply:
    /// (slot, first tick, pads).
    pub lefts: Vec<(usize, u32, Vec<Pad>)>,
    /// The tick the host paused the race at (`/cup pause`): `drive_tick` plays no ticks from it on.
    pub pause_at: Option<u32>,
}

/// A player whose sim has parted from ours.
pub struct Desync {
    pub slot: usize,
    pub tick: u32,
    /// The hashes that differed, if it was they that told.
    pub hashes: Option<race::desync::Hashes>,
    /// Whether the player has been told of it (by a toast).
    pub told: bool,
}

/// What a bot that drives `character` is called, a brain bot or a retail one alike.
pub fn bot_label(character: u8) -> String {
    let name = crate::CHARACTERS.get(character as usize).unwrap_or(&"?");
    format!("{name} (bot)")
}

impl NetRace {
    /// The netcode of a race of `ids` (a slot each), before anything of it is played; `slot` is
    /// ours.
    pub fn new(ids: Vec<EndpointId>, slot: usize) -> NetRace {
        let players = ids.len();
        NetRace {
            names: ids.iter().map(|id| id.fmt_short().to_string()).collect(),
            ids,
            inbox: Vec::new(),
            delayed: default(),
            corrections: 0,
            replayed: 0,
            checks: race::desync::Checks::new(players, slot),
            ahead: vec![0.0; players],
            buffered: 0.0,
            desync: Vec::new(),
            gone: vec![false; players],
            asked: Vec::new(),
            frozen: vec![false; players],
            lefts: Vec::new(),
            pause_at: None,
        }
    }

    /// Notes the players whose hash differs from ours. A sim that has parted stays apart, so
    /// only the first is kept for each player.
    pub fn found(&mut self, mismatches: impl IntoIterator<Item = race::desync::Mismatch>) {
        for m in mismatches {
            if self.desync.iter().all(|d| d.slot != m.slot) {
                self.desync.push(Desync {
                    slot: m.slot,
                    tick: m.tick,
                    hashes: m.hashes,
                    told: false,
                });
            }
        }
    }
}

impl CurrentRace {
    /// A race of `characters` (a slot each, the host's brain bots the last `bots`) on `track`,
    /// before anything of it is played. `net`'s ids, if a lobby's, are a slot each too.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        slot: usize,
        locals: Vec<usize>,
        characters: Vec<u8>,
        bots: usize,
        track: u32,
        hash: u64,
        rules: net::Rules,
        retail: net::RetailBots,
        net: Option<NetRace>,
    ) -> CurrentRace {
        let players = characters.len();
        CurrentRace {
            slot,
            locals,
            characters,
            bots,
            // (A lobby's are named after the nicknames once it has told them,
            // `Lobby::refresh_labels`.)
            labels: net.as_ref().map_or_else(Vec::new, |net| net.names.clone()),
            track,
            hash,
            rules,
            relic: false,
            relic_target: None,
            phase: Phase::Starting,
            go: false,
            specs: Vec::new(),
            retail,
            retail_specs: Vec::new(),
            announced: 0,
            finished: vec![false; players],
            watching: if slot < players { slot } else { 0 },
            net,
        }
    }

    /// A race of our local players alone, `characters` (the first view's first).
    pub fn alone(
        characters: Vec<u8>,
        nick: &str,
        track: u32,
        rules: net::Rules,
        relic: bool,
    ) -> CurrentRace {
        let locals = (0..characters.len()).collect();
        let hash = crate::maps::hash(track).unwrap_or(0);
        let mut race = CurrentRace::new(
            0,
            locals,
            characters,
            0,
            track,
            hash,
            rules,
            net::RetailBots::default(),
            None,
        );
        race.relic = relic;
        race.labels = (1..=race.characters.len())
            .map(|n| match (nick.is_empty(), n) {
                (true, n) => format!("Player {n}"),
                (false, 1) => nick.to_string(),
                (false, n) => format!("{nick}@{n}"),
            })
            .collect();
        race
    }

    /// What the sim plays.
    pub fn mode(&self) -> race::Mode {
        let laps = self.rules.laps;
        if self.relic {
            race::Mode::Relic { laps }
        } else {
            race::Mode::Race { laps }
        }
    }

    /// How the race's level has its instances (`lev_init`).
    pub fn lev_mode(&self) -> instance::lev_init::LevMode {
        use instance::lev_init::LevMode;
        if self.relic { LevMode::Relic } else { LevMode::Arcade }
    }

    /// Whether `track` is the race's level, loaded for its mode.
    pub fn is_shown(&self, track: &crate::Track) -> bool {
        self.track == track.1.0 && self.lev_mode() == track.2
    }

    /// Whether ticks of the race are played: it has gone, and the sim is the race's.
    pub fn racing(&self) -> bool {
        self.go && self.phase >= Phase::OnGrid
    }

    /// The short key of each slot's endpoint ([`NetRace::names`]): none in a race alone.
    pub fn keys(&self) -> &[String] {
        self.net.as_ref().map_or(&[], |net| &net.names)
    }

    /// Whether the player of `slot` left the race (only a lobby's player can).
    pub fn gone(&self, slot: usize) -> bool {
        self.net
            .as_ref()
            .is_some_and(|net| net.gone.get(slot).copied().unwrap_or(false))
    }

    /// The slots of the host's bots: the last of the players. Their pads are the host's to make
    /// (`learned.rs`), and come to everyone else as a player's do.
    pub fn bot_slots(&self) -> std::ops::Range<usize> {
        self.characters.len() - self.bots..self.characters.len()
    }

    /// Whether `slot` is one of the host's bots.
    pub fn is_bot(&self, slot: usize) -> bool {
        self.bot_slots().contains(&slot)
    }

    /// Whether `slot` is one of our local players (a split screen's other views too).
    pub fn local(&self, slot: usize) -> bool {
        self.locals.contains(&slot)
    }

    /// Whether the race has bots and they are ours: we host (the host is slot 0).
    pub fn hosts_bots(&self) -> bool {
        self.slot == 0 && self.bots > 0
    }

    /// Whether we make the pads of `slot`: one of our bots.
    pub fn drives(&self, slot: usize) -> bool {
        self.slot == 0 && self.is_bot(slot)
    }

    /// Who drives the kart of `slot`, for a cup to score: a player, or a bot of either kind (by
    /// its place among the bots; a race has one kind only, so the two never clash). A player is
    /// known by its endpoint, so a race alone has none.
    pub fn who(&self, slot: usize) -> Option<Who> {
        let players = self.characters.len();
        match slot.checked_sub(players) {
            None if self.is_bot(slot) => Some(Who::Bot((slot - self.bot_slots().start) as u8)),
            None => {
                let ids = &self.net.as_ref()?.ids;
                let id = *ids.get(slot)?;
                // (A machine's split-screen players come after its first: the seat counts them.)
                let seat = (0..slot).filter(|&s| !self.is_bot(s) && ids[s] == id).count();
                Some(Who::Player(id, seat as u8))
            }
            Some(bot) => (bot < self.retail.characters.len()).then_some(Who::Bot(bot as u8)),
        }
    }

    /// The kart of `slot` as a cup scores it.
    pub fn finisher(&self, slot: usize) -> Option<Finisher> {
        Some(Finisher {
            who: self.who(slot)?,
            name: self.kart_label(slot).to_string(),
            character: *self.kart_characters().get(slot)?,
        })
    }

    /// Whether we only watch: the slot is past the players', and there is no kart of ours.
    pub fn spectating(&self) -> bool {
        self.slot >= self.characters.len()
    }

    /// Whether we play only the ticks that have the pads of every player still racing, as a
    /// spectator does: a spectator, or a player whose kart has finished (see `drive_tick`).
    pub fn watches(&self) -> bool {
        self.spectating() || self.finished.get(self.slot).copied().unwrap_or(false)
    }

    /// How many karts the race has: the players' (the brain bots among them), then the retail
    /// bots'.
    pub fn karts(&self) -> usize {
        self.characters.len() + self.retail.characters.len()
    }

    /// The character of each kart, the retail bots' after the players'.
    pub fn kart_characters(&self) -> Vec<u8> {
        let bots = &self.retail.characters;
        self.characters.iter().chain(bots).copied().collect()
    }

    /// What the kart in `slot` is called: its player, or a bot of either kind as
    /// [`bot_label`] has it. (A slot past the players in [`labels`](Self::labels) is a
    /// spectator's, not a bot's.)
    pub fn kart_label(&self, slot: usize) -> std::borrow::Cow<'_, str> {
        let players = self.characters.len();
        if slot < players {
            return self.labels.get(slot).map_or("", String::as_str).into();
        }
        self.retail
            .characters
            .get(slot - players)
            .map_or("".into(), |&c| bot_label(c).into())
    }

    /// Whether the kart in `slot` is a bot, of either kind.
    pub fn is_any_bot(&self, slot: usize) -> bool {
        self.is_bot(slot) || (self.characters.len()..self.karts()).contains(&slot)
    }

    /// The kart of the sim that the view is of, and the HUD: ours, or the one a spectator follows.
    pub fn viewed(&self) -> usize {
        if self.spectating() {
            self.watching
        } else {
            self.slot
        }
    }

    /// A line for the HUD of a lobby's race: where it is, and how many ticks were played on a
    /// prediction, and how often (and how far back) a wrong one had to be put right.
    pub fn summary(&self, tick: u32, predicted: u32) -> Option<String> {
        let net = self.net.as_ref()?;
        let wait = if self.go { "" } else { " (waiting)" };
        let you = if self.spectating() {
            format!("watching p{}", self.watching)
        } else {
            format!("you p{}", self.slot)
        };
        let mut line = format!(
            "NET {tick} {you}/{}{wait}  predicting {predicted}  rollbacks {} ({} ticks)",
            self.characters.len(),
            net.corrections,
            net.replayed,
        );
        for d in &net.desync {
            line += &format!("  DESYNC with p{} at {}", d.slot, d.tick);
        }
        Some(line)
    }
}
