use std::time::{Duration, Instant};

use net::{Event, Pad, Session};

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
    let host = net::host(3);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let first = net::join(ticket.clone().unwrap(), 7);
    wait_for(&first, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the first peer", |e| {
        matches!(e, Event::PeerJoined(_))
    });
    let second = net::join(ticket.unwrap(), 11);
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
    host.start(5);

    // Everyone hears of the race, and says it is ready.
    let mut slots = [0; 3];
    for (i, session) in sessions.into_iter().enumerate() {
        wait_for(session, "the start", |e| match e {
            Event::Started {
                slot,
                characters,
                track,
            } => {
                assert_eq!(characters, [4, 9, 11]);
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
