//! Rollback against a toy simulation, over a network that delays, repeats and loses pads. No disc
//! image needed: the state is a hash of everything the pads did, so any tick played on the wrong
//! pad (and not put right) shows in it.

use race::Pad;
use race::rollback::{Hole, Rollback, Simulation, Step};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Toy {
    ticks: u32,
    hash: u64,
    /// What the players have done to each other, so one pad changes the others' futures.
    positions: Vec<i64>,
}

impl Toy {
    fn new(players: usize) -> Toy {
        Toy {
            ticks: 0,
            hash: 0xcbf2_9ce4_8422_2325,
            positions: vec![0; players],
        }
    }
}

impl Simulation for Toy {
    type Context = ();
    type Output = u32;

    fn step(&mut self, _: &(), pads: &[Pad]) -> u32 {
        let lead = self.positions.iter().copied().max().unwrap_or(0);
        for (slot, pad) in pads.iter().enumerate() {
            // Steering moves a player, and a player far behind the lead catches up faster.
            let push = pad.lx as i64 - 0x80 + (pad.held & 0xff) as i64;
            let catch_up = (lead - self.positions[slot]) / 16;
            self.positions[slot] += push + catch_up;
            self.hash = (self.hash ^ self.positions[slot] as u64).wrapping_mul(0x100_0000_01b3);
        }
        self.ticks += 1;
        self.ticks
    }

    fn checksum(&self) -> u64 {
        self.hash ^ self.ticks as u64
    }
}

/// What a player does at a tick: changes now and then, so that predictions fail.
fn pad_of(slot: usize, tick: u32) -> Pad {
    let phase = (tick as usize / 5 + slot * 3) % 7;
    Pad {
        held: [0, 1, 1, 4, 0, 9, 2][phase],
        lx: [0x80, 0x20, 0xe0, 0x80, 0x90, 0x40, 0xc0][(phase + slot) % 7],
        ..Pad::default()
    }
}

/// The simulation played with the real pads, as a client with a perfect network would.
fn reference(players: usize, ticks: u32) -> Toy {
    let mut toy = Toy::new(players);
    for tick in 0..ticks {
        let pads: Vec<Pad> = (0..players).map(|s| pad_of(s, tick)).collect();
        toy.step(&(), &pads);
    }
    toy
}

/// Everyone's pads, as the network hands them over: a pad made at tick `t` reaches the others
/// at tick `t + latency(from, to, t)`, in a packet that repeats the pads from a few ticks before.
struct Network {
    /// (arrives at tick, to, from, first tick, pads)
    packets: Vec<(u32, usize, usize, u32, Vec<Pad>)>,
}

impl Network {
    fn send(
        &mut self,
        now: u32,
        from: usize,
        players: usize,
        latency: impl Fn(usize, usize, u32) -> Option<u32>,
    ) {
        let first = now.saturating_sub(3);
        let pads: Vec<Pad> = (first..=now).map(|t| pad_of(from, t)).collect();
        for to in (0..players).filter(|&to| to != from) {
            if let Some(delay) = latency(from, to, now) {
                self.packets
                    .push((now + delay, to, from, first, pads.clone()));
            }
        }
    }

    /// The packets that are due by tick `now`, for `to`, in the order they were sent.
    fn due(&mut self, now: u32, to: usize) -> Vec<(usize, u32, Vec<Pad>)> {
        let mut due = Vec::new();
        self.packets.retain(|(at, dest, from, first, pads)| {
            if *at <= now && *dest == to {
                due.push((*from, *first, pads.clone()));
                false
            } else {
                true
            }
        });
        due
    }
}

