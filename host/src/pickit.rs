//! Microchip PICkit 4, PICkit 5, PICkit Basic and MPLAB Snap in MPLAB mode.
//!
//! These tools run "Gen4" bytecode scripts sent over USB. This module turns
//! the bit-level scripts of [`crate::algo`] (the ones the picflash board and
//! the simulator run) into Gen4 scripts, so every family and flow works
//! unchanged on a PICkit.
//!
//! Sources, no Microchip code copied:
//! - Gen4 opcodes and operand formats: the Python stack shipped in Microchip
//!   device packs (`scripts/*/common`, Apache-2.0): `gen4primitives.py`,
//!   `primitivegenerator.py`, `proginterfaceprovider.py`.
//! - USB framing, tool IDs and power commands: avrdude `src/pickit5.c` and
//!   `src/usbdevs.h` (GPL-2; protocol facts only).
//! - VPP level select for high-voltage entry (0x41) and the order of the
//!   entry steps: compared with the PICkit 4 script sequences in MPLAB X.
//! - Operand units, power and VPP behaviour, read routing and error codes:
//!   static analysis of the PICkit 4/5/Basic firmware for interoperability
//!   (REA + Ghidra), summarised in `docs/pickit-gen4.md`.

use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use nusb::transfer::{Buffer, Bulk, In, Out};
use nusb::MaybeFuture;
use picflash_protocol::{decode_shape, op, op_len, GROUP_GAP_US, PROTOCOL_VERSION};

use crate::algo::Entry;
use crate::device::{Algorithm, Device};
use crate::transport::Transport;

/// Gen4 opcodes used here.
pub mod g4 {
    /// `[vdd: u32][vpp: u32][vpp_op: u32]` in mV. Only stores the values.
    pub const SET_VOLTAGES: u8 = 0x40;
    /// MCLR "high" means VPP: starts the pump (needs vpp >= 7.6 V, else
    /// nothing happens), waits 20 ms.
    pub const VPP_LEVEL_PROGRAM: u8 = 0x41;
    /// MCLR "high" means the I/O rail (VDD); turns the pump path off.
    pub const VPP_LEVEL_OPERATIONAL: u8 = 0x42;
    /// Tool powers the target: VDD on (1.5-5 V). External power: sets the
    /// I/O rail to the measured target VDD, error 0x305 if below 0.6 V.
    pub const VDD_ON: u8 = 0x43;
    /// VDD off and MCLR released (floating).
    pub const POWER_OFF: u8 = 0x44;
    /// `[source: u32]`: 0 = target powered externally, 1 = tool supplies VDD.
    pub const POWER_SOURCE: u8 = 0x46;
    /// Reply carries the target VDD in mV at offset 28.
    pub const GET_VOLTAGES: u8 = 0x47;
    /// PIC16 6-bit family: clock in a 16-bit frame, LSb first; pushes the
    /// 14-bit word (2 bytes) when `SET_UPLOAD` is on.
    pub const P16F_READ_WORD: u8 = 0x80;
    /// `[bits: u8][value: u32]`, MSb first.
    pub const WRITE_BITS_MSB: u8 = 0x83;
    /// PIC16 8-bit family: clock in a 24-bit frame, MSb first; pushes the
    /// 16-bit payload (2 bytes, unmasked) when `SET_UPLOAD` is on.
    pub const P16ENV3_READ_WORD: u8 = 0x84;
    /// `[ms: u16]`.
    pub const DELAY_MS: u8 = 0x94;
    /// Read ops push their results (reset at every message): to the data
    /// endpoint for upload scripts, else to the reply from offset 24.
    pub const SET_UPLOAD: u8 = 0x95;
    /// `[us: u16]`.
    pub const DELAY_US: u8 = 0xA0;
    pub const VPP_ON: u8 = 0xB0;
    pub const VPP_OFF: u8 = 0xB1;
    /// `[pins: u8]`, see [`pins`].
    pub const SET_ICSP_PINS: u8 = 0xB2;
    /// `[bits: u8][value: u32]`, LSb first.
    pub const WRITE_BITS_LSB: u8 = 0xB5;
    /// `[period_ns: u32]`, full ICSP clock period. Default: fastest.
    pub const SET_SPEED: u8 = 0xEC;

