//! Bit-level model of a PIC in Program/Verify mode, used by the tests and by
//! the GUI's simulator mode. It is fed the same script frames the firmware
//! executes and decodes every ICSPCLK edge like the silicon would: key
//! sequence, command and payload bit order, start/stop bits, PC, write
//! latches, flash that only clears bits, and busy windows (any command during
//! a write or erase fails).

use anyhow::{bail, ensure, Result};
use picflash_protocol::{
    decode_shape, op, op_len, store_bit, wire_bit, GROUP_GAP_US, MAX_PAYLOAD, PROTOCOL_VERSION,
};

use crate::algo::LVP_KEY as KEY;
use crate::device::{Algorithm, Device};
use crate::transport::Transport;

#[derive(Debug, Clone, Copy)]
pub enum Phase {
    /// MCLR high: target runs, ICSP clocks are a host bug.
    Running,
    Key { bits: u32, n: u32 },
    /// 6-bit parts need one clock after the key.
    KeyTail,
    Command { acc: u32, n: u32 },
    DataIn { cmd: u8, acc: u32, n: u32 },
    DataOut { frame: u32, n: u32, then_increment: bool },
}

pub struct SimPic {
    pub algo: Algorithm,
    pub row: usize,
    /// Number of config words from 0x8007.
    pub config_words: usize,
    pub flash: Vec<u16>,
    /// Word addresses 0x8000..0x8010.
    pub config: [u16; 16],
    pub latches: Vec<u16>,
    /// Data EEPROM, empty when the part has none.
    pub eeprom: Vec<u8>,
    /// Byte loaded for the next EEPROM write.
    pub eeprom_latch: Option<u8>,
    pub pc: u32,
    pub phase: Phase,
    pub now_us: u64,
    pub busy_until_us: u64,
    /// Spec maxima: row write, config write, bulk erase.
    pub t_row: u64,
    pub t_config: u64,
    pub t_erase: u64,
    pub t_eeprom: u64,
    pub t_row_erase: u64,
    pub frames: usize,
}

/// 8-bit parts map the data EEPROM here.
const EEPROM_BASE: u32 = 0xF000;

impl SimPic {
    /// A blank part matching `device`: its IDs, memory layout and wait
    /// times. Commands arriving sooner than the device's waits fail.
    pub fn new(device: &Device) -> Result<Self> {
        let algo = device.algorithm()?;
        let row = device.row_words as usize;
        ensure!(row.is_power_of_two() && row <= 64, "unsupported row size {row}");
        let mut config = [0x3FFF; 16];
        config[5] = 0x2002; // revision
        config[6] = device.device_id & 0x3FFF;
        let config_words = device
            .config
            .iter()
            .filter(|c| (0x8007..0x8010).contains(&c.address))
            .count();
        Ok(Self {
            algo,
            row,
            config_words,
            flash: vec![0x3FFF; device.flash_words as usize],
            config,
            latches: vec![0x3FFF; row],
            eeprom: vec![0xFF; device.eeprom.as_ref().map_or(0, |r| r.len())],
            eeprom_latch: None,
            pc: 0,
            phase: Phase::Running,
            now_us: 0,
            busy_until_us: 0,
            t_row: device.t_program_us as u64,
            t_config: device.t_config_us as u64,
            t_erase: device.t_erase_us as u64,
            t_eeprom: device.t_eeprom_us as u64,
            t_row_erase: device.t_row_erase_us as u64,
            frames: 0,
        })
    }

    fn six(&self) -> bool {
        self.algo == Algorithm::Midrange6Bit
    }

    fn cmd_bits(&self) -> u32 {
        if self.six() {
            6
        } else {
            8
        }
    }

    fn data_bits(&self) -> u32 {
        if self.six() {
            16
        } else {
            24
        }
    }

    /// Shifts one received bit into `acc` in the family's wire order.
    fn shift(&self, acc: u32, n: u32, bit: bool) -> u32 {
        if self.six() {
            acc | (bit as u32) << n
        } else {
            acc << 1 | bit as u32
        }
    }

    /// EEPROM byte index addressed by the PC: low bits of the PC on 6-bit
    /// parts, 0xF000-based on 8-bit parts.
    fn eeprom_index(&self) -> Option<usize> {
        let index = if self.six() {
            (self.pc & 0xFF) as usize
        } else {
            self.pc.checked_sub(EEPROM_BASE)? as usize
        };
        (index < self.eeprom.len()).then_some(index)
    }

