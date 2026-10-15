//! Built-in device database, so picflash needs neither MPLAB X nor XC8.
//!
//! `data/devices.json.gz` holds every supported part: programming parameters
//! (from the device pack's `.PIC` file) and the assembler symbols of its
//! `xc8/pic/include/proc/<part>.inc` header, stored compactly as registers
//! and bit fields. Both sources are Microchip device family packs, licensed
//! Apache-2.0 (see `data/LICENSE-microchip-dfp.txt`).
//!
//! The database is rebuilt from installed packs with `picflash db-build`,
//! which checks that the compact form expands back to exactly the header's
//! symbols for every part.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};

use crate::device::{self, AsmHeader, Device, Register};

const EMBEDDED: &[u8] = include_bytes!("../data/devices.json.gz");

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Database {
    /// Device pack versions the data was extracted from, e.g. "PIC12-16F1xxx_DFP 1.9.258".
    pub sources: Vec<String>,
    pub parts: Vec<Device>,
}

impl Database {
    pub fn embedded() -> &'static Database {
        static DB: OnceLock<Database> = OnceLock::new();
        DB.get_or_init(|| Database::decode(EMBEDDED).expect("embedded device database is corrupt"))
    }

    pub fn decode(gz: &[u8]) -> Result<Database> {
        let mut json = String::new();
        GzDecoder::new(gz).read_to_string(&mut json)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(serde_json::to_string(self)?.as_bytes())?;
        Ok(gz.finish()?)
    }

    pub fn find(&self, name: &str) -> Option<&Device> {
        let name = name.trim().to_ascii_uppercase();
        let with_prefix = format!("PIC{name}");
        self.parts.iter().find(|d| d.name == name || d.name == with_prefix)
    }
}

/// `equ` symbols and object-like `#define`s of a device header.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HeaderSymbols {
    pub equ: BTreeMap<String, i64>,
    pub defines: BTreeMap<String, String>,
}

fn header_number(text: &str) -> Result<i64> {
    let t = text.trim();
    let parsed = match t.strip_suffix(['h', 'H']) {
        Some(hex) => i64::from_str_radix(hex, 16),
        None => t.parse(),
    };
    parsed.with_context(|| format!("nombre invalide dans le header : {t:?}"))
}

/// Reads a device header: its raw symbols, and its compact form.
pub fn parse_header(text: &str) -> Result<(AsmHeader, HeaderSymbols)> {
    let mut raw = HeaderSymbols::default();
    let mut header = AsmHeader::default();
    let mut current: Option<usize> = None;
    // Per field: (position, width) as declared by _POSN and _SIZE.
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("// Register: ") {
            header.registers.push(Register { name: name.trim().to_owned(), address: 0, fields: Vec::new() });
            current = Some(header.registers.len() - 1);
        } else if let Some(rest) = line.strip_prefix("#define ") {
            let rest = rest.trim();
            let Some((name, value)) = rest.split_once(char::is_whitespace) else { continue };
            // Function-like macros (BANKMASK, PAGEMASK) are assembler built-ins.
            if name.contains('(') {
                continue;
            }
            let value = value.trim();
            if value != name && !value.is_empty() {
                raw.defines.insert(name.to_owned(), value.to_owned());
            }
        } else if line.to_ascii_lowercase().starts_with("psect ") {
            header.psects.push(line.to_owned());
        } else {
            let mut words = line.split_whitespace();
            let (Some(name), Some(kw), Some(value)) = (words.next(), words.next(), words.next()) else {
                continue;
            };
            if !kw.eq_ignore_ascii_case("equ") {
                continue;
            }
            let value = header_number(value)?;
            raw.equ.insert(name.to_owned(), value);
            let Some(reg) = current.map(|i| &mut header.registers[i]) else { continue };
            let suffix = name.strip_prefix(&reg.name).and_then(|r| r.strip_prefix('_'));
            if name == reg.name {
                reg.address = value as u32;
            } else if let Some(field) = suffix.and_then(|r| r.strip_suffix("_POSN")) {
                reg.fields.push((field.to_owned(), value as u8, 0));
            } else if let Some(field) = suffix.and_then(|r| r.strip_suffix("_SIZE")) {
                if let Some(f) = reg.fields.iter_mut().rev().find(|f| f.0 == field) {
                    f.2 = value as u8;
                }
            }
        }
    }

    // Keep only what the alias rule does not reproduce.
    let derived = alias_defines(&header.registers);
    for (name, value) in &raw.defines {
        if derived.get(name) != Some(value) {
            header.extra_defines.push((name.clone(), value.clone()));
        }
    }
    for name in derived.keys() {
        if !raw.defines.contains_key(name) {
            header.skipped_aliases.push(name.clone());
        }
    }
    Ok((header, raw))
}

/// `#define BIT BANKMASK(REG), pos` for single-bit fields; first register wins.
fn alias_defines(registers: &[Register]) -> BTreeMap<String, String> {
    let mut aliases = BTreeMap::new();
    for reg in registers {
        for (field, pos, width) in &reg.fields {
            if *width == 1 {
                aliases
                    .entry(field.clone())
                    .or_insert_with(|| format!("BANKMASK({}), {pos}", reg.name));
            }
        }
    }
    aliases
}

