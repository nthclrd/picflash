//! Erase / program / verify / read flows, shared by every supported family.
//! Order follows the programming specs: program memory by rows, then user
//! IDs, then config words last.

use anyhow::{bail, Result};
use picflash_protocol::PROTOCOL_VERSION;

use crate::algo::{decode_word, Entry, Session};
use crate::device::{Algorithm, Device, DEVICE_ID_ADDRESS, REVISION_ID_ADDRESS};
use crate::image::Image;
use crate::transport::{Batch, Transport};

pub struct Programmer<'a, T: Transport + ?Sized> {
    link: &'a mut T,
    pub device: &'a Device,
    algo: Algorithm,
    pub entry: Entry,
    /// Leave the data EEPROM untouched by erases.
    pub keep_eeprom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ids {
    pub device_id: u16,
    pub revision_id: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub address: u32,
    pub expected: u16,
    pub found: u16,
}

impl<'a, T: Transport + ?Sized> Programmer<'a, T> {
    pub fn new(link: &'a mut T, device: &'a Device) -> Result<Self> {
        let algo = device.algorithm()?;
        Ok(Self { link, device, algo, entry: Entry::LowVoltage, keep_eeprom: false })
    }

    fn session(&mut self) -> Session<'_, T> {
        Session::new(self.link, self.algo, self.entry)
    }

    pub fn ping(&mut self) -> Result<u8> {
        let mut b = Batch::new(self.link);
        b.ping()?;
        let version = b.finish()?[0];
        if version != PROTOCOL_VERSION as u32 {
            bail!("firmware speaks protocol v{version}, host expects v{PROTOCOL_VERSION}");
        }
        Ok(version as u8)
    }

    /// Releases MCLR so the freshly programmed code runs.
    pub fn release(&mut self) -> Result<()> {
        let mut s = self.session();
        s.exit()?;
        s.finish().map(drop)
    }

    pub fn read_ids(&mut self) -> Result<Ids> {
        let mut s = self.session();
        s.enter()?;
        s.goto(REVISION_ID_ADDRESS)?;
        s.read(false)?;
        s.goto(DEVICE_ID_ADDRESS)?;
        s.read(false)?;
        s.exit()?;
        let raw = s.finish()?;
        Ok(Ids {
            revision_id: decode_word(raw[0]),
            device_id: decode_word(raw[1]),
        })
    }

    pub fn check_id(&mut self) -> Result<Ids> {
        let ids = self.read_ids()?;
        let d = self.device;
        if d.device_id_mask != 0 && ids.device_id & d.device_id_mask != d.device_id {
            bail!(
                "device ID {:#06x} is not a {} ({:#06x}); check wiring, power and LVP",
                ids.device_id,
                d.name,
                d.device_id
            );
        }
        Ok(ids)
    }

    /// Erases program memory, user IDs and config words, and the data
    /// EEPROM unless [`Self::keep_eeprom`] is set.
    pub fn erase(&mut self) -> Result<()> {
        let (device, keep) = (self.device, self.keep_eeprom);
        let mut s = self.session();
        s.enter()?;
        s.bulk_erase(device, keep)?;
        s.exit()?;
        s.finish().map(drop)
    }

    /// In low-voltage mode, refuses a config that clears the LVP fuse unless
    /// `allow_lvp_off`: low-voltage entry could never reach the chip again.
    fn check_lvp(&self, image: &Image, allow_lvp_off: bool) -> Result<()> {
        if let (Entry::LowVoltage, Some((lvp_addr, lvp_bit))) = (self.entry, self.device.lvp_fuse) {
            let word = image.config.get(&lvp_addr).copied().unwrap_or(ERASED);
            if word & lvp_bit == 0 && !allow_lvp_off {
                bail!(
                    "image clears LVP (config {lvp_addr:#06x} = {word:#06x}); the chip would \
                     then need high-voltage programming. Set LVP = ON, program in high-voltage \
                     mode, or pass --allow-lvp-off"
                );
            }
        }
        Ok(())
    }

    fn check_eeprom(&self, image: &Image) -> Result<()> {
        let size = self.device.eeprom.as_ref().map_or(0, |r| r.len() as u32);
        if let Some((&offset, _)) = image.eeprom.iter().find(|(&o, _)| o >= size) {
            bail!("EEPROM byte {offset:#x} is outside the part's {size}-byte data EEPROM");
        }
        Ok(())
    }

    /// Erases, then writes program memory, user IDs, data EEPROM and config
    /// words (config last: it carries LVP and code protection).
    pub fn write(&mut self, image: &Image, allow_lvp_off: bool) -> Result<()> {
        self.check_lvp(image, allow_lvp_off)?;
        self.check_eeprom(image)?;
        let (d, keep) = (self.device, self.keep_eeprom);
        let mut s = self.session();
        s.enter()?;
        s.bulk_erase(d, keep)?;
        write_rows(&mut s, d, image, false)?;
        write_words(&mut s, d, image, |a| d.user_ids.contains(&a))?;
        write_eeprom(&mut s, d, image, !keep)?;
        write_words(&mut s, d, image, |a| !d.user_ids.contains(&a))?;
        s.exit()?;
        s.finish().map(drop)
    }

    /// Writes only the selected regions, without a bulk erase (like IPE's
    /// `-MP`, `-ME`, `-MI`, `-MC`): program rows are row-erased first; EEPROM
    /// bytes erase themselves; user IDs and config words can only clear bits
    /// unless the part can row-erase its user IDs.
    pub fn write_regions(&mut self, image: &Image, regions: Regions, allow_lvp_off: bool) -> Result<()> {
        let d = self.device;
        if regions.config {
            self.check_lvp(image, allow_lvp_off)?;
        }
        if regions.eeprom {
            self.check_eeprom(image)?;
        }
        let ids_erasable = self.algo == Algorithm::Midrange8Bit && d.t_row_erase_us != 0;
        // Words that cannot be erased here must only clear bits.
        let mut checked: Vec<u32> = Vec::new();
        if regions.ids && !ids_erasable {
            checked.extend(d.user_ids.clone().filter(|a| image.config.contains_key(a)));
        }
        if regions.config {
            checked.extend(d.config.iter().map(|c| c.address).filter(|a| image.config.contains_key(a)));
        }
        if !checked.is_empty() {
            let current = self.read_words(&checked)?;
            for (addr, now) in checked.iter().zip(current) {
                let want = image.config[addr];
                if want & !now & 0x3FFF != 0 {
                    bail!(
                        "{addr:#06x}: {now:#06x} -> {want:#06x} needs bits set back to 1, which only a \
                         full erase can do; program everything instead"
                    );
                }
            }
        }
        if regions.program && !image.flash.is_empty() && d.t_row_erase_us == 0 {
            bail!("{} has no row erase: program everything instead", d.name);
        }

        let mut s = self.session();
        s.enter()?;
        if regions.program {
            write_rows(&mut s, d, image, true)?;
        }
        if regions.ids {
            if ids_erasable && d.user_ids.clone().any(|a| image.config.contains_key(&a)) {
                s.goto(d.user_ids.start)?;
                s.row_erase(d.t_row_erase_us as u32)?;
            }
            write_words(&mut s, d, image, |a| d.user_ids.contains(&a))?;
        }
        if regions.eeprom {
            write_eeprom(&mut s, d, image, false)?;
        }
        if regions.config {
            write_words(&mut s, d, image, |a| !d.user_ids.contains(&a))?;
        }
        s.exit()?;
        s.finish().map(drop)
    }

    /// Reads back every word present in `image` and lists differences.
    pub fn verify(&mut self, image: &Image) -> Result<Vec<Mismatch>> {
        self.verify_regions(image, Regions::ALL)
    }

    pub fn verify_regions(&mut self, image: &Image, regions: Regions) -> Result<Vec<Mismatch>> {
        let d = self.device;
        let order: Vec<u32> = image
            .flash
            .keys()
            .filter(|_| regions.program)
            .chain(image.config.keys().filter(|a| {
                if d.user_ids.contains(a) { regions.ids } else { regions.config }
            }))
            .copied()
            .collect();
        let words = self.read_words(&order)?;

        let mut bad = Vec::new();
        for (&addr, found) in order.iter().zip(words) {
            let (expected, mask) = match image.flash.get(&addr) {
                Some(&w) => (w, 0x3FFF),
                None => {
                    let mask = d.config.iter().find(|c| c.address == addr).map_or(0x3FFF, |c| c.mask);
                    (image.config[&addr], mask)
                }
            };
            if expected & mask != found & mask {
                bad.push(Mismatch { address: addr, expected, found });
            }
        }
        if regions.eeprom && !image.eeprom.is_empty() {
            let offsets: Vec<u32> = image.eeprom.keys().copied().collect();
            let bytes = self.read_eeprom(&offsets)?;
            let base = d.eeprom.as_ref().map_or(0, |r| r.start);
            for (offset, found) in offsets.iter().zip(bytes) {
                let expected = image.eeprom[offset];
                if expected != found {
                    bad.push(Mismatch {
                        address: base + offset,
                        expected: expected as u16,
                        found: found as u16,
                    });
                }
            }
        }
        Ok(bad)
    }

    /// Dumps program memory, user IDs, config words and data EEPROM.
    pub fn read(&mut self) -> Result<Image> {
        let d = self.device;
        let config_addrs: Vec<u32> = d.user_ids.clone().chain(d.config.iter().map(|c| c.address)).collect();
        let order: Vec<u32> = (0..d.flash_words).chain(config_addrs.iter().copied()).collect();
        let words = self.read_words(&order)?;
        let eeprom_len = d.eeprom.as_ref().map_or(0, |r| r.len() as u32);
        let eeprom = self.read_eeprom(&(0..eeprom_len).collect::<Vec<_>>())?;

        let (flash, config) = words.split_at(d.flash_words as usize);
        Ok(Image {
            // Skip erased words and bytes to keep dumps small, like IPE's export.
            flash: (0..).zip(flash.iter().copied()).filter(|&(_, w)| w != ERASED).collect(),
            config: config_addrs.into_iter().zip(config.iter().copied()).collect(),
            eeprom: (0..).zip(eeprom).filter(|&(_, b)| b != 0xFF).collect(),
        })
    }

    /// Reads words at ascending addresses, using read-and-increment runs.
    fn read_words(&mut self, addrs: &[u32]) -> Result<Vec<u16>> {
        let mut s = self.session();
        s.enter()?;
        for (i, &addr) in addrs.iter().enumerate() {
            s.goto(addr)?;
            let next_is_adjacent = addrs.get(i + 1) == Some(&(addr + 1));
            s.read(next_is_adjacent)?;
        }
        s.exit()?;
        Ok(s.finish()?.into_iter().map(decode_word).collect())
    }

    /// Reads data EEPROM bytes at ascending offsets.
    fn read_eeprom(&mut self, offsets: &[u32]) -> Result<Vec<u8>> {
        if offsets.is_empty() {
            return Ok(Vec::new());
        }
        let mut s = self.session();
        s.enter()?;
        for (i, &offset) in offsets.iter().enumerate() {
            s.goto_eeprom(offset)?;
            let next_is_adjacent = offsets.get(i + 1) == Some(&(offset + 1));
            s.read_eeprom(next_is_adjacent)?;
        }
        s.exit()?;
        Ok(s.finish()?.into_iter().map(|raw| decode_word(raw) as u8).collect())
    }
}