    fn word(&self, addr: u32) -> u16 {
        if !self.six() && addr >= EEPROM_BASE {
            let index = (addr - EEPROM_BASE) as usize;
            return self.eeprom.get(index).map_or(0x3FFF, |&b| b as u16);
        }
        if addr >= 0x8000 {
            self.config[(addr - 0x8000) as usize & 0xF]
        } else {
            self.flash.get(addr as usize).copied().unwrap_or(0x3FFF)
        }
    }

    fn writable_config(&self, index: usize) -> bool {
        index < 4 || (7..7 + self.config_words).contains(&index)
    }

    pub(crate) fn clock_in(&mut self, bit: bool) -> Result<()> {
        self.now_us += 2;
        self.phase = match self.phase {
            Phase::Running => bail!("ICSPCLK toggled while MCLR is high"),
            Phase::Key { bits, n } => {
                let bits = self.shift(bits, n, bit);
                if n + 1 < 32 {
                    Phase::Key { bits, n: n + 1 }
                } else if self.six() {
                    ensure!(bits == KEY, "bad LVP key {bits:#010x} (6-bit, LSb first)");
                    Phase::KeyTail
                } else {
                    // The last key bit is a don't care.
                    ensure!(bits >> 1 == KEY >> 1, "bad LVP key {bits:#010x} (8-bit, MSb first)");
                    self.pc = 0;
                    Phase::Command { acc: 0, n: 0 }
                }
            }
            Phase::KeyTail => {
                self.pc = 0;
                Phase::Command { acc: 0, n: 0 }
            }
            Phase::Command { acc, n } => {
                let acc = self.shift(acc, n, bit);
                if n + 1 < self.cmd_bits() {
                    Phase::Command { acc, n: n + 1 }
                } else {
                    self.dispatch(acc as u8)?
                }
            }
            Phase::DataIn { cmd, acc, n } => {
                let acc = self.shift(acc, n, bit);
                if n + 1 < self.data_bits() {
                    Phase::DataIn { cmd, acc, n: n + 1 }
                } else {
                    let data = if self.six() {
                        ensure!(acc & 0x8001 == 0, "start/stop bit set in {acc:#06x}");
                        (acc >> 1) & 0x3FFF
                    } else {
                        ensure!(acc & 1 == 0, "stop bit set in {acc:#08x}");
                        ensure!(acc >> 17 == 0, "start/pad bits set in {acc:#08x}");
                        acc >> 1
                    };
                    self.execute(cmd, data);
                    Phase::Command { acc: 0, n: 0 }
                }
            }
            Phase::DataOut { .. } => bail!("host drove ICSPDAT during a read"),
        };
        Ok(())
    }

    pub(crate) fn clock_out(&mut self) -> Result<bool> {
        self.now_us += 2;
        let Phase::DataOut { frame, n, then_increment } = self.phase else {
            bail!("host clocked a read outside a read command ({:?})", self.phase);
        };
        let total = self.data_bits();
        let pos = if self.six() { n } else { total - 1 - n };
        let bit = frame >> pos & 1 != 0;
        self.phase = if n + 1 < total {
            Phase::DataOut { frame, n: n + 1, then_increment }
        } else {
            if then_increment {
                self.pc += 1;
            }
            Phase::Command { acc: 0, n: 0 }
        };
        Ok(bit)
    }

    fn dispatch(&mut self, cmd: u8) -> Result<Phase> {
        ensure!(
            self.now_us >= self.busy_until_us,
            "command {cmd:#04x} at {} us while busy until {} us (pc {:#06x})",
            self.now_us,
            self.busy_until_us,
            self.pc
        );
        let read = |pic: &Self, then_increment| Phase::DataOut {
            frame: (pic.word(pic.pc) as u32) << 1,
            n: 0,
            then_increment,
        };
        let idle = Phase::Command { acc: 0, n: 0 };
        Ok(match (self.six(), cmd) {
            (true, 0x00 | 0x02 | 0x03) | (false, 0x80 | 0x00 | 0x02) => {
                Phase::DataIn { cmd, acc: 0, n: 0 }
            }
            (true, 0x05) => {
                let byte = self.eeprom_index().map_or(0, |i| self.eeprom[i]);
                Phase::DataOut { frame: (byte as u32) << 1, n: 0, then_increment: false }
            }
            (true, 0x11) | (false, 0xF0) => {
                self.row_erase()?;
                idle
            }
            (true, 0x04) | (false, 0xFC) => read(self, false),
            (false, 0xFE) => read(self, true),
            (true, 0x06) | (false, 0xF8) => {
                self.pc += 1;
                idle
            }
            (true, 0x16) => {
                self.pc = 0;
                idle
            }
            (true, 0x08) | (false, 0xE0) => {
                self.program();
                idle
            }
            (true, 0x09) | (false, 0x18) => {
                self.bulk_erase()?;
                idle
            }
            (true, 0x0B) => {
                self.eeprom.fill(0xFF);
                self.busy_until_us = self.now_us + self.t_erase;
                idle
            }
            _ => bail!("unexpected command {cmd:#04x}"),
        })
    }

