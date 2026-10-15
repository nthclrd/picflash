//! Per-part programming parameters, read at runtime from the Microchip device
//! family packs (DFP) installed with MPLAB X. Nothing from the packs is copied
//! into this repository.
//!
//! A part's `.PIC` file (`<pack>/edc/<NAME>.PIC`) gives memory layout, IDs,
//! config words and wait times. Its `edc:psid` attribute names the programming
//! specification (e.g. 40001457 = PIC16(L)F178X), which selects the algorithm.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};
use quick_xml::reader::Reader;
use quick_xml::XmlVersion;

/// Start of configuration space in word addresses (PIC12/16F1xxx).
pub const CONFIG_SPACE: u32 = 0x8000;
pub const DEVICE_ID_ADDRESS: u32 = 0x8006;
pub const REVISION_ID_ADDRESS: u32 = 0x8005;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigWord {
    pub address: u32,
    /// Implemented bits; others read back as 1.
    pub mask: u16,
    /// Value of the word when no `CONFIG` setting touches it.
    pub default: u16,
}

/// One named field of a configuration word, e.g. `FOSC` in CONFIG1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigField {
    pub address: u32,
    pub name: String,
    pub shift: u8,
    pub width: u8,
    /// Named settings and their (unshifted) values, e.g. `INTOSC` = 4.
    pub values: Vec<(String, u16)>,
}

/// A special function register and its bit fields, as the device's
/// assembler header declares them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Register {
    pub name: String,
    pub address: u32,
    /// Field name, bit position, width.
    pub fields: Vec<(String, u8, u8)>,
}

/// Assembler view of the part, distilled from its `xc8/pic/include/proc`
/// header. Expands to the header's `equ` and `#define` symbols, see
/// `crate::asm::symbols`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsmHeader {
    pub registers: Vec<Register>,
    /// `#define`s that the bit-alias rule does not produce.
    pub extra_defines: Vec<(String, String)>,
    /// Single-bit fields that get no `#define` alias (name clashes).
    pub skipped_aliases: Vec<String>,
    /// Standard psect declarations (`udata_shr`, `code`...), in order.
    pub psects: Vec<String>,
}

/// Programming algorithm families, one per command set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Algorithm {
    /// PIC12/16F1xxx: 6-bit commands, LSb first; "MCHP" key LSb first plus
    /// one extra clock; no direct PC load.
    Midrange6Bit,
    /// PIC16F18xxx and later: 8-bit commands and 24-bit payloads, MSb first;
    /// "MCHP" key MSb first; Load PC Address.
    Midrange8Bit,
}

/// Programming specs (DS numbers) whose parts all use exactly these command
/// sequences. Checked by comparing, for every part of each spec, the PICkit 4
/// script structure (entry, ID, erase, row/config write, read) against
/// PIC16F1789 (6-bit) and PIC16F18446 (8-bit), ignoring row sizes and wait
/// times, which come from the DFP. Specs left out differ somewhere: bulk
/// erase silicon workarounds (40001390, 40001397, 40001654), a different
/// command set (40001738), or region-select Bulk Erase payloads (40002266,
/// 40002276, 40002317, 40002500, 40002540, 40002646).
const SPECS: &[(&str, Algorithm)] = &[
    ("40001439", Algorithm::Midrange6Bit),
    ("40001442", Algorithm::Midrange6Bit),
    ("40001457", Algorithm::Midrange6Bit),
    ("40001573", Algorithm::Midrange6Bit),
    ("40001595", Algorithm::Midrange6Bit),
    ("40001620", Algorithm::Midrange6Bit),
    ("40001642", Algorithm::Midrange6Bit),
    ("40001683", Algorithm::Midrange6Bit),
    ("40001714", Algorithm::Midrange6Bit),
    ("40001720", Algorithm::Midrange6Bit),
    ("40001743", Algorithm::Midrange6Bit),
    ("40001754", Algorithm::Midrange6Bit),
    ("40001766", Algorithm::Midrange6Bit),
    ("40001792", Algorithm::Midrange6Bit),
    ("40001796", Algorithm::Midrange6Bit),
    ("40001753", Algorithm::Midrange8Bit),
    ("40001838", Algorithm::Midrange8Bit),
    ("40001846", Algorithm::Midrange8Bit),
    ("40001880", Algorithm::Midrange8Bit),
    ("40001970", Algorithm::Midrange8Bit),
    ("40002149", Algorithm::Midrange8Bit),
];

