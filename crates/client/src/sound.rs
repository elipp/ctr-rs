//! The game's sounds, which the sound engine of the `howl` crate plays: the beeps of the traffic
//! lights, what the karts' ticks ask for ([`vehicle::Effect`]), and the engines. Our kart's play
//! as retail plays a player's, in the middle; the other karts' are heard from where they are, as
//! `PlaySound3D` hears a sound from the camera (see `DIVERGENCE.md`).
//!
//! The sound engine is one [`Howl`] that the game and the audio thread share: the game starts,
//! changes and stops sounds in it, and Bevy plays the samples it renders ([`Mixer`]). Rendering
//! is the engine's clock too (the SPU's vertical blank, when sounds start, change and end).
//!
//! The XA the sound engine asks for (the voice lines, the jingles of the race) is read from the
//! disc and decoded on a thread of its own, as a track can span 24 MB of it, and handed back to
//! play ([`stream_xa`]).
//!
//! Each level sets its reverb as it loads (`Audio_SetReverbMode` in `Music_LoadBanks`), which
//! the sounds that echo (a kart in a tunnel) are heard through, and its song pack. Every tick the
//! sound engine hears where the race is ([`howl::state`]) as of our kart, and plays the level's
//! song (or a mask's) from the green light on.
//!
//! The engines loop (`EngineAudio_InitOnce`) at what `vehicle::engine_sound` makes of their
//! karts each tick (`EngineAudio_Recalculate`): ours on our class's loop, and the two other
//! karts nearest the camera on the two loops retail gives the AIs (`EngineSound_NearestAIs`).
//! Our kart's other loops (the ground, the skid, the wall rub and the turbo's roar) are the ones
//! its tick keeps going ([`Effect::Recycle`]); retail gives the AIs none. The other karts' skid
//! and turbo's roar are kept going too, heard from where they are, and not from far (see
//! `docs/notes/DIVERGENCE.md`).

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::{Arc, Mutex, MutexGuard};

use bevy::audio::{AddAudioSource, ChannelCount, Decodable, Sample, SampleRate, Source};
use bevy::prelude::*;
use howl::music::{AKU_SONG, UKA_SONG};
use howl::settings::Slider;
use howl::state::{RaceAudio, State};
use howl::{AntiSpam, CENTRE, Howl, NO_DISTORTION, Sound};
use loader::howl::Cseq;
use loader::lev::Level;
use loader::source::{Need, Source as DiscSource};
use race::{KartTick, Light, Race};
use render::mesh::SCALE;
use vehicle::calc::map_to_range;
use vehicle::engine_sound::{EngineSound, EngineVoice, FarVoice};
use vehicle::stats::CHARACTER_ENGINES;
use vehicle::{Driver, Effect, Sound3D, actions, audio_slot, fire};
use weapon::mine::MineKind;

use crate::Track;

/// `DotLights_AudioAndVideo`: a red light comes on, and the green.
const RED_LIGHT_SFX: u16 = 0x45;
const GREEN_LIGHT_SFX: u16 = 0x46;
/// `RB_CrateWeapon_ThCollide`: the item roulette spins.
const ROULETTE_SFX: u16 = 0x5d;
/// A crate breaks; a wumpa fruit of the level is picked up.
const CRATE_BREAK_SFX: u8 = 0x3c;
const FRUIT_SFX: u8 = 0x43;
/// `RB_MovingExplosive_Explode`: a bomb goes off; a missile or a shot shield.
const BOMB_EXPLODE_SFX: u8 = 0x49;
const MISSILE_EXPLODE_SFX: u8 = 0x4c;
/// `RB_GenericMine_ThDestroy` and the like: a TNT crate goes off; a Nitro crate or a beaker shatters.
const TNT_EXPLODE_SFX: u8 = 0x3d;
const SHATTER_SFX: u8 = 0x3f;
/// `RB_MovingExplosive_ThTick`: a bomb rolls, a missile flies, a shot shield rolls.
const BOMB_ROLL_SFX: u8 = 0x48;
const MISSILE_FLY_SFX: u8 = 0x4b;
const SHIELD_ROLL_SFX: u8 = 0x59;
/// `PLAYLEVEL_FINAL_LAP_SOUND`: the leading player starts a lap (or finishes the last).
const LAP_SFX: u16 = 0x66;
/// `Audio_Update1`: the player finishes first.
const WIN_SFX: u16 = 0x5f;
/// The pause menu's (`RECTMENU_ProcessInput`): the pick moves, an item is taken, the menu goes.
pub const MENU_MOVE_SFX: u16 = 0;
pub const MENU_TAKE_SFX: u16 = 1;
pub const MENU_BACK_SFX: u16 = 2;