    fn execute(&mut self, cmd: u8, data: u32) {
        let latch = self.pc as usize % self.row;
        match (self.six(), cmd) {
            (true, 0x00) => {
                self.pc = 0x8000;
                self.latches[0] = data as u16;
            }
            (false, 0x80) => self.pc = data,
            (true, 0x03) => self.eeprom_latch = Some(data as u8),
            (false, 0x00 | 0x02) if self.eeprom_index().is_some() => {
                self.eeprom_latch = Some(data as u8);
                if cmd == 0x02 {
                    self.pc += 1;
                }
            }
            (_, 0x00) | (true, 0x02) => self.latches[latch] = data as u16,
            (false, 0x02) => {
                self.latches[latch] = data as u16;
                self.pc += 1;
            }
            _ => unreachable!(),
        }
    }

    fn program(&mut self) {
        if let Some(byte) = self.eeprom_latch.take() {
            // EEPROM writes erase the byte first: the value is replaced.
            if let Some(i) = self.eeprom_index() {
                self.eeprom[i] = byte;
            }
            self.busy_until_us = self.now_us + self.t_eeprom;
            return;
        }
        if self.pc >= 0x8000 {
            let index = (self.pc - 0x8000) as usize;
            if index < 16 && self.writable_config(index) {
                self.config[index] &= self.latches[self.pc as usize % self.row];
            }
            self.busy_until_us = self.now_us + self.t_config;
        } else {
            let base = self.pc as usize & !(self.row - 1);
            for (i, latch) in self.latches.iter().enumerate() {
                if let Some(word) = self.flash.get_mut(base + i) {
                    *word &= latch;
                }
            }
            self.busy_until_us = self.now_us + self.t_row;
        }
        self.latches.fill(0x3FFF);
    }

    fn row_erase(&mut self) -> Result<()> {
        ensure!(self.t_row_erase != 0, "row erase on a part without the command");
        if (0x8000..0x8004).contains(&self.pc) {
            ensure!(!self.six(), "6-bit row erase at the user IDs");
            self.config[..4].fill(0x3FFF);
        } else {
            ensure!(self.pc < self.flash.len() as u32, "row erase at pc {:#06x}", self.pc);
            let base = self.pc as usize & !(self.row - 1);
            self.flash[base..base + self.row].fill(0x3FFF);
        }
        self.busy_until_us = self.now_us + self.t_row_erase;
        Ok(())
    }

    fn bulk_erase(&mut self) -> Result<()> {
        if !self.six() {
            ensure!(
                self.pc < 0x80FE || self.pc >= 0xE800,
                "bulk erase at pc {:#06x} is a no-op",
                self.pc
            );
            if self.pc >= 0xE800 {
                self.eeprom.fill(0xFF);
            }
        }
        let with_ids = self.pc >= 0x8000;
        self.flash.fill(0x3FFF);
        for i in 0..16 {
            if (7..7 + self.config_words).contains(&i) || (with_ids && i < 4) {
                self.config[i] = 0x3FFF;
            }
        }
        self.busy_until_us = self.now_us + self.t_erase;
        Ok(())
    }
}

