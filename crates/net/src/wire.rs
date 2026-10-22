//! What clients say to each other, and the bookkeeping that makes the inputs survive loss.
//!
//! Inputs go in unreliable datagrams, one packet per tick, and each packet repeats every input
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

/// The most ticks one packet repeats. Past this (about 2 s) the peer needs a snapshot.
pub const MAX_WINDOW: usize = 60;

/// The most frames one [`Frames`] packet carries. The sim never runs further ahead of the
/// slowest player than the input delay, so this is plenty, and it keeps eight players' pads
/// in one datagram.
pub const FRAMES_WINDOW: usize = 20;

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
        Pad { held: 0, lx: 0x80, ly: 0x80, ry: 0x80 }
    }
}

/// The pads of every player in one tick, by slot.
pub type Frame = Vec<Pad>;

/// A client's own inputs, as the unreliable datagram of a tick carries them.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Inputs {
    /// The tick of `pads[0]`.
    pub first: Tick,
    pub pads: Vec<Pad>,
    /// The client has all [`Frames`] below this tick.
    pub ack: Tick,
}

/// The host's answer: the ticks it has everyone's input for.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct Frames {
    /// The tick of `frames[0]`.
    pub first: Tick,
    pub frames: Vec<Frame>,
    /// The host has all of the client's [`Inputs`] below this tick.
    pub ack: Tick,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum Message {
    /// Reliable, client to host, right after joining: the character (`GAME_CHARACTER_IDS`
    /// order) this client drives. Characters have different stats, so every sim needs to know
    /// whose kart is whose.
    /// `seq` counts the client's Hellos from 0: they go on streams of their own, which can
    /// arrive in any order, and the host keeps the one with the highest.
    Hello { character: u8, seq: u32 },
    /// Reliable, host to each client: the race starts, and this is tick 0. `slot` is the
    /// client's player among the `players`; `characters` is the character of each slot, and
    /// `track` the `LevelId` the race is on.
    Start { slot: u8, characters: Vec<u8>, track: u32 },
    /// Unreliable, client to host.
    Inputs(Inputs),
    /// Unreliable, host to client.
    Frames(Frames),
    /// Reliable. The hash of the sim after `tick`, to compare with the peer's.
    Checksum { tick: Tick, hash: u64 },
    /// Reliable. The sim as it is at the start of `tick`. The bytes are the game's to make and
    /// read: this crate does not know what the sim holds.
    Snapshot { tick: Tick, state: Vec<u8> },
}

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
        Inputs { first: self.base, pads: self.pads.iter().copied().collect(), ack: ours_to_ack }
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
            return Err(Gap { have_until: next, packet_starts: packet.first });
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
        Pad { held: tick, ..Pad::default() }
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
            let Message::Inputs(packet) = packet else { unreachable!() };
            if !lose(tick) {
                receiver.accept(&packet).unwrap();
            }
        }
        (sender, receiver)
    }

    #[test]
    fn messages_round_trip() {
        for message in [
            Message::Hello { character: 9, seq: 3 },
            Message::Start { slot: 2, characters: vec![0, 5, 15], track: 4 },
            Message::Inputs(Inputs { first: 7, pads: vec![pad(1), Pad::default()], ack: 5 }),
            Message::Frames(Frames {
                first: 4,
                frames: vec![vec![pad(1), pad(2)], vec![pad(3), pad(4)]],
                ack: 9,
            }),
            Message::Checksum { tick: 90, hash: u64::MAX },
            Message::Snapshot { tick: 30, state: vec![1, 2, 3] },
        ] {
            assert_eq!(Message::from_bytes(&message.to_bytes()).unwrap(), message);
        }
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
        let packet = |first, n| Inputs { first, pads: (first..first + n).map(pad).collect(), ack: 0 };
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
            Err(Gap { have_until: 0, packet_starts: 140 })
        );
        // The snapshot is of tick 140; the rest of the packet then fits.
        receiver.restart(140);
        receiver.accept(&packet).unwrap();
        assert_eq!(receiver.next(), 200);
        assert_eq!(receiver.get(139), None);
        assert_eq!(receiver.get(150), Some(pad(150)));
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