/// A tick of the race, in milliseconds (`elapsedTimeMS`).
const TICK_MS: i16 = 32;

/// An engine class has a loop for each of four players (`engine * 4 + driverID`); ours is the
/// first player's.
const ENGINES_PER_CLASS: u16 = 4;
/// The loops of the two other karts nearest the camera: the AIs' (`EngineSound_NearestAIs`).
const FAR_ENGINES: [u16; 2] = [0x10, 0x11];

/// `PlaySound3D`: a sound this far from the camera or farther is not heard, and one this near
/// or nearer is at full volume, in level units.
const HEARD_WITHIN: i32 = 9000;
const FULL_WITHIN: i32 = 300;

/// The loops of the other karts that are heard: the skid and the turbo's roar.
const PEER_LOOPS: [u8; 2] = [audio_slot::SKID, audio_slot::TURBO];
/// What another kart makes (its loops, its bounces and crashes, its voice) is at full volume this
/// near the camera, and not heard from this far, in level units. Retail plays them in the middle
/// at full volume, and the objects and the weapons carry to [`HEARD_WITHIN`].
const KART_FULL_WITHIN: i32 = 300;
const KART_HEARD_WITHIN: i32 = 2500;

/// The output samples the audio thread renders at a time, with the sound engine locked: 3 ms.
const BLOCK: usize = 128;

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<Mixer>()
            .init_resource::<Kept>()
            .add_systems(Startup, start_mixer)
            .add_systems(
                Update,
                (level_audio, stream_xa).run_if(crate::drive::driving),
            )
            .add_systems(Update, volumes);
    }
}

/// The sound engine, which the audio thread renders, and the XA of the disc.
#[derive(Resource)]
pub struct Sounds {
    howl: Arc<Mutex<Howl>>,
    source: DiscSource,
}

