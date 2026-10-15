//! Device checksum as MPLAB IPE shows it, per the programming specs
//! (e.g. DS40001970 §2.4.3 "Checksum Computation").
//!
//! - Code protection off: sum of every program memory word (erased words
//!   count as 3FFFh) plus each Configuration Word ANDed with its implemented
//!   bits, truncated to 16 bits.
//! - Code protection on: the low nibbles of the four User IDs form a 16-bit
//!   value (8000h is the most significant), plus the masked Configuration
//!   Words.

use crate::device::Device;
use crate::image::Image;

const ERASED: u16 = 0x3FFF;

/// True when the image's config enables program memory code protection
/// (`CP` field cleared).
pub fn code_protected(image: &Image, device: &Device) -> bool {
    device.config_fields.iter().find(|f| f.name == "CP").is_some_and(|f| {
        let word = image.config.get(&f.address).copied().unwrap_or(ERASED);
        let mask = (1u16 << f.width) - 1;
        (word >> f.shift) & mask == 0
    })
}

pub fn checksum(image: &Image, device: &Device) -> u16 {
    let config = device.config.iter().fold(0u16, |sum, c| {
        let word = image.config.get(&c.address).copied().unwrap_or(ERASED);
        sum.wrapping_add(word & c.mask)
    });
    let body = if code_protected(image, device) {
        device.user_ids.clone().take(4).fold(0u16, |acc, addr| {
            let id = image.config.get(&addr).copied().unwrap_or(ERASED);
            acc << 4 | (id & 0xF)
        })
    } else {
        (0..device.flash_words).fold(0u16, |sum, addr| {
            sum.wrapping_add(image.flash.get(&addr).copied().unwrap_or(ERASED))
        })
    };
    body.wrapping_add(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devdb::Database;

    fn part(name: &str) -> Device {
        Database::embedded().find(name).cloned().unwrap()
    }

    /// DS40001970 appendix B, table "Device IDs and Checksums".
    #[test]
    fn matches_the_spec_table() {
        for (name, blank, aa, cp_blank, cp_aa) in [
            ("PIC16F18446", 0x977D, 0x18D3, 0x6EF9, 0xF04F),
            ("PIC16F18425", 0xB77D, 0x38D3, 0x8EF9, 0x104F),
            ("PIC16F18424", 0xC77D, 0x48D3, 0x9EF9, 0x204F),
        ] {
            let d = part(name);
            let mut image = Image::default();
            assert_eq!(checksum(&image, &d), blank, "{name} blank");

            image.flash.insert(0, 0x00AA);
            image.flash.insert(d.flash_words - 1, 0x00AA);
            let unprotected = checksum(&image, &d);
            assert_eq!(unprotected, aa, "{name} 00AAh first and last");

            // Code protection on, unprotected checksum stored in the User IDs.
            for (blank_or_aa, expected) in [(blank, cp_blank), (unprotected, cp_aa)] {
                let mut cp = Image::default();
                cp.config.insert(0x800B, 0x3FFE);
                for (i, addr) in (0x8000..0x8004).enumerate() {
                    cp.config.insert(addr, (blank_or_aa >> (12 - 4 * i)) & 0xF);
                }
                assert!(code_protected(&cp, &d));
                assert_eq!(checksum(&cp, &d), expected, "{name} code-protected");
            }
        }
    }
}