/// Supply limits from the EDC `Power` and `Programming` elements, in mV.
/// 0 means the pack does not give the value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Power {
    /// MCLR voltage for high-voltage program mode entry.
    pub vpp_mv: u16,
    pub vdd_min_mv: u16,
    pub vdd_max_mv: u16,
    /// Lowest VDD at which low-voltage programming works.
    pub lvp_min_mv: u16,
}

fn millivolts(value: Option<&String>) -> Result<u16> {
    let Some(v) = value else { return Ok(0) };
    let volts: f64 = v.trim().parse().with_context(|| format!("bad voltage {v:?}"))?;
    Ok((volts * 1000.0).round() as u16)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub arch: String,
    /// Programming specification document number (`edc:psid`).
    pub prog_spec: String,
    pub device_id: u16,
    /// 0 when the part has no device ID.
    pub device_id_mask: u16,
    /// Program memory size in words.
    pub flash_words: u32,
    pub row_words: u32,
    pub user_ids: Range<u32>,
    pub config: Vec<ConfigWord>,
    /// Address and bit of the LVP config fuse. Clearing it locks out a
    /// low-voltage-only programmer.
    pub lvp_fuse: Option<(u32, u16)>,
    #[serde(default)]
    pub power: Power,
    pub eeprom: Option<Range<u32>>,
    /// Special function register names by data address (banked, absolute).
    pub sfrs: BTreeMap<u32, String>,
    /// End of data memory: 0x1000 for 32 banks, 0x2000 for 64.
    pub data_end: u32,
    /// Device pack version directory the part was loaded from.
    #[serde(skip)]
    pub pack_dir: Option<PathBuf>,
    pub config_fields: Vec<ConfigField>,
    /// Banked general purpose RAM ranges (data addresses, end exclusive).
    pub gpr: Vec<(u32, u32)>,
    /// RAM visible from every bank (0x70..0x80 on enhanced mid-range).
    pub common_ram: Option<(u32, u32)>,
    #[serde(default)]
    pub asm: AsmHeader,
    pub t_erase_us: u16,
    pub t_program_us: u16,
    pub t_config_us: u16,
    /// Data EEPROM byte write.
    #[serde(default)]
    pub t_eeprom_us: u16,
    /// Row erase; 0 when the part has no row erase command.
    #[serde(default)]
    pub t_row_erase_us: u16,
}

impl Device {
    pub fn algorithm(&self) -> Result<Algorithm> {
        SPECS
            .iter()
            .find(|(ds, _)| *ds == self.prog_spec)
            .map(|&(_, a)| a)
            .ok_or_else(|| {
                anyhow!(
                    "{} uses programming spec DS{} (arch {}), not implemented yet",
                    self.name,
                    self.prog_spec,
                    self.arch
                )
            })
    }

