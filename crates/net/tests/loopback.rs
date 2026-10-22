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

#[test]
fn a_host_and_a_peer_play_ticks() {
    let host = net::host(3);
    let mut ticket = None;
    wait_for(&host, "the lobby to open", |e| match e {
        Event::Open(t) => {
            ticket = Some(t);
            true
        }
        _ => false,
    });
    let peer = net::join(ticket.unwrap(), 7);
    wait_for(&peer, "joining", |e| matches!(e, Event::Joined(_)));
    wait_for(&host, "the peer", |e| matches!(e, Event::PeerJoined(_)));
    std::thread::sleep(Duration::from_secs(8));
    for s in [&host, &peer] { s.poll(); }
    // Both change their minds after the lobby is up.
    host.set_character(4);
    peer.set_character(9);
    std::thread::sleep(Duration::from_millis(500));
    host.start(5);

    let mut ticks = [0u32; 2];
    let mut started = 0;
    let deadline = Instant::now() + Duration::from_secs(40);
    while ticks.iter().any(|&t| t < 60) {
        assert!(Instant::now() < deadline, "stalled at {ticks:?}");
        for (i, session) in [&host, &peer].into_iter().enumerate() {
            for event in session.poll() {
                if let Event::Started { characters, track, .. } = &event {
                    assert_eq!(characters, &[4, 9]);
                    assert_eq!(*track, 5);
                    started += 1;
                }
                if let Event::Frame { tick, .. } = event {
                    assert_eq!(tick, ticks[i]);
                    ticks[i] += 1;
                    session.input(Pad::default());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(33));
    }
    assert_eq!(started, 2);
}