    pub mod pins {
        pub const ALL_LOW: u8 = 0;
        pub const CLK_IN_DATA_IN: u8 = 3;
    }
}

/// ICSP clock period in ns: 1 MHz, slower than MPLAB's 250 ns (4 MHz) to
/// tolerate long wires. Must always be sent: the default is the fastest.
const SPEED: u32 = 1000;

/// Range in which the VPP pump regulates; below 7.6 V high-voltage entry
/// would silently put MCLR at VDD instead.
const VPP_RANGE_MV: std::ops::RangeInclusive<u16> = 7710..=13500;
/// Tool-supplied VDD range (the firmware clamps outside it).
const VDD_RANGE_MV: std::ops::RangeInclusive<u16> = 1500..=5000;

/// Gen4 script bytes per message. Long scripts are split at op boundaries;
/// the target state carries over between scripts.
const MAX_SCRIPT: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    PicKit4,
    PicKit5,
    PicKitBasic,
    Snap,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::PicKit4 => "PICkit 4",
            Kind::PicKit5 => "PICkit 5",
            Kind::PicKitBasic => "PICkit Basic",
            Kind::Snap => "MPLAB Snap",
        }
    }
    /// Can supply target VDD.
    pub fn can_power(self) -> bool {
        matches!(self, Kind::PicKit4 | Kind::PicKit5)
    }
    /// Can measure target VDD.
    pub fn reads_vdd(self) -> bool {
        !matches!(self, Kind::PicKitBasic)
    }
    /// Has a VPP pump for high-voltage entry.
    pub fn has_hv(self) -> bool {
        matches!(self, Kind::PicKit4 | Kind::PicKit5)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Mplab,
    /// AVR (CMSIS-DAP) personality of PICkit 4 / Snap.
    Avr,
    /// CMSIS-DAP firmware of the PICkit Basic.
    Cmsis,
    Bootloader,
}

const MICROCHIP: u16 = 0x04D8;
const ATMEL: u16 = 0x03EB;

const IDS: &[(u16, u16, Kind, Mode)] = &[
    (MICROCHIP, 0x9012, Kind::PicKit4, Mode::Mplab),
    (MICROCHIP, 0x9017, Kind::PicKit4, Mode::Bootloader),
    (ATMEL, 0x2177, Kind::PicKit4, Mode::Avr),
    (MICROCHIP, 0x9036, Kind::PicKit5, Mode::Mplab),
    (MICROCHIP, 0x9018, Kind::Snap, Mode::Mplab),
    (MICROCHIP, 0x9019, Kind::Snap, Mode::Bootloader),
    (ATMEL, 0x2180, Kind::Snap, Mode::Avr),
    (MICROCHIP, 0x9054, Kind::PicKitBasic, Mode::Mplab),
    (MICROCHIP, 0x9055, Kind::PicKitBasic, Mode::Mplab),
    (MICROCHIP, 0x9056, Kind::PicKitBasic, Mode::Mplab),
    (MICROCHIP, 0x9057, Kind::PicKitBasic, Mode::Bootloader),
    (MICROCHIP, 0x90AB, Kind::PicKitBasic, Mode::Cmsis),
    (MICROCHIP, 0x90AC, Kind::PicKitBasic, Mode::Cmsis),
    (MICROCHIP, 0x90AD, Kind::PicKitBasic, Mode::Cmsis),
    (MICROCHIP, 0x90AE, Kind::PicKitBasic, Mode::Cmsis),
];

fn lookup(vid: u16, pid: u16) -> Option<(Kind, Mode)> {
    IDS.iter().find(|i| i.0 == vid && i.1 == pid).map(|i| (i.2, i.3))
}

/// A connected Microchip tool, whatever its mode.
#[derive(Debug, Clone)]
pub struct Found {
    pub kind: Kind,
    pub usable: bool,
    pub serial: Option<String>,
    /// Why the tool cannot be used as is, and how to fix it.
    pub problem: Option<String>,
}