    /// Parses a `.PIC` file from a device family pack.
    pub fn from_pic_xml(xml: &str) -> Result<Self> {
        let mut reader = Reader::from_str(xml);
        let mut name = None;
        let mut arch = String::new();
        let mut prog_spec = String::new();
        let mut waits: HashMap<String, u32> = HashMap::new();
        let mut row_words = None;
        let mut flash_end = 0u32;
        let mut user_ids = None;
        let mut device_id = None;
        let mut eeprom = None;
        let mut sfrs = BTreeMap::new();
        let mut data_end = 0;
        let mut dcr_default = 0x3FFF;
        let mut field: Option<ConfigField> = None;
        let mut config_fields = Vec::new();
        let mut gpr = Vec::new();
        let mut common_ram = None;
        let mut in_sfr_sector = false;
        let mut config = Vec::new();
        let mut lvp_fuse = None;
        let mut power = Power::default();

        let mut in_config_sector = false;
        // Current config word being walked: address, mask, next bit, mode count.
        let mut dcr: Option<(u32, u16, i64, u32)> = None;

        loop {
            let (e, is_empty) = match reader.read_event()? {
                Event::Start(e) => (e, false),
                Event::Empty(e) => (e, true),
                Event::End(e) => {
                    match e.local_name().as_ref() {
                        "ConfigFuseSector" => in_config_sector = false,
                        "SFRDataSector" => in_sfr_sector = false,
                        "DCRDef" => {
                            if let Some((address, mask, _, _)) = dcr.take() {
                                config.push(ConfigWord { address, mask, default: dcr_default });
                            }
                        }
                        "DCRFieldDef" => {
                            if let Some(f) = field.take() {
                                config_fields.push(f);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }
                Event::Eof => break,
                _ => continue,
            };
            let a = attrs(&e)?;
            match e.local_name().as_ref() {
                "PIC" => {
                    name = a.get("name").cloned();
                    arch = a.get("arch").cloned().unwrap_or_default();
                    prog_spec = normalize_spec(a.get("psid").map_or("", String::as_str));
                }
                "VPP" => power.vpp_mv = millivolts(a.get("defaultvoltage"))?,
                "VDD" => {
                    power.vdd_min_mv = millivolts(a.get("minvoltage"))?;
                    power.vdd_max_mv = millivolts(a.get("maxvoltage"))?;
                }
                "Programming" => power.lvp_min_mv = millivolts(a.get("lvpthresh"))?,
                "ProgrammingWaitTime" => {
                    // A row erase listed without a time still has the command.
                    if let (Some(op), None) = (a.get("progop"), a.get("time")) {
                        waits.entry(op.clone()).or_insert(0);
                    }
                    if let (Some(op), Some(t)) = (a.get("progop"), a.get("time")) {
                        let t = num(t)?;
                        let us = match a.get("timeunits").map(String::as_str) {
                            Some("ms") => t * 1000,
                            _ => t,
                        };
                        waits.insert(op.clone(), us);
                    }
                }
                "ProgrammingRowSize" if a.get("progop").map(String::as_str) == Some("pgm") => {
                    row_words = a.get("nzsize").map(|n| num(n)).transpose()?;
                }
                "CodeSector" => flash_end = flash_end.max(num(req(&a, "endaddr")?)?),
                "UserIDSector" => user_ids = Some(range(&a)?),
                "EEDataSector" => eeprom = Some(range(&a)?),
                "DeviceIDSector" => {
                    let mask = a.get("mask").map(|m| num(m)).transpose()?;
                    device_id = Some((num(req(&a, "value")?)?, mask.unwrap_or(0xFFFF)));
                }
                "ConfigFuseSector" if !is_empty => in_config_sector = true,
                "SFRDataSector" if !is_empty => in_sfr_sector = true,
                "DataSpace" => {
                    if let Some(end) = a.get("endaddr") {
                        data_end = num(end)?;
                    }
                }
                "SFRDef" if in_sfr_sector => {
                    if let (Some(name), Some(addr)) = (a.get("cname"), a.get("_addr")) {
                        sfrs.insert(num(addr)?, name.clone());
                    }
                }
                "DCRDef" if in_config_sector => {
                    let addr = num(req(&a, "_addr")?)?;
                    let mask = num(req(&a, "impl")?)? as u16;
                    let default = a.get("default").map(|d| num(d)).transpose()?.unwrap_or(0x3FFF) as u16;
                    if is_empty {
                        config.push(ConfigWord { address: addr, mask, default });
                    } else {
                        dcr = Some((addr, mask, 0, 0));
                        dcr_default = default;
                    }
                }
                "GPRDataSector" if !a.contains_key("shadowidref") => {
                    let (begin, end) = (num(req(&a, "beginaddr")?)?, num(req(&a, "endaddr")?)?);
                    if begin & 0x7F >= 0x70 {
                        common_ram.get_or_insert((begin, end));
                    } else {
                        gpr.push((begin, end));
                    }
                }
                "DCRFieldSemantic" => {
                    if let (Some(field), Some(name), Some(when)) =
                        (field.as_mut(), a.get("cname"), a.get("when"))
                    {
                        // "(field & 0x7) == 0x4"
                        if let Some(value) = when.rsplit("==").next().map(|v| num(v.trim())).transpose()? {
                            field.values.push((name.clone(), value as u16));
                        }
                    }
                }
                // Field offsets only follow the first (default) mode.
                "DCRMode" => {
                    if let Some(d) = dcr.as_mut() {
                        d.3 += 1;
                    }
                }
                "AdjustPoint" => {
                    if let Some(d) = dcr.as_mut().filter(|d| d.3 == 1) {
                        d.2 += signed(req(&a, "offset")?)?;
                    }
                }
                "DCRFieldDef" => {
                    if let Some(d) = dcr.as_mut().filter(|d| d.3 == 1) {
                        let width = num(req(&a, "nzwidth")?)?;
                        if a.get("cname").map(String::as_str) == Some("LVP") && (0..16).contains(&d.2) {
                            lvp_fuse = Some((d.0, 1u16 << d.2));
                        }
                        if let (Some(name), true) = (a.get("cname"), (0..16).contains(&d.2)) {
                            let f = ConfigField {
                                address: d.0,
                                name: name.clone(),
                                shift: d.2 as u8,
                                width: width as u8,
                                values: Vec::new(),
                            };
                            if is_empty {
                                config_fields.push(f);
                            } else {
                                field = Some(f);
                            }
                        }
                        d.2 += width as i64;
                    }
                }
                _ => {}
            }
        }

        let name = name.context("no edc:PIC element")?;
        // Missing waits stay 0; each algorithm checks what it needs.
        let wait = |op: &str| -> Result<u16> {
            let us = waits.get(op).copied().unwrap_or(0);
            u16::try_from(us).with_context(|| format!("{name}: {op} wait {us} us too long"))
        };
        // OTP and legacy parts have no device ID; mask 0 disables the check.
        let (device_id, device_id_mask) = device_id.unwrap_or((0, 0));
        Ok(Device {
            arch,
            prog_spec,
            device_id: device_id as u16,
            device_id_mask: device_id_mask as u16,
            flash_words: flash_end,
            row_words: row_words.unwrap_or(1),
            user_ids: user_ids.unwrap_or(CONFIG_SPACE..CONFIG_SPACE),
            config,
            lvp_fuse,
            power,
            eeprom,
            sfrs,
            data_end,
            pack_dir: None,
            config_fields,
            gpr,
            common_ram,
            asm: AsmHeader::default(),
            t_erase_us: wait("erase")?,
            t_program_us: wait("pgm")?,
            t_config_us: wait("cfg")?,
            t_eeprom_us: wait("eedata")?,
            // Some packs list the row erase without a time: same as a row write.
            t_row_erase_us: match wait("rowerase")? {
                0 if waits.contains_key("rowerase") => wait("pgm")?,
                t => t,
            },
            name,
        })
    }
}

/// "DS-40001846", "DS40001846", "40001846" -> "40001846".
fn normalize_spec(psid: &str) -> String {
    let s = psid.trim();
    let s = s.strip_prefix("DS").unwrap_or(s);
    let s = s.strip_prefix("DS").unwrap_or(s);
    s.trim_start_matches(['-', ' ']).to_owned()
}

fn attrs(e: &BytesStart) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for attr in e.attributes() {
        let attr = attr?;
        let key = attr.key.local_name().as_ref().to_owned();
        map.insert(key, attr.normalized_value(XmlVersion::Implicit1_0)?.into_owned());
    }
    Ok(map)
}

fn req<'a>(a: &'a HashMap<String, String>, key: &str) -> Result<&'a String> {
    a.get(key).with_context(|| format!("missing attribute {key}"))
}

