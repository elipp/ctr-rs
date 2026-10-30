//! What clients say to each other, and the bookkeeping that makes the inputs survive loss.
//!
//! Every player sends its pads to every other player, directly. They go in unreliable datagrams,
//! one packet per tick (and one at once when a pad is made), and each packet repeats every input
//! the peer has not yet confirmed (`ack`). A lost or reordered packet is covered by the next one
//! without a resend or a wait for a timeout. The inputs are tiny (a few bytes a tick), so
//! repeating a handful of them costs next to nothing.
//!
//! Snapshots and checksums are for the cases the input stream cannot cover, and go on the
//! reliable stream: a peer that joins late or that fell further behind than [`MAX_WINDOW`]
//! restarts from a snapshot, and the checksums tell when two sims have drifted apart.

use std::collections::VecDeque;

use bitcode::{Decode, Encode};

/// A game tick (30 Hz) counted from the start of the race.
pub type Tick = u32;

/// The most ticks one packet repeats: the ticks a sim keeps to roll back (4 s). Past this the
/// peer needs a snapshot.
pub const MAX_WINDOW: usize = 120;

/// One tick of a controller: `vehicle::Input` without the taps, which follow from the
/// held buttons of the tick before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Pad {
    pub held: u32,
    pub lx: u8,
    pub ly: u8,
    pub ry: u8,
}

impl Default for Pad {
    /// Nothing pressed, sticks centred.
    fn default() -> Pad {
        Pad {
            held: 0,
            lx: 0x80,
            ly: 0x80,
            ry: 0x80,
        }
    }
}