/// Lists connected PICkit / Snap tools.
pub fn scan() -> Vec<Found> {
    let Ok(devices) = nusb::list_devices().wait() else {
        return Vec::new();
    };
    devices
        .filter_map(|d| {
            let (kind, mode) = lookup(d.vendor_id(), d.product_id())?;
            let name = kind.name();
            let problem = match mode {
                Mode::Mplab => None,
                Mode::Avr => Some(format!(
                    "{name} en mode AVR : passe-le en mode MPLAB \
                     (avrdude -c {} -x mode=mplab, ou une fois depuis MPLAB X)",
                    if kind == Kind::Snap { "snap" } else { "pickit4" }
                )),
                Mode::Cmsis => Some(format!(
                    "{name} avec le firmware CMSIS-DAP : charge le firmware « MPLAB » \
                     une fois depuis MPLAB X ou IPE"
                )),
                Mode::Bootloader => Some(format!(
                    "{name} en mode bootloader : débranche-le et rebranche-le, ou \
                     recharge son firmware depuis MPLAB X"
                )),
            };
            Some(Found {
                kind,
                usable: problem.is_none(),
                serial: d.serial_number().map(str::to_owned),
                problem,
            })
        })
        .collect()
}

/// What the program/verify session needs from the tool.
#[derive(Debug, Clone, Copy, Default)]
pub struct Setup {
    pub entry: Entry,
    /// Supply the target from the tool at this voltage (mV).
    pub power_mv: Option<u16>,
}

/// One translated Gen4 script and where its results come from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chunk {
    pub code: Vec<u8>,
    pub reads: Vec<Read>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    /// Two bytes (one word) from the upload stream.
    Word,
    /// Answered on the host (PING).
    Const(u32),
}

impl Chunk {
    fn device_reads(&self) -> usize {
        self.reads.iter().filter(|r| **r == Read::Word).count()
    }

    /// The bytes sent to the tool: results are routed to the upload stream
    /// when the script reads anything.
    pub fn script(&self) -> Vec<u8> {
        let mut s = Vec::with_capacity(self.code.len() + 1);
        if self.device_reads() > 0 {
            s.push(g4::SET_UPLOAD);
        }
        s.extend_from_slice(&self.code);
        s
    }

    /// Rebuilds the values the bit-level script expects from the words read.
    fn values(&self, words: &[u16]) -> Vec<u32> {
        let mut words = words.iter();
        self.reads
            .iter()
            .map(|r| match r {
                // Framed like a raw read: start bit, 14 data bits, stop bit.
                Read::Word => (*words.next().unwrap() as u32 & 0x3FFF) << 1,
                Read::Const(v) => *v,
            })
            .collect()
    }
}

