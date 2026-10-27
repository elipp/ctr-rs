## Unimplemented
- the particles of a shattering potion (`emSet_PotionShatter`)
- the Tiger Temple door that a shattering potion opens (`RB_Potion_OnShatter_TeethSearch`, and the open-door check in `RB_Potion_ThTick_InAir`)
- the icon of a damaged driver turning red or green (`damageColorTimer`), for every weapon
- the other uses of a TNT crate on the head (`instTntRecv`): half the speed, the fire button ignored, the driving animation held at its middle frame
- driving under water doesn't render the cart "water-tinted" and cause bubbles to come out of the exhaust
- the crates' shape in `Objects` (for a `/debug add crate` in free drive, and blasts that need no BSP walk). See "Crates and the BSP" below
- the shield bubble should look egg-shaped, not a squashed half-sphere: check the model's proportions and the pulse/grow scales (`vehicle::shield`, `view/src/shield.rs`)
- the shield bubble's hit on a kart it bumps (`SHIELD_FLAG_CRASH_ATTACK` in `VehPhysCrash_Attack`), and other karts' bubbles drawn over their karts (ours has its own camera, `SHIELD_LAYER`)
- the shield bomb's sounds (rolling loop 0x59, explosion 0x4c) and the `numTimesMissileLaunched` stat
- The crate depth bias is too much for Papy's Pyramid
- The flower doesn't hurt on Papu's Pyramid
- The spider is buggy and doesn't hurt 
- The few usf boost pads in the game
- Rollers in N.Gin Labs
- Red beaker effect should slow down and give the bouncing animation. Maybe bigger cloud too
- Bubble doesn't hurt when colliding
- With three players, changing track causes DESYNC
- Blizzard bluff seals don't move / hurt
- Spectate only mode
- submerged in the mud in tiny arena looks off
- /race restart doesn't include any pending clients (not a bug?)
- Show lobby members while waiting for race start

  While reading base_speed I noticed one more thing: C takes half of damaged_speed when instTntRecv is set (TNT on the head), and our subtract starts at 0. I haven't checked whether that's handled
  elsewhere.

## Maps
- a title for a map (the game's names are in the language file, which a patch does not change): `Map.name` is the patch's file name
- `ctr-map import` only reads the one-player level file and its VRAM; the 2/4-player and relic files a patch also changes are ignored

## Chores
- remove any hex constants that aren't used as a bitmask; the particular values serve no purpose in the port

## To check
- the screen flash of a burst: whether retail's `animFrame == 1` lands on the tick our effect's first advance does, and whether the quad sits under the HUD as ours does

## Crates and the BSP
Why the fruit and weapon crates (and wumpa fruit) stay in the level's BSP, and what could move out of it:
- A kart touches a level crate in its swept-sphere search (`vehicle::search::moved_player_search`). Their hitboxes are class 4 (`CLASS_TOUCH`, `ctr-query inst LEVEL --name crate` shows it), so a touch is only recorded and the kart is not bumped. But the search does one extra iteration after a recorded hitbox, so the kart moves in two segments on that tick. Taking crates out of the BSP would change those numbers. Kart-ground/wall and kart-to-kart physics must stay exact, so level crates keep their BSP hitboxes.
- Crate behavior (`CrateBox`, rewards, blasts) has no C reference test, only Rust ones. The C oracles cover the sweep against hitboxes (`coll/tests/reference.rs`) and `ScrubImpact`, with `NoInstances`, so never a crate's `LInC`.
- The plan: `Objects::new` copies each crate's hitbox shape out of the level (centre 48 above the instance, radius 76; fruit 64 and 64). Blasts then look at `Objects` instead of walking the BSP (`coll::instance::instances_in_reach`), which changes the order crates break in (and so the RNG draws of fruit crates). A crate added at runtime is marked as not in the BSP and gets its own distance test against karts and weapons.
- Not done: the viewer builds crate models once at load (`props.rs`, `CrateBody(i)`), so a runtime crate also needs an entity spawned.