impl Transport for SimPic {
    fn run(&mut self, script: &[u8]) -> Result<Vec<u32>> {
        ensure!(script.len() <= MAX_PAYLOAD, "frame too large");
        self.frames += 1;
        let mut reads = Vec::new();
        let mut i = 0;
        while i < script.len() {
            let code = script[i];
            let len = op_len(code).expect("unknown op");
            let arg = &script[i + 1..i + len];
            match code {
                op::PING => reads.push(PROTOCOL_VERSION as u32),
                op::ENTER => self.phase = Phase::Key { bits: 0, n: 0 },
                op::ENTER_HV => {
                    self.pc = 0;
                    self.phase = Phase::Command { acc: 0, n: 0 };
                }
                op::RELEASE => self.phase = Phase::Running,
                op::OUT => {
                    let (count, msb) = decode_shape(arg[0]).expect("bad shape");
                    let bits = u32::from_le_bytes([arg[1], arg[2], arg[3], arg[4]]);
                    for b in 0..count {
                        self.clock_in(wire_bit(bits, count, msb, b))?;
                    }
                    self.now_us += GROUP_GAP_US as u64;
                }
                op::IN => {
                    let (count, msb) = decode_shape(arg[0]).expect("bad shape");
                    let mut value = 0;
                    for b in 0..count {
                        value = store_bit(value, count, msb, b, self.clock_out()?);
                    }
                    reads.push(value);
                    self.now_us += GROUP_GAP_US as u64;
                }
                op::DELAY_US => self.now_us += u16::from_le_bytes([arg[0], arg[1]]) as u64,
                _ => unreachable!(),
            }
            i += len;
        }
        ensure!(reads.len() * 4 < MAX_PAYLOAD, "reply too large");
        Ok(reads)
    }
}

/// Executes the Gen4 scripts [`crate::pickit`] sends to a PICkit, on a
/// [`SimPic`]: MCLR follows VPP on/off and the selected VPP level, writes
/// and reads go through the same clock-edge model.
pub struct Gen4Sim {
    pub pic: SimPic,
    /// VPP level selected: true for VPP (high voltage), false for VDD.
    pub vpp_program: bool,
    /// Bytes of every script run, for size checks.
    pub sizes: Vec<usize>,
}

impl Gen4Sim {
    pub fn new(pic: SimPic) -> Self {
        Self { pic, vpp_program: false, sizes: Vec::new() }
    }
}

impl crate::pickit::Gen4 for Gen4Sim {
    fn exec(&mut self, script: &[u8], upload: usize) -> Result<Vec<u8>> {
        use crate::pickit::g4;
        self.sizes.push(script.len());
        let mut out = Vec::new();
        let mut i = 0;
        let arg = |i: usize, n: usize| -> Result<&[u8]> {
            script.get(i..i + n).ok_or_else(|| anyhow::anyhow!("truncated Gen4 script"))
        };
        while i < script.len() {
            let code = script[i];
            i += 1;
            match code {
                g4::VPP_LEVEL_PROGRAM => self.vpp_program = true,
                g4::VPP_LEVEL_OPERATIONAL => self.vpp_program = false,
                g4::VPP_ON => {
                    if self.vpp_program {
                        self.pic.pc = 0;
                        self.pic.phase = Phase::Command { acc: 0, n: 0 };
                    } else {
                        self.pic.phase = Phase::Running;
                    }
                }
                // MCLR low: reset, the part listens for the key.
                g4::VPP_OFF => self.pic.phase = Phase::Key { bits: 0, n: 0 },
                g4::SET_ICSP_PINS => i += 1,
                g4::SET_UPLOAD => {}
                g4::DELAY_US | g4::DELAY_MS => {
                    let a = arg(i, 2)?;
                    let n = u16::from_le_bytes([a[0], a[1]]) as u64;
                    self.pic.now_us += if code == g4::DELAY_MS { n * 1000 } else { n };
                    i += 2;
                }
                g4::WRITE_BITS_LSB | g4::WRITE_BITS_MSB => {
                    let a = arg(i, 5)?;
                    let count = a[0] as u32;
                    ensure!((1..=32).contains(&count), "bad bit count {count}");
                    let bits = u32::from_le_bytes([a[1], a[2], a[3], a[4]]);
                    for b in 0..count {
                        self.pic.clock_in(wire_bit(bits, count, code == g4::WRITE_BITS_MSB, b))?;
                    }
                    i += 5;
                }
                g4::P16F_READ_WORD | g4::P16ENV3_READ_WORD => {
                    let (count, msb) = if code == g4::P16F_READ_WORD { (16, false) } else { (24, true) };
                    ensure!(
                        (code == g4::P16F_READ_WORD) == self.pic.six(),
                        "read opcode {code:#04x} does not match the part's family"
                    );
                    let mut raw = 0;
                    for b in 0..count {
                        raw = store_bit(raw, count, msb, b, self.pic.clock_out()?);
                    }
                    let word = ((raw >> 1) & 0x3FFF) as u16;
                    out.extend_from_slice(&word.to_le_bytes());
                }
                _ => bail!("Gen4 opcode {code:#04x} not modelled"),
            }
        }
        ensure!(out.len() == upload, "script uploaded {} bytes, host expects {upload}", out.len());
        Ok(out)
    }
}
