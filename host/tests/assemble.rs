//! Builds the lab's blink.asm with the installed pic-as and checks the result
//! matches the HEX the lab Makefile produced. Skipped when XC8 or the device
//! pack is not installed.

use picflash::device;
use picflash::image::Image;
use picflash::toolchain::{assemble, find_pic_as};

#[test]
fn blink_asm_builds_like_the_makefile() {
    let (Some(pic_as), Ok(dev)) = (find_pic_as(), device::load_from_packs(&device::default_pack_roots(), "16F1789")) else {
        eprintln!("pic-as ou DFP absent : test ignoré");
        return;
    };
    let out = std::env::temp_dir().join(format!("picflash-test-{}", std::process::id()));
    let source = out.join("blink.asm");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blink.asm"), &source).unwrap();

    let build = assemble(&pic_as, &source, &dev, &[], &out.join("build")).unwrap();
    let hex = build.hex.unwrap_or_else(|| panic!("build failed:\n{}", build.log));
    let built = Image::parse_hex(&std::fs::read_to_string(hex).unwrap(), &dev).unwrap();
    let expected = Image::parse_hex(include_str!("blink.hex"), &dev).unwrap();
    assert_eq!(built, expected);
    std::fs::remove_dir_all(&out).ok();
}

#[test]
fn syntax_errors_point_at_lines() {
    let (Some(pic_as), Ok(dev)) = (find_pic_as(), device::load_from_packs(&device::default_pack_roots(), "16F1789")) else {
        return;
    };
    let out = std::env::temp_dir().join(format!("picflash-bad-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let source = out.join("bad.asm");
    std::fs::write(&source, "PROCESSOR 16F1789\n#include <xc.inc>\nPSECT code,class=CODE,delta=2\nstart:\n  bogus 4\n").unwrap();
    let build = assemble(&pic_as, &source, &dev, &[], &out.join("build")).unwrap();
    assert!(build.hex.is_none());
    let errors: Vec<_> = build.diagnostics.iter().filter(|d| d.error).collect();
    assert_eq!(errors[0].line, Some(5), "{}", build.log);
    std::fs::remove_dir_all(&out).ok();
}