/// A player's own inputs, as the unreliable datagram to another player carries them.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Inputs {
    /// The race these belong to: a race that starts again is a new epoch, and the packets of
    /// the one before, still on their way, are dropped.
    pub epoch: u32,
    /// The tick of `pads[0]`.
    pub first: Tick,
    pub pads: Vec<Pad>,
    /// The sender has all of the receiver's pads below this tick.
    pub ack: Tick,
    /// The sender's tick (the pads it has made) minus `ack`: how far past the receiver it is,
    /// as far as it can tell. The latency is in it, as it is in the receiver's own figure, so
    /// half the difference of the two is how far ahead the receiver's clock is.
    pub lead: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum Message {
    /// Reliable, client to host, right after joining: the character (`GAME_CHARACTER_IDS`
    /// order) this client drives. Characters have different stats, so every sim needs to know
    /// whose kart is whose.
    /// `seq` counts the client's Hellos from 0: they go on streams of their own, which can
    /// arrive in any order, and the host keeps the one with the highest.
    /// `build` is the version of the sim the client plays (`race::SIM_VERSION`): the host turns
    /// away one that differs, as its race would not come out the same.
    /// `nick` is what the client wants to be called (empty for none, see `clean_nick`). Its `seq`
    /// is shared with [`Message::Nick`], so that the latest of either wins.
    /// `spectator` is set by a client that only watches: it takes no slot among the players, and
    /// is on the end of the race's roster, past the last of them.
    Hello {
        character: u8,
        seq: u32,
        build: u32,
        nick: String,
        spectator: bool,
    },
    /// Reliable, client to host, at any time (a race included): the client is called `nick` from
    /// now on. The host passes it on in [`Message::Lobby`]. `seq` counts with the Hellos'.
    Nick { seq: u32, nick: String },
    /// Reliable, host to client: the Hello was of another build than the host's.
    Reject { host: u32, yours: u32 },
    /// Reliable, host to each client: a race is about to start. `slot` is the client's player
    /// among the players; `characters` is the character of each slot, and `track` the `LevelId`
    /// the race is on, and `hash` the hash of its contents when it is a custom track (0 for the
    /// game's own), which a client that lacks it asks for with [`Message::Want`]. `roster` is the endpoint id of each slot (the host is 0), which the
    /// clients connect to each other by. `characters` has the players only: a slot past them is a
    /// spectator's, which plays no kart and sends no pads.
    Start {
        /// Which race this is (see [`Message::Restart`]): 0 for the first of the lobby. A start
        /// after a race that is over takes new players in, and the ones that left out.
        epoch: u32,
        slot: u8,
        characters: Vec<u8>,
        track: u32,
        hash: u64,
        roster: Vec<[u8; 32]>,
    },
    /// Reliable, client to host: connected to every other player, with the level up. Once the
    /// host has this from everyone (and is ready itself) it sends [`Message::Go`].
    Ready,
    /// Reliable, host to each client: this is tick 0.
    Go,
    /// Unreliable, between any two players.
    Inputs(Inputs),
    /// Reliable, host to each client: the race starts again, on `track`. The same players, on
    /// the same connections; `epoch` counts the races and tells their packets apart.
    Restart { epoch: u32, track: u32, hash: u64 },
    /// Reliable, client to host: send the custom track `track` whose contents hash to `hash`.
    Want { track: u32, hash: u64 },
    /// Reliable, host to client: the custom track asked for, as the bytes it was provided as.
    Blob {
        track: u32,
        hash: u64,
        bytes: Vec<u8>,
    },
    /// Reliable, host to client: the host has no such track to send.
    NoBlob { track: u32, hash: u64 },
    /// Reliable, host to each client: who is in the lobby and what each drives, the host first.
    /// It comes whenever that changes, in full, so a client that joins late catches up. Each
    /// is the endpoint id, the character and the nickname (empty for none).
    /// The guests that only watch follow, as the endpoint id and the nickname.
    Lobby {
        players: Vec<([u8; 32], u8, String)>,
        watching: Vec<([u8; 32], String)>,
    },
    /// Reliable, host to each client: a message for the players to read.
    Announce { text: String },
    /// Reliable, host to each client, between races: the level to drive on (`hash` as in
    /// [`Message::Start`]). A client that joins is sent the latest.
    Track { track: u32, hash: u64 },
    /// Reliable. The hash of the sim after `tick`, to compare with the peer's.
    Checksum { epoch: u32, tick: Tick, hash: u64 },
    /// Reliable. The sim as it is at the start of `tick`. The bytes are the game's to make and
    /// read: this crate does not know what the sim holds.
    Snapshot { tick: Tick, state: Vec<u8> },
    /// Unreliable, between any two connected players, every [`PING_SECONDS`]. `stamp` is the
    /// sender's clock when it sent this, which only the sender reads: the answer repeats it.
    Ping { stamp: u64 },
    /// Unreliable: the answer to the [`Message::Ping`] with the same `stamp`, which gives the
    /// sender the round trip.
    Pong { stamp: u64 },
    /// Reliable, host to each client: the player in `slot` left the race. Every client answers
    /// with [`Message::Have`], and the host then settles where the player's pads end. (The
    /// clients that have the most of them hand them to the ones that lack some.)
    Gone { epoch: u32, slot: u8 },
    /// Reliable, client to host: the answer to [`Message::Gone`]. These are the latest pads of
    /// the player that left that the client holds, the first being that of `first`. After this
    /// the client takes no more pads of that player.
    Have {
        epoch: u32,
        slot: u8,
        first: Tick,
        pads: Vec<Pad>,
    },
    /// Reliable, host to each client: the pads of the player in `slot` are these (the most any
    /// client had, from `first` on), and it plays a neutral pad after them. Everyone ends up
    /// with the same pads for it, however many it had when it left.
    Left {
        epoch: u32,
        slot: u8,
        first: Tick,
        pads: Vec<Pad>,
    },
    /// Reliable, host to each client: the race of `epoch` is over, without a winner. The lobby
    /// goes on, and the host can start the same players again. (A stop that comes after the
    /// next race's [`Message::Restart`] is of the race before, and does not count.)
    Stop { epoch: u32 },
}

/// How often a player pings the ones it is connected to.
pub const PING_SECONDS: u64 = 2;

impl Message {
    pub fn to_bytes(&self) -> Vec<u8> {
        bitcode::encode(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Message, bitcode::Error> {
        bitcode::decode(bytes)
    }
}

/// Our own inputs, kept until the peer confirms them.
#[derive(Debug, Default)]
pub struct InputSender {
    /// The tick of `pads[0]`.
    base: Tick,
    pads: VecDeque<Pad>,
}

impl InputSender {
    /// Records the input of the next tick (the first call is tick 0).
    pub fn push(&mut self, pad: Pad) {
        self.pads.push_back(pad);
    }

    /// The tick the next [`push`](Self::push) is for.
    pub fn tick(&self) -> Tick {
        self.base + self.pads.len() as Tick
    }

    /// The packet to send now. `peer_has` is what the peer last confirmed of ours (its
    /// [`InputReceiver::next`]); `ours_to_ack` is [`InputReceiver::next`] of the peer's inputs.
    pub fn packet(&mut self, peer_has: Tick, ours_to_ack: Tick) -> Inputs {
        // The peer cannot have more than we have made, whatever it claims.
        let peer_has = peer_has.min(self.tick());
        let drop = (peer_has.saturating_sub(self.base) as usize).min(self.pads.len());
        self.pads.drain(..drop);
        self.base += drop as Tick;
        // A peer that stays silent must not make the packets grow. It will need a snapshot.
        let excess = self.pads.len().saturating_sub(MAX_WINDOW);
        self.pads.drain(..excess);
        self.base += excess as Tick;
        Inputs {
            epoch: 0,
            first: self.base,
            pads: self.pads.iter().copied().collect(),
            ack: ours_to_ack,
            lead: self.tick() as i32 - ours_to_ack as i32,
        }
    }
}

/// The inputs of one remote player, as far as they have come in.
#[derive(Debug, Default)]
pub struct InputReceiver {
    /// The tick of `pads[0]`.
    base: Tick,
    pads: VecDeque<Pad>,
}

/// A packet starts after the last input we have: the ticks in between are lost for good (the
/// sender stopped repeating them), and only a snapshot can bring us up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gap {
    pub have_until: Tick,
    pub packet_starts: Tick,
}

impl InputReceiver {
    /// The first tick we have no input for. Also what to confirm to the sender.
    pub fn next(&self) -> Tick {
        self.base + self.pads.len() as Tick
    }

