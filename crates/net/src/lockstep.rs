//! The race as every client runs it: the sim advances to a tick only once it has the pads of all
//! players for it, so all sims see the same inputs and, being deterministic, the same race.
//!
//! The host collects the clients' [`Inputs`], completes a [`Frame`] when it has every player's pad
//! for a tick, and sends the frames back in [`Frames`]. There is nothing to roll back, and nothing
//! to predict: a pad is applied [`INPUT_DELAY`] ticks after it was made, which gives the network
//! that long to deliver it before the sim has to wait.
//!
//! The game's part, each tick `t`: wait for frame `t`, play it, then push the local pad
//! for tick `t + INPUT_DELAY`.

use std::collections::VecDeque;

use crate::wire::{FRAMES_WINDOW, Frame, Frames, Gap, InputReceiver, InputSender, Inputs, Pad, Tick};

/// Ticks between making a pad and playing it. 3 ticks are 100 ms.
pub const INPUT_DELAY: Tick = 3;

/// The sender of a player's own pads. Ticks `0..INPUT_DELAY` have nothing pressed.
pub fn local_sender() -> InputSender {
    let mut sender = InputSender::default();
    for _ in 0..INPUT_DELAY {
        sender.push(Pad::default());
    }
    sender
}

/// The host's side. Slot 0 is the host's own player.
#[derive(Debug)]
pub struct Host {
    inputs: Vec<InputReceiver>,
    /// The frames each client has (index 0, the host, is unused).
    acked: Vec<Tick>,
    /// The tick of `frames[0]`: those before it every client has.
    base: Tick,
    frames: VecDeque<Frame>,
}

impl Host {
    pub fn new(players: usize) -> Host {
        let mut host = Host {
            inputs: (0..players).map(|_| InputReceiver::default()).collect(),
            acked: vec![0; players],
            base: 0,
            frames: VecDeque::new(),
        };
        for _ in 0..INPUT_DELAY {
            host.push_local(Pad::default());
        }
        host
    }

    /// The host's own pad for the next tick it has none for.
    pub fn push_local(&mut self, pad: Pad) {
        let first = self.inputs[0].next();
        let _ = self.inputs[0].accept(&Inputs { first, pads: vec![pad], ack: 0 });
    }

    /// Takes in a client's packet, in whatever order and however often it arrives.
    pub fn accept(&mut self, slot: usize, packet: &Inputs) -> Result<(), Gap> {
        self.acked[slot] = self.acked[slot].max(packet.ack);
        self.inputs[slot].accept(packet)
    }

    /// The frames that are complete now, which the host's game plays as well.
    pub fn advance(&mut self) -> Vec<(Tick, Frame)> {
        let mut done = Vec::new();
        loop {
            let tick = self.base + self.frames.len() as Tick;
            let Some(frame) = self.inputs.iter().map(|inputs| inputs.get(tick)).collect::<Option<Frame>>()
            else {
                break;
            };
            for inputs in &mut self.inputs {
                inputs.forget(tick + 1);
            }
            self.frames.push_back(frame.clone());
            done.push((tick, frame));
        }
        // Those every client has are not needed again.
        let end = self.base + self.frames.len() as Tick;
        let known = self.acked.iter().skip(1).copied().min().unwrap_or(end);
        let drop = (known.saturating_sub(self.base) as usize).min(self.frames.len());
        self.frames.drain(..drop);
        self.base += drop as Tick;
        done
    }

    /// What to send the client of `slot`: the frames it does not have yet, oldest first.
    pub fn packet(&self, slot: usize) -> Frames {
        let first = self.acked[slot].max(self.base);
        let skip = (first - self.base) as usize;
        Frames {
            first,
            frames: self.frames.iter().skip(skip).take(FRAMES_WINDOW).cloned().collect(),
            ack: self.inputs[slot].next(),
        }
    }
}

/// A client's side of the frames.
#[derive(Debug, Default)]
pub struct FrameReceiver {
    next: Tick,
}

impl FrameReceiver {
    /// The first tick without a frame. Also what to confirm to the host.
    pub fn next(&self) -> Tick {
        self.next
    }

