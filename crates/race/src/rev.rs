//! The learned bots' engine rev, at the start of a race and as the mask lowers a kart back onto
//! the track (`vehicle::stuck`): the gas is not the policy's then, but planned here. The policy
//! never learned the rev (nothing of it is in its observation, and no training episode starts on
//! the grid), and there is nothing to learn: the rev is a matter of timing alone, against a
//! moment known in advance ([`Race::rev_ticks_left`]).
//!
//! The rev gives the big boost if the boost meter was pumped up (twice: the gas held until the
//! rev is over the class's acceleration, and let go until it falls back under it) and the rev is
//! over the acceleration as it ends, without having been held at the top too long. A pumped
//! meter falls back once the rev runs out (some 16 ticks after the last pump), so the pumps come
//! late, and the end of the rev needs a hold of 3 to some 11 ticks.
//!
//! A [`Plan`] is when the pumps start and how long the last hold is, both in ticks before the
//! end. [`Rev`] simulates the rev ([`stuck::rev_preview`], the sim's own code) for every plan,
//! takes the one furthest inside the ones that give the boost it goes for, and then slips by up
//! to a few ticks, as by its [`Skill`]: how far, and how often it goes for the big boost at all,
//! is the difficulty's.

use vehicle::bot::difficulty_level::{EASY, HARD, MEDIUM};
use vehicle::buttons;
use vehicle::stats::Stats;
use vehicle::stuck::{self, RevBoost, RevEngine, charge, lockout};

use crate::rng::Rng;
use crate::{Pad, Race};

/// How well a bot revs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Skill {
    /// The chance, out of 100, that a rev goes for the big boost (when there is time to pump);
    /// otherwise it goes for the small one.
    pub big_percent: u32,
    /// The most ticks the start of the pumps and that of the last hold are each off by, early or
    /// late.
    pub slip: u32,
}

/// The skill of each difficulty (`vehicle::bot::difficulty_level`), the lowest first: a
/// difficulty has that of the last one at or below it (below all, the first's).
pub const SKILLS: [(i16, Skill); 3] = [
    (
        EASY,
        Skill {
            big_percent: 30,
            slip: 5,
        },
    ),
    (
        MEDIUM,
        Skill {
            big_percent: 65,
            slip: 3,
        },
    ),
    (
        HARD,
        Skill {
            big_percent: 90,
            slip: 1,
        },
    ),
];

impl Skill {
    /// The skill of `difficulty`: see [`SKILLS`].
    pub const fn of(difficulty: i16) -> Skill {
        let mut i = SKILLS.len() - 1;
        while i > 0 && SKILLS[i].0 > difficulty {
            i -= 1;
        }
        SKILLS[i].1
    }
}

/// The rev is planned once this few ticks of it are left; until then the gas is let go. (Pumps
/// any earlier would have run out by the end.)
const PLAN_AHEAD: u32 = 48;

/// The longest last hold planned (an over-rev comes long before).
const HOLD_MAX: u32 = 24;

/// A tick, in milliseconds (`vehicle::Env::elapsed_time_ms`).
const TICK_MS: i32 = 32;

/// When to hold the gas, by the ticks of the rev left (counting the one the gas is for).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// From here on the boost meter is pumped (until it is full); 0 for no pumps.
    pub pump_from: u32,
    /// From here on the gas is held, to the end.
    pub hold: u32,
}

impl Plan {
    /// Whether to hold the gas with `left` ticks of the rev `r` (of a kart with `stats`) left.
    pub fn gas(self, r: &RevEngine, stats: &Stats, left: u32) -> bool {
        let accel = stats.accel_speed_class_stat as i32;
        let full = accel + stats.sacred_fire_speed as i32;
        if left <= self.hold {
            true
        } else if left > self.pump_from || r.lockout_flags & lockout::ALL != 0 {
            false
        } else if r.charge_state == charge::ACTIVE {
            // A pump: held until over the acceleration.
            r.fire_level <= accel
        } else {
            // Let go: again once the cooldown (counted down before the rev, this tick) is over.
            r.release_cooldown_timer_ms <= TICK_MS as i16 && r.boost_meter < full
        }
    }

    /// The boost rev `r` gives on this plan, with `left` ticks of it left.
    pub fn outcome(self, r: &RevEngine, stats: &Stats, left: u32) -> RevBoost {
        let mut r = *r;
        for l in (1..=left).rev() {
            let gas = self.gas(&r, stats, l);
            stuck::rev_preview(&mut r, stats, gas, TICK_MS);
        }
        stuck::rev_boost(&r, stats)
    }
}

/// A bot's rev: its plan, made once a rev is near its end, and gone with the rev.
#[derive(Clone, Debug)]
pub struct Rev {
    pub skill: Skill,
    plan: Option<Plan>,
}

impl Rev {
    pub fn new(skill: Skill) -> Rev {
        Rev { skill, plan: None }
    }

    /// Whether to hold the gas on the next tick of `race` (the kart of `slot` revving its
    /// engine), or None if the kart is not revving, and the gas is the policy's.
    pub fn gas(&mut self, race: &Race, slot: usize, rng: &mut Rng) -> Option<bool> {
        let Some(left) = race.rev_ticks_left(slot) else {
            self.plan = None;
            return None;
        };
        if left == 0 {
            // The tick that ends the rev: off it goes.
            return Some(true);
        }
        let d = &race.karts[slot].driver;
        if self.plan.is_none() && left <= PLAN_AHEAD {
            self.plan = Some(plan(self.skill, &d.rev, &d.stats, left, rng));
        }
        Some(self.plan.is_some_and(|p| p.gas(&d.rev, &d.stats, left)))
    }
}

