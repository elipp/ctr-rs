//! The matchmaker (MATCHMAKING.md): the queue, the groups it forms, the ready check, and which
//! player of a group coordinates (hosts its lobby). Pure state, driven by `matching.rs`'s
//! connection tasks and a 1 s tick; time is unix seconds, which is as fine as the tick.
//!
//! Quick play only for now: rating is ignored, and a group is whoever has waited longest.
//!
//! A match's lobby outlives its ready check: whoever hosts it registers ([`Matchmaker::host`])
//! and says how many places it has free between races, and the queue fills those first
//! (backfill), with the players that want that lobby back (`QueueEntry::rejoin`) before anyone.
//! The host lets them in ([`ToClient::Admit`]) before they are told to go, so that none of them
//! dials a lobby that would turn it away.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use score::matching::{Match, MatchPlayer, Mode, Net, Refused, Searching, ToClient};
use tokio::sync::mpsc::UnboundedSender;

/// A group of at least this many forms once its oldest has waited [`GATHER_S`]: a few seconds
/// more for a fuller grid.
const TARGET_MIN: usize = 4;
const GATHER_S: u64 = 15;
/// Any group of 2 or more forms once its oldest has waited this long. (Not 0: clippy denies a
/// `>=` that is always true, and a tick is a second anyway.)
// TODO(matchmaking): back to 60 once there are players to gather: a pair races at once only to
// test with two clients (2026-11-26). Then `a_pair_races` wants its tick before the wait again.
const SMALL_AFTER_S: u64 = 1;

/// A decline (or a ready check let run out) keeps the key out of the queue this long, doubling
/// with each further one within [`STRIKES_FORGOTTEN_S`], up to [`COOLDOWN_MAX_S`].
const COOLDOWN_S: u64 = 30;
const COOLDOWN_MAX_S: u64 = 600;
const STRIKES_FORGOTTEN_S: u64 = 3600;

/// A relay a player didn't reach counts as this many milliseconds.
const UNREACHED_MS: u32 = 2000;

/// The fixed quick-play rules (MATCHMAKING.md, decided 2026-11-26).
const QUICK_LAPS: u8 = 3;

/// How long a lobby is remembered with nobody registered as its host: an heir takes a while to
/// find out that the host is gone (the session's idle timeout) and to register.
const LOBBY_GONE_S: u64 = 120;

/// How long a lobby's host has to answer an [`ToClient::Admit`].
const ADMIT_S: u64 = 10;

/// A player in the queue, or in a ready check.
pub struct Waiting {
    /// Of this queueing, given by [`Matchmaker::join`].
    pub id: u64,
    pub key: [u8; 32],
    pub mode: Mode,
    pub version: String,
    pub endpoint: [u8; 32],
    pub relay: Option<String>,
    pub net: Net,
    pub nick: String,
    /// The lobby it wants back (`QueueEntry::rejoin`).
    pub rejoin: Option<u64>,
    pub joined: u64,
    pub to: UnboundedSender<ToClient>,
}

impl Waiting {
    fn send(&self, message: ToClient) {
        // (A connection that ended is told nothing: its task calls `leave`.)
        let _ = self.to.send(message);
    }
}

struct Check {
    players: Vec<Waiting>,
    accepted: Vec<bool>,
    deadline: u64,
    /// The lobby they go into, if it is one that is on (backfill).
    lobby: Option<u64>,
}

/// Players that accepted a match of a lobby that is on, until its host has let them in.
struct Admitting {
    lobby: u64,
    players: Vec<Waiting>,
    deadline: u64,
}

/// The connection of a lobby's host: a registration of [`Matchmaker::host`].
pub struct Host {
    /// Given by [`Matchmaker::host`].
    pub id: u64,
    pub to: UnboundedSender<ToClient>,
}

/// A match's lobby, from its go until it has had no host for [`LOBBY_GONE_S`].
struct Lobby {
    /// What its matches are: mode, track and rules (`id`, `lobby` and `players` aside).
    template: Match,
    version: String,
    /// Every endpoint that was let in: the ones that may host it.
    members: HashSet<[u8; 32]>,
    host: Option<Host>,
    /// As its host last said (0 without one).
    open: u8,
    players: Vec<MatchPlayer>,
    /// When it last had a host.
    hosted: u64,
}