/// Plays `ticks` ticks on `players` clients over the network, then lets the last pads arrive.
fn play(
    players: usize,
    ticks: u32,
    window: usize,
    latency: impl Fn(usize, usize, u32) -> Option<u32> + Copy,
) -> Vec<Rollback<Toy>> {
    let mut clients: Vec<_> = (0..players)
        .map(|me| Rollback::new(Toy::new(players), players, Some(me), window).check_every(10))
        .collect();
    let mut network = Network {
        packets: Vec::new(),
    };
    let mut round = 0;
    while clients.iter().any(|c| c.tick() < ticks) {
        for me in 0..players {
            let client = &mut clients[me];
            for (from, first, pads) in network.due(round, me) {
                // A packet that comes before an older one has to wait for it.
                if client.confirm_all(from, first, &pads).is_err() {
                    network.packets.push((round + 1, me, from, first, pads));
                }
            }
            if client.tick() >= ticks {
                continue;
            }
            let tick = client.tick();
            client.confirm(me, tick, pad_of(me, tick)).unwrap();
            let advance = client.advance(&());
            if let Step::Played(_) = advance.step {
                network.send(tick, me, players, latency);
            }
        }
        round += 1;
        assert!(round < ticks * 20 + 1000, "the race does not go on");
    }
    // Let everything that is still on its way arrive, and put right what it changes.
    for step in 0..ticks + 200 {
        for me in 0..players {
            for (from, first, pads) in network.due(round + step + 100, me) {
                let _ = clients[me].confirm_all(from, first, &pads);
            }
            clients[me].correct(&());
        }
    }
    clients
}

#[test]
fn without_delay_nothing_is_ever_replayed() {
    let mut rollback = Rollback::new(Toy::new(2), 2, Some(0), 60);
    for tick in 0..100 {
        for slot in 0..2 {
            rollback.confirm(slot, tick, pad_of(slot, tick)).unwrap();
        }
        let advance = rollback.advance(&());
        assert_eq!(advance.replayed, 0);
        assert!(matches!(advance.step, Step::Played(_)));
        // Nothing waits on anyone, so nothing is kept.
        assert_eq!(rollback.unsettled(), 0);
        assert_eq!(rollback.kept(), 0);
    }
    assert_eq!(*rollback.state(), reference(2, 100));
}

#[test]
fn a_late_pad_puts_the_ticks_right() {
    let mut rollback = Rollback::new(Toy::new(2), 2, Some(0), 60);
    for tick in 0..40 {
        rollback.confirm(0, tick, pad_of(0, tick)).unwrap();
        rollback.advance(&());
    }
    // Player 1 has been predicted as doing nothing for 40 ticks.
    assert_eq!(rollback.unsettled(), 40);
    assert_ne!(*rollback.state(), reference(2, 40));
    let pads: Vec<Pad> = (0..40).map(|t| pad_of(1, t)).collect();
    rollback.confirm_all(1, 0, &pads).unwrap();
    assert!(rollback.dirty());
    assert_eq!(rollback.correct(&()), 40);
    assert_eq!(*rollback.state(), reference(2, 40));
    assert_eq!(rollback.unsettled(), 0);
}

#[test]
fn a_right_prediction_costs_nothing() {
    // Player 1 holds the same pad all the time, which is what is predicted once it is known.
    let still = Pad {
        held: 1,
        ..Pad::default()
    };
    let mut rollback = Rollback::new(Toy::new(2), 2, Some(0), 60);
    rollback.confirm(1, 0, still).unwrap();
    for tick in 0..20 {
        rollback.confirm(0, tick, pad_of(0, tick)).unwrap();
        rollback.advance(&());
    }
    for tick in 1..20 {
        rollback.confirm(1, tick, still).unwrap();
    }
    assert!(!rollback.dirty());
    assert_eq!(rollback.advance(&()).replayed, 0);
}

#[test]
fn a_pad_out_of_order_is_a_hole() {
    let mut rollback = Rollback::new(Toy::new(2), 2, Some(0), 60);
    assert_eq!(
        rollback.confirm(1, 3, Pad::default()),
        Err(Hole { expected: 0 })
    );
    rollback.confirm(1, 0, Pad::default()).unwrap();
    // A repeat is taken as one.
    rollback.confirm(1, 0, pad_of(1, 0)).unwrap();
    assert_eq!(rollback.confirmed_until(1), 1);
}

