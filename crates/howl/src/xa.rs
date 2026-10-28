//! XA: the jingles and the spoken lines the CD drive streams into the SPU, as `ctr-native` plays
//! them (`platform/native_audio.c`, the native path of `game/CDSYS.c`).
//!
//! The game asks for a track ([`Howl::play_xa`]) and from then on it plays, as far as the game
//! can tell, until it is over or has faded out ([`Howl::pause_xa`]); [`Howl::update_xa`], once a
//! frame of the game, sees to both (`CDSYS_XAPauseAtEnd`). The sectors of the track are read from
//! the disc by whoever owns it, which takes the request ([`Howl::take_xa_request`]) and hands the
//! track back decoded ([`Howl::start_xa`]); until then it is silent and its time does not run.
//!
//! The SPU hears the CD's input ([`Cd`]) at 44.1 kHz, through the CD drive's zig-zag
//! interpolation from 37.8 kHz (18.9 kHz holds each sample twice), at the CD's volume and the
//! master's.
//!
//! Not ported: the loudness of the XA as it plays (`XA_MaxSampleVal`), which only the talking
//! masks' mouths and the cutscenes read.

use loader::xa::{Category, Pcm, RATE_HALF, Track};

use crate::Howl;

/// The output frames of a group of the zig-zag interpolation, and the input frames it takes.
const PHASES: u64 = 7;
const INPUTS: u64 = 6;
const TAPS: usize = 29;

/// `s_xaZigZagTable`: by tap, the weight in 32768ths of each phase.
#[rustfmt::skip]
const ZIGZAG: [[i16; PHASES as usize]; TAPS] = [
    [0, 0, 0, 0, -0x0001, 0x0002, -0x0005],
    [0, 0, 0, -0x0001, 0x0003, -0x0008, 0x0011],
    [0, 0, -0x0001, 0x0003, -0x0008, 0x0010, -0x0023],
    [0, -0x0002, 0x0003, -0x0008, 0x0011, -0x0023, 0x0046],
    [0, 0, -0x0002, 0x0006, -0x0010, 0x002B, -0x0017],
    [-0x0002, 0x0003, -0x0005, 0x0005, 0x000A, 0x001A, -0x0044],
    [0x000A, -0x0013, 0x001F, -0x001B, 0x006B, -0x00EB, 0x015B],
    [-0x0022, 0x003C, -0x004A, 0x00A6, -0x016D, 0x027B, -0x0347],
    [0x0041, -0x004B, 0x00B3, -0x01A8, 0x0350, -0x0548, 0x080E],
    [-0x0054, 0x00A2, -0x0192, 0x0372, -0x0623, 0x0AFA, -0x1249],
    [0x0034, -0x00E3, 0x02B1, -0x05BF, 0x0BCD, -0x16FA, 0x3C07],
    [0x0009, 0x0132, -0x039E, 0x09B8, -0x1780, 0x53E0, 0x53E0],
    [-0x010A, -0x0043, 0x04F8, -0x11B4, 0x6794, 0x3C07, -0x16FA],
    [0x0400, -0x0267, -0x05A6, 0x74BB, 0x234C, -0x1249, 0x0AFA],
    [-0x0A78, 0x0C9D, 0x7939, 0x0C9D, -0x0A78, 0x080E, -0x0548],
    [0x234C, 0x74BB, -0x05A6, -0x0267, 0x0400, -0x0347, 0x027B],
    [0x6794, -0x11B4, 0x04F8, -0x0043, -0x010A, 0x015B, -0x00EB],
    [-0x1780, 0x09B8, -0x039E, 0x0132, 0x0009, -0x0044, 0x001A],
    [0x0BCD, -0x05BF, 0x02B1, -0x00E3, 0x0034, -0x0017, 0x002B],
    [-0x0623, 0x0372, -0x0192, 0x00A2, -0x0054, 0x0046, -0x0023],
    [0x0350, -0x01A8, 0x00B3, -0x004B, 0x0041, -0x0023, 0x0010],
    [-0x016D, 0x00A6, -0x004A, 0x003C, -0x0022, 0x0011, -0x0008],
    [0x006B, -0x001B, 0x001F, -0x0013, 0x000A, -0x0005, 0x0002],
    [0x000A, 0x0005, -0x0005, 0x0003, -0x0001, 0, 0],
    [-0x0010, 0x0006, -0x0002, 0, 0, 0, 0],
    [0x0011, -0x0008, 0x0003, -0x0002, 0x0001, 0, 0],
    [-0x0008, 0x0003, -0x0001, 0, 0, 0, 0],
    [0x0003, -0x0001, 0, 0, 0, 0, 0],
    [-0x0001, 0, 0, 0, 0, 0, 0],
];

