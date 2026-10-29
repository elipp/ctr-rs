# Unimplemented
- the other uses of a TNT crate on the head (`instTntRecv`): the fire button ignored, the driving animation held at its middle frame
- driving under water doesn't render the cart "water-tinted" and cause bubbles to come out of the exhaust
- the shield bubble should look egg-shaped, not a squashed half-sphere: check the model's proportions and the pulse/grow scales (`vehicle::shield`, `client/srcshield.rs`)
- the shield bomb's `numTimesMissileLaunched` stat
- the bombs and missiles look for mines oldest first (`weapon::fly`), but a mine's thread goes to the front of the `MINE` bucket, so `RB_Hazard_CollideWithBucket` finds the newest first, as the hazards do (`race::set_off_mines`)
- submerged in the mud in tiny arena looks off
- blizzard bluff: reflection of weapon crates on ice

## Maps
- a title for a map (the game's names are in the language file, which a patch does not change): `Map.name` is the patch's file name
- a map's music and reverb: it has none of its own, and gets no song and the reverb of the levels past the table. `ctr-map import` knows the track it was made over, whose song pack and reverb it could keep

## Sound
The sound effects and the engines play through the `howl` crate: an SPU as `ctr-native` emulates it (24 voices, ADPCM decoded as they play, the Gaussian interpolation, the ADSR envelopes) under the HOWL channels, so sounds can be changed and stopped as they play and the ones that loop do (`OtherFX_Modify`, `OtherFX_Stop1/2`, `OtherFX_RecycleNew/Mute`, `EngineAudio_*`, `howl_PauseAudio`). Our kart's ground, skid, wall-rub and turbo loops play (`vehicle::emitter_sound`, `turbo`), and the boost, the mask grab's whistle, the voice lines (`howl::voiceline`: a grunt, or a line of XA), the pause menu's, the lap's, the win's and the item roulette's. So do the level's objects (crates, wumpa fruit, armadillos, fireballs, minecarts, barrels: `instance::Tick::sounds` and `loops`) and the weapons (launches, flight loops, explosions, detonations, a TNT on a head), heard from where they are. The sounds that echo (a kart in a tunnel) go through the SPU's reverb (`howl::reverb`), in the level's mode (`Howl::set_level_reverb`). The music is the level's song pack (`howl::music`): its song from the green light on, a mask's while ours is out, quieter for "FINAL LAP!" and 20 faster after it, as the states of the race say (`howl::state`). The XA (`loader::xa`: `XA/ENG.XNF`, the tracks' sectors and their ADPCM; `howl::xa`: the CD's input of the SPU and the native path of `CDSYS`) plays the voice lines and the "FINAL LAP!" and race-end jingles, which the states wait for; the viewer reads the tracks on a thread. `cargo run -p howl --example render` renders sounds to a WAV (`r2` before them for a level's reverb, `m0` for a level's song, `xm6` for an XA). What is left, in order:
1. the rest of the XA: N. Tropy's lines at a time trial's end, and the loudness the talking masks' mouths follow (`XA_MaxSampleVal`). Not listened to in a race yet
2. the level's ambience (`Level_AmbientSound`: `levAmbientSound` along `SpawnType2` 5 and 6, `Level_RandomFX`), and the rain of the red beaker's cloud 0x82 (`DropRain`)
3. the volume options (`howl_VolumeSet`, `howl_ModeSet`)

## Net
- auto-kick suspicious peers: count what a well-behaved client never does per guest (a duplicate `Want`, which `transfers::Verdict::Duplicate` already tells apart; a `Want` for a map the host did not provide; a `Blob` nobody asked for) and kick with `KICKED` past a threshold. Log each as a rate-limited `tracing::warn!` with `fmt_short()` ids (the `net` crate has no logging yet), and say why in the kick. A guest that never reads its stream also holds a transfer slot until QUIC's idle timeout: put a deadline on `send_reliably` for maps

## Chores
- remove any hex constants that aren't used as a bitmask; the particular values serve no purpose in the port

## Tech debt
- a kart is many entities: the body (the root, with `KartMotion`), the wheels, shadow, smoke, sparks, tire marks, mask and shield, which sit at the top level (they draw in world or camera space) and are tied to it by `Attached(kart)` (despawn, and `kart::hide_parts` for a kart that is not drawn). The wheels are tied by their own `kart` field, as `reap` despawns whatever is `Attached` to a kart without a slot, which the grid display's is. One parent with an identity transform that holds the body and every part would let Bevy's visibility and despawning do all of it, and give a single thing to spawn and to hide (`kart::KartBuilder::spawn` and the `spawn` of each part)

## To check
- the heat shimmer of the flame jets (`heat.rs`, `heat.wgsl`, `Torch_Main`): it is a post-processing pass over the main camera's picture, so it warps what is in front of a lens too (retail's lens only covers what was drawn before it, by depth) (it looks right in the game)