    /// Takes in a packet, in whatever order and however often it arrives.
    pub fn accept(&mut self, packet: &Inputs) -> Result<(), Gap> {
        let next = self.next();
        if packet.first > next {
            return Err(Gap {
                have_until: next,
                packet_starts: packet.first,
            });
        }
        let known = (next - packet.first) as usize;
        self.pads.extend(packet.pads.iter().skip(known));
        Ok(())
    }

    /// The input of `tick`, if it has come in and was not [`forget`](Self::forget)ten.
    pub fn get(&self, tick: Tick) -> Option<Pad> {
        let at = tick.checked_sub(self.base)? as usize;
        self.pads.get(at).copied()
    }

    /// Lets go of the inputs before `tick`, once the sim is past them.
    pub fn forget(&mut self, tick: Tick) {
        let drop = (tick.saturating_sub(self.base) as usize).min(self.pads.len());
        self.pads.drain(..drop);
        self.base += drop as Tick;
    }

    /// Starts over at `tick`, with a snapshot of the sim there.
    pub fn restart(&mut self, tick: Tick) {
        self.pads.clear();
        self.base = tick;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(tick: Tick) -> Pad {
        Pad {
            held: tick,
            ..Pad::default()
        }
    }

    /// Both ends for one direction, with a link that drops the packets `lose` says to.
    fn run(ticks: Tick, lose: impl Fn(Tick) -> bool) -> (InputSender, InputReceiver) {
        let (mut sender, mut receiver) = (InputSender::default(), InputReceiver::default());
        for tick in 0..ticks {
            sender.push(pad(tick));
            // The ack is as old as the last packet that came back; here, the receiver's state
            // before this tick's packet, which is the slowest an ack can be.
            let peer_has = receiver.next();
            let packet = sender.packet(peer_has, 0);
            let packet = Message::from_bytes(&Message::Inputs(packet).to_bytes()).unwrap();
            let Message::Inputs(packet) = packet else {
                unreachable!()
            };
            if !lose(tick) {
                receiver.accept(&packet).unwrap();
            }
        }
        (sender, receiver)
    }

    #[test]
    fn messages_round_trip() {
        for message in [
            Message::Hello {
                character: 9,
                seq: 3,
                build: 1,
                nick: "Penta".into(),
                spectator: true,
            },
            Message::Nick {
                seq: 4,
                nick: "Penta Penguin".into(),
            },
            Message::Reject { host: 2, yours: 1 },
            Message::Gone { epoch: 1, slot: 2 },
            Message::Have {
                epoch: 1,
                slot: 2,
                first: 40,
                pads: vec![pad(40), pad(41)],
            },
            Message::Left {
                epoch: 1,
                slot: 2,
                first: 40,
                pads: vec![pad(40), pad(41)],
            },
            Message::Start {
                epoch: 1,
                slot: 2,
                characters: vec![0, 5, 15],
                track: 4,
                hash: 0,
                roster: vec![[1; 32], [2; 32], [3; 32]],
            },
            Message::Ready,
            Message::Go,
            Message::Restart {
                epoch: 2,
                track: 7,
                hash: 0xfeed,
            },
            Message::Want {
                track: 0x8000_0001,
                hash: 77,
            },
            Message::Blob {
                track: 0x8000_0001,
                hash: 77,
                bytes: vec![9; 5],
            },
            Message::NoBlob {
                track: 0x8000_0001,
                hash: 77,
            },
            Message::Lobby {
                players: vec![([1; 32], 4, String::new()), ([2; 32], 15, "Ripper".into())],
                watching: vec![([3; 32], "Eyes".into())],
            },
            Message::Announce {
                text: "Restarting in 5".into(),
            },
            Message::Track {
                track: 0x8000_0001,
                hash: 77,
            },
            Message::Inputs(Inputs {
                epoch: 1,
                first: 7,
                pads: vec![pad(1), Pad::default()],
                ack: 5,
                lead: -3,
            }),
            Message::Checksum {
                epoch: 3,
                tick: 90,
                hash: u64::MAX,
            },
            Message::Snapshot {
                tick: 30,
                state: vec![1, 2, 3],
            },
        ] {
            assert_eq!(Message::from_bytes(&message.to_bytes()).unwrap(), message);
        }
    }

    #[test]
    fn a_packet_of_the_whole_window_fits_in_a_datagram() {
        let mut sender = InputSender::default();
        for tick in 0..MAX_WINDOW as Tick {
            // The worst case for the encoding: every field changes, with big values.
            sender.push(Pad {
                held: 0xffff - tick,
                lx: tick as u8 ^ 0x5a,
                ly: 0xff - tick as u8,
                ry: tick as u8 | 0x81,
            });
        }
        let bytes = Message::Inputs(sender.packet(0, 12345)).to_bytes();
        assert!(bytes.len() < 1100, "{} bytes", bytes.len());
    }

    #[test]
    fn every_input_arrives_in_order() {
        let (_, receiver) = run(20, |_| false);
        assert_eq!(receiver.next(), 20);
        assert!((0..20).all(|t| receiver.get(t) == Some(pad(t))));
    }

    #[test]
    fn lost_packets_are_covered_by_the_next() {
        // Every other packet, and then a run of five, vanish.
        let (_, receiver) = run(40, |t| t % 2 == 0 || (20..25).contains(&t));
        assert_eq!(receiver.next(), 40);
        assert!((0..40).all(|t| receiver.get(t) == Some(pad(t))));
    }

    #[test]
    fn a_packet_that_comes_twice_or_late_changes_nothing() {
        let mut receiver = InputReceiver::default();
        let packet = |first, n| Inputs {
            epoch: 0,
            first,
            pads: (first..first + n).map(pad).collect(),
            ack: 0,
            lead: 0,
        };
        receiver.accept(&packet(0, 5)).unwrap();
        receiver.accept(&packet(3, 5)).unwrap();
        receiver.accept(&packet(0, 5)).unwrap();
        receiver.accept(&packet(3, 5)).unwrap();
        assert_eq!(receiver.next(), 8);
        assert!((0..8).all(|t| receiver.get(t) == Some(pad(t))));
    }

    #[test]
    fn a_silent_peer_leaves_a_gap_that_needs_a_snapshot() {
        let mut sender = InputSender::default();
        for tick in 0..200 {
            sender.push(pad(tick));
        }
        // The peer has confirmed nothing for 200 ticks.
        let packet = sender.packet(0, 0);
        assert_eq!(packet.pads.len(), MAX_WINDOW);
        assert_eq!(packet.first, 200 - MAX_WINDOW as Tick);

        let mut receiver = InputReceiver::default();
        assert_eq!(
            receiver.accept(&packet),
            Err(Gap {
                have_until: 0,
                packet_starts: 200 - MAX_WINDOW as Tick
            })
        );
        // The snapshot is of the tick the packet starts at; the rest of the packet then fits.
        let start = 200 - MAX_WINDOW as Tick;
        receiver.restart(start);
        receiver.accept(&packet).unwrap();
        assert_eq!(receiver.next(), 200);
        assert_eq!(receiver.get(start - 1), None);
        assert_eq!(receiver.get(start + 10), Some(pad(start + 10)));
    }

    #[test]
    fn confirmed_inputs_are_not_sent_again() {
        let mut sender = InputSender::default();
        for tick in 0..10 {
            sender.push(pad(tick));
        }
        let packet = sender.packet(6, 3);
        assert_eq!((packet.first, packet.pads.len(), packet.ack), (6, 4, 3));
        // A peer cannot confirm what was never made.
        assert_eq!(sender.packet(1000, 0).pads.len(), 0);
    }

    #[test]
    fn forgotten_inputs_are_gone() {
        let (_, mut receiver) = run(10, |_| false);
        receiver.forget(4);
        assert_eq!((receiver.get(3), receiver.get(4)), (None, Some(pad(4))));
        assert_eq!(receiver.next(), 10);
    }
}
