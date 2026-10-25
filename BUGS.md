# Bugs

## Maybe

Seen once, not reproduced, cause unknown.

- **Wumpa glow missing after reaching 10 a second time.** Reach 10 wumpa (the glow shows), lose
  some, then reach 10 again from pickups: the glow behind the counter does not render. Seen with
  `/debug-set-wumpa 10` first and pickups after; not tried with a real hit. Reading
  `shine` (`crates/view/src/weapon.rs`) and the wumpa sim (`crates/vehicle/src/wumpa.rs`,
  `spin.rs`) found nothing that would latch it off. Unknown: whether the weapon glow does the
  same, and whether the node is hidden or shown but not drawn.

## Confirmed

None yet.
