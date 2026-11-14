//! The scripts of overlay 233 that the podium's dancers follow (`CS_Thread`, `CS_ScriptCmd`): a
//! bytecode of animation ranges, jumps and sounds, read from the overlay as the disc has it.
//!
//! Only what the podium's scripts use is here: the animation ranges (`ANIM_ROT_RANGE`,
//! `ANIM_SYNC_MARKER`, `ANIM_RANGE`), `GOTO`, `BRANCH_IF_RANDOM_LE`, the end, the sound effects,
//! the level of detail and the particles' id, whose particles ([`Scripts::particles`]) the
//! viewer makes. The rest of
//! the opcodes stop a script where it is, as `CS_Thread_UseOpcode`'s default does.

use std::io;

/// Where retail loads the overlay (`OVR_Region3`): its pointers are addresses from here.
const BASE: u32 = 0x800a_b9f0;

/// `csOpcodeMetaPrefix`: for each opcode, which fields follow it (`CsOpcodeMetaFlags`).
const META_TABLE: usize = 0x5adc;
const META_LEN: usize = 0x34;

/// `danceFirstScripts` and `danceOtherScripts`: a pointer to the script of each character's
/// dance, on the top step and on a lower one.
const DANCE_FIRST: usize = 0x7488;
const DANCE_OTHER: usize = 0x879c;

/// `script_tawnaNormal`: the trophy girl's.
const TAWNA: usize = 0x5dc4;

/// `particleConfigs`: what each particle id of a script makes (`struct CsParticleConfig`).
const PARTICLE_CONFIGS: usize = 0x5a7c;
const PARTICLE_CONFIG_SIZE: usize = 0xc;
const PARTICLE_CONFIG_COUNT: usize = PARTICLE_IDS;

/// The particle ids that make particles: 0 to 7.
pub const PARTICLE_IDS: usize = 8;

/// `CsParticleConfigMeta::flags`: the config after this one is made too.
const PARTICLE_CONFIG_CHAINED: u8 = 1;

/// How many characters the dance tables have a script for (`CS_DANCE_MODEL_SCRIPT_COUNT`).
pub const DANCERS: usize = 0x10;

/// `CS_PODIUM_CHARACTER_SCALE`: the size of everyone on the podium.
pub const PODIUM_SCALE: i16 = 0x2800;

mod meta {
    pub const ANIM_INDEX: u8 = 0x01;
    pub const FRAME_START: u8 = 0x02;
    pub const FRAME_END: u8 = 0x04;
    pub const ARG0: u8 = 0x08;
    pub const ARG1: u8 = 0x10;
    pub const ALIGNED_ARG1: u8 = 0x20;
    pub const ROT_START: u8 = 0x40;
    pub const ROT_END: u8 = 0x80;
}

/// `CutsceneOpcode`, those the podium uses.
mod op {
    pub const ANIM_ROT_RANGE: u8 = 0x00;
    pub const GOTO: u8 = 0x01;
    pub const HIDE_INSTANCE_AND_END_THREAD: u8 = 0x02;
    pub const BRANCH_IF_RANDOM_LE: u8 = 0x04;
    pub const PLAY_CONTEXT_FX: u8 = 0x05;
    pub const STOP_FX: u8 = 0x06;
    pub const SET_VISIBLE_LOD: u8 = 0x09;
    pub const SET_PARTICLE_ID: u8 = 0x19;
    pub const ANIM_SYNC_MARKER: u8 = 0x2a;
    pub const ANIM_RANGE: u8 = 0x2b;
}

/// `CS_FRAME32_SHIFT`: an animation's clock counts 32 to a frame, in the game's milliseconds.
const FRAME32_SHIFT: i32 = 5;

/// How many opcodes one tick may run before it is taken for a script that loops without time
/// passing (retail would hang there).
const MAX_STEPS: usize = 10_000;

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Overlay 233 as the disc has it, where the scripts are read from.
#[derive(Clone, Debug)]
pub struct Scripts {
    bytes: Vec<u8>,
}

impl Scripts {
    /// The overlay's bytes; an error if it is too short for the tables the podium reads.
    pub fn parse(bytes: Vec<u8>) -> io::Result<Scripts> {
        let scripts = Scripts { bytes };
        scripts.word(DANCE_OTHER + (DANCERS - 1) * 4)?;
        scripts.bytes(META_TABLE, META_LEN)?;
        Ok(scripts)
    }