/// The plan for rev `r` of a kart with `stats`, `left` ticks of it left, by `skill`.
fn plan(skill: Skill, r: &RevEngine, stats: &Stats, left: u32, rng: &mut Rng) -> Plan {
    let holds = HOLD_MAX.min(left);
    // By pump start, then hold. (A pump start at or under the hold is no pump at all.)
    let grid: Vec<Vec<RevBoost>> = (0..=left)
        .map(|pump_from| {
            (0..=holds)
                .map(|hold| Plan { pump_from, hold }.outcome(r, stats, left))
                .collect()
        })
        .collect();
    let best = grid.iter().flatten().copied().max().unwrap_or(RevBoost::None);
    // The big boost if it goes for it, and has room to slip by its skill; else the small one,
    // which needs no pumps (unless there is no time to get it otherwise).
    let big = (best == RevBoost::Big && (rng.below(100) as u32) < skill.big_percent)
        .then(|| steadiest(&grid, RevBoost::Big, true))
        .flatten()
        .filter(|&(_, margin)| margin >= skill.slip as usize);
    let (ideal, pumps) = match big {
        Some((plan, _)) => (plan, true),
        None => {
            let aim = best.min(RevBoost::Small);
            let pumps = !grid[0].contains(&aim);
            let plan = steadiest(&grid, aim, pumps).map_or(
                Plan {
                    pump_from: 0,
                    hold: 0,
                },
                |(plan, _)| plan,
            );
            (plan, pumps)
        }
    };
    let mut slip = |at: u32| (at + rng.below(2 * skill.slip as usize + 1) as u32).saturating_sub(skill.slip);
    let hold = slip(ideal.hold);
    Plan {
        pump_from: if pumps { slip(ideal.pump_from) } else { 0 },
        hold,
    }
}

/// The plan of `grid` (outcomes by pump start, then hold) that gives `aim` the furthest from any
/// that does not, and that margin: the most ticks either moment can slip by, either way, still
/// giving it. Only the plans without pumps unless `pumps` (and then only the hold slips).
fn steadiest(grid: &[Vec<RevBoost>], aim: RevBoost, pumps: bool) -> Option<(Plan, usize)> {
    let hit = |p: usize, h: usize| grid.get(p).and_then(|row| row.get(h)) == Some(&aim);
    // How many steps from (p, h) by (dp, dh) still hit.
    let run = |p: usize, h: usize, dp: isize, dh: isize| {
        (1..)
            .take_while(|&k: &isize| {
                let (q, i) = (p as isize + dp * k, h as isize + dh * k);
                q >= 0 && i >= 0 && hit(q as usize, i as usize)
            })
            .count()
    };
    let rows = if pumps { 0..grid.len() } else { 0..1 };
    rows.flat_map(|p| (0..grid[p].len()).map(move |h| (p, h)))
        .filter(|&(p, h)| hit(p, h))
        .map(|(p, h)| {
            let mut margin = run(p, h, 0, -1).min(run(p, h, 0, 1));
            if pumps {
                margin = margin.min(run(p, h, -1, 0)).min(run(p, h, 1, 0));
            }
            (margin, p, h)
        })
        // The widest margin; of those, the latest pumps and the shortest hold.
        .max_by_key(|&(margin, p, h)| (margin, std::cmp::Reverse(p), std::cmp::Reverse(h)))
        .map(|(margin, p, h)| {
            let plan = Plan {
                pump_from: p as u32,
                hold: h as u32,
            };
            (plan, margin)
        })
}

/// `pad` with the gas held (Cross) or let go, and nothing else of the throttle: the rev's.
pub fn with_gas(mut pad: Pad, gas: bool) -> Pad {
    pad.held &= !(buttons::CROSS | buttons::SQUARE | buttons::DOWN);
    if gas {
        pad.held |= buttons::CROSS;
    }
    pad.ly = Pad::default().ly;
    pad
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats() -> Stats {
        Stats::for_class(vehicle::stats::EngineClass::Balanced)
    }

    fn fresh(stats: &Stats) -> RevEngine {
        RevEngine {
            boost_meter: stats.accel_speed_class_stat as i32 + stats.sacred_fire_speed as i32 / 3,
            ..RevEngine::default()
        }
    }

    #[test]
    fn a_steady_plan_gives_the_big_boost_with_time_to_pump() {
        let s = stats();
        let r = fresh(&s);
        let left = 40;
        let plan = plan(
            Skill {
                big_percent: 100,
                slip: 0,
            },
            &r,
            &s,
            left,
            &mut Rng::new(1),
        );
        assert!(plan.pump_from > plan.hold);
        assert_eq!(plan.outcome(&r, &s, left), RevBoost::Big);
    }

    #[test]
    fn without_time_to_pump_it_goes_for_the_small_boost() {
        let s = stats();
        let r = fresh(&s);
        let plan = plan(
            Skill {
                big_percent: 100,
                slip: 0,
            },
            &r,
            &s,
            10,
            &mut Rng::new(1),
        );
        assert_eq!(plan.pump_from, 0);
        assert_eq!(plan.outcome(&r, &s, 10), RevBoost::Small);
    }
}