impl Sounds {
    /// The sounds of `KART.HWL` and the XA of `source`. A disc without them plays none.
    pub fn load(source: DiscSource) -> Sounds {
        let howl = Howl::read_from(&source).unwrap_or_else(|e| {
            warn!("no sounds: {e}");
            Howl::new(Vec::new(), Vec::new(), Vec::new(), Vec::new())
        });
        Sounds {
            howl: Arc::new(Mutex::new(howl)),
            source,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Howl> {
        self.howl.lock().unwrap()
    }

    /// `MainFrame_TogglePauseAudio(1)`: the pause stops every sound, to start again with the race.
    pub fn pause(&self) {
        let mut sounds = self.lock();
        sounds.stop_id(MENU_TAKE_SFX);
        sounds.pause();
    }

    /// `MainFrame_TogglePauseAudio(0)`: the end of the pause stops the sounds played in it, and
    /// starts again the ones and the music it stopped.
    pub fn unpause(&self) {
        let mut sounds = self.lock();
        sounds.stop_all(false, false, true);
        sounds.unpause();
    }

    /// A sound of the menus, in the middle.
    pub fn menu(&self, id: u16) {
        self.lock().play(id, AntiSpam::CutRecent, howl::Sfx::FULL);
    }

    /// `OptionsMenu_TestSound`: the sound to hear the volume `slider` by plays while it is
    /// picked, `character`'s grunts for the voices, and stops once none is.
    pub fn test_sound(&self, slider: Option<Slider>, character: u8, frame: u32) {
        self.lock().test_sound(slider, character, frame);
    }
}

fn start_mixer(mut commands: Commands, mut mixers: ResMut<Assets<Mixer>>, sounds: Res<Sounds>) {
    let mixer = mixers.add(Mixer(sounds.howl.clone()));
    commands.spawn((AudioPlayer(mixer), PlaybackSettings::ONCE));
}

/// `Music_LoadBanks` and `Music_AsyncParseBanks`: a level that loads sets its reverb and loads
/// its song pack. There are no boss races. A map brings what its patch changed (`map::Audio`),
/// and has the game's for the track it was made over where it brings none.
fn level_audio(track: Res<Track>, sounds: Res<Sounds>) {
    if track.is_changed() {
        let audio = map::is_map_id(track.1.0)
            .then(|| crate::maps::get(track.1.0))
            .flatten()
            .and_then(|m| m.audio.as_ref());
        let mut sounds = sounds.lock();
        let Some(audio) = audio else {
            sounds.set_level_audio(track.1.0, None, None, Vec::new());
            return;
        };
        // The pack was checked when the map was read.
        let pack = audio.song_pack.clone().and_then(|p| Cseq::parse(p).ok());
        info!(
            "map audio: over track {}, song pack {:?} ({} bytes, {} songs), reverb {:?}, {} samples",
            audio.level,
            pack.is_some(),
            audio.song_pack.as_ref().map_or(0, Vec::len),
            pack.as_ref().map_or(0, |p| p.songs.len()),
            audio.reverb,
            audio.samples.len()
        );
        let samples = (audio.samples.iter())
            .map(|(n, adpcm)| (*n, adpcm.clone().into_boxed_slice()))
            .collect();
        sounds.set_level_audio(audio.level, pack, audio.reverb, samples);
    }
}

/// The volumes the config asks for, when it changes (and once the sounds are there).
fn volumes(config: Res<crate::config::Config>, sounds: Option<Res<Sounds>>) {
    if let Some(sounds) = sounds
        && (config.is_changed() || sounds.is_added())
    {
        sounds
            .lock()
            .set_volumes(config.volume_fx, config.volume_music, config.volume_voice);
    }
}

/// `CDSYS_XAPauseAtEnd`, every frame (a pause's too): the XA that is over is, and the one that
/// fades loses volume. The track the sound engine asked for is read and decoded on a thread, and
/// plays once it is.
fn stream_xa(sounds: Res<Sounds>) {
    let request = {
        let mut howl = sounds.lock();
        howl.update_xa();
        howl.take_xa_request()
    };
    let Some(request) = request else {
        return;
    };
    let (howl, source) = (sounds.howl.clone(), sounds.source.clone());
    let decode = move || {
        let sectors = source
            .get(Need::Xa(request.category, request.id))
            .inspect_err(|e| warn!("XA {:?}: {e}", request.track))
            .ok();
        let pcm = sectors.map(|sectors| loader::xa::decode(&sectors));
        howl.lock().unwrap().start_xa(request.serial, pcm);
    };
    // A browser has no threads: there it is done at once, and the frame waits for it.
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(decode);
    #[cfg(target_arch = "wasm32")]
    decode();
}

/// What Bevy plays: everything the sound engine renders, without end.
#[derive(Asset, TypePath)]
pub struct Mixer(Arc<Mutex<Howl>>);

impl Decodable for Mixer {
    type Decoder = Output;

    fn decoder(&self) -> Output {
        Output {
            howl: self.0.clone(),
            frames: [[0; 2]; BLOCK],
            at: 2 * BLOCK,
        }
    }
}

/// The [`Mixer`] played: [`BLOCK`] samples rendered at a time, each left then right.
pub struct Output {
    howl: Arc<Mutex<Howl>>,
    frames: [[i16; 2]; BLOCK],
    /// The next of the block's left and right samples.
    at: usize,
}

impl Iterator for Output {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.at == 2 * BLOCK {
            self.howl.lock().unwrap().render(&mut self.frames);
            self.at = 0;
        }
        let sample = self.frames[self.at / 2][self.at % 2];
        self.at += 1;
        Some(sample as f32 / 32768.0)
    }
}

impl Source for Output {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(howl::spu::RATE).unwrap()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

/// What the game's sounds keep between ticks.
#[derive(Resource, Default)]
struct Kept {
    /// The engine loops that play: ours, then the two of the other karts nearest the camera.
    engine_loops: Option<[u16; 3]>,
    /// By slot.
    engines: Vec<EngineSound>,
    /// What the two loops of the nearest others played last.
    far: [FarVoice; 2],
    /// Our kart's loops (`driverAudioPtrs`), by [`vehicle::audio_slot`].
    loops: [Option<Sound>; 4],
    /// The skid and the turbo's roar of the other karts, by slot ([`PEER_LOOPS`] order).
    peer_loops: Vec<[Option<Sound>; 2]>,
    /// By slot: `VehFire_AudioCooldown`, in milliseconds.
    boost_cooldowns: Vec<i16>,
    /// The loops of the level's objects (by instance) and of the weapons in flight (by id).
    object_loops: HashMap<usize, Option<Sound>>,
    weapon_loops: HashMap<u32, Option<Sound>>,
    /// The breaks of each crate, and whether each fruit is gone, as last heard.
    crate_breaks: Vec<u32>,
    fruit_taken: Vec<bool>,
    /// `ROLLING_ITEM`: our item roulette spins, and with it its sound.
    rolling: bool,
    /// The last tick heard: a race that starts again starts everything again.
    tick: u32,
}

/// What plays the game's sounds.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Sfx<'w> {
    sounds: Res<'w, Sounds>,
    kept: ResMut<'w, Kept>,
}

impl Sfx<'_> {
    /// What tick `tick` of `race` (on `level`) asks to be heard: the traffic light that came on,
    /// the engines, each kart's sounds and voice lines (`karts`, by slot), our kart's loops, our
    /// laps and item roulette, the sounds of the objects and the weapons, and where the race is
    /// for the music. Ours (`own`) play in the middle, the others' are heard from where they are
    /// (`places`, in world space, by slot) by the camera `ear`. A race that starts again first
    /// stops every sound and the music, and is at the traffic lights.
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        tick: u32,
        race: &Race,
        level: &Level,
        karts: &[KartTick],
        own: usize,
        places: &[Vec3],
        ear: &Transform,
    ) {
        let drivers: Vec<&Driver> = race.karts.iter().map(|kart| &kart.driver).collect();
        let drivers = &drivers[..];
        let light = race.light;
        let Some(&ours) = drivers.get(own) else {
            return;
        };
        let mut sounds = self.sounds.lock();
        let kept = &mut *self.kept;
        if tick < kept.tick || kept.engines.len() != drivers.len() {
            sounds.stop_all(true, true, true);
            sounds.stop_xa();
            sounds.clear_voicelines();
            sounds.clear_state();
            // A race with the fly-in starts with its jingle (`MainMain`'s stage 1).
            sounds.set_state(if race.intro_ticks > 0 {
                State::RaceIntro
            } else {
                State::Traffic
            });
            *kept = Kept {
                engines: vec![EngineSound::default(); drivers.len()],
                peer_loops: vec![[None; 2]; drivers.len()],
                boost_cooldowns: vec![0; drivers.len()],
                ..Kept::default()
            };
        }
        kept.tick = tick;
        sounds.frame = tick;
        engines(&mut sounds, kept, drivers, own, places, ear);

        if let Some(light) = light {
            let id = match light {
                Light::Red1 | Light::Red2 | Light::Red3 => RED_LIGHT_SFX,
                Light::Green => GREEN_LIGHT_SFX,
            };
            sounds.play(id, AntiSpam::Off, howl::Sfx::FULL);
        }

        let finished = ours.actions_flag_set & actions::RACE_FINISHED != 0;
        for (slot, kart) in karts.iter().enumerate() {
            let Some(&driver) = drivers.get(slot) else {
                continue;
            };
            let cooldown = &mut kept.boost_cooldowns[slot];
            *cooldown = (*cooldown - TICK_MS).max(0);
            let heard = if slot == own {
                Some((u8::MAX, CENTRE))
            } else {
                places.get(slot).and_then(|&at| heard_kart(ear, at))
            };
            for effect in &kart.effects {
                // The voice lines are not heard from anywhere, the loops are ours alone, and what
                // a kart sends off is heard from where it is.
                match *effect {
                    Effect::Sound3D(sound) => {
                        let echo = driver.actions_flag_set & actions::ENGINE_ECHO != 0;
                        play_3d(&mut sounds, ear, &sound, echo);
                        continue;
                    }
                    Effect::Voiceline {
                        id,
                        driver_id,
                        priority,
                    } => {
                        let character = drivers
                            .iter()
                            .find(|d| d.driver_id == driver_id)
                            .map_or(driver.character, |d| d.character);
                        // A kart's voice is heard from where it is; out of earshot it is not said.
                        if let Some((volume, pan)) = heard {
                            let sfx = howl::Sfx {
                                volume,
                                pan,
                                ..howl::Sfx::FULL
                            };
                            sounds.request_voiceline(id, character, priority, finished, sfx);
                        }
                        continue;
                    }
                    Effect::Recycle {
                        slot: loop_slot,
                        id,
                        volume,
                        distort,
                        pan,
                        echo,
                    } if slot == own => {
                        let sfx = howl::Sfx {
                            volume,
                            distort,
                            pan,
                            echo,
                        };
                        let playing = &mut kept.loops[loop_slot as usize];
                        sounds.recycle(playing, id.map(u16::from), sfx);
                        continue;
                    }
                    // The other karts' skid and roar are heard from where they are, and not
                    // from far: their ground and wall rub are not heard at all.
                    Effect::Recycle {
                        slot: loop_slot,
                        id,
                        volume,
                        distort,
                        pan,
                        echo,
                    } if PEER_LOOPS.contains(&loop_slot) => {
                        let near = places.get(slot).and_then(|&at| heard_kart(ear, at));
                        let (id, sfx) = match near {
                            Some((near, side)) => {
                                // (The skid's own pan, around the middle, moves the side.)
                                let pan = (pan as i32 + side as i32 - CENTRE as i32).clamp(0, 255);
                                let volume = (volume as u32 * near as u32 / 255) as u8;
                                let sfx = howl::Sfx {
                                    volume,
                                    distort,
                                    pan: pan as u8,
                                    echo,
                                };
                                (id.map(u16::from), sfx)
                            }
                            None => (None, howl::Sfx::SILENT),
                        };
                        let index = PEER_LOOPS.iter().position(|&l| l == loop_slot).unwrap();
                        if let Some(playing) = kept.peer_loops.get_mut(slot) {
                            sounds.recycle(&mut playing[index], id, sfx);
                        }
                        continue;
                    }
                    _ => {}
                }
                let Some((volume, pan)) = heard else {
                    continue;
                };
                let sfx = |of: u8, distort, echo| howl::Sfx {
                    volume: (of as u32 * volume as u32 / 255) as u8,
                    distort,
                    pan,
                    echo,
                };
                match *effect {
                    Effect::Sound { id, echo } => {
                        let sfx = sfx(u8::MAX, NO_DISTORTION, echo);
                        sounds.play(id as u16, AntiSpam::CutRecent, sfx);
                    }
                    Effect::DriverCrashing { echo, volume: bump } => {
                        let sfx = sfx(bump, NO_DISTORTION, echo);
                        sounds.play(howl::crash_sound(bump), AntiSpam::Off, sfx);
                    }
                    // `VehFire_Audio`, which its cooldown holds back.
                    Effect::FireAudio { fire_level } if *cooldown == 0 => {
                        let (loud, distort, cheer) = fire::audio(fire_level);
                        let echo = driver.actions_flag_set & actions::ENGINE_ECHO != 0;
                        let sfx = sfx(loud, distort, echo);
                        sounds.play(fire::AUDIO_SFX as u16, AntiSpam::CutRecent, sfx);
                        if cheer {
                            let (voice, priority) =
                                (fire::BIG_BOOST_VOICELINE, fire::VOICELINE_PRIORITY);
                            sounds.request_voiceline(
                                voice,
                                driver.character,
                                priority,
                                finished,
                                howl::Sfx::FULL,
                            );
                        }
                        *cooldown = fire::AUDIO_COOLDOWN_MS;
                    }
                    _ => {}
                }
            }
        }
        // A fire that went out in a tick played again (a rollback) still stops its roar.
        if ours.turbo.is_none() {
            sounds.recycle_mute(&mut kept.loops[audio_slot::TURBO as usize]);
        }
        for (slot, driver) in drivers.iter().enumerate() {
            if slot != own && driver.turbo.is_none() {
                let turbo = PEER_LOOPS
                    .iter()
                    .position(|&l| l == audio_slot::TURBO)
                    .unwrap();
                if let Some(playing) = kept.peer_loops.get_mut(slot) {
                    sounds.recycle_mute(&mut playing[turbo]);
                }
            }
        }

        // `PlayLevel_UpdateLapStats` and `Audio_Update1`: a lap of ours, and our win.
        if let Some(lap) = karts.get(own).map(|kart| kart.lap) {
            if lap.new_lap {
                sounds.play(LAP_SFX, AntiSpam::CutRecent, howl::Sfx::FULL);
                sounds.clear_voiceline_times();
            }
            if lap.finished && ours.driver_rank == 0 {
                sounds.play(WIN_SFX, AntiSpam::Off, howl::Sfx::FULL);
            }
        }

        // The item roulette spins from a weapon crate's break until the item is picked
        // (`RB_CrateWeapon_ThCollide`, `UI_RenderFrame_Racing`).
        let rolling = ours.item_roll_timer != 0;
        if rolling && !kept.rolling {
            sounds.play(ROULETTE_SFX, AntiSpam::Off, howl::Sfx::FULL);
        } else if !rolling && kept.rolling {
            sounds.stop_id(ROULETTE_SFX);
        }
        kept.rolling = rolling;

        // The objects and the weapons are heard from the camera, as the echo of our kart.
        let echo = ours.actions_flag_set & actions::ENGINE_ECHO != 0;
        objects(&mut sounds, kept, race, level, ear, echo);

        // `Audio_Update1`, as our kart is the human driver in the lead.
        let mask = ours
            .mask
            .filter(|_| ours.actions_flag_set & actions::MASK_WEAPON != 0);
        sounds.update_state(&RaceAudio {
            lights_out: race.lights_ms < 1,
            laps: race.laps,
            lap: ours.lap_index,
            to_finish: ours.distance_to_finish_curr as i32,
            finished,
            first: ours.driver_rank == 0,
            mask_song: mask.map(|m| if m.good_guy { AKU_SONG } else { UKA_SONG }),
            wrong_way: race.laps != race::FREE_DRIVE_LAPS && vehicle::lap::wrong_way(ours),
            good_guy: vehicle::stuck::good_guy(ours.character),
        });
    }
}

