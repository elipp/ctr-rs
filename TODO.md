## Unimplemented
- the red beaker's rain cloud (`RB_RainCloud_Init`): a kart hit by one only spins out and flashes
- the particles of a shattering potion (`emSet_PotionShatter`)
- the Tiger Temple door that a shattering potion opens (`RB_Potion_OnShatter_TeethSearch`, and the open-door check in `RB_Potion_ThTick_InAir`)
- the icon of a damaged driver turning red or green (`damageColorTimer`), for every weapon
- the other uses of a TNT crate on the head (`instTntRecv`): half the speed, the fire button ignored, the driving animation held at its middle frame
- driving under water doesn't render the cart "water-tinted" and cause bubbles to come out of the exhaust

## Chores
- remove any hex constants that aren't used as a bitmask; the particular values serve no purpose

## To check
- the screen flash of a burst: whether retail's `animFrame == 1` lands on the tick our effect's first advance does, and whether the quad sits under the HUD as ours does
