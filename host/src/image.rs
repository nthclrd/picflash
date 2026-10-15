//! Intel HEX <-> word-addressed memory image.
//!
//! PIC HEX files use byte addresses with little-endian 14-bit words, so word
//! address = byte address / 2 (config word 0x8007 sits at byte 0x1000E).

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use ihex::{Reader, Record};

use crate::device::{Device, CONFIG_SPACE};

/// Data EEPROM sits at word address 0xF000 in PIC12/16F1xxx HEX files.
const EEPROM_SPACE: u32 = 0xF000;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Image {
    pub flash: BTreeMap<u32, u16>,
    /// User IDs and config words, keyed by word address.
    pub config: BTreeMap<u32, u16>,
    pub eeprom: BTreeMap<u32, u8>,
}

impl Image {
    pub fn parse_hex(text: &str, device: &Device) -> Result<Self> {
        let mut bytes: BTreeMap<u32, u8> = BTreeMap::new();
        let mut base = 0u32;
        for record in Reader::new(text) {
            match record.context("invalid HEX record")? {
                Record::Data { offset, value } => {
                    for (i, b) in value.iter().enumerate() {
                        bytes.insert(base + offset as u32 + i as u32, *b);
                    }
                }
                Record::ExtendedLinearAddress(hi) => base = (hi as u32) << 16,
                Record::ExtendedSegmentAddress(seg) => base = (seg as u32) << 4,
                Record::EndOfFile => break,
                Record::StartLinearAddress(_) | Record::StartSegmentAddress { .. } => {}
            }
        }

        let mut image = Image::default();
        for (&byte_addr, &lo) in &bytes {
            if byte_addr % 2 != 0 {
                continue;
            }
            let hi = bytes.get(&(byte_addr + 1)).copied().unwrap_or(0xFF);
            let addr = byte_addr / 2;
            let word = u16::from_le_bytes([lo, hi]) & 0x3FFF;
            if addr < device.flash_words {
                image.flash.insert(addr, word);
            } else if device.user_ids.contains(&addr)
                || device.config.iter().any(|c| c.address == addr)
            {
                image.config.insert(addr, word);
            } else if device.eeprom.as_ref().is_some_and(|r| r.contains(&addr)) {
                image.eeprom.insert(addr - EEPROM_SPACE, lo);
            } else if addr >= CONFIG_SPACE {
                // Device/revision IDs are read-only; ignore like IPE does.
            } else {
                bail!("word address {addr:#06x} is outside {}", device.name);
            }
        }
        Ok(image)
    }

    /// Start addresses of program memory rows holding at least one word.
    pub fn rows(&self, device: &Device) -> Vec<u32> {
        let mut rows: Vec<u32> = self
            .flash
            .keys()
            .map(|a| a - a % device.row_words)
            .collect();
        rows.dedup();
        rows
    }

    pub fn to_hex(&self) -> Result<String> {
        let mut records = Vec::new();
        let mut regions: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut push = |addr: u32, bytes: [u8; 2]| match regions.last_mut() {
            Some((start, data)) if *start + data.len() as u32 == addr => {
                data.extend_from_slice(&bytes)
            }
            _ => regions.push((addr, bytes.to_vec())),
        };
        for (&a, &w) in self.flash.iter().chain(self.config.iter()) {
            push(a * 2, w.to_le_bytes());
        }
        for (&a, &b) in &self.eeprom {
            push((EEPROM_SPACE + a) * 2, [b, 0]);
        }

        let mut upper = 0u16;
        for (start, data) in regions {
            for (i, chunk) in data.chunks(16).enumerate() {
                let addr = start + (i * 16) as u32;
                if (addr >> 16) as u16 != upper {
                    upper = (addr >> 16) as u16;
                    records.push(Record::ExtendedLinearAddress(upper));
                }
                records.push(Record::Data {
                    offset: addr as u16,
                    value: chunk.to_vec(),
                });
            }
        }
        records.push(Record::EndOfFile);
        Ok(ihex::create_object_file_representation(&records)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::tests::pic16f1789;

    const BLINK: &str = ":020000001028C6
:100008000900F830F200F101F001F00F0928F10FB2
:100018000928F20F092808002100F8309900230068
:100028008C0121008C0188010C1405200C1005207E
:02003800182886
:020000040001F9
:04000E00E43FFF3F8D
:00000001FF
";

    #[test]
    fn parses_pic_as_output() {
        let img = Image::parse_hex(BLINK, &pic16f1789()).unwrap();
        assert_eq!(img.flash[&0], 0x2810);
        assert_eq!(img.flash[&4], 0x0009);
        assert_eq!(img.flash[&0x1C], 0x2818);
        assert_eq!(img.config[&0x8007], 0x3FE4);
        assert_eq!(img.config[&0x8008], 0x3FFF);
        assert_eq!(img.rows(&pic16f1789()), vec![0]);
    }

    #[test]
    fn hex_roundtrip() {
        let img = Image::parse_hex(BLINK, &pic16f1789()).unwrap();
        let again = Image::parse_hex(&img.to_hex().unwrap(), &pic16f1789()).unwrap();
        assert_eq!(img, again);
    }
}
