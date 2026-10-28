//! The game's sounds, which the sound engine of the `howl` crate plays: the beeps of the traffic
//! lights, and what the karts' ticks ask for ([`vehicle::Effect`]). Our kart's play
//! as retail plays a player's, in the middle; the other karts' are heard from where they are, as
//! `PlaySound3D` hears a sound from the camera (see `DIVERGENCE.md`).
//!
//! The sound engine is one [`Howl`] that the game and the audio thread share: the game starts,
//! changes and stops sounds in it, and Bevy plays the samples it renders ([`Mixer`]). Rendering
//! is the engine's clock too (the SPU's vertical blank, when sounds start, change and end).

use std::f32::consts::TAU;
use std::sync::{Arc, Mutex, MutexGuard};

use bevy::audio::{AddAudioSource, ChannelCount, Decodable, Sample, SampleRate, Source};
use bevy::prelude::*;
use howl::{AntiSpam, CENTRE, Howl, NO_DISTORTION};
use race::{KartTick, Light, Race};
use render::mesh::SCALE;
use vehicle::Effect;
use vehicle::calc::map_to_range;

/// `DotLights_AudioAndVideo`: a red light comes on, and the green.
const RED_LIGHT_SFX: u16 = 0x45;
const GREEN_LIGHT_SFX: u16 = 0x46;

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
            Howl::new(Vec::new(), Vec::new())
        });
        Sounds(Arc::new(Mutex::new(howl)))
    }

    fn lock(&self) -> MutexGuard<'_, Howl> {
        self.0.lock().unwrap()
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

/// What plays the game's sounds.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Sfx<'w> {
    sounds: Res<'w, Sounds>,
}

impl Sfx<'_> {
    /// What tick `tick` of `race` asks to be heard: the traffic light that came on, and each
    /// kart's sounds (`karts`, by slot). Ours (`own`) play in the middle, the others' are heard
    /// from where they are (`places`, in world space, by slot) by the camera `ear`.
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
        let light = race.light;
        let mut sounds = self.sounds.lock();
        sounds.frame = tick;

        if let Some(light) = light {
            let id = match light {
                Light::Red1 | Light::Red2 | Light::Red3 => RED_LIGHT_SFX,
                Light::Green => GREEN_LIGHT_SFX,
            };
            sounds.play(id, AntiSpam::Off, howl::Sfx::FULL);
        }

        for (slot, kart) in karts.iter().enumerate() {
            let heard = if slot == own {
                Some((u8::MAX, CENTRE))
            } else {
                places.get(slot).and_then(|&at| heard(ear, at))
            };
            for effect in &kart.effects {
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
                    _ => {}
                }
            }
        }
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
        let howl = Arc::new(Mutex::new(Howl::new(Vec::new(), Vec::new())));
        let mut output = Mixer(howl).decoder();
        let samples: Vec<f32> = output.by_ref().take(4 * BLOCK).collect();
        assert_eq!(samples, vec![0.0; 4 * BLOCK]);
        assert_eq!(output.channels().get(), 2);
        assert_eq!(output.sample_rate().get(), howl::spu::RATE);
    }
}