/// Expands a compact header back to its `equ` and `#define` symbols.
pub fn header_symbols(header: &AsmHeader) -> HeaderSymbols {
    let mut out = HeaderSymbols::default();
    for reg in &header.registers {
        out.equ.insert(reg.name.clone(), reg.address as i64);
        for (field, pos, width) in &reg.fields {
            let base = format!("{}_{field}", reg.name);
            let (pos, width) = (*pos as i64, *width as i64);
            out.equ.insert(format!("{base}_POSN"), pos);
            out.equ.insert(format!("{base}_POSITION"), pos);
            out.equ.insert(format!("{base}_SIZE"), width);
            out.equ.insert(format!("{base}_LENGTH"), width);
            out.equ.insert(format!("{base}_MASK"), ((1i64 << width) - 1) << pos);
        }
    }
    out.defines = alias_defines(&header.registers);
    for name in &header.skipped_aliases {
        out.defines.remove(name);
    }
    for (name, value) in &header.extra_defines {
        out.defines.insert(name.clone(), value.clone());
    }
    out
}

/// Builds the database from installed device packs: every part with a
/// supported programming algorithm and an assembler header.
pub fn build(roots: &[PathBuf]) -> Result<Database> {
    let mut parts: BTreeMap<String, Device> = BTreeMap::new();
    let mut sources = std::collections::BTreeSet::new();
    for (path, parsed) in device::scan(roots) {
        let Ok(mut dev) = parsed else { continue };
        // Debug-header variants (AC244064_AS_PIC16F1789.PIC) reuse the part
        // name with another device ID: only take <part>.PIC itself.
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
        if stem.as_deref() != Some(dev.name.as_str())
            || dev.algorithm().is_err()
            || parts.contains_key(&dev.name)
        {
            continue;
        }
        // <pack>/<version>/edc/<part>.PIC
        let version_dir = path.parent().and_then(|p| p.parent()).context("chemin de pack inattendu")?;
        let inc = version_dir
            .join("xc8/pic/include/proc")
            .join(format!("{}.inc", dev.name.to_ascii_lowercase()));
        let Ok(text) = std::fs::read_to_string(&inc) else { continue };
        let (header, raw) = parse_header(&text).with_context(|| format!("{}", inc.display()))?;
        let expanded = header_symbols(&header);
        if expanded != raw {
            let missing: Vec<_> = raw.equ.keys().filter(|k| !expanded.equ.contains_key(*k)).take(5).collect();
            bail!("{}: la forme compacte ne redonne pas le header ({missing:?}...)", dev.name);
        }
        dev.asm = header;
        let pack = version_dir.parent().and_then(|p| p.file_name()).unwrap_or_default();
        let version = version_dir.file_name().unwrap_or_default();
        sources.insert(format!("{} {}", pack.to_string_lossy(), version.to_string_lossy()));
        parts.insert(dev.name.clone(), dev);
    }
    if parts.is_empty() {
        bail!("aucune puce supportée trouvée dans les packs");
    }
    Ok(Database { sources: sources.into_iter().collect(), parts: parts.into_values().collect() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "\
// Register: STATUS
#define STATUS STATUS
STATUS                                   equ 0003h
// bitfield definitions
STATUS_C_POSN                            equ 0000h
STATUS_C_POSITION                        equ 0000h
STATUS_C_SIZE                            equ 0001h
STATUS_C_LENGTH                          equ 0001h
STATUS_C_MASK                            equ 0001h
STATUS_PS_POSN                           equ 0001h
STATUS_PS_POSITION                       equ 0001h
STATUS_PS_SIZE                           equ 0002h
STATUS_PS_LENGTH                         equ 0002h
STATUS_PS_MASK                           equ 0006h
// Register: PORTA
#define PORTA PORTA
PORTA                                    equ 000Ch
PORTA_C_POSN                             equ 0005h
PORTA_C_POSITION                         equ 0005h
PORTA_C_SIZE                             equ 0001h
PORTA_C_LENGTH                           equ 0001h
PORTA_C_MASK                             equ 0020h
#define C                                BANKMASK(STATUS), 0
#define SPACE_CODE   0
#define PAGEMASK(addr) ((addr) and 07FFh)
psect code,class=CODE,space=SPACE_CODE,delta=2
";

    #[test]
    fn header_roundtrip() {
        let (header, raw) = parse_header(HEADER).unwrap();
        assert_eq!(header.registers.len(), 2);
        assert_eq!(header.registers[0].fields, [("C".into(), 0, 1), ("PS".into(), 1, 2)]);
        assert_eq!(header.extra_defines, [("SPACE_CODE".to_string(), "0".to_string())]);
        assert!(header.skipped_aliases.is_empty(), "C resolves to STATUS, first register");
        assert_eq!(header.psects.len(), 1);
        assert_eq!(header_symbols(&header), raw);
        assert_eq!(raw.equ["STATUS_PS_MASK"], 6);
    }

    #[test]
    fn embedded_database_has_the_lab_part() {
        let db = Database::embedded();
        let dev = db.find("16F1789").expect("PIC16F1789 in the embedded database");
        assert_eq!(dev.device_id, 0x302A);
        let syms = header_symbols(&dev.asm);
        assert_eq!(syms.equ["OSCCON"], 0x99);
        assert_eq!(syms.equ["ANSELA"], 0x18C);
        assert_eq!(syms.defines["RA2"], "BANKMASK(PORTA), 2");
        let fosc = dev.config_fields.iter().find(|f| f.name == "FOSC").unwrap();
        assert_eq!((fosc.address, fosc.shift, fosc.width), (0x8007, 0, 3));
        assert!(fosc.values.contains(&("INTOSC".to_string(), 4)));
        assert_eq!(dev.common_ram, Some((0x70, 0x80)));
        assert_eq!(dev.gpr[0], (0x20, 0x70));
    }
}
