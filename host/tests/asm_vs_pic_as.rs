//! Our assembler against pic-as: every `tests/asm/<name>.asm` must produce
//! the same memory image as `<name>.hex`, which pic-as (XC8 v3.10) built
//! with the lab Makefile's options. No Microchip tool is needed to run this.

use std::path::Path;

use picflash::asm::{assemble_file, Options};
use picflash::image::Image;

fn check(name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/asm");
    let out = assemble_file(&dir.join(format!("{name}.asm")), &Options::default()).unwrap();
    let image = out.image.as_ref().unwrap_or_else(|| panic!("{name} :\n{}", out.log()));
    let device = out.device.as_ref().unwrap();
    let expected = Image::parse_hex(&std::fs::read_to_string(dir.join(format!("{name}.hex"))).unwrap(), device).unwrap();
    for (addr, word) in &expected.flash {
        assert_eq!(image.flash.get(addr), Some(word), "{name} : mot {addr:#06x}, carte {:?}", out.map);
    }
    assert_eq!(image.flash.len(), expected.flash.len(), "{name} : mots en trop, carte {:?}", out.map);
    assert_eq!(image.config, expected.config, "{name} : config");
    assert_eq!(image.eeprom, expected.eeprom, "{name} : eeprom");
}

#[test]
fn blink() {
    check("blink");
}

#[test]
fn blink_18446() {
    check("blink_18446");
}

#[test]
fn data_and_expr() {
    check("data_and_expr");
}

#[test]
fn instructions() {
    check("instructions");
}

/// Random programs from `fuzz_gen.py`, kept with pic-as's output. 400 such
/// programs matched when this was written; these are a sample.
#[test]
fn random_programs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/asm/fuzz");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".asm").map(str::to_owned))
        .collect();
    names.sort();
    assert!(names.len() >= 30);
    for name in names {
        check(&format!("fuzz/{name}"));
    }
}