/// `XA_CurrOffset` counts blocks of this many output frames.
const OFFSET_SHIFT: u32 = 8;
/// `CDSYS_XA_VOLUME_SHIFT`: the game's volume (0 to 255) to the CD's.
const VOLUME_SHIFT: u32 = 7;
/// `CDSYS_XA_FADE_VOLUME_STEP`: what a fading track loses each block of output frames.
const FADE_STEP: i32 = 0x400;

/// The CD's input of the SPU (`struct NativeAudioXA`, the CD attributes of `SpuCommonAttr`).
#[derive(Clone, Debug)]
pub struct Cd {
    /// The track, None while it is read.
    pcm: Option<Box<Pcm>>,
    /// It plays (or is read, to play).
    active: bool,
    /// The output frames played.
    played: u64,
    /// Left, right, in 32768ths (`SpuSetCommonCDVolume`).
    pub volume: [i16; 2],
    /// It is heard (`SpuSetCommonCDMix`); its time runs either way.
    pub mix: bool,
    /// It is heard through the reverb (`SpuSetCommonCDReverb`).
    pub reverb: bool,
}

impl Default for Cd {
    /// As `howl_LoadHeader` leaves it: heard, without the reverb, at no volume.
    fn default() -> Cd {
        Cd {
            pcm: None,
            active: false,
            played: 0,
            volume: [0; 2],
            mix: true,
            reverb: false,
        }
    }
}

impl Cd {
    /// `NativeAudio_GetXAOutputFrameCount`: the output frames of the track.
    fn length(&self) -> u64 {
        let Some(pcm) = &self.pcm else {
            return 0;
        };
        let frames = pcm.frames.len() as u64;
        match pcm.rate {
            RATE_HALF => (frames * PHASES).div_ceil(INPUTS / 2),
            _ => (frames * PHASES).div_ceil(INPUTS),
        }
    }

    /// Whether it plays (`NativeAudio_IsXAPlaying`).
    pub fn playing(&self) -> bool {
        self.active
    }

    /// `NativeAudio_GetXACurrOffset`: the blocks of output frames played.
    fn offset(&self) -> i32 {
        (self.played.min(self.length()) >> OFFSET_SHIFT).min(i32::MAX as u64) as i32
    }

    /// `NativeAudio_CloseXANoLock`.
    fn stop(&mut self) {
        self.pcm = None;
        self.active = false;
        self.played = 0;
    }

    /// The next output frame of the track (`NativeAudio_MixFrame`'s), if it plays and is heard.
    pub(crate) fn next(&mut self) -> Option<[i32; 2]> {
        if !self.active || self.pcm.is_none() {
            return None;
        }
        let length = self.length();
        if self.played >= length {
            self.active = false;
            return None;
        }
        let frame = self.mix.then(|| [0, 1].map(|side| self.zigzag(side)));
        // `NativeAudio_AdvanceXAOutputFrameNoLock`.
        self.played += 1;
        if self.played >= length {
            self.active = false;
        }
        frame
    }

    /// `NativeAudio_ZigZagInterpolateXASampleNoLock`: the output frame `played` of `side`, from
    /// the 29 input frames before the group's end.
    fn zigzag(&self, side: usize) -> i32 {
        let Some(pcm) = &self.pcm else {
            return 0;
        };
        let (group, phase) = (self.played / PHASES, (self.played % PHASES) as usize);
        let end = (group + 1) * INPUTS;
        let sample = |at: u64| {
            // 18.9 kHz plays each frame twice.
            let at = if pcm.rate == RATE_HALF { at >> 1 } else { at };
            pcm.frames
                .get(at as usize)
                .map_or(0, |frame| frame[side] as i32)
        };
        let sum: i32 = (0..TAPS)
            .filter_map(|tap| end.checked_sub(tap as u64 + 1).map(|at| (tap, at)))
            .map(|(tap, at)| ((sample(at) as i64 * ZIGZAG[tap][phase] as i64) >> 15) as i32)
            .sum();
        sum.clamp(i16::MIN as i32, i16::MAX as i32)
    }
}

/// `XA_State`, as the native path has it: a track plays from when it is asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum XaState {
    #[default]
    Idle,
    Playing,
    /// It fades out, and then stops.
    Fading,
}

/// A track to read from the disc, for [`Howl::start_xa`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XaRequest {
    /// Which request it is: a track asked for after it takes its place.
    pub serial: u32,
    pub category: Category,
    pub track: Track,
}