/// The sounds of the level's objects and of the weapons in flight, heard by the camera `ear`.
fn objects(
    sounds: &mut Howl,
    kept: &mut Kept,
    race: &Race,
    level: &Level,
    ear: &Transform,
    echo: bool,
) {
    let place = |instance: usize| {
        let pos = level.instances[instance].pos;
        [pos.x as i32, pos.y as i32, pos.z as i32]
    };

    // A crate breaks (`RB_Crate*_ThCollide`), and a fruit is picked up (`RB_Fruit_LInC`). What
    // was broken or taken before a new race or level is not heard.
    let crates = &race.objects.crates;
    if kept.crate_breaks.len() != crates.len() {
        kept.crate_breaks = crates.iter().map(|c| c.sim.breaks).collect();
    }
    for (placed, heard) in crates.iter().zip(&mut kept.crate_breaks) {
        if placed.sim.breaks != *heard {
            *heard = placed.sim.breaks;
            let sound = Sound3D {
                id: CRATE_BREAK_SFX,
                pos: place(placed.instance),
            };
            play_3d(sounds, ear, &sound, echo);
        }
    }
    let fruit = &race.objects.fruit;
    if kept.fruit_taken.len() != fruit.len() {
        kept.fruit_taken = fruit.iter().map(|f| f.sim.collected).collect();
    }
    for (placed, heard) in fruit.iter().zip(&mut kept.fruit_taken) {
        if placed.sim.collected && !*heard {
            let sound = Sound3D {
                id: FRUIT_SFX,
                pos: place(placed.instance),
            };
            play_3d(sounds, ear, &sound, echo);
        }
        *heard = placed.sim.collected;
    }

    // What the objects and the weapons played, and how the weapons went off.
    let weapons = &race.last_weapons;
    let explosions = weapons.explosions.iter().map(|e| Sound3D {
        id: match e.kind {
            weapon::Kind::Bomb => BOMB_EXPLODE_SFX,
            weapon::Kind::Missile | weapon::Kind::Shield => MISSILE_EXPLODE_SFX,
        },
        pos: e.pos,
    });
    let detonations = weapons.detonations.iter().map(|d| Sound3D {
        id: match d.kind {
            MineKind::Tnt => TNT_EXPLODE_SFX,
            MineKind::Nitro | MineKind::GreenBeaker | MineKind::RedBeaker => SHATTER_SFX,
        },
        pos: d.pos,
    });
    let played = race.last.sounds.iter().chain(&weapons.sounds).copied();
    for sound in played.chain(explosions).chain(detonations) {
        play_3d(sounds, ear, &sound, echo);
    }

    // The loops: the objects' each tick they send one, the weapons' while they fly.
    let objects = race.last.loops.iter().copied();
    keep_loops(sounds, &mut kept.object_loops, objects, ear, echo);
    let flying = race.weapons.projectiles().iter().map(|p| {
        let id = match p.kind {
            weapon::Kind::Bomb => BOMB_ROLL_SFX,
            weapon::Kind::Missile => MISSILE_FLY_SFX,
            weapon::Kind::Shield => SHIELD_ROLL_SFX,
        };
        (p.id, Sound3D { id, pos: p.pos })
    });
    keep_loops(sounds, &mut kept.weapon_loops, flying, ear, echo);
}