    fn bytes(&self, at: usize, len: usize) -> io::Result<&[u8]> {
        self.bytes
            .get(at..at + len)
            .ok_or_else(|| invalid(format!("overlay 233 has no {len} bytes at {at:#x}")))
    }

    fn word(&self, at: usize) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(at, 4)?.try_into().unwrap()))
    }

    fn half(&self, at: usize) -> io::Result<i16> {
        Ok(i16::from_le_bytes(self.bytes(at, 2)?.try_into().unwrap()))
    }

    /// Where an address of the overlay is in it.
    fn offset(&self, address: u32) -> io::Result<usize> {
        let at = address.wrapping_sub(BASE) as usize;
        if at < self.bytes.len() {
            Ok(at)
        } else {
            Err(invalid(format!("{address:#x} is not in overlay 233")))
        }
    }

    /// The script of `character`'s dance (`GAME_CHARACTER_IDS`), on the top step or a lower one.
    pub fn dance(&self, character: usize, first: bool) -> io::Result<usize> {
        if character >= DANCERS {
            return Err(invalid(format!("no dance for character {character}")));
        }
        let table = if first { DANCE_FIRST } else { DANCE_OTHER };
        self.offset(self.word(table + character * 4)?)
    }

    /// The trophy girl's script.
    pub fn tawna(&self) -> usize {
        TAWNA
    }

    /// The overlay's bytes, where [`Burst::emitters`] are.
    pub fn overlay(&self) -> &[u8] {
        &self.bytes
    }

    /// `CS_Thread_Particles`: what a dancer whose script set `particle_id` makes each tick. None
    /// for an id of no particles (retail also lets 8 through, into the opcodes' table).
    pub fn particles(&self, particle_id: i32) -> io::Result<Vec<Burst>> {
        let mut bursts = Vec::new();
        let mut id = particle_id as u8 as usize;
        while id < PARTICLE_CONFIG_COUNT {
            let at = PARTICLE_CONFIGS + id * PARTICLE_CONFIG_SIZE;
            let meta = self.bytes(at + 4, 5)?;
            bursts.push(Burst {
                emitters: self.offset(self.word(at)?)?,
                frame_offset: meta[1] as usize,
                count: meta[2],
            });
            if meta[3] & PARTICLE_CONFIG_CHAINED == 0 {
                break;
            }
            id += 1;
        }
        Ok(bursts)
    }

    /// `CS_ScriptCmd_ReadOpcode_Main`: the opcode at `at`, and where the next one is.
    fn decode(&self, at: usize) -> io::Result<Opcode> {
        let opcode = *self
            .bytes
            .get(at)
            .ok_or_else(|| invalid(format!("no opcode at {at:#x}")))?;
        // Retail reads the table past its end for an opcode it does not know, into the scripts.
        let flags = *self
            .bytes
            .get(META_TABLE + opcode as usize)
            .ok_or_else(|| invalid(format!("opcode {opcode:#x} at {at:#x}")))?;
        let mut cursor = at + 1;
        let mut half = |bit: u8| -> io::Result<i16> {
            if flags & bit == 0 {
                return Ok(0);
            }
            let value = self.half(cursor)?;
            cursor += 2;
            Ok(value)
        };
        let anim_index = half(meta::ANIM_INDEX)?;
        let frame_start = half(meta::FRAME_START)?;
        let frame_end = half(meta::FRAME_END)?;
        let mut word = |bit: u8| -> io::Result<i32> {
            if flags & bit == 0 {
                return Ok(0);
            }
            let value = self.word(cursor)? as i32;
            cursor += 4;
            Ok(value)
        };
        let arg0 = word(meta::ARG0)?;
        let mut arg1 = word(meta::ARG1)?;
        if flags & meta::ALIGNED_ARG1 != 0 {
            // (The overlay's base is aligned, so its offsets align as its addresses do.)
            cursor = cursor.next_multiple_of(4);
            arg1 = self.word(cursor)? as i32;
            cursor += 5;
        }
        let mut half = |bit: u8| -> io::Result<i16> {
            if flags & bit == 0 {
                return Ok(0);
            }
            let value = self.half(cursor)?;
            cursor += 2;
            Ok(value)
        };
        let rot_start = half(meta::ROT_START)?;
        let rot_end = half(meta::ROT_END)?;
        Ok(Opcode {
            opcode,
            anim_index,
            frame_start,
            frame_end,
            arg0,
            arg1,
            rot_start,
            rot_end,
            next: cursor,
        })
    }
}