struct Strikes {
    count: u32,
    last: u64,
    until: u64,
}

#[derive(Default)]
pub struct Matchmaker {
    queued: Vec<Waiting>,
    checks: HashMap<u64, Check>,
    admitting: HashMap<u64, Admitting>,
    lobbies: HashMap<u64, Lobby>,
    strikes: HashMap<[u8; 32], Strikes>,
    next_id: u64,
}

/// Unpredictable enough for a track and a seed: std's hasher keys are random per process and
/// differ for every `RandomState`.
fn random() -> u64 {
    std::collections::hash_map::RandomState::new().hash_one(0u8)
}

impl Matchmaker {
    /// Queues a player. `waiting.id` is set here.
    pub fn join(&mut self, mut waiting: Waiting, now: u64) -> Result<u64, Refused> {
        if waiting.mode != Mode::Quick {
            return Err(Refused::NotOpen { mode: waiting.mode });
        }
        if let Some(s) = self.strikes.get(&waiting.key).filter(|s| s.until > now) {
            return Err(Refused::Cooldown { until: s.until });
        }
        let in_check = self.checks.values().flat_map(|c| &c.players);
        let admitting = self.admitting.values().flat_map(|a| &a.players);
        if self.queued.iter().chain(in_check).chain(admitting).any(|w| w.key == waiting.key) {
            return Err(Refused::AlreadyQueued);
        }
        self.next_id += 1;
        waiting.id = self.next_id;
        waiting.joined = now;
        self.queued.push(waiting);
        Ok(self.next_id)
    }

    /// The connection of queueing `id` ended. Leaving a ready check is declining it.
    pub fn leave(&mut self, id: u64, now: u64) {
        self.queued.retain(|w| w.id != id);
        // (Its host may let it in all the same: an endpoint that never comes is harmless.)
        for a in self.admitting.values_mut() {
            a.players.retain(|w| w.id != id);
        }
        let check = self.checks.iter().find(|(_, c)| c.players.iter().any(|w| w.id == id));
        if let Some((&match_id, _)) = check {
            self.fail(match_id, |w| w.id == id, now);
        }
    }

    /// Queueing `id`'s answer to the ready check of match `match_id`.
    pub fn answer(&mut self, id: u64, match_id: u64, accept: bool, now: u64) {
        let Some(check) = self.checks.get_mut(&match_id) else {
            return;
        };
        let Some(i) = check.players.iter().position(|w| w.id == id) else {
            return;
        };
        if !accept {
            self.fail(match_id, |w| w.id == id, now);
            return;
        }
        check.accepted[i] = true;
        if !check.accepted.iter().all(|&a| a) {
            return;
        }
        let check = self.checks.remove(&match_id).expect("just found");
        match check.lobby {
            None => {
                let players: Vec<MatchPlayer> = check.players.iter().map(player).collect();
                let first = &check.players[0];
                let lobby = Lobby {
                    template: self.template(first.mode, &players, match_id),
                    version: first.version.clone(),
                    members: check.players.iter().map(|w| w.endpoint).collect(),
                    host: None,
                    open: 0,
                    players,
                    hosted: now,
                };
                self.lobbies.insert(match_id, lobby);
                for w in &check.players {
                    w.send(ToClient::Go { id: match_id });
                }
            }
            Some(lobby_id) => match self.lobbies.get(&lobby_id).and_then(|l| l.host.as_ref()) {
                Some(host) => {
                    let players = check.players.iter().map(player).collect();
                    let _ = host.to.send(ToClient::Admit { id: match_id, players });
                    let admitting = Admitting { lobby: lobby_id, players: check.players, deadline: now + ADMIT_S };
                    self.admitting.insert(match_id, admitting);
                }
                None => self.requeue(check.players),
            },
        }
    }

    /// Whoever hosts lobby `lobby` (one of its players: `endpoint`) registers, in the place of
    /// whoever did before.
    pub fn host(&mut self, lobby: u64, endpoint: [u8; 32], to: UnboundedSender<ToClient>, now: u64) -> Result<u64, Refused> {
        let Some(l) = self.lobbies.get_mut(&lobby).filter(|l| l.members.contains(&endpoint)) else {
            return Err(Refused::NoSuchLobby);
        };
        self.next_id += 1;
        l.host = Some(Host { id: self.next_id, to });
        l.open = 0;
        l.hosted = now;
        Ok(self.next_id)
    }

