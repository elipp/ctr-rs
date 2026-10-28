# Bugs

## Maybe

- **A dropped red beaker did not exist on the tick after the throw.** The test
  `a_red_beaker_that_spins_out_a_kart_makes_it_rain_on_it` (`crates/race/tests/weapons.rs`)
  waited for the potion with `while mines().first().is_some_and(|m| m.state != Ground)`, which
  stops at once when there is no mine yet, and then panicked on `mines()[0]` (index out of bounds,
  len 0). It was fixed in the test (`is_none_or`), but it is not known why it started failing, or
  whether it ever passed: the item-table commit (`b5d1e20`) does not touch the fire path. If the
  throw really spawns the mine a tick late, check that `WEAPON_FIRE_REQUEST` (from
  `item::request_fire`) is handled on the tick it is set, as the neighbouring potion test's
  `debug_roll` and wait loop assume. Not checked on the commit before it.

- The few usf boost pads in the game (Claude claims it's implemented)
- Red beaker effect should slow down and give the bouncing animation. Maybe bigger cloud too

## Confirmed

- other karts' bubbles drawn over their karts (ours has its own camera, `SHIELD_LAYER`)

## Not bugs

- **Wumpa glow missing at 10 wumpa, and a shield that pops by itself.** Both are the end of the
  race: crossing the finish line on the last lap sets `RACE_FINISHED`, which pops the shield
  (`vehicle::shield::tick`, as `RB_ShieldDark_ThTick_Grow` does) and hides the glows
  (`client/srcweapon.rs`, `shine`). Free drive used to have the three-lap limit too; it now has none
  (`race::FREE_DRIVE_LAPS`), so only an online race ends.
