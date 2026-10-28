//! The sounds of `KART.HWL` on the disc image, played.

use std::path::PathBuf;

use howl::{AntiSpam, Howl, Sfx};
use loader::howl::other_fx_flags;

/// `CTR_BIN` overrides the default of `<repo>/assets/ctr-u.bin`.
fn howl() -> Option<Howl> {
    let path = std::env::var_os("CTR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin")
        });
    if !path.exists() {
        eprintln!("skipping: {} not found (set CTR_BIN)", path.display());
        return None;
    }
    Some(Howl::read(&path).expect("read KART.HWL"))
}

/// `DotLights_AudioAndVideo`: a red light's beep.
const RED_LIGHT: u16 = 0x45;

#[test]
fn a_red_light_beeps_for_as_long_as_its_voice_is_held() {
    let Some(mut howl) = howl() else {
        return;
    };
    howl.play(RED_LIGHT, AntiSpam::Off, Sfx::FULL).unwrap();
    let mut out = vec![[0; 2]; howl::spu::RATE as usize];
    howl.render(&mut out);
    let heard: Vec<usize> = (0..out.len()).filter(|&i| out[i] != [0, 0]).collect();
    let (first, last) = (heard[0], *heard.last().unwrap());
    // Held 189/300 s, 5 a vertical blank from the one that starts it: the 37th lets it go,
    // before its sample (0.63 s) is over.
    let blank = howl::spu::RATE as usize / 60;
    // After the sample's first block, which is silent: 28 samples at a quarter of the rate.
    assert_eq!(first, 114);
    assert!(
        (36 * blank..=37 * blank + 8).contains(&last),
        "{last} samples"
    );
    let loudest = out
        .iter()
        .flatten()
        .map(|s| s.unsigned_abs())
        .max()
        .unwrap();
    assert!((1_000..16_000).contains(&loudest), "{loudest}");
}

#[test]
fn the_sounds_the_karts_send_once_do_not_loop() {
    let Some(howl) = howl() else {
        return;
    };
    // The ids of `Effect::Sound` in `vehicle` and `race`, the bumps of `crash_sound`, the boost,
    // the menu's, the lap's and the win's.
    let once = [
        6, 7, 8, 10, 11, 12, 0xf, 0x10, 21, 0x41, 0x42, 0x47, 0x4a, 0x52, 0x53, 0x54, 0x55, 0x57,
        0x58, 0x5a, 0x5b, 0x69, 0x71, 0x7e, 0xd, 0, 1, 2, 5, 0x66, 0x5f, 0x3c, 0x43, 0x49, 0x4c,
        0x3d, 0x3f, 0x50, 0x51, 0x3e, 0x70, 0x81,
    ];
    let fx = howl.other_fx();
    for id in once {
        assert_eq!(
            fx[id].flags & other_fx_flags::LOOPS,
            0,
            "sound {id:#x} loops"
        );
    }
}

#[test]
fn the_loops_loop() {
    let Some(howl) = howl() else {
        return;
    };
    // The turbo's, the wall rub's, the skids', the grounds', the item roulette's, and the loops
    // of the bomb, the missile, the shot shield, the minecart and the barrel.
    let fx = howl.other_fx();
    for id in [
        0xe, 0x14, 17, 18, 19, 22, 23, 24, 25, 26, 27, 0x5d, 0x48, 0x4b, 0x59, 0x72, 0x74,
    ] {
        assert_ne!(fx[id].flags & other_fx_flags::LOOPS, 0, "sound {id:#x}");
    }
}
