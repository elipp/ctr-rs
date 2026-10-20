mod common;

use loader::bigindex::BigIndex;
use loader::dram::DramFile;
use loader::icons::parse_groups;

#[test]
fn packs_have_the_wheel_icons() {
    let Some(mut b) = common::open_bigfile() else { return };
    for character in 0..16 {
        let file = b.big.read(&mut b.disc, BigIndex::ARCADE_1P_PACK.nth(character).get()).unwrap();
        let groups = parse_groups(&DramFile::parse(&file).unwrap()).unwrap();
        let wheels = groups.iter().find(|g| g.group_id == 0).expect("group 0");
        if character == 0 {
            for g in &groups {
                eprintln!("{:?} id {} icons {}", g.name, g.group_id, g.icons.len());
            }
            for i in &wheels.icons {
                eprintln!("{:?} {:?}", i.name, i.layout);
            }
        }
        assert_eq!(wheels.icons.len(), 17, "character {character}");
    }
}

#[test]
fn packs_have_the_shadow_icons() {
    let Some(mut b) = common::open_bigfile() else { return };
    for character in 0..16 {
        let file = b.big.read(&mut b.disc, BigIndex::ARCADE_1P_PACK.nth(character).get()).unwrap();
        let icons = loader::icons::parse_icons(&DramFile::parse(&file).unwrap()).unwrap();
        for i in icons.iter().filter(|i| (0..4).contains(&i.global_index)) {
            eprintln!("{character}: {:?} {} {:?}", i.name, i.global_index, i.layout);
        }
    }
}

#[test]
fn packs_have_the_tiremark_icon() {
    let Some(mut b) = common::open_bigfile() else { return };
    for character in 0..16 {
        let file = b.big.read(&mut b.disc, BigIndex::ARCADE_1P_PACK.nth(character).get()).unwrap();
        let icons = loader::icons::parse_icons(&DramFile::parse(&file).unwrap()).unwrap();
        let mark = icons.iter().find(|i| i.global_index == 0x2f).unwrap_or_else(|| panic!("character {character}: no tiremark"));
        if character == 0 {
            eprintln!("{:?} {:?}", mark.name, mark.layout);
        }
    }
}