    /// Host registration `host` of `lobby` says how it stands.
    pub fn open(&mut self, host: u64, lobby: u64, open: u8, players: Vec<MatchPlayer>) {
        if let Some(l) = self.lobby_of(host, lobby) {
            let max = score::matching::MAX_MATCH as u8;
            l.players = players.into_iter().filter(|p| l.members.contains(&p.endpoint)).collect();
            l.open = open.min(max.saturating_sub(l.players.len() as u8));
        }
    }

    /// Host registration `host` of `lobby` has let the players of admission `id` in: they go.
    pub fn admitted(&mut self, host: u64, lobby: u64, id: u64) {
        if self.admitting.get(&id).is_none_or(|a| a.lobby != lobby) || self.lobby_of(host, lobby).is_none() {
            return;
        }
        let a = self.admitting.remove(&id).expect("just found");
        let l = self.lobbies.get_mut(&lobby).expect("just found");
        l.open = l.open.saturating_sub(a.players.len() as u8);
        for w in &a.players {
            l.members.insert(w.endpoint);
            w.send(ToClient::Go { id });
        }
    }

    /// Host registration `host` of `lobby` ended.
    pub fn unhost(&mut self, host: u64, lobby: u64, now: u64) {
        if let Some(l) = self.lobby_of(host, lobby) {
            l.host = None;
            l.open = 0;
            l.hosted = now;
        }
    }

    fn lobby_of(&mut self, host: u64, lobby: u64) -> Option<&mut Lobby> {
        self.lobbies.get_mut(&lobby).filter(|l| l.host.as_ref().is_some_and(|h| h.id == host))
    }

    /// Back in the queue, their wait kept.
    fn requeue(&mut self, players: Vec<Waiting>) {
        for w in players {
            w.send(ToClient::Requeued);
            self.queued.push(w);
        }
    }

    /// Ends the ready check `match_id`: the players `out` matches are struck and refused, the
    /// rest go back to the queue with their wait kept.
    fn fail(&mut self, match_id: u64, out: impl Fn(&Waiting) -> bool, now: u64) {
        let Some(check) = self.checks.remove(&match_id) else {
            return;
        };
        for w in check.players {
            if out(&w) {
                self.strike(w.key, now);
                w.send(ToClient::Refused(Refused::Declined));
            } else {
                self.requeue(vec![w]);
            }
        }
    }

    fn strike(&mut self, key: [u8; 32], now: u64) {
        let s = self.strikes.entry(key).or_insert(Strikes { count: 0, last: 0, until: 0 });
        if now.saturating_sub(s.last) > STRIKES_FORGOTTEN_S {
            s.count = 0;
        }
        s.count += 1;
        s.last = now;
        let wait = COOLDOWN_S.saturating_mul(1 << (s.count - 1).min(16)).min(COOLDOWN_MAX_S);
        s.until = now + wait;
    }

