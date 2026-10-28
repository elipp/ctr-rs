//! Renders sounds of `KART.HWL` one after another to a WAV file, to listen to the sound engine
//! without the game:
//!
//! ```text
//! cargo run -p howl --example render -- out.wav 0x45 0x45 0x45 0x46
//! ```
//!
//! A number is a sound effect, played until its time is up (one that loops is stopped after two
//! seconds). The disc image is `CTR_BIN`, or `assets/ctr-u.bin`.

use std::io::Write;
use std::path::PathBuf;

use howl::{AntiSpam, Howl, Sfx};

/// A sixtieth of a second of output.
const BLANK: usize = howl::spu::RATE as usize / 60;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("usage: render <out.wav> <sound>...");
    let disc = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin")
        });
    let mut howl = Howl::read(&disc)?;
    let sfx = Sfx::FULL;
    let mut frames = Vec::new();
    let mut blank = |howl: &mut Howl, n: usize| {
        let at = frames.len();
        frames.resize(at + n * BLANK, [0; 2]);
        howl.render(&mut frames[at..]);
    };
    for arg in args {
        let number = |s: &str| match s.strip_prefix("0x") {
            Some(hex) => u16::from_str_radix(hex, 16),
            None => s.parse(),
        };
        let id = number(&arg).expect("a sound number");
        let Some(sound) = howl.play(id, AntiSpam::Off, sfx) else {
            eprintln!("sound {arg} does not play");
            continue;
        };
        let fx = howl.other_fx()[id as usize];
        let loops = fx.flags & loader::howl::other_fx_flags::LOOPS != 0;
        // Its duration is in 300ths of a second, 5 a blank.
        let held = if loops {
            120
        } else {
            fx.duration as usize / 5 + 1
        };
        blank(&mut howl, held);
        howl.stop(sound);
        blank(&mut howl, 12);
    }

    let mut wav = std::io::BufWriter::new(std::fs::File::create(&out)?);
    let data = (frames.len() * 4) as u32;
    let rate = howl::spu::RATE;
    wav.write_all(b"RIFF")?;
    wav.write_all(&(36 + data).to_le_bytes())?;
    wav.write_all(b"WAVEfmt ")?;
    // PCM, 2 channels, 16 bits.
    for field in [
        16u32.to_le_bytes(),
        [1, 0, 2, 0],
        rate.to_le_bytes(),
        (rate * 4).to_le_bytes(),
    ] {
        wav.write_all(&field)?;
    }
    wav.write_all(&[4, 0, 16, 0])?;
    wav.write_all(b"data")?;
    wav.write_all(&data.to_le_bytes())?;
    for [left, right] in frames {
        wav.write_all(&left.to_le_bytes())?;
        wav.write_all(&right.to_le_bytes())?;
    }
    eprintln!("{out}: {:.1} s", data as f32 / 4.0 / rate as f32);
    Ok(())
}