/// Turns one bit-level script into Gen4 scripts of at most [`MAX_SCRIPT`]
/// bytes.
pub fn translate(script: &[u8], algo: Algorithm) -> Result<Vec<Chunk>> {
    let gap = (GROUP_GAP_US as u16).to_le_bytes();
    let mut chunks = Vec::new();
    let mut cur = Chunk::default();
    let mut i = 0;
    while i < script.len() {
        let code = script[i];
        let len = op_len(code).with_context(|| format!("unknown op {code:#04x}"))?;
        ensure!(i + len <= script.len(), "truncated script");
        let arg = &script[i + 1..i + len];
        i += len;

        let mut bytes = Vec::new();
        let mut read = None;
        match code {
            op::PING => read = Some(Read::Const(PROTOCOL_VERSION as u32)),
            // MCLR high at VDD first, ICSP lines low, then MCLR low: the
            // target waits for the key.
            // 20 ms lets a pump left on by a high-voltage session settle.
            op::ENTER => bytes.extend_from_slice(&[
                g4::VPP_LEVEL_OPERATIONAL,
                g4::DELAY_MS, 20, 0,
                g4::VPP_ON,
                g4::DELAY_MS, 1, 0,
                g4::SET_ICSP_PINS, g4::pins::ALL_LOW,
                g4::VPP_OFF,
                g4::DELAY_MS, 2, 0,
            ]),
            // Reset, ICSP lines low, then MCLR straight to VPP.
            op::ENTER_HV => bytes.extend_from_slice(&[
                g4::VPP_OFF,
                g4::DELAY_MS, 2, 0,
                g4::SET_ICSP_PINS, g4::pins::ALL_LOW,
                g4::VPP_LEVEL_PROGRAM,
                g4::VPP_ON,
                g4::DELAY_MS, 5, 0,
            ]),
            // Release ICSP lines, leave program mode, then MCLR to VDD so the
            // target runs.
            op::RELEASE => bytes.extend_from_slice(&[
                g4::SET_ICSP_PINS, g4::pins::CLK_IN_DATA_IN,
                g4::VPP_OFF,
                g4::DELAY_MS, 1, 0,
                g4::VPP_LEVEL_OPERATIONAL,
                g4::DELAY_MS, 20, 0,
                g4::VPP_ON,
            ]),
            op::OUT => {
                let (count, msb) = decode_shape(arg[0]).context("bad shape")?;
                bytes.push(if msb { g4::WRITE_BITS_MSB } else { g4::WRITE_BITS_LSB });
                bytes.push(count as u8);
                bytes.extend_from_slice(&arg[1..5]);
                bytes.push(g4::DELAY_US);
                bytes.extend_from_slice(&gap);
            }
            op::IN => {
                let shape = decode_shape(arg[0]).context("bad shape")?;
                let opcode = match (algo, shape) {
                    (Algorithm::Midrange6Bit, (16, false)) => g4::P16F_READ_WORD,
                    (Algorithm::Midrange8Bit, (24, true)) => g4::P16ENV3_READ_WORD,
                    _ => bail!("PICkit: no Gen4 read for {shape:?} on {algo:?}"),
                };
                bytes.push(opcode);
                bytes.push(g4::DELAY_US);
                bytes.extend_from_slice(&gap);
                read = Some(Read::Word);
            }
            op::DELAY_US => {
                bytes.push(g4::DELAY_US);
                bytes.extend_from_slice(arg);
            }
            _ => bail!("op {code:#04x} has no Gen4 translation"),
        }

        // One byte reserved for SET_UPLOAD.
        if cur.code.len() + bytes.len() + 1 > MAX_SCRIPT {
            chunks.push(std::mem::take(&mut cur));
        }
        cur.code.extend_from_slice(&bytes);
        cur.reads.extend(read);
    }
    if !cur.code.is_empty() || !cur.reads.is_empty() {
        chunks.push(cur);
    }
    Ok(chunks)
}

/// Runs Gen4 scripts on something: the real tool, or a test double.
pub trait Gen4 {
    /// Runs `script` and returns `upload` bytes from the data stream.
    fn exec(&mut self, script: &[u8], upload: usize) -> Result<Vec<u8>>;
}

/// [`Transport`] over any [`Gen4`] executor.
pub struct Gen4Link<G: Gen4> {
    pub tool: G,
    pub algo: Algorithm,
    pub scripts: usize,
}

impl<G: Gen4> Gen4Link<G> {
    pub fn new(tool: G, algo: Algorithm) -> Self {
        Self { tool, algo, scripts: 0 }
    }
}

impl<G: Gen4> Transport for Gen4Link<G> {
    fn run(&mut self, script: &[u8]) -> Result<Vec<u32>> {
        let mut values = Vec::new();
        for chunk in translate(script, self.algo)? {
            let script = chunk.script();
            ensure!(script.len() <= MAX_SCRIPT, "Gen4 script too long");
            let n = chunk.device_reads();
            let data = if chunk.code.is_empty() {
                Vec::new()
            } else {
                self.scripts += 1;
                self.tool.exec(&script, n * 2)?
            };
            ensure!(data.len() == n * 2, "expected {} bytes, got {}", n * 2, data.len());
            let words: Vec<u16> =
                data.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            if self.algo == Algorithm::Midrange8Bit && words.contains(&0xFFFF) {
                // A real target never shifts out 16 ones: ICSPDAT floats.
                bail!("lecture 0xFFFF : cible absente, non alimentée ou ICSPDAT non connecté");
            }
            values.extend(chunk.values(&words));
        }
        Ok(values)
    }
}

// --- USB -----------------------------------------------------------------

const EP_CMD_OUT: u8 = 0x02;
const EP_CMD_IN: u8 = 0x81;
const EP_DATA_OUT: u8 = 0x04;
const EP_DATA_IN: u8 = 0x83;

const TYPE_SCRIPT: u32 = 0x0000_0100;
const TYPE_SCRIPT_UPLOAD: u32 = 0x8000_0102;
const TYPE_SCRIPT_DONE: u32 = 0x0000_0103;
/// Reply type of every script message.
const REPLY_OK: u32 = 0x0D;
const GET_FIRMWARE_INFO: u8 = 0xE1;

