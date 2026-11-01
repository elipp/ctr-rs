# Unimplemented
- the other uses of a TNT crate on the head (`instTntRecv`): the fire button ignored, the driving animation held at its middle frame
- the shield bubble should look egg-shaped, not a squashed half-sphere: check the model's proportions and the pulse/grow scales (`vehicle::shield`, `client/srcshield.rs`)
- the shield bomb's `numTimesMissileLaunched` stat
- the bombs and missiles look for mines oldest first (`weapon::fly`), but a mine's thread goes to the front of the `MINE` bucket, so `RB_Hazard_CollideWithBucket` finds the newest first, as the hazards do (`race::set_off_mines`)
- missile exhaust
- flattening when jumping on top of another player?

## Sound
The sound effects and the engines play through the `howl` crate: an SPU as `ctr-native` emulates it (24 voices, ADPCM decoded as they play, the Gaussian interpolation, the ADSR envelopes) under the HOWL channels, so sounds can be changed and stopped as they play and the ones that loop do (`OtherFX_Modify`, `OtherFX_Stop1/2`, `OtherFX_RecycleNew/Mute`, `EngineAudio_*`, `howl_PauseAudio`). Our kart's ground, skid, wall-rub and turbo loops play (`vehicle::emitter_sound`, `turbo`), and the boost, the mask grab's whistle, the voice lines (`howl::voiceline`: a grunt, or a line of XA), the pause menu's, the lap's, the win's and the item roulette's. So do the level's objects (crates, wumpa fruit, armadillos, fireballs, minecarts, barrels: `instance::Tick::sounds` and `loops`) and the weapons (launches, flight loops, explosions, detonations, a TNT on a head), heard from where they are. The sounds that echo (a kart in a tunnel) go through the SPU's reverb (`howl::reverb`), in the level's mode (`Howl::set_level_reverb`). The music is the level's song pack (`howl::music`): its song from the green light on, a mask's while ours is out, quieter for "FINAL LAP!" and 20 faster after it, as the states of the race say (`howl::state`). The XA (`loader::xa`: `XA/ENG.XNF`, the tracks' sectors and their ADPCM; `howl::xa`: the CD's input of the SPU and the native path of `CDSYS`) plays the voice lines and the "FINAL LAP!" and race-end jingles, which the states wait for; the viewer reads the tracks on a thread. `cargo run -p howl --example render` renders sounds to a WAV (`r2` before them for a level's reverb, `m0` for a level's song, `xm6` for an XA). What is left, in order:
1. the rest of the XA: N. Tropy's lines at a time trial's end, and the loudness the talking masks' mouths follow (`XA_MaxSampleVal`). Not listened to in a race yet
2. the level's ambience (`Level_AmbientSound`: `levAmbientSound` along `SpawnType2` 5 and 6, `Level_RandomFX`), and the rain of the red beaker's cloud 0x82 (`DropRain`)
3. the volume options (`howl_VolumeSet`, `howl_ModeSet`)
4. the other players' karts, heard in space (an intentional divergence; the skid and the turbo's roar of the others, their one-shot sounds and their voice lines are heard by distance now, see `docs/notes/DIVERGENCE.md`). What is left: the falloff (300 to 2500 units, `KART_FULL_WITHIN` and `KART_HEARD_WITHIN`) is a guess to tune by ear, perhaps by the bounce's loudness (`Effect::DriverCrashing::volume`). The peer loops want a cap on how many play (the SPU has 24 voices, and the engines and our own use some: the nearest few, as the engines do), and the ground and wall rub if they should be heard. Karts in a tunnel keep the echo of their `ENGINE_ECHO` flag. Nothing here touches the sim: it is all in the listener (`client::sound`)
- maybe don't reset the music when changing character

## Net
- auto-kick suspicious peers: count what a well-behaved client never does per guest (a duplicate `Want`, which `transfers::Verdict::Duplicate` already tells apart; a `Want` for a map the host did not provide; a `Blob` nobody asked for) and kick with `KICKED` past a threshold. Log each as a rate-limited `tracing::warn!` with `fmt_short()` ids (the `net` crate has no logging yet), and say why in the kick. A guest that never reads its stream also holds a transfer slot until QUIC's idle timeout: put a deadline on `send_reliably` for maps
- host migration in a race: a guest stops the race when the host leaves during it, and the lobby carries on (`race_on` in `session.rs`), but the race itself is lost. The heir would have to take over the host's part in it (settling a leaver's pads with `Departure`, the `Ready`s and `Go`) without the sims diverging.

## Chores
- remove any hex constants that aren't used as a bitmask; the particular values serve no purpose in the port

## Tech debt
- a kart is many entities: the body (the root, with `KartMotion`), the wheels, shadow, smoke, sparks, tire marks, mask and shield, which sit at the top level (they draw in world or camera space) and are tied to it by `Attached(kart)` (despawn, and `kart::hide_parts` for a kart that is not drawn). The wheels are tied by their own `kart` field, as `reap` despawns whatever is `Attached` to a kart without a slot, which the grid display's is. One parent with an identity transform that holds the body and every part would let Bevy's visibility and despawning do all of it, and give a single thing to spawn and to hide (`kart::KartBuilder::spawn` and the `spawn` of each part)

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

## Annoyances
- Fix the checkpoint split thing to be a toast

# Bugs

## Confirmed
- sewer speedway rollers / probably the rolling stone doesn't pop beakers/tnt
- other karts' shield bubbles drawn over their karts (ours has its own camera, `SHIELD_LAYER`)

## Maybe
- backing up and turning on N.Gin (some other drivers too?) looks weird (?)
- The few usf boost pads in the game (probably work)

## Not a bug?
- /race restart doesn't include any pending clients. Instead, one must /race stop and /race start
- `ctr-map import` only reads the one-player level file and its VRAM; the 2/4-player and relic files a patch also changes are ignored
- hot air skyway mega drop after missing the "main road" from the shortcut from the left side ("levypallo päätien kautta") - CONFIRMED NOT-A-BUG: retail does this too


