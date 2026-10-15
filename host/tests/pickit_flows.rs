//! The same flows as `simulated_pic.rs`, but through the PICkit path: the
//! bit-level scripts are translated to Gen4 and executed on the PIC model.

use std::sync::LazyLock;

use picflash::algo::Entry;
use picflash::device::{Algorithm, Device};
use picflash::image::Image;
use picflash::pickit::Gen4Link;
use picflash::programmer::Programmer;
use picflash::sim::{Gen4Sim, Phase, SimPic};

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

fn link(algo: Algorithm) -> Gen4Link<Gen4Sim> {
    Gen4Link::new(Gen4Sim::new(SimPic::new(device(algo)).unwrap()), algo)
}

fn image(algo: Algorithm) -> Image {
    if algo == Algorithm::Midrange6Bit {
        return Image::parse_hex(BLINK, &SIX).unwrap();
    }
    let mut image = Image::default();
    for addr in (0..40).chain(0x1FF0..0x2010) {
        image.flash.insert(addr, (addr as u16).wrapping_mul(0x2B) & 0x3FFF);
    }
    image.config.insert(0x8000, 0x1234);
    image.config.extend([(0x8007, 0x3FEC), (0x8008, 0x3FFE), (0x8009, 0x3F9F)]);
    image
}

const ALL: [(Algorithm, Entry); 4] = [
    (Algorithm::Midrange6Bit, Entry::LowVoltage),
    (Algorithm::Midrange6Bit, Entry::HighVoltage),
    (Algorithm::Midrange8Bit, Entry::LowVoltage),
    (Algorithm::Midrange8Bit, Entry::HighVoltage),
];

#[test]
fn id_write_verify_read_through_gen4() {
    for (algo, entry) in ALL {
        let d = device(algo);
        let image = image(algo);
        let mut link = link(algo);
        link.tool.pic.flash[0x10] = 0;

        let mut prog = Programmer::new(&mut link, d).unwrap();
        prog.entry = entry;
        assert_eq!(prog.ping().unwrap(), picflash_protocol::PROTOCOL_VERSION);
        let ids = prog.check_id().unwrap();
        assert_eq!(ids.device_id, d.device_id, "{algo:?} {entry:?}");
        prog.write(&image, false).unwrap();
        assert!(prog.verify(&image).unwrap().is_empty(), "{algo:?} {entry:?}");
        let dump = prog.read().unwrap();
        for (addr, word) in &image.flash {
            assert_eq!(dump.flash.get(addr).copied().unwrap_or(0x3FFF), *word);
        }

        let pic = &link.tool.pic;
        for (&addr, &word) in &image.flash {
            assert_eq!(pic.flash[addr as usize], word, "{algo:?} {entry:?} {addr:#06x}");
        }
        assert!(matches!(pic.phase, Phase::Running), "target released");
        assert!(link.tool.sizes.iter().all(|&n| n <= 1024), "script too long");
    }
}

#[test]
fn high_voltage_programs_with_lvp_off() {
    let algo = Algorithm::Midrange6Bit;
    let d = device(algo);
    let mut image = image(algo);
    let (addr, bit) = d.lvp_fuse.unwrap();
    *image.config.get_mut(&addr).unwrap() &= !bit;

    let mut link = link(algo);
    let mut prog = Programmer::new(&mut link, d).unwrap();
    assert!(prog.write(&image, false).is_err(), "LVP entry refuses to clear LVP");
    prog.entry = Entry::HighVoltage;
    prog.write(&image, false).unwrap();
    assert!(prog.verify(&image).unwrap().is_empty());
}
