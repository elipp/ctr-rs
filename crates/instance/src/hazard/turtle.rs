//! `RB_Turtle_*`: the turtles of Mystery Caves' water. A turtle waits with its shell up, sinks
//! (its animation), waits underwater, and rises again. A kart that hits one at speed is thrown
//! into a jump: a low one while it is fully down, a high one otherwise (`RB_Turtle_LInC`, which
//! the kart's collision search calls when it reaches the turtle's hitbox: [`Turtle::linc`]).
//!
//! The turtles are put on different cycles by the digit their names end in. They are silent: the
//! sound retail gives a turtle (0x7d) waits for its timer to reach a value that the wait ends
//! before.

use loader::lev::InstDef;
use vehicle::{Driver, forced_jump};

use crate::{Context, Instance, Tick, Touch};

/// `elapsedTimeMS` of a tick.
const TICK_MS: i32 = 32;
/// How long a turtle waits (`timer`) before its animation moves.
const WAIT_MS: i32 = 0x3c0;
/// The most `timer` can hold. A wait ends before it gets there, so it is only kept for fidelity.
const TIMER_MAX: i32 = 0x5a0;
/// `RB_Turtle_LInC` ignores a kart this slow (`speedApprox`).
const MIN_SPEED: i32 = 0x1400;

/// `TURTLE_DIRECTION_*`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Rising,
    Falling,
}

/// `TURTLE_STATE_*`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    FullyDown,
    NotFullyDown,
}

/// `struct Turtle` and the `animFrame` of its `Instance`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turtle {
    pub direction: Direction,
    timer: i32,
    pub state: State,
    /// `animFrame` of the sinking animation (animation 0).
    pub anim_frame: i16,
}

impl Turtle {
    /// `RB_Turtle_LInB`. `frames` is the number of frames of the animation. The game takes the
    /// turtle's number from the last character of its name, whatever that is: odd ones start
    /// fully down, about to rise, the others fully up.
    pub fn new(def: &InstDef, frames: i32) -> Turtle {
        let id = def
            .name
            .bytes()
            .last()
            .map_or(0, |c| c as i32 - b'0' as i32);
        if id & 1 != 0 {
            Turtle {
                direction: Direction::Rising,
                timer: 0,
                state: State::FullyDown,
                anim_frame: frames as i16,
            }
        } else {
            Turtle {
                direction: Direction::Falling,
                timer: 0,
                state: State::NotFullyDown,
                anim_frame: 0,
            }
        }
    }

    /// `RB_Turtle_ThTick`, one game tick. `frames` is the number of frames of the animation.
    pub fn tick(&mut self, frames: i32) {
        if self.direction != Direction::Rising {
            if self.timer < WAIT_MS {
                self.timer = (self.timer + TICK_MS).min(TIMER_MAX);
            } else if (self.anim_frame as i32) + 1 < frames {
                self.anim_frame += 1;
            } else {
                self.direction = Direction::Rising;
                self.timer = 0;
                self.state = State::FullyDown;
            }
        } else if self.timer < WAIT_MS {
            self.timer = (self.timer + TICK_MS).min(TIMER_MAX);
        } else {
            // Raising the shell changes the jump height before the animation ends.
            self.state = State::NotFullyDown;
            if self.anim_frame as i32 - 1 > 0 {
                self.anim_frame -= 1;
            } else {
                self.direction = Direction::Falling;
                self.timer = 0;
            }
        }
    }

    /// `RB_Turtle_LInC`, when a kart touches the hitbox: a fast one is thrown into a jump and goes
    /// on (2); a slow one collides with the turtle like a wall (1).
    pub fn linc(&self, driver: &mut Driver) -> i32 {
        match self.forced_jump(driver.speed_approx as i32) {
            Some(jump) => {
                driver.forced_jump_type = jump;
                2
            }
            None => 1,
        }
    }

    /// The `ForcedJumpType` a kart of this `speed_approx` is given, or `None` when it is too slow
    /// to be thrown.
    fn forced_jump(&self, speed_approx: i32) -> Option<u8> {
        (speed_approx.abs() > MIN_SPEED).then_some(match self.state {
            State::FullyDown => forced_jump::LOW,
            State::NotFullyDown => forced_jump::HIGH,
        })
    }
}

impl Instance for Turtle {
    fn th_tick(&mut self, cx: &Context, _out: &mut Tick) {
        self.tick(cx.frames[0]);
    }

    fn touch(&mut self, hit: &mut Touch) -> i32 {
        self.linc(hit.driver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str) -> InstDef {
        InstDef {
            name: name.into(),
            model: 0,
            model_name: "turtle".into(),
            scale: Default::default(),
            color_rgba: 0,
            flags: 0,
            pos: Default::default(),
            rot: Default::default(),
            model_id: 0,
        }
    }

    #[test]
    fn the_last_character_of_the_name_picks_the_cycle() {
        // "turtle#4": even, fully up; "turtle#3": odd, fully down; "turtle": 'e' is odd.
        assert_eq!(Turtle::new(&def("turtle#4"), 11).state, State::NotFullyDown);
        assert_eq!(
            Turtle::new(&def("turtle#10"), 11).state,
            State::NotFullyDown
        );
        let down = Turtle::new(&def("turtle#3"), 11);
        assert_eq!((down.state, down.anim_frame), (State::FullyDown, 11));
        assert_eq!(Turtle::new(&def("turtle"), 11).state, State::FullyDown);
    }

    #[test]
    fn a_turtle_waits_sinks_waits_and_rises() {
        let mut t = Turtle::new(&def("turtle#4"), 11);
        // 30 ticks of waiting (960 ms), then a frame a tick down to the last one.
        for _ in 0..30 {
            t.tick(11);
        }
        assert_eq!(t.anim_frame, 0);
        for frame in 1..=10 {
            t.tick(11);
            assert_eq!(t.anim_frame, frame);
            assert_eq!(t.state, State::NotFullyDown);
        }
        // The tick after the last frame is down.
        t.tick(11);
        assert_eq!(
            (t.direction, t.state),
            (Direction::Rising, State::FullyDown)
        );
        // Waits fully down, then the state changes before the shell is all the way up.
        for _ in 0..30 {
            t.tick(11);
            assert_eq!(t.state, State::FullyDown);
        }
        t.tick(11);
        assert_eq!((t.state, t.anim_frame), (State::NotFullyDown, 9));
        while t.direction == Direction::Rising {
            t.tick(11);
        }
        // `animFrame - 1 > 0` stops it at frame 1.
        assert_eq!((t.direction, t.anim_frame), (Direction::Falling, 1));
    }

    #[test]
    fn the_jump_is_low_only_when_fully_down_and_needs_speed() {
        let up = Turtle::new(&def("turtle#4"), 11);
        let down = Turtle::new(&def("turtle#3"), 11);
        assert_eq!(up.forced_jump(0x1401), Some(forced_jump::HIGH));
        assert_eq!(down.forced_jump(-0x1401), Some(forced_jump::LOW));
        assert_eq!(up.forced_jump(0x1400), None);
    }
}