/// `CsOpcodeMeta`: an opcode and its fields, and where the next one is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Opcode {
    opcode: u8,
    anim_index: i16,
    frame_start: i16,
    frame_end: i16,
    arg0: i32,
    arg1: i32,
    rot_start: i16,
    rot_end: i16,
    next: usize,
}

/// `struct CsParticleConfig`: `count` particles of the emitter table at `emitters` (an offset in
/// [`Scripts::overlay`]), from the vertex `frame_offset` of the dancer's pose
/// (`CS_Instance_GetFrameData`). The icons are of `iconGroup[1]`, the smoke's, for all of the
/// podium's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Burst {
    pub emitters: usize,
    pub frame_offset: usize,
    pub count: u8,
}

/// What a dancer's model has, that its script goes by: the frames of each animation of each
/// level of detail (`ModelAnim::numFrames`, both the stored frames and the blended ones).
pub trait Frames {
    /// 0 if there is no such animation.
    fn num_frames(&self, lod: usize, anim: usize) -> i32;
    /// How many levels of detail the model has.
    fn lods(&self) -> usize;
}

/// What a script asks of the world as it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// `OtherFX_Play(id, 1)`.
    Play(u16),
    /// `OtherFX_Stop2(id)`.
    Stop(u16),
}

/// A thread of the cutscene with a model (`struct CutsceneObj`), as far as the podium uses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dancer {
    op: Opcode,
    /// `animFrame32`: where the animation is, 32 to a frame.
    frame32: i32,
    duration: i32,
    lod: usize,
    /// `baseRotY`: its heading when it was put on the podium.
    base_yaw: i16,
    /// `rot.y`: its heading now.
    yaw: i16,
    /// `particleID`, which no particles are drawn from yet.
    pub particle_id: i32,
    /// `Instance::animIndex` and `animFrame`: the pose to draw.
    pub anim: usize,
    pub frame: i32,
    /// The script ended (`HIDE_INSTANCE_AND_END_THREAD`): the model is no longer drawn.
    pub ended: bool,
}

impl Dancer {
    /// `CS_Thread_Init` for a model: its script starts at `script`, it faces `yaw`.
    pub fn new(scripts: &Scripts, script: usize, yaw: i16, rng: &mut i32) -> io::Result<Dancer> {
        let op = scripts.decode(script)?;
        let random = math::mix_rng_scramble(rng);
        let duration = op.frame_start as i32 + random_span(random, &op) as i16 as i32;
        let yaw = yaw & 0xfff;
        Ok(Dancer {
            op,
            // (Not in frames of 32: retail takes the first range's start as it is.)
            frame32: op.arg0,
            duration,
            lod: 0,
            base_yaw: yaw,
            yaw,
            particle_id: -1,
            anim: 0,
            frame: 0,
            ended: false,
        })
    }

    /// Its heading now, 0x1000 a turn.
    pub fn yaw(&self) -> i16 {
        self.yaw
    }

    /// The level of detail whose model is drawn.
    pub fn lod(&self) -> usize {
        self.lod
    }

