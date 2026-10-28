mod common;

use loader::disc::open_howl;
use loader::howl::{decode, other_fx_flags};

/// The traffic lights' sounds (`DotLights_AudioAndVideo`): one beep, the green a fourth higher.
const RED_LIGHT: usize = 0x45;
const GREEN_LIGHT: usize = 0x46;

#[test]
fn kart_hwl_sounds() {
    let path = common::disc_path();
    if !path.exists() {
        eprintln!(
            "skipping: {} not found (set CTR_BIN to override)",
            path.display()
        );
        return;
    }
    let (mut disc, howl) = open_howl(&path).expect("read KART.HWL");
    assert_eq!(howl.sample_sizes.len(), 528);
    assert_eq!(howl.other_fx.len(), 258);
    assert_eq!(howl.bank_offsets.len(), 71);

    let (red, green) = (howl.other_fx[RED_LIGHT], howl.other_fx[GREEN_LIGHT]);
    assert_eq!((red.sample, red.pitch, red.volume), (110, 0x400, 178));
    assert_eq!((green.sample, green.pitch), (110, 0x556));
    // The level banks have it, bank 0 does not; it is found either way.
    let adpcm = howl
        .adpcm(&mut disc, red.sample)
        .unwrap()
        .expect("in a bank");
    let pcm = decode(&adpcm);
    // 248 blocks to the end flag: 0.63 s at the red light's 11025 Hz, as long as its voice is held
    // (189 300ths of a second).
    assert_eq!(pcm.samples.len(), 248 * 28);
    assert!(pcm.samples.iter().any(|&s| s.unsigned_abs() > 20_000));

    // Every sound's sample is in some bank.
    for (id, fx) in howl.other_fx.iter().enumerate() {
        let adpcm = howl.adpcm(&mut disc, fx.sample).unwrap();
        assert!(
            adpcm.is_some(),
            "sound {id:#x}: sample {} in no bank",
            fx.sample
        );
    }
    // An engine class has four loops, one for each player, of one sample; the AIs' are the fifth
    // class's.
    assert_eq!(howl.engine_fx.len(), 20);
    for (id, fx) in howl.engine_fx.iter().enumerate() {
        assert_eq!(fx.sample, howl.engine_fx[id & !3].sample);
        let adpcm = howl
            .adpcm(&mut disc, fx.sample)
            .unwrap()
            .expect("in a bank");
        assert!(
            decode(&adpcm).loop_start.is_some(),
            "engine {id:#x} does not loop"
        );
    }
    assert_eq!(howl.engine_fx[0x10].sample, 4);
    // The ground sounds of terrains (`MetaDataTerrain::sound`) loop, except wood's and track's.
    assert_eq!(howl.other_fx[21].flags & other_fx_flags::LOOPS, 0);
    assert_ne!(howl.other_fx[24].flags & other_fx_flags::LOOPS, 0);
}
