//! The game's sounds, which the sound engine of the `howl` crate plays: the beeps of the traffic
//! lights, what the karts' ticks ask for ([`vehicle::Effect`]), and the engines. Our kart's play
//! as retail plays a player's, in the middle; the other karts' are heard from where they are, as
//! `PlaySound3D` hears a sound from the camera (see `DIVERGENCE.md`).
//!
//! The sound engine is one [`Howl`] that the game and the audio thread share: the game starts,
//! changes and stops sounds in it, and Bevy plays the samples it renders ([`Mixer`]). Rendering
//! is the engine's clock too (the SPU's vertical blank, when sounds start, change and end).
//!
//! The engines loop (`EngineAudio_InitOnce`) at what `vehicle::engine_sound` makes of their
//! karts each tick (`EngineAudio_Recalculate`): ours on our class's loop, and the two other
//! karts nearest the camera on the two loops retail gives the AIs (`EngineSound_NearestAIs`).
//! Our kart's other loops (the ground, the skid, the wall rub and the turbo's roar) are the ones
//! its tick keeps going ([`Effect::Recycle`]); retail gives the AIs none, and the other karts
//! have none here either.

use std::f32::consts::TAU;
use std::sync::{Arc, Mutex, MutexGuard};

use bevy::audio::{AddAudioSource, ChannelCount, Decodable, Sample, SampleRate, Source};
use bevy::prelude::*;
use howl::{AntiSpam, CENTRE, Howl, NO_DISTORTION, Sound};
use race::{KartTick, Light, Race};
use render::mesh::SCALE;
use vehicle::calc::map_to_range;
use vehicle::engine_sound::{EngineSound, EngineVoice, FarVoice};
use vehicle::stats::CHARACTER_ENGINES;
use vehicle::{Driver, Effect, actions, audio_slot, fire};

/// `DotLights_AudioAndVideo`: a red light comes on, and the green.
const RED_LIGHT_SFX: u16 = 0x45;
const GREEN_LIGHT_SFX: u16 = 0x46;
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

/// The output samples the audio thread renders at a time, with the sound engine locked: 3 ms.
const BLOCK: usize = 128;

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<Mixer>()
            .init_resource::<Kept>()
            .add_systems(Startup, start_mixer);
    }
}

/// The sound engine, which the audio thread renders.
#[derive(Resource)]
pub struct Sounds(Arc<Mutex<Howl>>);

impl Sounds {
    /// The sounds of `KART.HWL` on the disc image at `path`. A disc without it plays none.
    pub fn load(path: &std::path::Path) -> Sounds {
        let howl = Howl::read(path).unwrap_or_else(|e| {
            warn!("no sounds: {}: {e}", path.display());
            Howl::new(Vec::new(), Vec::new(), Vec::new())
        });
        Sounds(Arc::new(Mutex::new(howl)))
    }

    fn lock(&self) -> MutexGuard<'_, Howl> {
        self.0.lock().unwrap()
    }

    /// `MainFrame_TogglePauseAudio(1)`: the pause stops every sound, to start again with the race.
    pub fn pause(&self) {
        let mut sounds = self.lock();
        sounds.stop_id(MENU_TAKE_SFX);
        sounds.pause();
    }

    /// `MainFrame_TogglePauseAudio(0)`: the end of the pause stops the sounds played in it, and
    /// starts again the ones it stopped.
    pub fn unpause(&self) {
        let mut sounds = self.lock();
        sounds.stop_all(false, true);
        sounds.unpause();
    }

    /// A sound of the menus, in the middle.
    pub fn menu(&self, id: u16) {
        self.lock().play(id, AntiSpam::CutRecent, howl::Sfx::FULL);
    }
}

fn start_mixer(mut commands: Commands, mut mixers: ResMut<Assets<Mixer>>, sounds: Res<Sounds>) {
    let mixer = mixers.add(Mixer(sounds.0.clone()));
    commands.spawn((AudioPlayer(mixer), PlaybackSettings::ONCE));
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
    /// By slot: `VehFire_AudioCooldown`, in milliseconds.
    boost_cooldowns: Vec<i16>,
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
    /// What tick `tick` of `race` asks to be heard: the traffic light that came on, the engines,
    /// each kart's sounds (`karts`, by slot), our kart's loops, and our laps. Ours (`own`) play
    /// in the middle, the others' are heard from where they are (`places`, in world space, by
    /// slot) by the camera `ear`. A race that starts again first stops every sound.
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        tick: u32,
        race: &Race,
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
            sounds.stop_all(true, true);
            *kept = Kept {
                engines: vec![EngineSound::default(); drivers.len()],
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

        for (slot, kart) in karts.iter().enumerate() {
            let Some(&driver) = drivers.get(slot) else {
                continue;
            };
            let cooldown = &mut kept.boost_cooldowns[slot];
            *cooldown = (*cooldown - TICK_MS).max(0);
            let heard = if slot == own {
                Some((u8::MAX, CENTRE))
            } else {
                places.get(slot).and_then(|&at| heard(ear, at))
            };
            for effect in &kart.effects {
                // The loops are ours alone.
                match *effect {
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
                        let (loud, distort, _) = fire::audio(fire_level);
                        let echo = driver.actions_flag_set & actions::ENGINE_ECHO != 0;
                        let sfx = sfx(loud, distort, echo);
                        sounds.play(fire::AUDIO_SFX as u16, AntiSpam::CutRecent, sfx);
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

        // `PlayLevel_UpdateLapStats` and `Audio_Update1`: a lap of ours, and our win.
        if let Some(lap) = karts.get(own).map(|kart| kart.lap) {
            if lap.new_lap {
                sounds.play(LAP_SFX, AntiSpam::CutRecent, howl::Sfx::FULL);
            }
            if lap.finished && ours.driver_rank == 0 {
                sounds.play(WIN_SFX, AntiSpam::Off, howl::Sfx::FULL);
            }
        }
    }
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
    let to = at - ear.translation;
    let distance = (to.length() / SCALE) as i32;
    if distance >= HEARD_WITHIN {
        return None;
    }
    let volume = map_to_range(distance, FULL_WITHIN, HEARD_WITHIN, 255, 0);
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
    fn the_pan_follows_the_angle_and_mirrors_what_is_behind() {
        let pan = |degrees| heard(&ear(), at(1000.0, degrees)).unwrap().1;
        assert_eq!(pan(90.0), 255);
        assert_eq!(pan(-90.0), 0);
        assert_eq!(pan(45.0), CENTRE + 64);
        // 45 degrees behind the right is heard as 45 degrees in front of it.
        assert_eq!(pan(135.0), pan(45.0));
        assert_eq!(pan(180.0), CENTRE);
    }

    #[test]
    fn the_mixer_plays_what_the_engine_renders_left_then_right() {
        let howl = Arc::new(Mutex::new(Howl::new(Vec::new(), Vec::new(), Vec::new())));
        let mut output = Mixer(howl).decoder();
        let samples: Vec<f32> = output.by_ref().take(4 * BLOCK).collect();
        assert_eq!(samples, vec![0.0; 4 * BLOCK]);
        assert_eq!(output.channels().get(), 2);
        assert_eq!(output.sample_rate().get(), howl::spu::RATE);
    }
}