    /// `CS_Thread_UseOpcode`, `elapsed` milliseconds of the game later (32 a tick): runs the
    /// script that far, and says what it asked for in `events`.
    pub fn tick(
        &mut self,
        scripts: &Scripts,
        model: &impl Frames,
        elapsed: i32,
        rng: &mut i32,
        events: &mut Vec<Event>,
    ) -> io::Result<()> {
        if self.ended {
            return Ok(());
        }
        let entry = self.op;
        let mut frame32 = self.frame32;
        let mut lod = self.lod;
        let mut duration = self.duration;
        let mut remaining = elapsed;
        let mut anim = self.op.anim_index as usize;
        let mut changed = false;
        let safe = |anim: usize, lod: usize, desired: i32| safe_frame(model, anim, lod, desired);
        let mut steps = 0;
        while remaining != 0 || changed {
            steps += 1;
            if steps > MAX_STEPS {
                return Err(invalid("a podium script loops without time passing"));
            }
            let op = self.op;
            match op.opcode {
                op::SET_PARTICLE_ID => {
                    self.particle_id = op.arg1;
                    self.op = scripts.decode(op.next)?;
                }
                op::SET_VISIBLE_LOD => {
                    if model.lods() != 0 {
                        lod = (op.arg1.max(0) as usize).min(model.lods() - 1);
                    }
                    self.op = scripts.decode(op.next)?;
                }
                op::HIDE_INSTANCE_AND_END_THREAD => {
                    self.op = entry;
                    self.ended = true;
                    return Ok(());
                }
                op::ANIM_ROT_RANGE | op::ANIM_SYNC_MARKER | op::ANIM_RANGE => {
                    if changed {
                        anim = op.anim_index as usize;
                        frame32 = safe(anim, lod, op.arg0) << FRAME32_SHIFT;
                        let random = math::mix_rng_scramble(rng);
                        changed = false;
                        duration = random_span(random, &op) + op.frame_start as i32;
                    }
                    let mut boundary = false;
                    if op.arg0 <= op.arg1 {
                        let end = (safe(anim, lod, op.arg1) + 1) << FRAME32_SHIFT;
                        frame32 += remaining;
                        if frame32 >= end {
                            boundary = true;
                            remaining = if end != 0 { frame32 - end } else { 0 };
                        }
                    } else {
                        let end = op.arg1 << FRAME32_SHIFT;
                        frame32 -= remaining;
                        if frame32 < end {
                            boundary = true;
                            remaining = end - frame32;
                        }
                    }
                    if boundary || duration < 1 {
                        duration -= 1;
                        if duration < 1 {
                            self.op = scripts.decode(op.next)?;
                            changed = true;
                        } else {
                            frame32 = safe(anim, lod, op.arg0) << FRAME32_SHIFT;
                        }
                    } else {
                        remaining = 0;
                    }
                }
                op::GOTO => {
                    self.op = scripts.decode(scripts.offset(op.arg1 as u32)?)?;
                    changed = true;
                }
                op::BRANCH_IF_RANDOM_LE => {
                    let random = math::mix_rng_scramble(rng);
                    self.op = if op.arg0 >= (random >> 2) & 0xff {
                        scripts.decode(scripts.offset(op.arg1 as u32)?)?
                    } else {
                        scripts.decode(op.next)?
                    };
                    changed = true;
                }
                op::PLAY_CONTEXT_FX => {
                    events.push(Event::Play(op.arg1 as i16 as u16));
                    self.op = scripts.decode(op.next)?;
                }
                op::STOP_FX => {
                    events.push(Event::Stop(op.arg1 as i16 as u16));
                    self.op = scripts.decode(op.next)?;
                }
                // What the podium does not use stops the script here.
                _ => {
                    self.op = entry;
                    return Ok(());
                }
            }
        }

        self.frame32 = frame32;
        self.duration = duration;
        self.lod = lod;
        let op = self.op;
        let frame = frame32 >> FRAME32_SHIFT;
        let mut yaw = op.rot_start as i32;
        if yaw != op.rot_end as i32 && op.arg1 != op.arg0 {
            let turn = ((op.rot_end as i32 - yaw + 0x800) & 0xfff) - 0x800;
            yaw += turn * (frame - op.arg0) / (op.arg1 - op.arg0).abs();
        }
        self.yaw = (yaw + self.base_yaw as i32) as i16;
        let (anim, frame) = if safe(anim, lod, frame) == frame {
            (anim, frame)
        } else {
            (0, 0)
        };
        self.anim = anim;
        self.frame = frame;
        Ok(())
    }
}

/// The random part of an animation range's repeats: up to `frameEnd - frameStart` more.
fn random_span(random: i32, op: &Opcode) -> i32 {
    (((random >> 2) & 0xfff) * (op.frame_end as i32 - op.frame_start as i32 + 1)) >> 12
}