/// What `CDSYS` keeps of the XA.
#[derive(Clone, Debug, Default)]
pub struct Xa {
    pub(crate) state: XaState,
    /// `XA_VolumeBitshift`: the CD's volume, which a fade takes down.
    volume: i32,
    /// `XA_VolumeDeduct`.
    deduct: i32,
    /// `XA_CurrOffset`: the blocks of output frames played, as of the last frame of the game.
    pub(crate) offset: i32,
    /// `XA_Playing_Category`, `XA_Playing_Index`.
    playing: Option<(Category, u16)>,
    /// The track to read, and the request the track that is read must be.
    request: Option<XaRequest>,
    serial: u32,
}

impl Howl {
    /// `CDSYS_XAPlay`: track `id` of `category` plays, at the music's volume or the voices'.
    /// False if there is no such track.
    pub fn play_xa(&mut self, category: Category, id: u16) -> bool {
        let Some(track) = self.xnf.track(category, id) else {
            return false;
        };
        let volume = match category {
            Category::Music => self.volume_music,
            _ => self.volume_voice,
        };
        let volume = (volume as i32) << VOLUME_SHIFT;
        // `NativeAudio_PlayXATrack`.
        let cd = &mut self.spu.cd;
        cd.stop();
        cd.active = true;
        cd.volume = [volume as i16; 2];
        let xa = &mut self.xa;
        xa.serial = xa.serial.wrapping_add(1);
        xa.request = Some(XaRequest {
            serial: xa.serial,
            category,
            track,
        });
        xa.state = XaState::Playing;
        xa.playing = Some((category, id));
        xa.volume = volume;
        xa.offset = 0;
        true
    }

    /// `CDSYS_XAGetTrackLength`: the sectors track `id` of `category` spans, 0 if there is none.
    pub fn xa_length(&self, category: Category, id: u16) -> u16 {
        self.xnf
            .track(category, id)
            .map_or(0, |track| track.sectors)
    }

    /// `CDSYS_XAPauseRequest`: what plays fades out.
    pub fn pause_xa(&mut self) {
        if self.xa.state == XaState::Playing {
            self.xa.state = XaState::Fading;
            self.xa.deduct = FADE_STEP;
        }
    }

    /// `CDSYS_XAPauseForce`: what plays stops at once.
    pub fn stop_xa(&mut self) {
        self.spu.cd.stop();
        self.xa.request = None;
        self.xa.state = XaState::Idle;
    }

    /// `CDSYS_XAPauseAtEnd`, once a frame of the game: a track that is over is, and one that
    /// fades loses as much volume as the time that has played since.
    pub fn update_xa(&mut self) {
        let playing = self.spu.cd.playing();
        let xa = &mut self.xa;
        let mut steps = 0;
        if playing {
            let now = self.spu.cd.offset();
            steps = (now - xa.offset.min(now)).max(0);
            xa.offset = now;
        }
        if xa.state == XaState::Fading {
            // `CDSYS_NativeGetXAFadeAmount`: a step at least, and no more than is left.
            let steps = steps.max(1);
            let fade = if xa.deduct <= 0 || steps >= (xa.volume + xa.deduct - 1) / xa.deduct {
                xa.volume
            } else {
                xa.deduct * steps
            };
            xa.volume -= fade;
            if xa.volume <= 0 {
                xa.volume = 0;
                self.stop_xa();
            } else {
                self.spu.cd.volume = [xa.volume as i16; 2];
            }
        } else if !playing {
            xa.state = XaState::Idle;
        }
    }

    /// `XA_State`.
    pub fn xa_state(&self) -> XaState {
        self.xa.state
    }

    /// What plays: its category and number.
    pub fn xa_playing(&self) -> Option<(Category, u16)> {
        (self.xa.state != XaState::Idle)
            .then_some(self.xa.playing)
            .flatten()
    }

    /// The track that was asked for and is to be read, once.
    pub fn take_xa_request(&mut self) -> Option<XaRequest> {
        self.xa.request.take()
    }