## Tick oracle coverage
`vehicle/tests/tick.rs` compares the drivers with ctr-native, and has the real `LInC` of the wumpa fruit only (`has_linc` in `tests/reference/tick.c`); the other models' hitboxes give 1 on both sides. What would still change kart positions:
- Fruit crates (`RB_CrateFruit_LInC`, `RB_Crate.c`): 5 to 8 wumpa raise the top speed (`linear::base_speed`), and the break draws from `sdata->randomNumber`, which a tumble's direction draws from too. Check the regrow timing: `race` ticks the objects before the karts, retail's STATIC bucket after the drivers' stages, so the port may let a crate pay out again a tick late. Needs `INSTANCE_BirthWithThread_Stack` stubbed (the explosion) and the crate's LInC in `has_linc` and `data.MetaDataModels`. The crate itself never moves a kart: its hitbox is class 4, passed through whatever the `LInC` returns.
- The turtles (`RB_Turtle_LInC`) and the Tiger Temple teeth (`RB_Teeth_LInC`): their `LInC` result decides whether the kart is bounced (`ScrubImpact`) or passes, so they move karts directly. The turtles' and the door's own state has to be ticked on both sides.
- Weapon crates (`RB_CrateWeapon_LInC`): the item, and when, decides turbo boosts and the hits on other karts. Retail runs the roulette in `VehPhysProc_Driving_Update` (the UPDATE stage, its pick drawing from the random number), `race` runs `item::tick` after `tick_all`; moving it into the sim's update stage comes first. The pick needs `VehPhysGeneral_SetHeldItem` and its rank tables on the C side.

## Future?
Why the fruit and weapon crates (and wumpa fruit) stay in the level's BSP, and what could move out of it:
- A kart touches a level crate in its swept-sphere search (`vehicle::search::moved_player_search`). Their hitboxes are class 4 (`CLASS_TOUCH`, `ctr-query inst LEVEL --name crate` shows it), so a touch is only recorded and the kart is not bumped. But the search does one extra iteration after a recorded hitbox, so the kart moves in two segments on that tick. Taking crates out of the BSP would change those numbers. Kart-ground/wall and kart-to-kart physics must stay exact, so level crates keep their BSP hitboxes.
- Crate behavior (`CrateBox`, rewards, blasts) has no C reference test, only Rust ones. The C oracles cover the sweep against hitboxes (`coll/tests/reference.rs`) and `ScrubImpact`, and the tick oracle the wumpa fruit's `LInC`, but never a crate's.
- The plan: `Objects::new` copies each crate's hitbox shape out of the level (centre 48 above the instance, radius 76; fruit 64 and 64). Blasts then look at `Objects` instead of walking the BSP (`coll::instance::instances_in_reach`), which changes the order crates break in (and so the RNG draws of fruit crates). A crate added at runtime is marked as not in the BSP and gets its own distance test against karts and weapons.
- Not done: the viewer builds crate models once at load (`props.rs`, `CrateBody(i)`), so a runtime crate also needs an entity spawned.
- the crates' shape in `Objects` (for a `/debug add crate` in free drive, and blasts that need no BSP walk). See "Crates and the BSP" below

# Bugs

## Confirmed

- With three players, changing track causes DESYNC (sometimes!!)
- **Bevy reports a command on a despawned entity (heisenbug).** Seen on a peer after the host quit
  mid-race (2026-10-26), not reproduced since:
  `bevy_ecs::error::handler: Encountered an error in command <name>: Entity despawned: The entity
  with ID 931v4 is invalid; its index now has generation 5`, then `WARN bevy_ecs::relationship:
  The <name>(948v4) relationship on entity 951v5 relates to an entity that does not exist. The
  invalid relationship has been removed.` The names are hidden: run with `--features debug`
  (forwards to `bevy/debug`) to get the command and the component. Not fixed: turning the
  `despawn()` calls into `try_despawn()` would only silence it.
- other karts' bubbles drawn over their karts (ours has its own camera, `SHIELD_LAYER`)

## Maybe

- The few usf boost pads in the game

- **A color band near the top of the sky on Crash Cove.** The sky dome has open holes, filled by
  the level's clear colors (`Level::clear_colors`, `CAM_ClearScreen`); we fill them with the top
  color only. Looking up, a hard edge shows between the clear color `(144, 159, 255)` and the
  dome below it. Not understood: every vertex of the dome's top rim (y 1301) is exactly the top
  clear color, so the data has no such edge (`ctr-query sky 3` prints the rim). Retail's sky top
  cannot be seen, so there is nothing to compare with. Converting the dome's vertex colors from
  sRGB (they were read as linear) changed the shade but not the edge. Splitting the clear color
  by camera pitch (top/bottom halves, as `CAM_ClearScreen` does) would not fix it either; it
  would only show the bottom color (black on Crash Cove) under the horizon. Next step if it
  matters: draw the dome in a flat debug color to see which triangles make the edge.

## Not a bug?
- /race restart doesn't include any pending clients. Instead, one must /race stop and /race start
- `ctr-map import` only reads the one-player level file and its VRAM; the 2/4-player and relic files a patch also changes are ignored
