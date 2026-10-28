This file is for the places where the port _intentionally_ differs from the original: a different choice, a stand-in for something the engine cannot do, a retail quirk that is not reproduced. Something that is not implemented yet is not a divergence; it goes in `TODO.md`, and in the "Not ported" note of the module that would have it.

**Ground shadow (2026-10-20, uncommitted, NOT visually verified):** `vehicle::shadow` (cull + orientation), `render::shadow` (4 quads, atlas of global icons 0/1 `shadow1`/`shadow2`), `view::shadow` (subtractive mesh, `--drive` only). Known differences from `VehGroundShadow_Main`:
- Camera-distance fade: retail only fades in the "small screen" case (the push buffer's screen-distance value is <= 0x100, which I read as split-screen). One player uses the fixed colour 0x1f1f1f, and so do we. The fade is not ported.
- Texel blending: every visible texel is subtracted. Retail forces the tpage to subtractive, so only texels with the semi-transparency (STP) bit blend and the rest draw opaque. Not checked whether any shadow texels lack STP.
- Depth: relies on the 3-unit lift above the ground plus the depth test. On a steep or bumpy surface the flat shadow can dip into the ground. If it flickers or gets cut off, add a material depth bias.
- Standing kart: the non-driving view (kart on the grid) has no shadow yet.
- No ordering-table depth bias (`depthBiasNormal` / `depthBiasSecondary`). Bevy's depth test stands in for it.

**Exhaust fire (2026-10-20, NOT visually verified):** `vehicle::turbo` (state + `VehTurbo_ThTick`), `fire::increment` lights/restarts it, `view::flame` (8 models `turbo`..`turbo7` from the racer pack, per pipe, second mirrored and 3 frames ahead). Known differences:
- Depth fade: retail's `DEPTH_FADE` hides the fire when `alphaScale + 2*viewDepth > 0x1000`, and fades vertices near the camera. We hide it when `alphaScale >= 0x800` (a guess at a typical chase distance) and do no vertex fade.
- `DRAW_BILLBOARD` is not consumed anywhere in the reference, so the fire just follows the kart's orientation.
- Not modelled: sound, rumble, the driver's alpha halving while the fire burns, water split/reflection flags.
- `exhaust` still uses `reserves > 0` instead of retail's `PROC_SearchForModel` for "the flame exists".

**Skid marks (2026-10-20, NOT visually verified):** `vehicle::skid` (`VehEmitter_Skidmarks` ring of 8 ticks x 4 tires, plus the segment walk of `VehGroundSkids_Main`), `view::skid` (two meshes: dark = subtractive, light = quarter-add for `FORCE_SKIDMARKS` terrain; `tread` icon, global index 0x2f). Known differences:
- The marks' OT depth bias bytes (`depthBiasNormal/Secondary`) are not stored; the marks are lifted 1 game unit and get a material `depth_bias` of 4 instead.
- Brightness follows retail: one value per driver from the depth of the newest mark (full inside 0x180 units, halving per doubling beyond, gone below 0x10), halved per tick of age. We take the depth along the Bevy camera's forward axis in place of the GTE view matrix's z, and skip the per-vertex "depth > 0x20" test.
- Blend strength: the maths is retail's (dark = background - texel, light = background + texel/4), but Bevy blends in linear light and the PS1 in raw values, so the marks read lighter. `DARK_GAIN` / `LIGHT_GAIN` in `view/src/skid.rs` are tuned by eye.
- The wheelie now changes which tires skid, as in `VehEmitter_DriverMain`.
- The strips are at tick resolution; the kart is interpolated between ticks, the marks are not.

**Wumpa fruit spin and bob (2026-10-23, NOT visually verified):** `view::props::spin_fruit` turns the fruit around their own vertical axis (once per `FRUIT_SPIN_TICKS`, 60) and bobs them up and down (`FRUIT_BOB_HEIGHT` 10 level units each way, `FRUIT_BOB_TICKS` 90, a different phase per fruit). Added on purpose, tuned by eye. Whether retail moves them was not checked, so this may or may not differ from it; the reference (`RB_Fruit_*`) has to be read before anyone calls it a match. Known differences:
- The speeds, the axis and the height are ours. The angle and the height are functions of the tick (`Objects::ticks`) and the render's place in it, so they are cosmetic: peers agree and none of it is in the sim state.
- The hitbox does not bob: it stays at the placed position, up to 10 units off the drawn fruit at the top and bottom of a bob. Syncing it would put the height in the sim and make the pickup test follow it.
- It does not go back while ticks wait (a pause, a stalled network frame), but then it stands still until they go on.
- The other placed props (bobbing crates) still stand still.

**Screen flash of blasts (2026-10-25, NOT visually verified):** `view::flash` (the fade of `PushBuffer_FadeOneWindow`: `0x1fff` back to `0x1000` by `-0x88` a tick, drawn at `(value - 0x1000) >> 4` of 255), set by `weapon::Tick::flashes` (a kart a crate's blowup, a bomb/missile burst or a beaker reached; `RB_Burst_CollThread`) and by `RB_Burst_DrawAll`'s proximity test of the burst's second frame. Intentional differences:
- Blend: retail's quad is additive (`tpage 0xe1000a20`), ours is a white UI node laid over the view with an alpha. The same on dark pixels, a little less bright on light ones (white added to 0.5 is 1.0; white laid over at alpha 0.5 is 0.75). Bevy's UI nodes do not blend additively, and a UI material for it was not worth it yet.
- The proximity test uses the chase camera as of the last rendered frame (the tick runs before the camera moves) and takes `distanceToScreen` as `0x100`, the one-player value, in place of `distanceToScreen_PREV`.
- Only our own kart's screen flashes: there is one view.

**Potions (2026-10-25, NOT visually verified):** `weapon::mine` (`MineKind::GreenBeaker`/`RedBeaker`: dropped and thrown, `RB_Potion_ThTick_InAir`, the hit of `RB_GenericMine_ThTick`), `vehicle::item` (`Launch::Potion`), `race` (Up/Down pick the throw), `view::projectile` (the `beakergreen`/`beakerred` models, the shatter). Intentional differences:
- `RB_Potion_ThTick_InAir` goes on after it has shattered a potion and may shatter it again, which is a second, identical shatter on the same tick. We stop at the first.
- The shatter (`RB_Explosion_InitPotion`) always faces the camera, at half size. In retail the shockwave model faces it only once `RB_Blowup_Init` has set the billboard bit on the shared model header (a crate blowing up earlier in the race), so a potion's shatter may not face it.
- Keys: Up is E (or the D-pad), Down is S (or the D-pad); W is the gas, so it cannot be Up. Setting Down also makes a bomb's backward shot reachable from the keyboard, which it was not.

**Sound effects (2026-10-27, NOT listened to):** `loader::howl` (`KART.HWL`: the tables, the banks, the SPU's ADPCM), `view::sound` (`OtherFX_Play_LowLevel`'s volume, pan and length; `PlaySound3D`'s distance and angle), played from `drive_tick`: the traffic lights' beeps and every kart's `Effect::Sound` and `Effect::DriverCrashing`. Intentional differences:
- Everyone's karts are heard. Retail has no karts from other consoles: a player's sounds play in the middle at full volume. Ours still do; the other players' are heard from where they are, with `PlaySound3D`'s volume (full within 300 units, nothing from 9000) and pan, from our chase camera without the debug orbit.
- No banks: the samples of every sound effect and engine are loaded at start. Retail only has the samples of the banks the level loaded in sound RAM (a sound whose sample is missing does not play), so a sound that is silent on some level in retail may be heard here.
- The `howl` crate's SPU has no sound RAM: a voice plays a sample by its number, from its first block, and key on sets the loop's start back to it (the SPU keeps the repeat address of what played before, until a block marks a loop start). Past a sample's end it is silent, where the SPU plays on into whatever follows in sound RAM.
- The SPU's vertical blank (`howl_PlayAudio_Update`) comes every 735 samples the audio thread renders, not from the game's clock: a sound starts within a 60th of a second of audio after the tick that plays it, and the anti-spam's 10 frames are the game's ticks, as in retail. Bevy then resamples the 44.1 kHz output to the device's rate.
- The loudness is the console's (each voice's and the master volume in 32768ths), a quarter of what the first version of `view::sound` played at.
- The pan's angle is the float `atan2`, rounded to 512ths of a turn; retail takes `ratan2` and shifts it down, rounding towards minus.

**Engine sounds (2026-10-27, NOT listened to):** `vehicle::engine_sound` (`EngineSound_Player`; `EngineSound_AI` with the easing of `EngineSound_NearestAIs`; checked by reading against `game/HOWL/HOWL_Engine.c`, no oracle test yet), `view::sound` (`EngineAudio_InitOnce` at the start of a race, `EngineAudio_Recalculate` every tick), from `drive_tick`. Intentional differences:
- The other players' karts are heard as retail hears AIs: the two nearest the camera, on the AIs' two loops (`EngineFX` 0x10 and 0x11, every character the same), faded from full within 200 units to nothing from 2000, the pitch bent by how fast the distance changes, and the revs following the class's speed stat rather than the kart's fire speed (`EngineSound_AI` reads `const_AccelSpeed_ClassStat` there). Retail has no karts from other consoles.
- Our engine is the first player's loop of our class (`engine * 4`); retail adds `driverID`, which online goes past the four players a class has loops for.
- A far voice with no kart left to hear is silenced; retail leaves it as it was (it never loses an AI).

**The kart's loops, the boost (2026-10-27, NOT listened to):** `vehicle::emitter_sound` (the sounds of `VehEmitter_DriverMain`), `vehicle::turbo` (the roar of `VehTurbo_ThTick`), `Effect::FireAudio` from `fire::increment` (`VehFire_Audio`), the whistle of `VehStuckProc_MaskGrab_Animate`, and `view::sound` and `view::pause` for the lap, the win and the menu. Intentional differences:
- The boost's cooldown (`VehFire_AudioCooldown`) is the listener's, not the driver's: the reference's C oracle never makes a turbo fire, so it never plays the boost, and a driver whose cooldown ran would no longer match it. The listener counts it down every tick; the driver counts it down only while driving.
- Our kart alone has the loops of the ground, the skid, the wall rub and the turbo: retail gives them to the players' karts and not to the AIs, and the other karts are heard as retail hears AIs. The wall rub plays whatever the number of players (retail: one player on the console).
- The lap's sound plays for our kart's laps; retail plays it for the laps of the player in the lead. The win's plays as our kart finishes first; retail plays it from the music's last-lap state, and in a cup whatever the place.

**The sounds of the objects and the weapons (2026-10-27, NOT listened to):** `instance::Tick::sounds` and `loops` (the armadillo's turn, the fireball's whoosh, the minecart's and the barrel's rolling), `weapon::Tick::sounds` (a TNT crate hit, landing on a head and honking), and `view::sound` for the rest: the crates' breaks, the fruit picked up, the item roulette, the weapons' launches, flight loops, explosions and detonations. Intentional differences:
- A crate's break and a fruit picked up are heard when the listener sees them in the objects (a crate's count of breaks, a fruit gone), not where the sim breaks or takes them: a break that a rollback plays again is heard once, when it shows.
- The explosions' and the detonations' sounds are picked by the listener from what went off (a bomb, a missile or a shot shield; a TNT crate, or a Nitro crate or a beaker), as every path in retail picks the same.
- The item roulette's spin is our kart's alone; retail's (`ROLLING_ITEM`) spins while any player's on the console rolls.
- A launch is heard from the weapon just sent off, or from the kart for a mine that went off at once. The objects and weapons echo as our kart does, where retail takes the camera's quad block.
- As in retail, the snowball of Blizzard Bluff is silent (retail looks for `DYNAMIC_SNOWBALL`, the level has `TEMP_SNOWBALL`), and so are the turtles (their sound waits for a timer value the wait ends before).

**The echo of tunnels (2026-10-27, NOT listened to):** `howl::reverb` (the SPU's reverb unit of `platform/native_audio.c`: its ten modes, the 22.05 kHz work area and the filter in and out; the same output as the C, sample for sample, on a test signal in every mode), `Howl::set_reverb` and `set_level_reverb` (`SetReverbMode`, `Audio_SetReverbMode` with `data.reverbParams`, `data.reverbMode` and `reverbModeBossID`), set by `view::sound` as a track loads. Intentional differences:
- A map's number (the top bit set) is past the levels' table, so a map has the reverb of the levels after it (studio A). Retail's `levelID` is signed and has no maps.
- There are no boss races, so no boss's reverb is ever set.

**The music (2026-10-27, NOT listened to):** `loader::howl::Cseq` (the song packs of `KART.HWL`, `howl_ParseCseqHeader`), `howl::music` (`CseqMusic_*`, `SongPool_*`, the opcodes of `HOWL_CseqOpcode.c`, `Channel_ParseSongToChannels`, `howl_InstrumentPitch`, `HOWL_Music.c`), `howl::state` (`Audio_SetState`, `Audio_Update1`), and `view::sound`, which loads each level's pack and tells the states every tick. Checked by walking every sequence of every pack to its end, and by reading against the C; no oracle test. Intentional differences:
- The race's states follow our kart, where retail follows the human driver in the lead, and a mask's song plays while ours is out (retail: anyone's on the console, if a human picked it up while not revving).
- No XA: "FINAL LAP!" and the end of the race's jingle are not heard, and the states that wait for them go on at once. The music dips for a frame of the game for "FINAL LAP!" (retail: until the jingle has played a second), and after the race fades out and plays from its start again (retail: once the jingle is over).
- A race starts at the traffic lights (`AUDIO_TRAFFIC`): the fly-in's state waits for its jingle and nothing else.
- All 33 song packs and their samples are loaded at start (1.7 MB more); retail loads the level's pack and music bank as the level loads.
- A map has no song pack, so no music (its number is past the levels' table, as for its reverb). Nor do the adventure hubs play theirs as retail does (no song sets, `SongPool_AdvHub*`).
- A note whose delay never runs out with an opcode the game does not have hangs retail; here its sequence just stops. No song of the game has one.