fn num(s: &str) -> Result<u32> {
    let s = s.trim();
    let n = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => s.parse(),
    };
    n.with_context(|| format!("bad number {s:?}"))
}

fn signed(s: &str) -> Result<i64> {
    match s.trim().strip_prefix('-') {
        Some(rest) => Ok(-(num(rest)? as i64)),
        None => Ok(num(s)? as i64),
    }
}

fn range(a: &HashMap<String, String>) -> Result<Range<u32>> {
    Ok(num(req(a, "beginaddr")?)?..num(req(a, "endaddr")?)?)
}

/// Pack roots searched when `--packs` is not given.
pub fn default_pack_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = std::env::var_os("PICFLASH_PACKS") {
        roots.push(PathBuf::from(dir));
    }
    // MPLAB X installs, newest first.
    let mut bases = vec![
        PathBuf::from("/Applications/microchip/mplabx"),
        PathBuf::from("/opt/microchip/mplabx"),
    ];
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(var) {
            bases.push(Path::new(&dir).join("Microchip").join("MPLABX"));
        }
    }
    for base in bases {
        let mut versions: Vec<PathBuf> = std::fs::read_dir(base)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join("packs"))
            .filter(|p| p.is_dir())
            .collect();
        versions.sort_by_key(|p| std::cmp::Reverse(version_key(p.parent().unwrap())));
        roots.extend(versions);
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        roots.push(Path::new(&home).join(".mchp_packs"));
    }
    roots
}

/// Finds `<root>/<vendor>/<pack>/<version>/edc/<part>.PIC`, newest pack version first.
pub fn find_pic_file(roots: &[PathBuf], part: &str) -> Result<PathBuf> {
    let file = format!("{}.PIC", part.to_ascii_uppercase());
    for root in roots {
        let mut hits = Vec::new();
        for vendor in read_dirs(root) {
            for pack in read_dirs(&vendor) {
                for version in read_dirs(&pack) {
                    let path = version.join("edc").join(&file);
                    if path.is_file() {
                        hits.push((version_key(&version), path));
                    }
                }
            }
        }
        if let Some((_, path)) = hits.into_iter().max() {
            return Ok(path);
        }
    }
    bail!(
        "no {file} in device packs ({}); install the pack in MPLAB X or pass --packs",
        roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    )
}

