//! Host flows against the bit-level PIC model in `picflash::sim`.

use std::sync::LazyLock;

use picflash::device::{Algorithm, Device};
use picflash::image::Image;
use picflash::programmer::Programmer;
use picflash::sim::{Phase, SimPic};

static SIX: LazyLock<Device> =
    LazyLock::new(|| Device::from_pic_xml(include_str!("fixtures/sample.PIC")).unwrap());
static EIGHT: LazyLock<Device> =
    LazyLock::new(|| Device::from_pic_xml(include_str!("fixtures/sample-8bit.PIC")).unwrap());

const BLINK: &str = include_str!("blink.hex");

fn device(algo: Algorithm) -> &'static Device {
    match algo {
        Algorithm::Midrange6Bit => &SIX,
        Algorithm::Midrange8Bit => &EIGHT,
    }
}

/// A program spanning several rows plus a user ID and every config word, with
/// LVP left on.
fn sample_image(algo: Algorithm) -> Image {
    if algo == Algorithm::Midrange6Bit {
        return Image::parse_hex(BLINK, &SIX).unwrap();
    }
    let mut image = Image::default();
    for addr in (0..40).chain(0x1FF0..0x2010) {
        image.flash.insert(addr, (addr as u16).wrapping_mul(0x2B) & 0x3FFF);
    }
    image.config.insert(0x8000, 0x1234);
    let config = [(0x8007, 0x3FEC), (0x8008, 0x3FFE), (0x8009, 0x3F9F), (0x800A, 0x3FFF), (0x800B, 0x3FFF)];
    image.config.extend(config);
    image
}

const BOTH: [Algorithm; 2] = [Algorithm::Midrange6Bit, Algorithm::Midrange8Bit];

#[test]
fn reads_device_id() {
    for algo in BOTH {
        let mut pic = SimPic::new(device(algo)).unwrap();
        let ids = Programmer::new(&mut pic, device(algo)).unwrap().check_id().unwrap();
        assert_eq!(ids.device_id, device(algo).device_id, "{algo:?}");
        assert_eq!(ids.revision_id, 0x2002, "{algo:?}");
        assert!(matches!(pic.phase, Phase::Running), "target released after ID");
    }
}

#[test]
fn writes_and_verifies() {
    for algo in BOTH {
        let image = sample_image(algo);
        let mut pic = SimPic::new(device(algo)).unwrap();
        // Leftovers from a previous program must be erased first.
        pic.flash[0x10] = 0x0000;
        pic.config[7] = 0x0000;
        pic.config[1] = 0x0000;

        let mut prog = Programmer::new(&mut pic, device(algo)).unwrap();
        prog.ping().unwrap();
        prog.write(&image, false).unwrap();
        assert!(prog.verify(&image).unwrap().is_empty(), "{algo:?}");

        for (&addr, &word) in &image.flash {
            assert_eq!(pic.flash[addr as usize], word, "{algo:?} flash {addr:#06x}");
        }
        for (&addr, &word) in &image.config {
            let found = pic.config[(addr - 0x8000) as usize];
            assert_eq!(found, word, "{algo:?} config {addr:#06x}");
        }
        assert_eq!(pic.config[1], 0x3FFF, "{algo:?}: user IDs erased");
        assert_eq!(pic.flash[0x3000], 0x3FFF);
    }
}

#[test]
fn erase_blanks_everything() {
    for algo in BOTH {
        let mut pic = SimPic::new(device(algo)).unwrap();
        pic.flash[0x123] = 0;
        pic.config[0] = 0;
        pic.config[8] = 0;
        Programmer::new(&mut pic, device(algo)).unwrap().erase().unwrap();
        assert!(pic.flash.iter().all(|&w| w == 0x3FFF), "{algo:?}");
        assert_eq!((pic.config[0], pic.config[8]), (0x3FFF, 0x3FFF), "{algo:?}");
    }
}

#[test]
fn sparse_rows_and_read_back_across_frames() {
    for algo in BOTH {
        let mut image = Image::default();
        for addr in [0x0000, 0x0041, 0x1FFF, 0x2000, 0x3FFF] {
            image.flash.insert(addr, ((addr as u16) & 0x3FFF) ^ 0x1555);
        }
        let mut pic = SimPic::new(device(algo)).unwrap();
        let mut prog = Programmer::new(&mut pic, device(algo)).unwrap();
        prog.write(&image, false).unwrap();
        assert!(prog.verify(&image).unwrap().is_empty(), "{algo:?}");
        let dump = prog.read().unwrap();
        assert_eq!(dump.flash, image.flash, "{algo:?}");
        assert!(pic.frames > 3, "large scripts must be split into frames");
    }
}

#[test]
fn verify_reports_mismatch() {
    for algo in BOTH {
        let image = sample_image(algo);
        let mut pic = SimPic::new(device(algo)).unwrap();
        Programmer::new(&mut pic, device(algo)).unwrap().write(&image, false).unwrap();
        pic.flash[4] = 0x0000;
        let bad = Programmer::new(&mut pic, device(algo)).unwrap().verify(&image).unwrap();
        assert_eq!(bad.len(), 1, "{algo:?}");
        assert_eq!(bad[0].address, 4);
    }
}

#[test]
fn refuses_to_clear_lvp() {
    for algo in BOTH {
        let mut image = sample_image(algo);
        let (addr, bit) = device(algo).lvp_fuse.unwrap();
        *image.config.entry(addr).or_insert(0x3FFF) &= !bit;
        let mut pic = SimPic::new(device(algo)).unwrap();
        pic.flash[0] = 0x1234;
        let err = Programmer::new(&mut pic, device(algo))
            .unwrap()
            .write(&image, false)
            .unwrap_err();
        assert!(err.to_string().contains("LVP"), "{algo:?}");
        assert_eq!(pic.flash[0], 0x1234, "chip must be untouched");
    }
}

#[test]
fn model_catches_short_waits() {
    // Checks the model itself: a device file with a too-short row write time
    // must make programming fail.
    for algo in BOTH {
        let mut fast = device(algo).clone();
        fast.t_program_us = 100;
        let mut pic = SimPic::new(device(algo)).unwrap();
        let err = Programmer::new(&mut pic, &fast)
            .unwrap()
            .write(&sample_image(algo), false)
            .unwrap_err();
        assert!(err.to_string().contains("busy"), "{algo:?}: {err}");
    }
}