/// `PlaySound3D`: `sound` heard by the camera `ear` from where it is, if it is near enough.
fn play_3d(sounds: &mut Howl, ear: &Transform, sound: &Sound3D, echo: bool) {
    if let Some((volume, pan)) = heard(ear, level_at(sound.pos)) {
        let sfx = howl::Sfx {
            volume,
            distort: NO_DISTORTION,
            pan,
            echo,
        };
        sounds.play(sound.id as u16, AntiSpam::CutRecent, sfx);
    }
}

/// `PlaySound3D_Flags` for each of `wanted`, by its owner: the owner's loop plays its sound, heard
/// by the camera `ear` from where it is. Out of hearing a loop goes on as it was, and the loop of
/// an owner with none this tick stops.
fn keep_loops<K: Copy + Eq + std::hash::Hash>(
    sounds: &mut Howl,
    loops: &mut HashMap<K, Option<Sound>>,
    wanted: impl Iterator<Item = (K, Sound3D)>,
    ear: &Transform,
    echo: bool,
) {
    let mut seen = Vec::new();
    for (owner, sound) in wanted {
        seen.push(owner);
        let playing = loops.entry(owner).or_default();
        if let Some(old) = *playing
            && old.id() != sound.id as u16
        {
            sounds.stop(old);
            *playing = None;
        }
        let Some((volume, pan)) = heard(ear, level_at(sound.pos)) else {
            continue;
        };
        let sfx = howl::Sfx {
            volume,
            distort: NO_DISTORTION,
            pan,
            echo,
        };
        sounds.recycle(playing, Some(sound.id as u16), sfx);
    }
    loops.retain(|owner, playing| {
        let kept = seen.contains(owner);
        if !kept {
            sounds.recycle_mute(playing);
        }
        kept
    });
}