/// Program memory regions for partial programming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Regions {
    pub program: bool,
    pub eeprom: bool,
    pub ids: bool,
    pub config: bool,
}

impl Regions {
    pub const ALL: Regions = Regions { program: true, eeprom: true, ids: true, config: true };
    pub const NONE: Regions = Regions { program: false, eeprom: false, ids: false, config: false };
}

const ERASED: u16 = 0x3FFF;

/// Writes every program row the image touches, row-erasing it first when
/// the chip was not bulk-erased.
fn write_rows<T: Transport + ?Sized>(s: &mut Session<'_, T>, d: &Device, image: &Image, erase: bool) -> Result<()> {
    for row in image.rows(d) {
        s.goto(row)?;
        if erase {
            s.row_erase(d.t_row_erase_us as u32)?;
        }
        for i in 0..d.row_words {
            let word = image.flash.get(&(row + i)).copied().unwrap_or(ERASED);
            s.load(word, i + 1 < d.row_words)?;
        }
        s.program(d.t_program_us as u32)?;
    }
    Ok(())
}

/// Writes the image's user ID / config words matching `pick`, one at a time.
fn write_words<T: Transport + ?Sized>(
    s: &mut Session<'_, T>,
    d: &Device,
    image: &Image,
    pick: impl Fn(u32) -> bool,
) -> Result<()> {
    for (&addr, &word) in image.config.iter().filter(|(&a, _)| pick(a)) {
        s.goto(addr)?;
        s.load(word, false)?;
        s.program(d.t_config_us as u32)?;
    }
    Ok(())
}

/// Writes the image's EEPROM bytes; after an erase, 0xFF bytes are skipped.
fn write_eeprom<T: Transport + ?Sized>(s: &mut Session<'_, T>, d: &Device, image: &Image, erased: bool) -> Result<()> {
    for (&offset, &byte) in &image.eeprom {
        if erased && byte == 0xFF {
            continue;
        }
        s.goto_eeprom(offset)?;
        s.load_eeprom(byte)?;
        s.program(d.t_eeprom_us as u32)?;
    }
    Ok(())
}

// read_ids reads the device ID right after the revision ID.
const _: () = assert!(DEVICE_ID_ADDRESS == REVISION_ID_ADDRESS + 1);
