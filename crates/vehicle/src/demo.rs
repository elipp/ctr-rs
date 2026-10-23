//! Demos: the controller state of every game tick, recorded and played back.
//!
//! The sim is deterministic (every tick is 32 ms, see [`crate::Env`]), so the inputs alone
//! reproduce a drive from the starting grid. Only the ticks on which the input changed are
//! stored, which is also what an online simulation would send. `buttons_tapped` is not
//! stored: it follows from the held buttons of the tick before.
//!
//! The text form is a header line, `demo 1 <character>`, then one `<tick> <held> <lx> <ly>
//! <ry>` line per change (held in hex).

use crate::Input;

const HEADER: &str = "demo 1";

/// One change of the controller: from `tick` on, until the next change, this is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub tick: u32,
    pub buttons_held: u32,
    pub stick_lx: u8,
    pub stick_ly: u8,
    pub stick_ry: u8,
}

impl Change {
    fn of(tick: u32, input: &Input) -> Change {
        Change {
            tick,
            buttons_held: input.buttons_held,
            stick_lx: input.stick_lx,
            stick_ly: input.stick_ly,
            stick_ry: input.stick_ry,
        }
    }

    fn same_as(&self, input: &Input) -> bool {
        Change::of(self.tick, input) == *self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Demo {
    pub character: usize,
    pub changes: Vec<Change>,
    /// The ticks the demo lasts: the one after the last recorded.
    pub length: u32,
}

impl Demo {
    pub fn new(character: usize) -> Demo {
        Demo {
            character,
            ..Demo::default()
        }
    }

    /// Records the input of `tick`. Ticks must come in order, from 0.
    pub fn record(&mut self, tick: u32, input: &Input) {
        debug_assert!(
            tick == self.length,
            "tick {tick} recorded after {}",
            self.length
        );
        if !self.changes.last().is_some_and(|last| last.same_as(input)) {
            self.changes.push(Change::of(tick, input));
        }
        self.length = tick + 1;
    }

    /// The input of `tick`, or `None` once the demo is over. `previous_held` is the held
    /// buttons of the tick before (0 for tick 0), for the taps.
    pub fn input(&self, tick: u32, previous_held: u32) -> Option<Input> {
        if tick >= self.length {
            return None;
        }
        let at = self.changes.partition_point(|c| c.tick <= tick);
        Some(match at.checked_sub(1).map(|i| self.changes[i]) {
            Some(c) => Input {
                buttons_held: c.buttons_held,
                buttons_tapped: c.buttons_held & !previous_held,
                stick_lx: c.stick_lx,
                stick_ly: c.stick_ly,
                stick_ry: c.stick_ry,
            },
            None => Input::default(),
        })
    }

    pub fn to_text(&self) -> String {
        let mut text = format!("{HEADER} {}\n", self.character);
        for c in &self.changes {
            text += &format!(
                "{} {:x} {} {} {}\n",
                c.tick, c.buttons_held, c.stick_lx, c.stick_ly, c.stick_ry
            );
        }
        text += &format!("end {}\n", self.length);
        text
    }

    pub fn from_text(text: &str) -> Result<Demo, String> {
        let mut lines = text.lines();
        let head = lines.next().ok_or("empty demo")?;
        let character = head
            .strip_prefix(HEADER)
            .and_then(|rest| rest.trim().parse().ok())
            .ok_or_else(|| format!("not a demo: '{head}'"))?;
        let mut demo = Demo::new(character);
        let mut ended = false;
        for line in lines.filter(|l| !l.trim().is_empty()) {
            let bad = || format!("bad demo line '{line}'");
            let mut words = line.split_whitespace();
            if let Some(("end", length)) = line.split_once(' ') {
                demo.length = length.trim().parse().map_err(|_| bad())?;
                ended = true;
                continue;
            }
            let tick = words.next().and_then(|w| w.parse().ok()).ok_or_else(bad)?;
            let buttons_held = words
                .next()
                .and_then(|w| u32::from_str_radix(w, 16).ok())
                .ok_or_else(bad)?;
            let mut stick = || words.next().and_then(|w| w.parse().ok()).ok_or_else(bad);
            let (stick_lx, stick_ly, stick_ry) = (stick()?, stick()?, stick()?);
            if demo.changes.last().is_some_and(|last| last.tick >= tick) {
                return Err(format!("demo ticks out of order at '{line}'"));
            }
            demo.changes.push(Change {
                tick,
                buttons_held,
                stick_lx,
                stick_ly,
                stick_ry,
            });
        }
        if !ended {
            return Err("demo has no end line (cut short?)".into());
        }
        Ok(demo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buttons;

    fn input(held: u32, lx: u8) -> Input {
        Input {
            buttons_held: held,
            buttons_tapped: 0,
            stick_lx: lx,
            ..Input::default()
        }
    }

    fn sample() -> Demo {
        let mut demo = Demo::new(3);
        let script = [
            (0, 0x80),
            (buttons::CROSS, 0x80),
            (buttons::CROSS, 0x80),
            (buttons::CROSS, 0),
            (buttons::CROSS | buttons::CIRCLE, 0),
            (0, 0xff),
        ];
        for (tick, &(held, lx)) in script.iter().enumerate() {
            demo.record(tick as u32, &input(held, lx));
        }
        demo
    }

    #[test]
    fn only_changes_are_stored() {
        let demo = sample();
        assert_eq!(demo.length, 6);
        assert_eq!(
            demo.changes.iter().map(|c| c.tick).collect::<Vec<_>>(),
            [0, 1, 3, 4, 5]
        );
    }

    #[test]
    fn plays_back_what_was_recorded() {
        let demo = sample();
        let held: Vec<u32> = (0..6)
            .map(|t| demo.input(t, 0).unwrap().buttons_held)
            .collect();
        assert_eq!(
            held,
            [
                0,
                buttons::CROSS,
                buttons::CROSS,
                buttons::CROSS,
                buttons::CROSS | buttons::CIRCLE,
                0
            ]
        );
        assert_eq!(demo.input(6, 0), None);
    }

    #[test]
    fn taps_follow_from_the_held_buttons() {
        let demo = sample();
        let mut held = 0;
        let tapped: Vec<u32> = (0..6)
            .map(|t| {
                let input = demo.input(t, held).unwrap();
                held = input.buttons_held;
                input.buttons_tapped
            })
            .collect();
        assert_eq!(tapped, [0, buttons::CROSS, 0, 0, buttons::CIRCLE, 0]);
    }

    #[test]
    fn text_round_trips() {
        let demo = sample();
        assert_eq!(Demo::from_text(&demo.to_text()), Ok(demo));
    }

    #[test]
    fn a_cut_short_file_is_refused() {
        let text = sample().to_text();
        let cut = &text[..text.rfind("end").unwrap()];
        assert!(Demo::from_text(cut).is_err());
        assert!(Demo::from_text("hello").is_err());
    }
}