/// A place in level units, in world space.
fn level_at([x, y, z]: [i32; 3]) -> Vec3 {
    Vec3::new(x as f32, y as f32, z as f32) * SCALE
}

/// The engines after a tick: ours (`own`) as `EngineSound_Player` hears it, and the two other
/// karts nearest the camera `ear` as `EngineSound_NearestAIs` does (`places` in world space, by
/// slot). Another engine of ours starts the loops again.
fn engines(
    sounds: &mut Howl,
    kept: &mut Kept,
    drivers: &[&Driver],
    own: usize,
    places: &[Vec3],
    ear: &Transform,
) {
    let ours = drivers[own];
    let class = CHARACTER_ENGINES
        .get(ours.character as usize)
        .map_or(0, |&class| class as u16);
    let loops = [class * ENGINES_PER_CLASS, FAR_ENGINES[0], FAR_ENGINES[1]];
    if kept.engine_loops != Some(loops) {
        for id in kept.engine_loops.take().into_iter().flatten() {
            sounds.stop_engine(id);
        }
        // `VehBirth_EngineAudio_AllPlayers`, and `MainInit` for the AIs'.
        for id in loops {
            sounds.start_engine(id, howl::Sfx::SILENT);
        }
        kept.engine_loops = Some(loops);
        kept.engines = vec![EngineSound::default(); drivers.len()];
        kept.far = Default::default();
    }
    let voice = kept.engines[own].player(ours);
    sounds.set_engine(loops[0], engine_sfx(voice));

    // The nearest and the next nearest, the first found of equals.
    let mut nearest = [(None, i32::MAX); 2];
    for (slot, &at) in places.iter().enumerate().take(drivers.len()) {
        if slot == own {
            continue;
        }
        let distance = ((at - ear.translation).length() / SCALE) as i32;
        if distance < nearest[0].1 {
            nearest = [(Some(slot), distance), nearest[0]];
        } else if distance < nearest[1].1 {
            nearest[1] = (Some(slot), distance);
        }
    }
    let echo = ours.actions_flag_set & actions::ENGINE_ECHO != 0;
    for (i, (slot, distance)) in nearest.into_iter().enumerate() {
        let sfx = slot.map_or(howl::Sfx::SILENT, |slot| {
            let pan = CENTRE as i32 + side_of(ear, places[slot]);
            let kart = &mut kept.engines[slot];
            engine_sfx(kart.far(drivers[slot], distance, pan, &mut kept.far[i], echo))
        });
        sounds.set_engine(loops[1 + i], sfx);
    }
}