#[test]
fn the_window_stops_the_race_until_the_pads_come() {
    let mut rollback = Rollback::new(Toy::new(2), 2, Some(0), 30);
    let mut played = 0;
    for tick in 0..100 {
        rollback.confirm(0, tick, pad_of(0, tick)).unwrap();
        if let Step::Played(_) = rollback.advance(&()).step {
            played += 1;
        }
    }
    assert_eq!(played, 30);
    assert!(matches!(rollback.advance(&()).step, Step::Full));
    // The pads arrive: everything is put right, and the race goes on.
    let pads: Vec<Pad> = (0..30).map(|t| pad_of(1, t)).collect();
    rollback.confirm_all(1, 0, &pads).unwrap();
    assert!(matches!(rollback.advance(&()).step, Step::Played(_)));
    assert_eq!(rollback.tick(), 31);
}

#[test]
fn no_local_pad_no_tick() {
    let mut rollback = Rollback::new(Toy::new(2), 2, Some(0), 60);
    assert!(matches!(rollback.advance(&()).step, Step::NoInput));
    assert_eq!(rollback.tick(), 0);
}

#[test]
fn clients_agree_with_the_perfect_network_whatever_the_latency() {
    for latency in [0, 1, 3, 8, 20] {
        let clients = play(3, 150, 120, |_, _, _| Some(latency));
        for (me, client) in clients.iter().enumerate() {
            assert_eq!(
                *client.state(),
                reference(3, client.tick()),
                "latency {latency}, client {me}"
            );
        }
    }
}

#[test]
fn clients_agree_when_the_latencies_differ_and_packets_go_missing() {
    let clients = play(4, 200, 150, |from, to, tick| {
        // Some packets are lost (the repeats in the next ones make up for them), and the
        // others take a time that depends on who sends to whom, and jitters.
        if (tick as usize * 7 + from * 3 + to) % 5 == 0 {
            None
        } else {
            Some(2 + ((from * 5 + to * 3 + tick as usize * 11) % 9) as u32)
        }
    });
    for (me, client) in clients.iter().enumerate() {
        assert_eq!(*client.state(), reference(4, client.tick()), "client {me}");
    }
}

#[test]
fn the_snapshots_stay_within_the_window() {
    let mut rollback = Rollback::new(Toy::new(2), 2, Some(0), 25);
    for tick in 0..200 {
        rollback.confirm(0, tick, pad_of(0, tick)).unwrap();
        rollback.advance(&());
        assert!(rollback.unsettled() <= 25);
        // The other player's pads trickle in, ten ticks behind.
        if tick >= 10 {
            rollback
                .confirm(1, tick - 10, pad_of(1, tick - 10))
                .unwrap();
        }
    }
}

#[test]
fn the_checksums_are_those_of_the_perfect_network_each_once() {
    for latency in [0, 3, 12] {
        let mut clients = play(3, 150, 120, |from, to, tick| {
            if (tick as usize + from + to) % 6 == 0 {
                None
            } else {
                Some(latency + (tick % 3))
            }
        });
        for (me, client) in clients.iter_mut().enumerate() {
            let sums = client.take_checksums();
            let ticks: Vec<u32> = sums.iter().map(|&(t, _)| t).collect();
            // Every tenth tick, from the first, once and in order (the last few may still wait
            // for a trim).
            let expected: Vec<u32> = (0..ticks.len() as u32 * 10).step_by(10).collect();
            assert_eq!(ticks, expected, "latency {latency}, client {me}");
            assert!(
                ticks.len() >= 10,
                "latency {latency}, client {me}: {ticks:?}"
            );
            for (tick, hash) in sums {
                assert_eq!(
                    hash,
                    reference(3, tick).checksum(),
                    "latency {latency}, client {me}, tick {tick}"
                );
            }
        }
    }
}