    /// Once a second: ready checks that ran out, new groups, and everyone's wait.
    pub fn tick(&mut self, now: u64) {
        let expired: Vec<(u64, Vec<bool>)> = (self.checks.iter())
            .filter(|(_, c)| c.deadline <= now)
            .map(|(&id, c)| (id, c.accepted.clone()))
            .collect();
        for (id, accepted) in expired {
            let late: Vec<u64> = (self.checks[&id].players.iter().zip(&accepted))
                .filter(|(_, a)| !**a)
                .map(|(w, _)| w.id)
                .collect();
            self.fail(id, |w| late.contains(&w.id), now);
        }
        self.strikes.retain(|_, s| now.saturating_sub(s.last) <= STRIKES_FORGOTTEN_S);
        let late: Vec<u64> = (self.admitting.iter())
            .filter(|(_, a)| a.deadline <= now)
            .map(|(&id, _)| id)
            .collect();
        for id in late {
            let a = self.admitting.remove(&id).expect("just found");
            self.requeue(a.players);
        }
        self.lobbies.retain(|_, l| l.host.is_some() || now.saturating_sub(l.hosted) <= LOBBY_GONE_S);

        self.queued.sort_by_key(|w| (w.joined, w.id));
        self.backfill(now);
        // (A player held for a lobby it wants back is not grouped elsewhere while that lobby is.)
        let lobbies: HashSet<u64> = self.lobbies.keys().copied().collect();
        let held = |w: &Waiting| w.rejoin.is_some_and(|id| lobbies.contains(&id));
        while let Some(group) = form(&self.queued, now, held) {
            let mut players = Vec::new();
            // (From the back, so that the indices before stay put.)
            for &i in group.iter().rev() {
                players.push(self.queued.remove(i));
            }
            players.reverse();
            self.found(players, now);
        }
        let mut counts: HashMap<(Mode, &str), u16> = HashMap::new();
        for w in &self.queued {
            *counts.entry((w.mode, &w.version)).or_default() += 1;
        }
        for w in &self.queued {
            w.send(ToClient::Searching(Searching {
                players: counts[&(w.mode, w.version.as_str())],
                waited_s: now.saturating_sub(w.joined) as u32,
            }));
        }
    }

    /// Fills the lobbies that have places free, one ready check each: the players that want the
    /// lobby back first, then whoever has waited longest (`queued` is in that order).
    fn backfill(&mut self, now: u64) {
        let busy: HashSet<u64> = (self.checks.values().filter_map(|c| c.lobby))
            .chain(self.admitting.values().map(|a| a.lobby))
            .collect();
        let open: Vec<u64> = (self.lobbies.iter())
            .filter(|(id, l)| l.host.is_some() && l.open > 0 && !busy.contains(id))
            .map(|(&id, _)| id)
            .collect();
        for lobby_id in open {
            let l = &self.lobbies[&lobby_id];
            let fits = |w: &Waiting| w.mode == l.template.mode && w.version == l.version;
            let theirs = (self.queued.iter().enumerate()).filter(|(_, w)| fits(w) && w.rejoin == Some(lobby_id));
            let anyone = (self.queued.iter().enumerate())
                .filter(|(_, w)| fits(w) && w.rejoin.is_none_or(|id| !self.lobbies.contains_key(&id)));
            let mut picks: Vec<usize> = theirs.chain(anyone).map(|(i, _)| i).take(l.open as usize).collect();
            if picks.is_empty() {
                continue;
            }
            picks.sort();
            let mut players = Vec::new();
            for &i in picks.iter().rev() {
                players.push(self.queued.remove(i));
            }
            players.reverse();
            self.found_for(lobby_id, players, now);
        }
    }

    /// The rules of a new lobby's matches.
    fn template(&self, mode: Mode, players: &[MatchPlayer], id: u64) -> Match {
        let random = random();
        Match {
            id,
            lobby: id,
            mode,
            players: players.to_vec(),
            track: (random % u64::from(race::RACING_TRACKS)) as u32,
            laps: QUICK_LAPS,
            masked_weapons: 0,
            even_odds: false,
            seed: (random >> 32) as u16,
            bots: 0,
        }
    }

    /// A ready check for `players` to go into `lobby`, behind the players in it.
    fn found_for(&mut self, lobby: u64, players: Vec<Waiting>, now: u64) {
        self.next_id += 1;
        let id = self.next_id;
        let l = &self.lobbies[&lobby];
        let found = Match {
            id,
            lobby,
            players: l.players.iter().cloned().chain(players.iter().map(player)).collect(),
            ..l.template.clone()
        };
        tracing::info!("match {id}: {} players into lobby {lobby}", players.len());
        self.check(found, players, Some(lobby), now);
    }

    fn found(&mut self, players: Vec<Waiting>, now: u64) {
        let order = coordinator_order(&players);
        let mut players: Vec<Option<Waiting>> = players.into_iter().map(Some).collect();
        let players: Vec<Waiting> = order.iter().map(|&i| players[i].take().expect("a permutation")).collect();
        self.next_id += 1;
        let id = self.next_id;
        let listed: Vec<MatchPlayer> = players.iter().map(player).collect();
        let found = self.template(players[0].mode, &listed, id);
        tracing::info!(
            "match {id}: {} players on track {}, coordinator {}",
            players.len(),
            found.track,
            data_encoding::HEXLOWER.encode(&players[0].key[..4]),
        );
        self.check(found, players, None, now);
    }