const TIMEOUT: Duration = Duration::from_secs(10);

fn u32_at(buf: &[u8], at: usize) -> u32 {
    buf.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// mV to V, for messages.
fn volts(mv: u16) -> f64 {
    mv as f64 / 1000.0
}

/// Error names reported by the tool: firmware codes found by analysis, then
/// avrdude's table.
fn error_name(code: u32) -> &'static str {
    match code {
        0x300 | 0x301 => "VDD hors de 1,5-5 V",
        0x305 => "cible non alimentée",
        0x308 | 0x309 => "VPP n'atteint pas la tension demandée",
        0x500 => "message trop long",
        0x502 => "trop de résultats",
        0x503 => "longueurs incohérentes",
        32 => "NO_DEVICE_FOUND",
        34 => "NO_TARGET_POWER",
        49 => "INVALID_PHYSICAL_STATE",
        50 => "ILLEGAL_STATE",
        51 => "INVALID_CONFIG",
        57 => "ILLEGAL_VALUE",
        60 => "TIMEOUT",
        81 => "NO_RESPONSE_CHECK_CONNECTIONS",
        82 => "NO_VOUT_SET",
        83 => "VOUT_ERROR",
        84 => "VTG_TOO_LOW_FOR_FEATURE",
        112 => "READ_ERROR",
        113 => "WRITE_ERROR",
        114 => "WRITE_TIMEOUT",
        144 => "NOT_SUPPORTED",
        145 => "NOT_IMPLEMENTED",
        _ => "UNKNOWN",
    }
}

pub struct PicKit {
    pub kind: Kind,
    pub serial: Option<String>,
    pub firmware: String,
    /// Target VDD measured or supplied, in mV.
    pub vdd_mv: Option<u16>,
    powered: bool,
    cmd_out: nusb::Endpoint<Bulk, Out>,
    cmd_in: nusb::Endpoint<Bulk, In>,
    data_in: nusb::Endpoint<Bulk, In>,
    _data_out: nusb::Endpoint<Bulk, Out>,
    _interface: nusb::Interface,
}

impl PicKit {
    /// Opens the first usable tool, or the one whose serial number matches.
    pub fn open(serial: Option<&str>) -> Result<Self> {
        let found: Vec<_> = nusb::list_devices()
            .wait()
            .context("cannot list USB devices")?
            .filter_map(|d| lookup(d.vendor_id(), d.product_id()).map(|k| (d, k)))
            .filter(|(d, _)| serial.is_none() || d.serial_number() == serial)
            .collect();
        let Some((info, (kind, _))) = found.iter().find(|(_, (_, m))| *m == Mode::Mplab) else {
            if let Some(problem) = scan().into_iter().find_map(|f| f.problem) {
                bail!("{problem}");
            }
            bail!("aucun PICkit / Snap trouvé (branché en mode MPLAB ?)");
        };
        let kind = *kind;
        let device = info.open().wait().with_context(|| format!("cannot open the {}", kind.name()))?;

        // The interface holding the command endpoint.
        let config = device.active_configuration().context("no USB configuration")?;
        let number = config
            .interfaces()
            .find(|i| {
                i.alt_settings().any(|a| a.endpoints().any(|e| e.address() == EP_CMD_OUT))
            })
            .map(|i| i.interface_number())
            .context("no command endpoint: unexpected firmware")?;
        let interface = match device.claim_interface(number).wait() {
            Ok(i) => i,
            Err(_) => device
                .detach_and_claim_interface(number)
                .wait()
                .with_context(|| format!("cannot claim the {} (used by MPLAB?)", kind.name()))?,
        };
        let mut pk = PicKit {
            kind,
            serial: info.serial_number().map(str::to_owned),
            firmware: String::new(),
            vdd_mv: None,
            powered: false,
            cmd_out: interface.endpoint::<Bulk, Out>(EP_CMD_OUT)?,
            cmd_in: interface.endpoint::<Bulk, In>(EP_CMD_IN)?,
            data_in: interface.endpoint::<Bulk, In>(EP_DATA_IN)?,
            _data_out: interface.endpoint::<Bulk, Out>(EP_DATA_OUT)?,
            _interface: interface,
        };
        pk.firmware = pk.firmware_info()?;
        if kind == Kind::PicKit5 {
            // Leaves programmer-to-go mode (as avrdude does).
            pk.exec(&[0x5E, 0, 0, 0, 0], 4)?;
        }
        Ok(pk)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let c = self.cmd_out.transfer_blocking(bytes.to_vec().into(), TIMEOUT);
        c.status.context("USB write failed")?;
        let mps = self.cmd_out.max_packet_size();
        if bytes.len().is_multiple_of(mps) {
            // A full last packet needs a zero-length one to end the transfer.
            self.cmd_out.transfer_blocking(Buffer::new(0), TIMEOUT).status?;
        }
        Ok(())
    }