    /// The track of request `serial`, read and decoded (None if it could not be): it plays from
    /// its start, if it is still the one asked for.
    pub fn start_xa(&mut self, serial: u32, pcm: Option<Pcm>) {
        let cd = &mut self.spu.cd;
        if serial != self.xa.serial || !cd.active || cd.pcm.is_some() {
            return;
        }
        match pcm {
            Some(pcm) if !pcm.frames.is_empty() => cd.pcm = Some(Box::new(pcm)),
            _ => cd.stop(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use loader::xa::{RATE_FULL, Xnf};

    /// An index of a track in each category, `sectors` long.
    pub(crate) fn xnf(sectors: u16) -> Xnf {
        let track = Track {
            channel: 0,
            file: 0,
            sectors,
        };
        Xnf {
            files: [1; 3],
            tracks: [vec![track; 8], vec![track; 62], vec![track; 304]],
        }
    }

    /// `seconds` of a loud square wave at the full rate.
    pub(crate) fn tone(seconds: usize) -> Pcm {
        let frames = (0..seconds * RATE_FULL as usize)
            .map(|i| [if i / 50 % 2 == 0 { 20_000 } else { -20_000 }; 2])
            .collect();
        Pcm {
            rate: RATE_FULL,
            frames,
        }
    }

    fn howl() -> Howl {
        let mut howl = Howl::new(Vec::new(), Vec::new(), Vec::new(), Vec::new());
        howl.xnf = xnf(100);
        howl
    }

    /// Plays music track 0, read at once.
    fn playing() -> Howl {
        let mut howl = howl();
        assert!(howl.play_xa(Category::Music, 0));
        let request = howl.take_xa_request().unwrap();
        howl.start_xa(request.serial, Some(tone(1)));
        howl
    }

    #[test]
    fn a_track_plays_to_its_end_at_44_1_khz() {
        let mut howl = playing();
        assert_eq!(howl.xa_state(), XaState::Playing);
        let mut out = vec![[0; 2]; 46_000];
        howl.render(&mut out);
        // 37800 frames are 44100 frames of output.
        let last = out.iter().rposition(|&f| f != [0, 0]).unwrap();
        assert!((44_090..44_110).contains(&last), "{last}");
        // At the music's volume (175 << 7) and the master's (0x3fff): 20000 to about 6840, a
        // little more where the interpolation rings at the square's edges.
        let loudest = out.iter().map(|f| f[0].unsigned_abs()).max().unwrap();
        assert!((6_500..8_500).contains(&loudest), "{loudest}");
        assert_eq!(howl.xa_state(), XaState::Playing);
        howl.update_xa();
        assert_eq!(howl.xa_state(), XaState::Idle);
    }

    #[test]
    fn a_track_being_read_waits_silently_and_counts_as_playing() {
        let mut howl = howl();
        howl.play_xa(Category::Game, 3);
        let request = howl.take_xa_request().unwrap();
        assert_eq!(howl.take_xa_request(), None);
        let mut out = vec![[0; 2]; 2_000];
        howl.render(&mut out);
        howl.update_xa();
        assert!(out.iter().all(|&f| f == [0, 0]));
        assert_eq!(howl.xa_playing(), Some((Category::Game, 3)));
        assert_eq!(howl.xa.offset, 0);
        // Another track asked for meanwhile takes its place: the first, read, is not played.
        howl.play_xa(Category::Game, 4);
        howl.start_xa(request.serial, Some(tone(1)));
        howl.render(&mut out);
        assert!(out.iter().all(|&f| f == [0, 0]));
        // A track that cannot be read is over.
        let request = howl.take_xa_request().unwrap();
        howl.start_xa(request.serial, None);
        howl.update_xa();
        assert_eq!(howl.xa_state(), XaState::Idle);
    }

    #[test]
    fn a_pause_fades_the_track_out_by_the_time_it_plays() {
        let mut howl = playing();
        howl.render(&mut [[0; 2]; 512]);
        howl.update_xa();
        assert_eq!(howl.xa.offset, 2);
        howl.pause_xa();
        assert_eq!(howl.xa_state(), XaState::Fading);
        // A step for each block of 256 output frames: 175 << 7 is 22 steps.
        howl.render(&mut [[0; 2]; 256 * 10]);
        howl.update_xa();
        assert_eq!(howl.spu.cd.volume, [(175 << 7) - 10 * FADE_STEP as i16; 2]);
        howl.render(&mut [[0; 2]; 256 * 12]);
        howl.update_xa();
        assert_eq!(howl.xa_state(), XaState::Idle);
        assert!(!howl.spu.cd.playing());
    }

    #[test]
    fn there_is_no_track_past_the_index() {
        let mut howl = howl();
        assert!(!howl.play_xa(Category::Music, 8));
        assert_eq!(howl.xa_length(Category::Music, 8), 0);
        assert_eq!(howl.xa_length(Category::Extra, 61), 100);
        assert_eq!(howl.xa_state(), XaState::Idle);
    }

    #[test]
    fn the_cd_unheard_still_plays_on() {
        let mut howl = playing();
        howl.spu.cd.mix = false;
        let mut out = vec![[0; 2]; 1_000];
        howl.render(&mut out);
        assert!(out.iter().all(|&f| f == [0, 0]));
        howl.update_xa();
        assert_eq!(howl.xa.offset, 3);
    }
}