/// `CS_Instance_SafeCheckAnimFrame`: `desired`, within the frames of the animation.
fn safe_frame(model: &impl Frames, anim: usize, lod: usize, desired: i32) -> i32 {
    if desired <= 0 {
        return 0;
    }
    let frames = model.num_frames(lod, anim);
    if frames > 0 {
        desired.min(frames - 1)
    } else {
        0
    }
}

/// The frames of the camera's path (`CS_PODIUM_CAMERA_FRAME`) over which `CS_Thread_UseOpcode`
/// does not draw the top step's dancer, and the second step's.
pub const FIRST_HIDDEN: std::ops::Range<i32> = 0x83..0x83 + 0x69;
pub const SECOND_HIDDEN: std::ops::Range<i32> = 0x65..0x65 + 0x87;

#[cfg(test)]
mod tests {
    use super::*;

    /// A model with one animation of `frames` frames.
    struct One(i32);

    impl Frames for One {
        fn num_frames(&self, lod: usize, anim: usize) -> i32 {
            if lod == 0 && anim == 0 { self.0 } else { 0 }
        }
        fn lods(&self) -> usize {
            1
        }
    }

    /// An overlay with the podium's tables, and `script` at the trophy girl's.
    fn overlay(script: &[u8]) -> Scripts {
        let mut bytes = vec![0; DANCE_OTHER + DANCERS * 4];
        let flags: [(u8, u8); 4] = [
            (
                op::ANIM_RANGE,
                meta::FRAME_START | meta::FRAME_END | meta::ARG0 | meta::ARG1,
            ),
            (op::GOTO, meta::ALIGNED_ARG1),
            (op::PLAY_CONTEXT_FX, meta::ARG1),
            (op::HIDE_INSTANCE_AND_END_THREAD, 0),
        ];
        for (op, flags) in flags {
            bytes[META_TABLE + op as usize] = flags;
        }
        bytes[TAWNA..TAWNA + script.len()].copy_from_slice(script);
        Scripts::parse(bytes).unwrap()
    }

    fn anim_range(first: i32, last: i32, repeats: i16) -> Vec<u8> {
        let mut out = vec![op::ANIM_RANGE];
        out.extend(repeats.to_le_bytes());
        out.extend(repeats.to_le_bytes());
        out.extend(first.to_le_bytes());
        out.extend(last.to_le_bytes());
        out
    }

    #[test]
    fn an_animation_range_plays_a_frame_a_tick_and_then_the_next_opcode() {
        let mut script = anim_range(0, 3, 1);
        script.push(op::PLAY_CONTEXT_FX);
        script.extend(0xa3i32.to_le_bytes());
        script.push(op::HIDE_INSTANCE_AND_END_THREAD);
        let scripts = overlay(&script);
        let model = One(10);
        let mut rng = 100;
        let mut dancer = Dancer::new(&scripts, scripts.tawna(), 0, &mut rng).unwrap();
        let mut events = Vec::new();
        let mut frames = Vec::new();
        for _ in 0..5 {
            dancer
                .tick(&scripts, &model, 32, &mut rng, &mut events)
                .unwrap();
            frames.push(dancer.frame);
        }
        assert_eq!(frames[..3], [1, 2, 3]);
        assert_eq!(events, [Event::Play(0xa3)]);
        assert!(dancer.ended);
    }

    #[test]
    fn a_goto_loops_the_range() {
        let mut script = anim_range(0, 1, 1);
        script.push(op::GOTO);
        while (TAWNA + script.len()) % 4 != 0 {
            script.push(0);
        }
        script.extend((BASE + TAWNA as u32).to_le_bytes());
        let scripts = overlay(&script);
        let mut rng = 100;
        let mut dancer = Dancer::new(&scripts, scripts.tawna(), 0, &mut rng).unwrap();
        let mut events = Vec::new();
        for _ in 0..20 {
            dancer
                .tick(&scripts, &One(10), 32, &mut rng, &mut events)
                .unwrap();
            assert!(dancer.frame <= 1 && !dancer.ended);
        }
    }

    #[test]
    fn a_frame_past_the_animation_is_its_last() {
        assert_eq!(safe_frame(&One(10), 0, 0, 50), 9);
        assert_eq!(safe_frame(&One(10), 0, 0, -1), 0);
        assert_eq!(safe_frame(&One(10), 1, 0, 5), 0);
    }
}
