use std::time::{Duration, Instant};

use net::{Event, Pad, Session};

/// The version of the sim the sessions say they play.
const BUILD: u32 = 1;

fn wait_for(session: &Session, what: &str, mut f: impl FnMut(Event) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        for event in session.poll() {
            if let Event::Failed(why) = &event {
                panic!("{what}: failed: {why}");
            }
            if f(event) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {what}");
}

/// What a player holds at a tick, which tells who made a pad and when.
fn pad(slot: usize, tick: u32) -> Pad {
    Pad {
        held: (slot as u32) << 16 | tick,
        ..Pad::default()
    }
}

#[test]
fn a_host_and_two_peers_race_over_a_mesh() {
    let host = net::host(3, "", BUILD);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let first = net::join(ticket.clone().unwrap(), 7, "", BUILD);
    wait_for(&first, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the first peer", |e| {
        matches!(e, Event::PeerJoined(_))
    });
    let second = net::join(ticket.unwrap(), 11, "", BUILD);
    wait_for(&second, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the second peer", |e| {
        matches!(e, Event::PeerJoined(_))
    });
    std::thread::sleep(Duration::from_secs(8));
    let sessions = [&host, &first, &second];
    for s in sessions {
        s.poll();
    }
    // Both change their minds after the lobby is up.
    host.set_character(4);
    first.set_character(9);
    std::thread::sleep(Duration::from_millis(500));
    host.start(5, 0, 3);

    // Everyone hears of the race, and says it is ready.
    let mut slots = [0; 3];
    for (i, session) in sessions.into_iter().enumerate() {
        wait_for(session, "the start", |e| match e {
            Event::Started {
                slot,
                characters,
                players,
                track,
                hash,
                laps,
            } => {
                assert_eq!(hash, 0);
                assert_eq!(laps, 3);
                assert_eq!(characters, [4, 9, 11]);
                assert_eq!(players.len(), 3);
                assert_eq!(track, 5);
                slots[i] = slot;
                true
            }
            _ => false,
        });
        session.ready();
    }
    assert_eq!(slots, [0, 1, 2]);

    // Tick 0 comes for all. (A pad that is early is not lost: it comes as an event, before or after.)
    let mut early: [Vec<(usize, u32, Vec<Pad>)>; 3] = Default::default();
    for (i, session) in sessions.into_iter().enumerate() {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut go = false;
        while !go {
            assert!(Instant::now() < deadline, "timed out waiting for tick 0");
            for event in session.poll() {
                match event {
                    Event::Go => go = true,
                    Event::Pads { slot, first, pads } => early[i].push((slot, first, pads)),
                    Event::Failed(why) => panic!("tick 0: failed: {why}"),
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // Each makes a pad per tick, with no delay, and gets the pads of the other two in order.
    let mut got = [[0u32; 3]; 3];
    let mut made = 0;
    let deadline = Instant::now() + Duration::from_secs(40);
    while got
        .iter()
        .enumerate()
        .any(|(me, g)| (0..3).any(|s| s != me && g[s] < 60))
    {
        assert!(Instant::now() < deadline, "stalled at {got:?}");
        for (me, session) in sessions.into_iter().enumerate() {
            if made < 90 {
                session.input(pad(me, made));
            }
            let events = std::mem::take(&mut early[me])
                .into_iter()
                .map(|(slot, first, pads)| Event::Pads { slot, first, pads })
                .chain(session.poll());
            for event in events {
                if let Event::Pads { slot, first, pads } = event {
                    assert_ne!(slot, me);
                    assert_eq!(first, got[me][slot], "pads of {slot} for {me} out of order");
                    for (i, p) in pads.iter().enumerate() {
                        assert_eq!(*p, pad(slot, first + i as u32));
                    }
                    got[me][slot] += pads.len() as u32;
                }
            }
        }
        made += 1;
        std::thread::sleep(Duration::from_millis(33));
    }

    // Each tells the others its hash of a tick, and hears theirs.
    for (me, session) in sessions.into_iter().enumerate() {
        session.checksum(30, 0xabc0 + me as u64);
    }
    for (me, session) in sessions.into_iter().enumerate() {
        let mut heard = [None; 3];
        wait_for(session, "the checksums", |e| {
            if let Event::Checksum { slot, tick, hash } = e {
                assert_eq!(tick, 30);
                heard[slot] = Some(hash);
            }
            (0..3).all(|s| s == me || heard[s] == Some(0xabc0 + s as u64))
        });
    }
}

#[test]
fn a_character_changed_between_races_is_the_one_that_races() {
    let host = net::host(3, "", BUILD);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let guest = net::join(ticket.unwrap(), 7, "", BUILD);
    wait_for(&guest, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the guest", |e| matches!(e, Event::PeerJoined(_)));
    std::thread::sleep(Duration::from_secs(2));
    let sessions = [&host, &guest];

    host.start(5, 0, 3);
    for session in sessions {
        wait_for(session, "the first start", |e| match e {
            Event::Started { characters, .. } => {
                assert_eq!(characters, [3, 7]);
                true
            }
            _ => false,
        });
    }

    // The race is over, and both pick another racer for the next.
    for session in sessions {
        session.over();
    }
    host.set_character(4);
    guest.set_character(9);
    std::thread::sleep(Duration::from_millis(500));
    host.start(5, 0, 3);
    for session in sessions {
        wait_for(session, "the second start", |e| match e {
            Event::Started { characters, .. } => {
                assert_eq!(characters, [4, 9]);
                true
            }
            _ => false,
        });
    }
}

#[test]
fn a_guest_fetches_a_custom_track_from_the_host() {
    let host = net::host(3, "", BUILD);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let guest = net::join(ticket.unwrap(), 7, "", BUILD);
    wait_for(&guest, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the guest", |e| matches!(e, Event::PeerJoined(_)));

    // A track bigger than a datagram, so that it takes a stream of its own.
    let bytes: Vec<u8> = (0..700_000u32).map(|i| (i * 31 >> 3) as u8).collect();
    let (track, hash) = (0x8000_1234, 0xabcd_ef01);
    host.provide(track, hash, std::sync::Arc::new(bytes.clone()));

    guest.fetch(track, hash);
    wait_for(&guest, "the track", |e| match e {
        Event::Blob {
            track: t,
            hash: h,
            bytes: got,
        } => {
            assert_eq!((t, h), (track, hash));
            assert!(got == bytes, "the track came through changed");
            true
        }
        _ => false,
    });

    // A track the host does not have, or has under another hash, is said to be missing.
    guest.fetch(track, hash + 1);
    wait_for(
        &guest,
        "the refusal",
        |e| matches!(e, Event::BlobMissing { track: t, hash: h } if t == track && h == hash + 1),
    );
}

#[test]
fn asking_for_a_track_twice_brings_it_once() {
    let host = net::host(3, "", BUILD);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let guest = net::join(ticket.unwrap(), 7, "", BUILD);
    wait_for(&guest, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the guest", |e| matches!(e, Event::PeerJoined(_)));

    let bytes: Vec<u8> = (0..700_000u32).map(|i| (i * 31 >> 3) as u8).collect();
    let (track, hash) = (0x8000_1234, 0xabcd_ef01);
    host.provide(track, hash, std::sync::Arc::new(bytes));

    guest.fetch(track, hash);
    guest.fetch(track, hash);
    wait_for(&guest, "the track", |e| matches!(e, Event::Blob { .. }));
    // Neither a second copy nor a refusal follows.
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        for event in guest.poll() {
            assert!(
                !matches!(event, Event::Blob { .. } | Event::BlobMissing { .. }),
                "the second ask was answered"
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_guest_with_another_build_is_turned_away() {
    let host = net::host(3, "", BUILD);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let guest = net::join(ticket.unwrap(), 7, "", BUILD + 1);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut why = None;
    while why.is_none() {
        assert!(Instant::now() < deadline, "the guest was not turned away");
        for event in guest.poll() {
            if let Event::Failed(text) = event {
                why = Some(text);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(why.unwrap().contains("another version"));
}

#[test]
fn a_player_who_leaves_ends_with_the_most_pads_anyone_had() {
    let host = net::host(3, "", BUILD);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let first = net::join(ticket.clone().unwrap(), 7, "", BUILD);
    wait_for(&first, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the first peer", |e| {
        matches!(e, Event::PeerJoined(_))
    });
    let second = net::join(ticket.unwrap(), 11, "", BUILD);
    wait_for(&second, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the second peer", |e| {
        matches!(e, Event::PeerJoined(_))
    });
    std::thread::sleep(Duration::from_secs(8));
    host.start(5, 0, 3);
    for session in [&host, &first, &second] {
        wait_for(session, "the start", |e| matches!(e, Event::Started { .. }));
        session.ready();
    }
    for session in [&host, &first, &second] {
        wait_for(session, "tick 0", |e| matches!(e, Event::Go));
    }

    // The last guest goes. The host and the other guest hold different amounts of its pads.
    drop(second);
    wait_for(&host, "the host to hear of it", |e| {
        matches!(e, Event::Gone { slot: 2 })
    });
    wait_for(&first, "the guest to hear of it", |e| {
        matches!(e, Event::Gone { slot: 2 })
    });
    let pads: Vec<Pad> = (0..5).map(|t| pad(2, t)).collect();
    host.have(2, 0, pads[..3].to_vec());
    first.have(2, 0, pads.clone());
    for session in [&host, &first] {
        wait_for(session, "the pads to be settled", |e| match e {
            Event::Left {
                slot,
                first,
                pads: got,
            } => {
                assert_eq!((slot, first), (2, 0));
                assert_eq!(got, pads);
                true
            }
            _ => false,
        });
    }
}

#[test]
fn a_player_who_left_the_race_is_not_in_the_lobby_after_it() {
    let host = net::host(3, "", BUILD);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let first = net::join(ticket.clone().unwrap(), 7, "", BUILD);
    wait_for(&first, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the first peer", |e| {
        matches!(e, Event::PeerJoined(_))
    });
    let second = net::join(ticket.unwrap(), 11, "", BUILD);
    wait_for(&second, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the second peer", |e| {
        matches!(e, Event::PeerJoined(_))
    });
    std::thread::sleep(Duration::from_secs(8));
    host.start(5, 0, 3);
    for session in [&host, &first, &second] {
        wait_for(session, "the start", |e| matches!(e, Event::Started { .. }));
        session.ready();
    }
    for session in [&host, &first, &second] {
        wait_for(session, "tick 0", |e| matches!(e, Event::Go));
    }

    drop(second);
    for session in [&host, &first] {
        wait_for(session, "the departure", |e| {
            matches!(e, Event::Gone { slot: 2 })
        });
    }

    // The race ends: the lobby is the host and the guest who stayed.
    host.over();
    first.over();
    for session in [&host, &first] {
        wait_for(session, "the lobby after the race", |e| match e {
            Event::Lobby { players, .. } => players.len() == 2,
            _ => false,
        });
    }
}

/// Opens a lobby and has `guests` join it, in order, each driving character 7 + its number. Waits
/// until every session sees all of them in the lobby.
fn lobby_of(guests: usize) -> (Session, Vec<Session>) {
    let host = net::host(3, "", BUILD);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let guests: Vec<Session> = (0..guests)
        .map(|i| {
            let guest = net::join(ticket.clone().unwrap(), 7 + i as u8, "", BUILD);
            wait_for(&guest, "joining", |e| matches!(e, Event::Joined(_)));
            wait_for(&host, "a peer", |e| matches!(e, Event::PeerJoined(_)));
            guest
        })
        .collect();
    let everyone = guests.len() + 1;
    for session in guests.iter().chain([&host]) {
        wait_for(session, "everyone in the lobby", |e| match e {
            Event::Lobby { players, .. } => players.len() == everyone,
            _ => false,
        });
    }
    // (The players' addresses take a while to be found by id, which migrating needs.)
    std::thread::sleep(Duration::from_secs(8));
    (host, guests)
}

/// The ticket that a session that became the host says the others join with.
fn promoted(session: &Session) -> net::Ticket {
    let mut ticket = None;
    wait_for(session, "becoming the host", |e| match e {
        Event::Promoted { ticket: t, .. } => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    ticket.unwrap()
}

#[test]
fn the_first_guest_hosts_the_lobby_when_the_host_leaves() {
    let (host, guests) = lobby_of(2);
    let [first, second] = <[Session; 2]>::try_from(guests).ok().unwrap();
    drop(host);

    let ticket = promoted(&first);
    wait_for(&second, "following the new host", |e| {
        matches!(e, Event::Joined(id) if id == ticket.id)
    });
    // The lobby is the new host and the guest that followed, in that order.
    for session in [&first, &second] {
        wait_for(session, "the new lobby", |e| match e {
            Event::Lobby { players, .. } => {
                players.len() == 2 && players[0].id == ticket.id && players[0].character == 7
                    && players[1].character == 8
            }
            _ => false,
        });
    }
    // It works as a lobby: the new host starts a race with the guest.
    std::thread::sleep(Duration::from_secs(1));
    first.start(5, 0, 3);
    for session in [&first, &second] {
        wait_for(session, "the start", |e| match e {
            Event::Started { characters, .. } => {
                assert_eq!(characters, [7, 8]);
                true
            }
            _ => false,
        });
    }
}

#[test]
fn the_next_guest_hosts_when_the_first_is_gone_as_well() {
    let (host, guests) = lobby_of(3);
    let [first, second, third] = <[Session; 3]>::try_from(guests).ok().unwrap();
    // The heir is gone before it took over, and the guests behind it have to find that out.
    drop(host);
    drop(first);

    let ticket = promoted(&second);
    wait_for(&third, "following the second guest", |e| {
        matches!(e, Event::Joined(id) if id == ticket.id)
    });
    for session in [&second, &third] {
        wait_for(session, "the new lobby", |e| match e {
            Event::Lobby { players, .. } => {
                players.len() == 2 && players[0].id == ticket.id && players[1].character == 9
            }
            _ => false,
        });
    }
}

#[test]
fn the_last_guest_hosts_a_lobby_of_one() {
    let (host, guests) = lobby_of(1);
    let [guest] = <[Session; 1]>::try_from(guests).ok().unwrap();
    drop(host);
    promoted(&guest);
    // (The lobby event that comes with becoming the host is read with that one.)
    guest.set_character(5);
    wait_for(&guest, "the lobby of one", |e| match e {
        Event::Lobby { players, .. } => players.len() == 1,
        _ => false,
    });
}

#[test]
fn the_race_stops_and_the_lobby_goes_on_when_the_host_leaves_it() {
    let (host, guests) = lobby_of(2);
    let [first, second] = <[Session; 2]>::try_from(guests).ok().unwrap();
    host.start(5, 0, 3);
    for session in [&host, &first, &second] {
        wait_for(session, "the start", |e| matches!(e, Event::Started { .. }));
        session.ready();
    }
    for session in [&host, &first, &second] {
        wait_for(session, "tick 0", |e| matches!(e, Event::Go));
    }

    drop(host);
    // The race is stopped (not failed) for both, and the first guest hosts the lobby.
    let (mut stopped, mut ticket) = ([false; 2], None);
    let deadline = Instant::now() + Duration::from_secs(20);
    while !stopped.iter().all(|&s| s) || ticket.is_none() {
        assert!(Instant::now() < deadline, "stopped {stopped:?}, host {ticket:?}");
        for (i, session) in [&first, &second].into_iter().enumerate() {
            for event in session.poll() {
                match event {
                    Event::Stopped => stopped[i] = true,
                    Event::Promoted { ticket: t, .. } => ticket = Some(t),
                    Event::Failed(why) => panic!("failed: {why}"),
                    _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // The game says the race is over, and the lobby is the new host and the other guest.
    first.over();
    second.over();
    let ticket = ticket.unwrap();
    wait_for(&second, "following the new host", |e| {
        matches!(e, Event::Joined(id) if id == ticket.id)
    });
    wait_for(&first, "the new lobby", |e| match e {
        Event::Lobby { players, .. } => players.len() == 2 && players[0].id == ticket.id,
        _ => false,
    });
}
