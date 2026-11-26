//! The matchmaker (MATCHMAKING.md): the queue, the groups it forms, the ready check, and which
//! player of a group coordinates (hosts its lobby). Pure state, driven by `matching.rs`'s
//! connection tasks and a 1 s tick; time is unix seconds, which is as fine as the tick.
//!
//! Quick play only for now: rating is ignored, and a group is whoever has waited longest.

use std::collections::HashMap;
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
        if self.queued.iter().chain(in_check).any(|w| w.key == waiting.key) {
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
        if check.accepted.iter().all(|&a| a) {
            let check = self.checks.remove(&match_id).expect("just found");
            for w in &check.players {
                w.send(ToClient::Go { id: match_id });
            }
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
                w.send(ToClient::Requeued);
                self.queued.push(w);
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

        self.queued.sort_by_key(|w| (w.joined, w.id));
        while let Some(group) = form(&self.queued, now) {
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

    fn found(&mut self, players: Vec<Waiting>, now: u64) {
        let order = coordinator_order(&players);
        let mut players: Vec<Option<Waiting>> = players.into_iter().map(Some).collect();
        let players: Vec<Waiting> = order.iter().map(|&i| players[i].take().expect("a permutation")).collect();
        self.next_id += 1;
        let id = self.next_id;
        let random = random();
        let found = Match {
            id,
            mode: players[0].mode,
            players: (players.iter())
                .map(|w| MatchPlayer {
                    endpoint: w.endpoint,
                    relay: w.relay.clone(),
                    nick: w.nick.clone(),
                })
                .collect(),
            track: (random % u64::from(race::RACING_TRACKS)) as u32,
            laps: QUICK_LAPS,
            masked_weapons: 0,
            even_odds: false,
            seed: (random >> 32) as u16,
            bots: 0,
        };
        tracing::info!(
            "match {id}: {} players on track {}, coordinator {}",
            players.len(),
            found.track,
            data_encoding::HEXLOWER.encode(&players[0].key[..4]),
        );
        for w in &players {
            w.send(ToClient::Found(found.clone()));
        }
        self.checks.insert(
            id,
            Check {
                accepted: vec![false; players.len()],
                players,
                deadline: now + u64::from(score::matching::ACCEPT_SECONDS),
            },
        );
    }
}

/// The indices of a group to form from `queued` (oldest first), if one is due: the first
/// [`MAX_MATCH`](score::matching::MAX_MATCH) of the oldest player's mode and version.
fn form(queued: &[Waiting], now: u64) -> Option<Vec<usize>> {
    // Each mode and version, by its oldest player.
    let mut seen: Vec<(Mode, &str)> = Vec::new();
    for oldest in queued {
        let kind = (oldest.mode, oldest.version.as_str());
        if seen.contains(&kind) {
            continue;
        }
        seen.push(kind);
        let group: Vec<usize> = (queued.iter().enumerate())
            .filter(|(_, w)| (w.mode, w.version.as_str()) == kind)
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