fn engine_sfx(voice: EngineVoice) -> howl::Sfx {
    howl::Sfx {
        volume: voice.volume,
        distort: voice.distort,
        pan: voice.pan,
        echo: voice.echo,
    }
}

/// `PlaySound3D`: the volume and pan a sound at `at` (world space) is heard with by the camera
/// `ear`, or `None` if it is too far to be heard. The volume falls in a line from full at 300
/// level units to nothing at 9000; the pan follows the angle from the camera's forward, all on
/// one side at a right angle, and a sound behind is heard as the one in front that it mirrors.
fn heard(ear: &Transform, at: Vec3) -> Option<(u8, u8)> {
    heard_within(ear, at, FULL_WITHIN, HEARD_WITHIN)
}

/// As [`heard`] for another kart, which carries less far.
fn heard_kart(ear: &Transform, at: Vec3) -> Option<(u8, u8)> {
    heard_within(ear, at, KART_FULL_WITHIN, KART_HEARD_WITHIN)
}

fn heard_within(ear: &Transform, at: Vec3, full: i32, far: i32) -> Option<(u8, u8)> {
    let to = at - ear.translation;
    let distance = (to.length() / SCALE) as i32;
    if distance >= far {
        return None;
    }
    let volume = map_to_range(distance, full, far, 255, 0);
    let pan = CENTRE as i32 + side_of(ear, at);
    Some((volume as u8, pan.clamp(0, 255) as u8))
}

