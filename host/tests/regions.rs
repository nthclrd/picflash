//! Data EEPROM, EEPROM preservation and partial (region) programming, on the
//! bit-level model, directly and through the PICkit (Gen4) translation.

use picflash::algo::Entry;
use picflash::checksum::checksum;
use picflash::devdb::Database;
use picflash::device::Device;
use picflash::image::Image;
use picflash::pickit::Gen4Link;
use picflash::programmer::{Programmer, Regions};
use picflash::sim::{Gen4Sim, SimPic};
use picflash::transport::Transport;

const PARTS: [&str; 2] = ["PIC16F1789", "PIC16F18446"];

fn part(name: &str) -> Device {
    Database::embedded().find(name).cloned().unwrap()
}

fn program(d: &Device, seed: u16) -> Image {
    let mut image = Image::default();
    for addr in (0..40).chain(0x400..0x410) {
        image.flash.insert(addr, (addr as u16).wrapping_mul(seed) & 0x3FFF);
    }
    image.config.insert(d.user_ids.start, 0x0012);
    for c in &d.config {
        // Implemented bits cleared where the default allows, LVP kept on.
        let mut word = c.default & 0x3FFF & !(seed & 0x0003);
        if let Some((addr, bit)) = d.lvp_fuse {
            if addr == c.address {
                word |= bit;
            }
        }
        image.config.insert(c.address, word | !c.mask & 0x3FFF);
    }
    image
}

/// Runs `f` on a plain simulated PIC and on the same model behind the Gen4
/// translation, and hands back the PIC for inspection.
fn both(d: &Device, setup: impl Fn(&mut SimPic), f: impl Fn(&mut dyn Transport, &Device)) -> [SimPic; 2] {
    let mut plain = SimPic::new(d).unwrap();
    setup(&mut plain);
    f(&mut plain, d);
    let mut pic = SimPic::new(d).unwrap();
    setup(&mut pic);
    let mut link = Gen4Link::new(Gen4Sim::new(pic), d.algorithm().unwrap());
    f(&mut link, d);
    [plain, link.tool.pic]
}

#[test]
fn eeprom_write_verify_read() {
    for name in PARTS {
        let d = part(name);
        let mut image = program(&d, 0x2B);
        for i in 0..20u32 {
            image.eeprom.insert(i * 7, (i * 13) as u8);
        }
        image.eeprom.insert(255, 0x5A);
        for pic in both(&d, |_| {}, |link, d| {
            let mut prog = Programmer::new(link, d).unwrap();
            prog.write(&image, false).unwrap();
            assert!(prog.verify(&image).unwrap().is_empty(), "{name}");
            let dump = prog.read().unwrap();
            assert_eq!(dump.eeprom, image.eeprom, "{name} EEPROM read back");
            assert_eq!(checksum(&dump, d), checksum(&image, d), "{name} checksum");
        }) {
            for (&offset, &byte) in &image.eeprom {
                assert_eq!(pic.eeprom[offset as usize], byte, "{name} EEPROM {offset}");
            }
        }
    }
}

#[test]
fn erase_keeps_eeprom_on_request() {
    for name in PARTS {
        let d = part(name);
        let image = program(&d, 0x11);
        let stale = |pic: &mut SimPic| pic.eeprom[3] = 0x42;
        for keep in [true, false] {
            for pic in both(&d, stale, |link, d| {
                let mut prog = Programmer::new(link, d).unwrap();
                prog.keep_eeprom = keep;
                prog.write(&image, false).unwrap();
            }) {
                let expected = if keep { 0x42 } else { 0xFF };
                assert_eq!(pic.eeprom[3], expected, "{name} keep={keep}");
            }
        }
    }
}

#[test]
fn program_region_leaves_the_rest() {
    for name in PARTS {
        let d = part(name);
        let old = program(&d, 0x2B);
        let new = program(&d, 0x35);
        let mut only_flash = new.clone();
        only_flash.config.clear();
        let regions = Regions { program: true, ..Regions::NONE };
        for pic in both(&d, |_| {}, |link, d| {
            let mut prog = Programmer::new(link, d).unwrap();
            prog.write(&old, false).unwrap();
            // A word outside the new image's rows must survive.
            let mut extra = Image::default();
            extra.flash.insert(0x1000, 0x0ABC);
            prog.write_regions(&extra, regions, false).unwrap();
            prog.write_regions(&only_flash, regions, false).unwrap();
            assert!(prog.verify(&only_flash).unwrap().is_empty(), "{name}");
        }) {
            assert_eq!(pic.flash[0x1000], 0x0ABC, "{name} untouched row kept");
            for (&addr, &word) in &new.flash {
                assert_eq!(pic.flash[addr as usize], word, "{name} {addr:#x}");
            }
            assert_eq!(pic.config[0], 0x0012, "{name} user ID kept");
        }
    }
}

#[test]
fn config_region_can_only_clear_bits() {
    for name in PARTS {
        let d = part(name);
        let base = program(&d, 0x2B);
        let first = d.config[0].address;
        let mut sim = SimPic::new(&d).unwrap();
        let mut prog = Programmer::new(&mut sim, &d).unwrap();
        prog.write(&base, false).unwrap();
        let regions = Regions { config: true, ..Regions::NONE };

        let mut clear = Image::default();
        let low = d.config[0].mask & base.config[&first] & 0x3FFF;
        let bit = low.isolate_lowest_one();
        clear.config.insert(first, base.config[&first] & !bit);
        prog.write_regions(&clear, regions, false).unwrap();
        assert!(prog.verify_regions(&clear, regions).unwrap().is_empty(), "{name}");

        let mut set = Image::default();
        set.config.insert(first, 0x3FFF);
        let err = prog.write_regions(&set, regions, false).unwrap_err();
        assert!(err.to_string().contains("full erase"), "{name}: {err}");
    }
}

#[test]
fn eight_bit_user_ids_are_row_erased() {
    let d = part("PIC16F18446");
    let base = program(&d, 0x2B);
    let mut ids = Image::default();
    ids.config.insert(0x8000, 0x3F21);
    for pic in both(&d, |_| {}, |link, d| {
        let mut prog = Programmer::new(link, d).unwrap();
        prog.entry = Entry::HighVoltage;
        prog.write(&base, false).unwrap();
        prog.write_regions(&ids, Regions { ids: true, ..Regions::NONE }, false).unwrap();
    }) {
        assert_eq!(pic.config[0], 0x3F21);
    }
}