    fn check(&mut self, found: Match, players: Vec<Waiting>, lobby: Option<u64>, now: u64) {
        for w in &players {
            w.send(ToClient::Found(found.clone()));
        }
        self.checks.insert(
            found.id,
            Check {
                accepted: vec![false; players.len()],
                players,
                deadline: now + u64::from(score::matching::ACCEPT_SECONDS),
                lobby,
            },
        );
    }
}

fn player(w: &Waiting) -> MatchPlayer {
    MatchPlayer {
        endpoint: w.endpoint,
        relay: w.relay.clone(),
        nick: w.nick.clone(),
    }
}

/// The indices of a group to form from `queued` (oldest first), if one is due: the first
/// [`MAX_MATCH`](score::matching::MAX_MATCH) of the oldest player's mode and version.
fn form(queued: &[Waiting], now: u64, held: impl Fn(&Waiting) -> bool) -> Option<Vec<usize>> {
    // Each mode and version, by its oldest player.
    let mut seen: Vec<(Mode, &str)> = Vec::new();
    for oldest in queued.iter().filter(|w| !held(w)) {
        let kind = (oldest.mode, oldest.version.as_str());
        if seen.contains(&kind) {
            continue;
        }
        seen.push(kind);
        let group: Vec<usize> = (queued.iter().enumerate())
            .filter(|(_, w)| (w.mode, w.version.as_str()) == kind && !held(w))
            .map(|(i, _)| i)
            .take(score::matching::MAX_MATCH)
            .collect();
        let waited = now.saturating_sub(oldest.joined);
        let due = group.len() >= score::matching::MAX_MATCH
            || (group.len() >= TARGET_MIN && waited >= GATHER_S)
            || (group.len() >= 2 && waited >= SMALL_AFTER_S);
        if due {
            return Some(group);
        }
    }
    None
}

