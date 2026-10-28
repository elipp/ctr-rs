//! The sounds of `KART.HWL` on the disc image, played.

use std::path::PathBuf;

use howl::{AntiSpam, Howl, Sfx};

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