    fn reply(&mut self) -> Result<Vec<u8>> {
        let c = self.cmd_in.transfer_blocking(Buffer::new(512), TIMEOUT);
        c.status.context("no reply from the PICkit")?;
        Ok(c.buffer.into_vec())
    }

    fn firmware_info(&mut self) -> Result<String> {
        self.write(&[GET_FIRMWARE_INFO])?;
        let r = self.reply()?;
        ensure!(r.first() == Some(&GET_FIRMWARE_INFO), "unexpected firmware info reply");
        Ok(r.get(3..6).map_or_else(String::new, |v| format!("{:02x}.{:02x}.{:02x}", v[0], v[1], v[2])))
    }

    fn send_script(&mut self, kind: u32, script: &[u8], transfer: usize) -> Result<()> {
        let total = 16 + 8 + script.len();
        let mut msg = Vec::with_capacity(total);
        for v in [kind, 0, total as u32, transfer as u32, 0, script.len() as u32] {
            msg.extend_from_slice(&v.to_le_bytes());
        }
        msg.extend_from_slice(script);
        self.write(&msg)
    }

    fn check_reply(&mut self) -> Result<Vec<u8>> {
        let r = self.reply()?;
        let status = u32_at(&r, 0);
        ensure!(status == REPLY_OK, "unexpected PICkit reply {status:#x}");
        let err = u32_at(&r, 16);
        if err != 0 {
            let hint = match err {
                34 | 81 | 0x305 => " : vérifie l'alimentation et le câblage de la cible",
                0x308 | 0x309 => " : MCLR en court-circuit ou chargé ? essaie l'entrée basse tension",
                _ => "",
            };
            bail!("PICkit: erreur {err:#x} ({}){hint}", error_name(err));
        }
        Ok(r)
    }

    fn script_done(&mut self) -> Result<()> {
        let mut msg = Vec::with_capacity(16);
        for v in [TYPE_SCRIPT_DONE, 0, 16, 0] {
            msg.extend_from_slice(&v.to_le_bytes());
        }
        self.write(&msg)?;
        self.check_reply().map(drop)
    }

    /// Runs a script that returns its result in the reply buffer.
    fn command(&mut self, script: &[u8]) -> Result<Vec<u8>> {
        self.send_script(TYPE_SCRIPT, script, 0)?;
        self.check_reply()
    }

