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

## Confirmed

### NYI:
The crate depth bias is too much for Papy's Pyramid
The flower doesn't hurt on Papu's Pyramid
The spider is buggy and doesn't hurt 
The few usf boost pads in the game
Rollers in N.Gin Labs
Red beaker effect should slow down and give the bouncing animation. Maybe bigger cloud too
Bubble doesn't hurt when colliding
With three players, changing track causes DESYNC
Blizzard bluff seals don't move / hurt

Spectate only mode

submerged in the mud in tiny arena looks off

/race restart doesn't include any pending clients (not a bug?)

Show lobby members while waiting

## Not bugs

- **Wumpa glow missing at 10 wumpa, and a shield that pops by itself.** Both are the end of the
  race: crossing the finish line on the last lap sets `RACE_FINISHED`, which pops the shield
  (`vehicle::shield::tick`, as `RB_ShieldDark_ThTick_Grow` does) and hides the glows
  (`view/src/weapon.rs`, `shine`). Free drive used to have the three-lap limit too; it now has none
  (`race::FREE_DRIVE_LAPS`), so only an online race ends.