/// `GTE_AudioLR_Inst` and the fold after it: how far to the side of the camera `ear` the point
/// `at` is, from -128 (all left) to 128 (all right), a sound behind heard as the one in front
/// that it mirrors.
fn side_of(ear: &Transform, at: Vec3) -> i32 {
    // The camera looks along -Z with +X on its right. In 512ths of a turn, right positive.
    let local = ear.rotation.inverse() * (at - ear.translation);
    let turn = (local.x.atan2(-local.z) / TAU * 512.0).round() as i32;
    match turn {
        t if t > 128 => 256 - t,
        t if t < -128 => -256 - t,
        t => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ear() -> Transform {
        Transform::from_xyz(0.0, 0.0, 0.0).looking_at(Vec3::NEG_Z, Vec3::Y)
    }

    /// A point `units` level units from the ear, `degrees` to its right of straight ahead.
    fn at(units: f32, degrees: f32) -> Vec3 {
        let a = degrees.to_radians();
        Vec3::new(a.sin(), 0.0, -a.cos()) * units * SCALE
    }

    #[test]
    fn near_sounds_are_full_and_far_ones_fade_out() {
        assert_eq!(heard(&ear(), at(200.0, 0.0)), Some((255, CENTRE)));
        assert_eq!(heard(&ear(), at(6000.0, 0.0)).unwrap().0, 88);
        assert_eq!(heard(&ear(), at(9500.0, 0.0)), None);
    }

    #[test]
    fn another_kart_is_heard_from_less_far() {
        assert_eq!(heard_kart(&ear(), at(200.0, 0.0)), Some((255, CENTRE)));
        assert_eq!(heard_kart(&ear(), at(1400.0, 0.0)).unwrap().0, 128);
        assert_eq!(heard_kart(&ear(), at(2600.0, 0.0)), None);
        // (Where an object, 1400 units off, is still nearly full.)
        assert!(heard(&ear(), at(1400.0, 0.0)).unwrap().0 > 200);
    }

    #[test]
    fn the_pan_follows_the_angle_and_mirrors_what_is_behind() {
        let pan = |degrees| heard(&ear(), at(1000.0, degrees)).unwrap().1;
        assert_eq!(pan(90.0), 255);
        assert_eq!(pan(-90.0), 0);
        assert_eq!(pan(45.0), CENTRE + 64);
        // 45 degrees behind the right is heard as 45 degrees in front of it.
        assert_eq!(pan(135.0), pan(45.0));
        assert_eq!(pan(180.0), CENTRE);
    }

    /// A sound engine whose sound effects all loop.
    fn looping() -> Howl {
        let fx = loader::howl::OtherFx {
            flags: loader::howl::other_fx_flags::LOOPS,
            volume: 255,
            pitch: 0x1000,
            sample: 0,
            duration: 0,
        };
        let samples = vec![Some(vec![0u8; 64].into_boxed_slice())];
        Howl::new(vec![fx; 0x80], Vec::new(), Vec::new(), samples)
    }

    #[test]
    fn an_owners_loop_plays_while_it_is_sent_and_stops_after() {
        let mut sounds = looping();
        let mut loops = HashMap::new();
        let near = Sound3D {
            id: 0x72,
            pos: [0, 0, -1000],
        };
        keep_loops(
            &mut sounds,
            &mut loops,
            [(7, near)].into_iter(),
            &ear(),
            false,
        );
        let playing = loops[&7].unwrap();
        // Sent again, it plays on, changed.
        let nearer = Sound3D {
            pos: [0, 0, -500],
            ..near
        };
        keep_loops(
            &mut sounds,
            &mut loops,
            [(7, nearer)].into_iter(),
            &ear(),
            false,
        );
        assert_eq!(loops[&7], Some(playing));
        // Out of hearing it is left as it was; a new one does not start there.
        let far = Sound3D {
            pos: [0, 0, -20_000],
            ..near
        };
        keep_loops(
            &mut sounds,
            &mut loops,
            [(7, far), (8, far)].into_iter(),
            &ear(),
            false,
        );
        assert_eq!((loops[&7], loops[&8]), (Some(playing), None));
        // Not sent, it stops.
        keep_loops(&mut sounds, &mut loops, std::iter::empty(), &ear(), false);
        assert!(loops.is_empty());
    }

    #[test]
    fn the_mixer_plays_what_the_engine_renders_left_then_right() {
        let howl = Arc::new(Mutex::new(Howl::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )));
        let mut output = Mixer(howl).decoder();
        let samples: Vec<f32> = output.by_ref().take(4 * BLOCK).collect();
        assert_eq!(samples, vec![0.0; 4 * BLOCK]);
        assert_eq!(output.channels().get(), 2);
        assert_eq!(output.sample_rate().get(), howl::spu::RATE);
    }
}