    /// Prepares power and VPP for a session on `device`.
    pub fn setup(&mut self, device: &Device, setup: Setup) -> Result<()> {
        let p = device.power;
        if setup.entry == Entry::HighVoltage && !self.kind.has_hv() {
            bail!("le {} ne fait pas de haute tension : utilise l'entrée basse tension (LVP)", self.kind.name());
        }
        if self.kind.can_power() {
            // Start from a known state: tool supply off.
            self.command(&[g4::POWER_SOURCE, 0, 0, 0, 0])?;
            self.command(&[g4::POWER_OFF])?;
            self.powered = false;
            std::thread::sleep(Duration::from_millis(50));
        }
        let measured = if self.kind.reads_vdd() { Some(self.read_vdd()?) } else { None };

        let vdd = match setup.power_mv {
            Some(mv) => {
                ensure!(self.kind.can_power(), "le {} ne peut pas alimenter la cible", self.kind.name());
                ensure!(VDD_RANGE_MV.contains(&mv), "le PICkit fournit 1,5 à 5 V, pas {:.2} V", volts(mv));
                // The firmware does not check: powering an already supplied
                // board would fight its regulator.
                if let Some(m) = measured {
                    ensure!(m < 1000, "la cible est déjà alimentée ({:.2} V) : désactive l'alimentation par le PICkit", volts(m));
                }
                if p.vdd_max_mv != 0 {
                    ensure!(
                        (p.vdd_min_mv..=p.vdd_max_mv).contains(&mv),
                        "{:.2} V hors de la plage de {} ({:.2}-{:.2} V)",
                        volts(mv),
                        device.name,
                        volts(p.vdd_min_mv),
                        volts(p.vdd_max_mv)
                    );
                }
                let mut s = vec![g4::POWER_SOURCE, 1, 0, 0, 0, g4::SET_VOLTAGES];
                for v in [mv, mv, mv] {
                    s.extend_from_slice(&(v as u32).to_le_bytes());
                }
                s.extend_from_slice(&[g4::VPP_LEVEL_OPERATIONAL, g4::VDD_ON]);
                self.command(&s)?;
                self.powered = true;
                std::thread::sleep(Duration::from_millis(100));
                let got = self.read_vdd()?;
                let low = (mv as i32 - 300).min(4400);
                if (got as i32) < low || got as i32 > mv as i32 + 200 {
                    self.power_off()?;
                    bail!("tension cible {:.2} V au lieu de {:.2} V : court-circuit ou consommation trop forte ?", volts(got), volts(mv));
                }
                Some(mv)
            }
            None => match measured {
                Some(m) if m < 1800 => bail!(
                    "cible non alimentée ({:.2} V) : alimente-la ou active « Alimenter la cible »",
                    volts(m)
                ),
                m => m,
            },
        };
        self.vdd_mv = vdd;

        if let Some(v) = vdd {
            if setup.entry == Entry::LowVoltage && p.lvp_min_mv != 0 && v < p.lvp_min_mv {
                bail!(
                    "{:.2} V est sous le seuil LVP de {} ({:.2} V) : monte VDD ou utilise la haute tension",
                    volts(v),
                    device.name,
                    volts(p.lvp_min_mv)
                );
            }
            let vpp = match setup.entry {
                Entry::HighVoltage => {
                    ensure!(
                        VPP_RANGE_MV.contains(&p.vpp_mv),
                        "{} : VPP {:.2} V hors de la plage du PICkit (7,7-13,5 V)",
                        device.name,
                        volts(p.vpp_mv)
                    );
                    p.vpp_mv
                }
                Entry::LowVoltage => v,
            };
            let mut s = vec![g4::SET_VOLTAGES];
            for x in [v, vpp, v] {
                s.extend_from_slice(&(x as u32).to_le_bytes());
            }
            if !self.powered && self.kind.can_power() {
                // External supply: 0x43 sets the I/O and MCLR rail to the
                // measured target VDD (else it keeps a stale level).
                s.splice(0..0, [g4::POWER_SOURCE, 0, 0, 0, 0]);
                s.extend_from_slice(&[g4::VPP_LEVEL_OPERATIONAL, g4::VDD_ON]);
            }
            self.command(&s)?;
        } else if setup.entry == Entry::HighVoltage {
            bail!("VDD de la cible inconnu : impossible de régler VPP");
        }

        let mut s = vec![g4::SET_SPEED];
        s.extend_from_slice(&SPEED.to_le_bytes());
        self.command(&s).map(drop)
    }

    /// Target VDD in mV.
    pub fn read_vdd(&mut self) -> Result<u16> {
        let r = self.command(&[g4::GET_VOLTAGES])?;
        ensure!(u32_at(&r, 20) == 40, "unexpected voltage reply ({} bytes)", u32_at(&r, 20));
        Ok(u32_at(&r, 28).min(u16::MAX as u32) as u16)
    }

    /// Turns the tool's VDD off when it supplies the target.
    pub fn power_off(&mut self) -> Result<()> {
        if self.powered {
            self.command(&[g4::POWER_SOURCE, 0, 0, 0, 0])?;
            self.command(&[g4::POWER_OFF])?;
            self.powered = false;
        }
        Ok(())
    }

    /// End of session: VDD off if the tool supplied it; otherwise ICSP lines
    /// and MCLR released so nothing back-powers an externally supplied board.
    fn release_lines(&mut self) -> Result<()> {
        if self.powered {
            return self.power_off();
        }
        if self.kind.can_power() {
            self.command(&[g4::SET_ICSP_PINS, g4::pins::CLK_IN_DATA_IN, g4::POWER_OFF])?;
        }
        Ok(())
    }
}