    /// Takes in a packet, and gives the frames that were new.
    pub fn accept(&mut self, packet: &Frames) -> Result<Vec<(Tick, Frame)>, Gap> {
        if packet.first > self.next {
            return Err(Gap { have_until: self.next, packet_starts: packet.first });
        }
        let skip = (self.next - packet.first) as usize;
        let new: Vec<_> = packet
            .frames
            .iter()
            .skip(skip)
            .enumerate()
            .map(|(i, frame)| (self.next + i as Tick, frame.clone()))
            .collect();
        self.next += new.len() as Tick;
        Ok(new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Message;

    fn pad(player: u32, tick: Tick) -> Pad {
        Pad { held: player << 16 | tick, ..Pad::default() }
    }

    /// Through the wire format, and lost if `lose` says so.
    fn send(message: Message, lose: bool) -> Option<Message> {
        let bytes = message.to_bytes();
        assert!(bytes.len() < 1100, "{} bytes", bytes.len());
        (!lose).then(|| Message::from_bytes(&bytes).unwrap())
    }

    /// A host and clients, each playing the frames it gets and making its pad for `INPUT_DELAY` ticks
    /// later, over a link that loses the packets `lose(round, slot)` says. Returns what each saw.
    fn race(players: usize, rounds: u32, lose: impl Fn(u32, usize) -> bool) -> Vec<Vec<Frame>> {
        let mut host = Host::new(players);
        let mut seen = vec![Vec::<Frame>::new(); players];
        let mut senders: Vec<_> = (0..players).map(|_| local_sender()).collect();
        let mut receivers: Vec<_> = (0..players).map(|_| FrameReceiver::default()).collect();
        let mut queued: Vec<Vec<Frame>> = vec![Vec::new(); players];
        let mut host_has = vec![0; players];

        for round in 0..rounds {
            // The games: each plays its next frame if it has it.
            for slot in 0..players {
                let played = seen[slot].len() as Tick;
                if let Some(frame) = queued[slot].get(played as usize) {
                    seen[slot].push(frame.clone());
                    let pad = pad(slot as u32, played + INPUT_DELAY);
                    if slot == 0 {
                        host.push_local(pad);
                    } else {
                        senders[slot].push(pad);
                    }
                }
            }
            // The clients send, every round, as the real ones do on a timer.
            for slot in 1..players {
                let packet = senders[slot].packet(host_has[slot], receivers[slot].next());
                if let Some(Message::Inputs(packet)) = send(Message::Inputs(packet), lose(round, slot)) {
                    let _ = host.accept(slot, &packet);
                }
            }
            for (tick, frame) in host.advance() {
                assert_eq!(queued[0].len() as Tick, tick);
                queued[0].push(frame);
            }
            // The host answers.
            for slot in 1..players {
                let packet = host.packet(slot);
                let lose = lose(round, slot + 100);
                if let Some(Message::Frames(packet)) = send(Message::Frames(packet), lose) {
                    host_has[slot] = host_has[slot].max(packet.ack);
                    for (tick, frame) in receivers[slot].accept(&packet).unwrap() {
                        assert_eq!(queued[slot].len() as Tick, tick);
                        queued[slot].push(frame);
                    }
                }
            }
        }
        seen
    }

    #[test]
    fn everyone_plays_the_same_race() {
        let seen = race(3, 60, |_, _| false);
        assert!(seen.iter().all(|s| s == &seen[0]));
        assert!(seen[0].len() > 40, "{} ticks", seen[0].len());
        // Each player's pad is the one made INPUT_DELAY ticks before, blank at first.
        for (tick, frame) in seen[0].iter().enumerate() {
            for (slot, got) in frame.iter().enumerate() {
                let want = tick.checked_sub(INPUT_DELAY as usize).map(|t| pad(slot as u32, t as Tick + INPUT_DELAY));
                assert_eq!(*got, want.unwrap_or_default(), "tick {tick}, slot {slot}");
            }
        }
    }

    #[test]
    fn packet_loss_slows_the_race_but_does_not_change_it() {
        let clean = race(3, 120, |_, _| false);
        let lossy = race(3, 120, |round, link| (round * 7 + link as u32 * 3) % 4 == 0);
        assert!(lossy.iter().all(|s| s == &lossy[0]));
        let n = lossy[0].len();
        assert!(n > 30, "{n} ticks");
        assert_eq!(lossy[0][..], clean[0][..n]);
    }

    #[test]
    fn a_race_of_one_runs_on_its_own() {
        let seen = race(1, 30, |_, _| false);
        assert!(seen[0].len() > 20);
    }
}
