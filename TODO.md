## Unimplemented
- the particles of a shattering potion (`emSet_PotionShatter`)
- the Tiger Temple door that a shattering potion opens (`RB_Potion_OnShatter_TeethSearch`, and the open-door check in `RB_Potion_ThTick_InAir`)
- the icon of a damaged driver turning red or green (`damageColorTimer`), for every weapon
- the other uses of a TNT crate on the head (`instTntRecv`): half the speed, the fire button ignored, the driving animation held at its middle frame
- driving under water doesn't render the cart "water-tinted" and cause bubbles to come out of the exhaust
- the shield bubble should look egg-shaped, not a squashed half-sphere: check the model's proportions and the pulse/grow scales (`vehicle::shield`, `client/srcshield.rs`)
- the shield bomb's `numTimesMissileLaunched` stat
- The crate depth bias is too much for Papy's Pyramid
- The flower doesn't hurt on Papu's Pyramid
- The spider is buggy and doesn't hurt 
- Blizzard bluff seals don't move / hurt
- Spectate only mode
- submerged in the mud in tiny arena looks off

  While reading base_speed I noticed one more thing: C takes half of damaged_speed when instTntRecv is set (TNT on the head), and our subtract starts at 0. I haven't checked whether that's handled
  elsewhere.

## Maps
- a title for a map (the game's names are in the language file, which a patch does not change): `Map.name` is the patch's file name
- a map's music and reverb: it has none of its own, and gets no song and the reverb of the levels past the table. `ctr-map import` knows the track it was made over, whose song pack and reverb it could keep
- `ctr-map import` only reads the one-player level file and its VRAM; the 2/4-player and relic files a patch also changes are ignored

## Sound
The sound effects and the engines play through the `howl` crate: an SPU as `ctr-native` emulates it (24 voices, ADPCM decoded as they play, the Gaussian interpolation, the ADSR envelopes) under the HOWL channels, so sounds can be changed and stopped as they play and the ones that loop do (`OtherFX_Modify`, `OtherFX_Stop1/2`, `OtherFX_RecycleNew/Mute`, `EngineAudio_*`, `howl_PauseAudio`). Our kart's ground, skid, wall-rub and turbo loops play (`vehicle::emitter_sound`, `turbo`), and the boost, the mask grab's whistle, the voice lines (`howl::voiceline`: a grunt, or a line of XA), the pause menu's, the lap's, the win's and the item roulette's. So do the level's objects (crates, wumpa fruit, armadillos, fireballs, minecarts, barrels: `instance::Tick::sounds` and `loops`) and the weapons (launches, flight loops, explosions, detonations, a TNT on a head), heard from where they are. The sounds that echo (a kart in a tunnel) go through the SPU's reverb (`howl::reverb`), in the level's mode (`Howl::set_level_reverb`). The music is the level's song pack (`howl::music`): its song from the green light on, a mask's while ours is out, quieter for "FINAL LAP!" and 20 faster after it, as the states of the race say (`howl::state`). The XA (`loader::xa`: `XA/ENG.XNF`, the tracks' sectors and their ADPCM; `howl::xa`: the CD's input of the SPU and the native path of `CDSYS`) plays the voice lines and the "FINAL LAP!" and race-end jingles, which the states wait for; the viewer reads the tracks on a thread. `cargo run -p howl --example render` renders sounds to a WAV (`r2` before them for a level's reverb, `m0` for a level's song, `xm6` for an XA). What is left, in order:
1. the rest of the XA: the wrong-way lines of Aku Aku and Uka Uka (`Voiceline_Update`, `boolCanPlayWrongWaySFX`, which need the wrong-way state of the race), N. Tropy's lines at a time trial's end, and the loudness the talking masks' mouths follow (`XA_MaxSampleVal`). Not listened to in a race yet
2. the level's ambience (`Level_AmbientSound`: `levAmbientSound` along `SpawnType2` 5 and 6, `Level_RandomFX`), and the rain of the red beaker's cloud 0x82 (`DropRain`)
3. the volume options (`howl_VolumeSet`, `howl_ModeSet`)

Online, a one-shot comes from the newest tick only, so a rollback does not take it back. The loops are sent again every tick (`Effect::Recycle`), and the engines set from the karts' state, so they put themselves right.

## Chores
- remove any hex constants that aren't used as a bitmask; the particular values serve no purpose in the port

## To check
- the screen flash of a burst: whether retail's `animFrame == 1` lands on the tick our effect's first advance does, and whether the quad sits under the HUD as ours does

## Future?
Why the fruit and weapon crates (and wumpa fruit) stay in the level's BSP, and what could move out of it:
- A kart touches a level crate in its swept-sphere search (`vehicle::search::moved_player_search`). Their hitboxes are class 4 (`CLASS_TOUCH`, `ctr-query inst LEVEL --name crate` shows it), so a touch is only recorded and the kart is not bumped. But the search does one extra iteration after a recorded hitbox, so the kart moves in two segments on that tick. Taking crates out of the BSP would change those numbers. Kart-ground/wall and kart-to-kart physics must stay exact, so level crates keep their BSP hitboxes.
- Crate behavior (`CrateBox`, rewards, blasts) has no C reference test, only Rust ones. The C oracles cover the sweep against hitboxes (`coll/tests/reference.rs`) and `ScrubImpact`, with `NoInstances`, so never a crate's `LInC`.
- The plan: `Objects::new` copies each crate's hitbox shape out of the level (centre 48 above the instance, radius 76; fruit 64 and 64). Blasts then look at `Objects` instead of walking the BSP (`coll::instance::instances_in_reach`), which changes the order crates break in (and so the RNG draws of fruit crates). A crate added at runtime is marked as not in the BSP and gets its own distance test against karts and weapons.
- Not done: the viewer builds crate models once at load (`props.rs`, `CrateBody(i)`), so a runtime crate also needs an entity spawned.
- the crates' shape in `Objects` (for a `/debug add crate` in free drive, and blasts that need no BSP walk). See "Crates and the BSP" below