impl Gen4 for PicKit {
    fn exec(&mut self, script: &[u8], upload: usize) -> Result<Vec<u8>> {
        if upload == 0 {
            self.command(script)?;
            return Ok(Vec::new());
        }
        self.send_script(TYPE_SCRIPT_UPLOAD, script, upload)?;
        if let Err(e) = self.check_reply() {
            let _ = self.script_done();
            return Err(e);
        }
        let mut data = Vec::with_capacity(upload);
        let mut failed = None;
        while data.len() < upload {
            let c = self.data_in.transfer_blocking(Buffer::new(512), TIMEOUT);
            if let Err(e) = c.status {
                failed = Some(e);
                break;
            }
            if c.actual_len == 0 {
                break;
            }
            data.extend_from_slice(&c.buffer);
        }
        // Execution errors are only reported in the SCRIPT_DONE reply; a
        // failing script also stops early, so its data is short.
        self.script_done()?;
        if let Some(e) = failed {
            bail!("PICkit upload failed: {e}");
        }
        ensure!(data.len() >= upload, "PICkit: {} octets reçus sur {upload}", data.len());
        data.truncate(upload);
        Ok(data)
    }
}

impl Drop for PicKit {
    fn drop(&mut self) {
        let _ = self.release_lines();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Batch;

    fn ops(f: impl FnOnce(&mut Batch<'_, Recorder>) -> Result<()>) -> Vec<u8> {
        let mut rec = Recorder::default();
        let mut b = Batch::new(&mut rec);
        f(&mut b).unwrap();
        b.finish().unwrap();
        rec.0
    }

    #[derive(Default)]
    struct Recorder(Vec<u8>);
    impl Transport for Recorder {
        fn run(&mut self, script: &[u8]) -> Result<Vec<u32>> {
            self.0.extend_from_slice(script);
            Ok(Vec::new())
        }
    }

    #[test]
    fn low_voltage_entry_of_the_6_bit_family() {
        let script = ops(|b| {
            b.enter()?;
            b.out(crate::algo::LVP_KEY, 32, false)?;
            b.out(0, 1, false)
        });
        let chunks = translate(&script, Algorithm::Midrange6Bit).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].script(),
            [
                0x42, 0x94, 20, 0, 0xB0, 0x94, 1, 0, // MCLR high at VDD
                0xB2, 0, 0xB1, 0x94, 2, 0, // lines low, MCLR low
                0xB5, 32, 0x50, 0x48, 0x43, 0x4D, 0xA0, 2, 0, // "MCHP", LSb first
                0xB5, 1, 0, 0, 0, 0, 0xA0, 2, 0, // 33rd clock
            ]
        );
    }

    #[test]
    fn reads_route_to_the_upload_stream() {
        let script = ops(|b| {
            b.out(0x04, 6, false)?;
            b.input(16, false)
        });
        let chunks = translate(&script, Algorithm::Midrange6Bit).unwrap();
        assert_eq!(chunks[0].script()[0], g4::SET_UPLOAD);
        assert_eq!(chunks[0].reads, [Read::Word]);
        assert_eq!(chunks[0].values(&[0x302A]), [0x302A << 1]);
        // 6-bit reads have no 24-bit form.
        let bad = ops(|b| b.input(24, true));
        assert!(translate(&bad, Algorithm::Midrange6Bit).is_err());
    }

    #[test]
    fn long_scripts_are_split() {
        let script = ops(|b| {
            for _ in 0..400 {
                b.out(0x06, 6, false)?;
            }
            Ok(())
        });
        let chunks = translate(&script, Algorithm::Midrange6Bit).unwrap();
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.script().len() <= MAX_SCRIPT));
        let total: usize = chunks.iter().map(|c| c.code.len()).sum();
        assert_eq!(total, 400 * 9);
    }

    #[test]
    fn every_known_tool_has_a_mode() {
        assert_eq!(lookup(0x04D8, 0x9036), Some((Kind::PicKit5, Mode::Mplab)));
        assert_eq!(lookup(0x03EB, 0x2177), Some((Kind::PicKit4, Mode::Avr)));
        assert!(!Kind::PicKitBasic.has_hv() && Kind::PicKit5.has_hv());
    }
}
