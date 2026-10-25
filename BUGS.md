# Bugs

## Maybe

None.

## Confirmed

None yet.

## Not bugs

- **Wumpa glow missing at 10 wumpa, and a shield that pops by itself.** Both are the end of the
  race: crossing the finish line on the last lap sets `RACE_FINISHED`, which pops the shield
  (`vehicle::shield::tick`, as `RB_ShieldDark_ThTick_Grow` does) and hides the glows
  (`view/src/weapon.rs`, `shine`). Free drive used to have the three-lap limit too; it now has none
  (`race::FREE_DRIVE_LAPS`), so only an online race ends.