/// Loads a part by name. Like ipecmd, accepts it without the `PIC` prefix
/// (`16F1789`).
pub fn load(roots: &[PathBuf], part: &str) -> Result<Device> {
    if let Some(device) = crate::devdb::Database::embedded().find(part) {
        return Ok(device.clone());
    }
    load_from_packs(roots, part)
}

/// Loads a part from installed device packs (MPLAB X), bypassing the
/// embedded database.
pub fn load_from_packs(roots: &[PathBuf], part: &str) -> Result<Device> {
    let part = part.trim().to_ascii_uppercase();
    let path = find_pic_file(roots, &part).or_else(|e| {
        if part.starts_with("PIC") {
            Err(e)
        } else {
            find_pic_file(roots, &format!("PIC{part}")).map_err(|_| e)
        }
    })?;
    let xml = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let mut device =
        Device::from_pic_xml(&xml).with_context(|| format!("cannot parse {}", path.display()))?;
    // <pack>/<version>/edc/<part>.PIC
    device.pack_dir = path.parent().and_then(Path::parent).map(Path::to_path_buf);
    Ok(device)
}

/// Every `.PIC` file of the newest version of each pack under the first root
/// that has any, parsed. Used to list parts and track algorithm coverage.
pub fn scan(roots: &[PathBuf]) -> Vec<(PathBuf, Result<Device>)> {
    for root in roots {
        let mut files = Vec::new();
        for vendor in read_dirs(root) {
            for pack in read_dirs(&vendor) {
                let Some(version) = read_dirs(&pack).max_by_key(|v| version_key(v)) else {
                    continue;
                };
                let edc = version.join("edc");
                files.extend(
                    std::fs::read_dir(&edc)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x == "PIC")),
                );
            }
        }
        if !files.is_empty() {
            files.sort();
            return files
                .into_iter()
                .map(|path| {
                    let dev = std::fs::read_to_string(&path)
                        .map_err(anyhow::Error::from)
                        .and_then(|xml| Device::from_pic_xml(&xml));
                    (path, dev)
                })
                .collect();
        }
    }
    Vec::new()
}

fn read_dirs(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
}

/// "1.9.258" or "v6.35" -> [1, 9, 258] / [6, 35] for ordering.
fn version_key(dir: &Path) -> Vec<u32> {
    dir.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .trim_start_matches('v')
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Hand-written sample in the `.PIC` schema with PIC16F1789 values.
    pub const SAMPLE: &str = include_str!("../tests/fixtures/sample.PIC");

    pub fn pic16f1789() -> Device {
        Device::from_pic_xml(SAMPLE).unwrap()
    }

    #[test]
    fn parses_sample() {
        let d = pic16f1789();
        assert_eq!(d.name, "PIC16F1789");
        assert_eq!(d.prog_spec, "40001457");
        assert_eq!(d.algorithm().unwrap(), Algorithm::Midrange6Bit);
        assert_eq!((d.device_id, d.device_id_mask), (0x302A, 0x3FFF));
        assert_eq!(d.flash_words, 0x4000);
        assert_eq!(d.row_words, 32);
        assert_eq!(d.user_ids, 0x8000..0x8004);
        assert_eq!(
            d.config,
            vec![
                ConfigWord { address: 0x8007, mask: 0x3FFF, default: 0x3FFF },
                ConfigWord { address: 0x8008, mask: 0x3F23, default: 0x3FFF },
            ]
        );
        assert_eq!(d.lvp_fuse, Some((0x8008, 1 << 13)));
        assert_eq!(d.eeprom, Some(0xF000..0xF100));
        assert_eq!((d.t_erase_us, d.t_program_us, d.t_config_us), (5000, 2500, 5000));
    }

    #[test]
    fn rejects_unknown_spec() {
        let mut d = pic16f1789();
        d.prog_spec = "99999999".into();
        assert!(d.algorithm().unwrap_err().to_string().contains("DS99999999"));
    }

    #[test]
    fn version_ordering() {
        assert!(version_key(Path::new("1.10.2")) > version_key(Path::new("1.9.258")));
        assert!(version_key(Path::new("v6.35")) > version_key(Path::new("v6.5")));
    }
}