/// The order a group's players coordinate in: the first hosts, the rest are its heirs. An easy
/// NAT first (a hard one can only dial out, and the lobby is a star whose guests dial the host),
/// then working UDP, then the least latency to the relay the group is nearest to all told, then
/// the longest wait.
fn coordinator_order(players: &[Waiting]) -> Vec<usize> {
    let latency = |w: &Waiting, relay: &str| {
        (w.net.relays.iter())
            .find(|(url, _)| url == relay)
            .map_or(UNREACHED_MS, |&(_, ms)| u32::from(ms))
    };
    let relays: Vec<&str> = (players.iter())
        .flat_map(|w| w.net.relays.iter().map(|(url, _)| url.as_str()))
        .collect();
    let shared = relays
        .iter()
        .min_by_key(|relay| players.iter().map(|w| latency(w, relay)).sum::<u32>());
    let mut order: Vec<usize> = (0..players.len()).collect();
    order.sort_by_key(|&i| {
        let w = &players[i];
        let nat = match w.net.hard {
            Some(false) => 0,
            None => 1,
            Some(true) => 2,
        };
        let ms = shared.map_or(0, |relay| latency(w, relay));
        (nat, !w.net.udp, ms, w.joined, w.id)
    });
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    fn waiting(n: u8, net: Net) -> (Waiting, UnboundedReceiver<ToClient>) {
        let (to, rx) = unbounded_channel();
        let w = Waiting {
            id: 0,
            key: [n; 32],
            mode: Mode::Quick,
            version: "v".into(),
            endpoint: [n; 32],
            relay: None,
            net,
            nick: format!("p{n}"),
            rejoin: None,
            joined: 0,
            to,
        };
        (w, rx)
    }

    fn drain(rx: &mut UnboundedReceiver<ToClient>) -> Vec<ToClient> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    fn found(messages: &[ToClient]) -> Option<&Match> {
        messages.iter().find_map(|m| match m {
            ToClient::Found(found) => Some(found),
            _ => None,
        })
    }

    #[test]
    fn a_pair_races() {
        let mut mm = Matchmaker::default();
        let (a, mut ra) = waiting(1, Net::default());
        let (b, mut rb) = waiting(2, Net::default());
        mm.join(a, 0).unwrap();
        mm.join(b, 0).unwrap();
        mm.tick(0);
        assert!(found(&drain(&mut ra)).is_none());
        mm.tick(SMALL_AFTER_S);
        let m = found(&drain(&mut ra)).unwrap().clone();
        assert_eq!(found(&drain(&mut rb)), Some(&m));
        assert_eq!(m.players.len(), 2);
        assert!(m.track < race::RACING_TRACKS);
        mm.answer(1, m.id, true, 61);
        assert!(drain(&mut ra).is_empty());
        mm.answer(2, m.id, true, 62);
        assert_eq!(drain(&mut ra), [ToClient::Go { id: m.id }]);
        assert_eq!(drain(&mut rb), [ToClient::Go { id: m.id }]);
        assert!(mm.queued.is_empty() && mm.checks.is_empty());
    }

    #[test]
    fn four_gather_and_eight_go_at_once() {
        let mut mm = Matchmaker::default();
        let mut rxs = Vec::new();
        for n in 0..9 {
            let (w, rx) = waiting(n, Net::default());
            mm.join(w, 0).unwrap();
            rxs.push(rx);
        }
        mm.tick(1);
        let groups: Vec<usize> = rxs.iter_mut().map(|rx| found(&drain(rx)).map_or(0, |m| m.players.len())).collect();
        assert_eq!(groups, [8, 8, 8, 8, 8, 8, 8, 8, 0]);
    }

    #[test]
    fn a_decliner_is_struck_and_the_others_requeued() {
        let mut mm = Matchmaker::default();
        let (a, mut ra) = waiting(1, Net::default());
        let (b, mut rb) = waiting(2, Net::default());
        mm.join(a, 0).unwrap();
        mm.join(b, 0).unwrap();
        mm.tick(SMALL_AFTER_S);
        let m = found(&drain(&mut ra)).unwrap().clone();
        drain(&mut rb);
        mm.answer(2, m.id, false, 61);
        assert_eq!(drain(&mut rb), [ToClient::Refused(Refused::Declined)]);
        assert_eq!(drain(&mut ra), [ToClient::Requeued]);
        assert_eq!(mm.queued.len(), 1);
        assert_eq!(mm.queued[0].joined, 0, "the wait is kept");
        let (b, _) = waiting(2, Net::default());
        assert_eq!(mm.join(b, 70), Err(Refused::Cooldown { until: 61 + COOLDOWN_S }));
    }

    #[test]
    fn silence_runs_the_check_out() {
        let mut mm = Matchmaker::default();
        let (a, mut ra) = waiting(1, Net::default());
        let (b, mut rb) = waiting(2, Net::default());
        mm.join(a, 0).unwrap();
        mm.join(b, 0).unwrap();
        mm.tick(SMALL_AFTER_S);
        let m = found(&drain(&mut ra)).unwrap().clone();
        mm.answer(1, m.id, true, 61);
        mm.tick(SMALL_AFTER_S + u64::from(score::matching::ACCEPT_SECONDS));
        assert!(drain(&mut rb).contains(&ToClient::Refused(Refused::Declined)));
        assert!(drain(&mut ra).contains(&ToClient::Requeued));
    }

    #[test]
    fn the_coordinator_has_the_easiest_nat_and_the_nearest_relay() {
        let net = |hard, udp, ms: u16| Net {
            hard,
            udp,
            relays: vec![("eu".into(), ms), ("us".into(), 150)],
        };
        let players: Vec<Waiting> = [
            net(Some(true), true, 10),
            net(None, true, 10),
            net(Some(false), false, 10),
            net(Some(false), true, 40),
            net(Some(false), true, 20),
        ]
        .into_iter()
        .enumerate()
        .map(|(n, net)| waiting(n as u8, net).0)
        .collect();
        assert_eq!(coordinator_order(&players), [4, 3, 2, 1, 0]);
    }

    /// A pair's match gone, and its coordinator registered as the lobby's host.
    fn hosted_lobby(mm: &mut Matchmaker) -> (Match, u64, UnboundedReceiver<ToClient>) {
        let (a, mut ra) = waiting(1, Net::default());
        let (b, mut rb) = waiting(2, Net::default());
        let ia = mm.join(a, 0).unwrap();
        let ib = mm.join(b, 0).unwrap();
        mm.tick(SMALL_AFTER_S);
        let m = found(&drain(&mut ra)).unwrap().clone();
        mm.answer(ia, m.id, true, 2);
        mm.answer(ib, m.id, true, 2);
        assert!(drain(&mut rb).contains(&ToClient::Go { id: m.id }));
        assert_eq!(m.lobby, m.id);
        let (to, rh) = unbounded_channel();
        let host = mm.host(m.lobby, m.players[0].endpoint, to, 3).unwrap();
        (m, host, rh)
    }

    #[test]
    fn a_lobby_with_room_is_filled_once_its_host_lets_them_in() {
        let mut mm = Matchmaker::default();
        let (m, host, mut rh) = hosted_lobby(&mut mm);
        assert_eq!(mm.host(m.lobby, [9; 32], unbounded_channel().0, 3), Err(Refused::NoSuchLobby));
        mm.open(host, m.lobby, 6, m.players.clone());
        let (c, mut rc) = waiting(3, Net::default());
        let ic = mm.join(c, 4).unwrap();
        mm.tick(4);
        let f = found(&drain(&mut rc)).unwrap().clone();
        assert_eq!(f.lobby, m.lobby);
        assert_eq!(f.players[..2], m.players[..], "the lobby's players first");
        assert_eq!(f.players[2].endpoint, [3; 32]);
        mm.answer(ic, f.id, true, 5);
        assert!(drain(&mut rc).is_empty(), "no go before the host lets it in");
        assert!(matches!(drain(&mut rh)[..], [ToClient::Admit { id, .. }] if id == f.id));
        mm.admitted(host, m.lobby, f.id);
        assert_eq!(drain(&mut rc), [ToClient::Go { id: f.id }]);
        assert!(mm.host(m.lobby, [3; 32], unbounded_channel().0, 6).is_ok(), "a member now");
    }

    #[test]
    fn a_rejoiner_waits_for_its_lobby_and_goes_first() {
        let mut mm = Matchmaker::default();
        let (m, host, _rh) = hosted_lobby(&mut mm);
        let (mut c, mut rc) = waiting(3, Net::default());
        c.rejoin = Some(m.lobby);
        mm.join(c, 4).unwrap();
        let (d, mut rd) = waiting(4, Net::default());
        mm.join(d, 3).unwrap();
        mm.tick(10);
        assert!(found(&drain(&mut rc)).is_none(), "held while the lobby races");
        assert!(found(&drain(&mut rd)).is_none(), "and not grouped with d");
        mm.open(host, m.lobby, 1, m.players.clone());
        mm.tick(11);
        assert_eq!(found(&drain(&mut rc)).map(|f| f.lobby), Some(m.lobby));
        assert!(found(&drain(&mut rd)).is_none(), "the one place was c's, though d waited longer");
    }

    #[test]
    fn a_lobby_without_a_host_is_forgotten_and_its_rejoiners_released() {
        let mut mm = Matchmaker::default();
        let (m, host, _rh) = hosted_lobby(&mut mm);
        mm.unhost(host, m.lobby, 5);
        let (mut c, mut rc) = waiting(3, Net::default());
        c.rejoin = Some(m.lobby);
        mm.join(c, 5).unwrap();
        mm.join(waiting(4, Net::default()).0, 5).unwrap();
        mm.tick(6);
        assert!(found(&drain(&mut rc)).is_none());
        mm.tick(6 + LOBBY_GONE_S);
        assert!(found(&drain(&mut rc)).is_some_and(|f| f.lobby != m.lobby));
    }

    #[test]
    fn one_key_queues_once_and_ranked_is_closed() {
        let mut mm = Matchmaker::default();
        mm.join(waiting(1, Net::default()).0, 0).unwrap();
        assert_eq!(mm.join(waiting(1, Net::default()).0, 1), Err(Refused::AlreadyQueued));
        let (mut ranked, _) = waiting(2, Net::default());
        ranked.mode = Mode::Ranked;
        assert_eq!(mm.join(ranked, 1), Err(Refused::NotOpen { mode: Mode::Ranked }));
    }
}
