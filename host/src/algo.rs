//! ICSP command layer for each supported programming-spec family, on top of
//! the bit-level [`Batch`]. A [`Session`] tracks the target's PC so callers
//! can just say "go to address X".
//!
//! Sources: PIC16(L)F178X Memory Programming Specification (6-bit commands)
//! and PIC16(L)F184XX Memory Programming Specification DS40001970 (8-bit
//! commands), cross-checked against the PICkit 4 script sequences.

use anyhow::Result;

use crate::device::{Algorithm, Device, CONFIG_SPACE};
use crate::transport::{Batch, Transport};

/// "MCHP". 6-bit parts take it LSb first plus one extra clock; 8-bit parts
/// take it MSb first.
pub const LVP_KEY: u32 = 0x4D43_4850;

/// MCLR low to first key bit, and key to first command (TENTH >= 250 µs).
const T_ENTRY_US: u32 = 1000;

/// 6-bit command set (PIC12/16F1xxx, e.g. PIC16F1789).
pub mod six {
    pub const LOAD_CONFIG: u8 = 0x00;
    pub const LOAD_PROGRAM: u8 = 0x02;
    pub const READ_PROGRAM: u8 = 0x04;
    pub const INCREMENT_ADDRESS: u8 = 0x06;
    pub const BEGIN_INTERNAL_PROGRAMMING: u8 = 0x08;
    pub const BULK_ERASE_PROGRAM: u8 = 0x09;
    pub const BULK_ERASE_DATA: u8 = 0x0B;
    pub const RESET_ADDRESS: u8 = 0x16;
    pub const LOAD_DATA_MEMORY: u8 = 0x03;
    pub const READ_DATA_MEMORY: u8 = 0x05;
    pub const ROW_ERASE_PROGRAM: u8 = 0x11;
}

/// 8-bit command set (PIC16F18xxx / PIC16F1xxxx, e.g. PIC16F18446).
pub mod eight {
    pub const LOAD_PC: u8 = 0x80;
    pub const BULK_ERASE: u8 = 0x18;
    pub const LOAD_DATA: u8 = 0x00;
    pub const LOAD_DATA_INC: u8 = 0x02;
    pub const READ_DATA: u8 = 0xFC;
    pub const READ_DATA_INC: u8 = 0xFE;
    pub const INCREMENT_ADDRESS: u8 = 0xF8;
    pub const BEGIN_INTERNAL_PROGRAMMING: u8 = 0xE0;
    pub const ROW_ERASE: u8 = 0xF0;
}

/// Start of data EEPROM in the 8-bit family's address space; a bulk erase
/// with the PC at or above it also clears the EEPROM.
pub const EEPROM_SPACE: u32 = 0xF000;

/// Data bits of a word read back: both families frame data between a start
/// bit (bit 0 after shifting) and stop bit.
pub fn decode_word(raw: u32) -> u16 {
    ((raw >> 1) & 0x3FFF) as u16
}

/// How the target is put into Program/Verify mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Entry {
    /// MCLR low, then the "MCHP" key. Needs the LVP fuse set.
    #[default]
    LowVoltage,
    /// MCLR raised to VPP (8.5-9 V), no key. Works whatever the LVP fuse.
    HighVoltage,
}

pub struct Session<'a, T: Transport + ?Sized> {
    b: Batch<'a, T>,
    algo: Algorithm,
    entry: Entry,
    pc: u32,
}

impl<'a, T: Transport + ?Sized> Session<'a, T> {
    pub fn new(link: &'a mut T, algo: Algorithm, entry: Entry) -> Self {
        Self { b: Batch::new(link), algo, entry, pc: 0 }
    }

    /// Enters Program/Verify mode. PC is 0 after.
    pub fn enter(&mut self) -> Result<()> {
        self.pc = 0;
        if self.entry == Entry::HighVoltage {
            self.b.enter_hv()?;
            return self.b.delay_us(T_ENTRY_US);
        }
        self.b.enter()?;
        self.b.delay_us(T_ENTRY_US)?;
        match self.algo {
            Algorithm::Midrange6Bit => {
                self.b.out(LVP_KEY, 32, false)?;
                self.b.out(0, 1, false)?;
            }
            Algorithm::Midrange8Bit => self.b.out(LVP_KEY, 32, true)?,
        }
        self.b.delay_us(T_ENTRY_US)
    }

    /// Leaves Program/Verify mode and lets the target run.
    pub fn exit(&mut self) -> Result<()> {
        self.b.release()
    }

    pub fn finish(self) -> Result<Vec<u32>> {
        self.b.finish()
    }

    fn cmd(&mut self, cmd: u8) -> Result<()> {
        match self.algo {
            Algorithm::Midrange6Bit => self.b.out(cmd as u32, 6, false),
            Algorithm::Midrange8Bit => self.b.out(cmd as u32, 8, true),
        }
    }

    fn cmd_data(&mut self, cmd: u8, data: u32) -> Result<()> {
        self.cmd(cmd)?;
        match self.algo {
            // Start bit, 14 data bits, stop bit; LSb first.
            Algorithm::Midrange6Bit => self.b.out((data & 0x3FFF) << 1, 16, false),
            // Start bit, pad bits, data, stop bit; 24 bits MSb first.
            Algorithm::Midrange8Bit => self.b.out((data & 0x7F_FFFF) << 1, 24, true),
        }
    }

    fn cmd_read(&mut self, cmd: u8) -> Result<()> {
        self.cmd(cmd)?;
        match self.algo {
            Algorithm::Midrange6Bit => self.b.input(16, false),
            Algorithm::Midrange8Bit => self.b.input(24, true),
        }
    }

    fn increment(&mut self) -> Result<()> {
        let cmd = match self.algo {
            Algorithm::Midrange6Bit => six::INCREMENT_ADDRESS,
            Algorithm::Midrange8Bit => eight::INCREMENT_ADDRESS,
        };
        self.cmd(cmd)?;
        self.pc += 1;
        Ok(())
    }

    /// Moves the target PC to `addr`.
    pub fn goto(&mut self, addr: u32) -> Result<()> {
        match self.algo {
            Algorithm::Midrange6Bit => {
                // No direct PC load: jump to 0x0000 or 0x8000, then increment.
                if addr >= CONFIG_SPACE {
                    if self.pc < CONFIG_SPACE || addr < self.pc {
                        self.cmd_data(six::LOAD_CONFIG, 0x3FFF)?;
                        self.pc = CONFIG_SPACE;
                    }
                } else if self.pc >= CONFIG_SPACE || addr < self.pc {
                    self.cmd(six::RESET_ADDRESS)?;
                    self.pc = 0;
                }
                while self.pc < addr {
                    self.increment()?;
                }
            }
            Algorithm::Midrange8Bit => {
                if self.pc != addr {
                    self.cmd_data(eight::LOAD_PC, addr)?;
                    self.pc = addr;
                }
            }
        }
        Ok(())
    }

    /// Loads one write latch at the current PC, optionally moving to the next.
    pub fn load(&mut self, word: u16, then_increment: bool) -> Result<()> {
        match self.algo {
            Algorithm::Midrange6Bit => {
                self.cmd_data(six::LOAD_PROGRAM, word as u32)?;
                if then_increment {
                    self.increment()?;
                }
            }
            Algorithm::Midrange8Bit => {
                let cmd = if then_increment {
                    eight::LOAD_DATA_INC
                } else {
                    eight::LOAD_DATA
                };
                self.cmd_data(cmd, word as u32)?;
                if then_increment {
                    self.pc += 1;
                }
            }
        }
        Ok(())
    }

    /// Reads the word at the current PC into the reply, optionally moving on.
    pub fn read(&mut self, then_increment: bool) -> Result<()> {
        match self.algo {
            Algorithm::Midrange6Bit => {
                self.cmd_read(six::READ_PROGRAM)?;
                if then_increment {
                    self.increment()?;
                }
            }
            Algorithm::Midrange8Bit => {
                let cmd = if then_increment {
                    eight::READ_DATA_INC
                } else {
                    eight::READ_DATA
                };
                self.cmd_read(cmd)?;
                if then_increment {
                    self.pc += 1;
                }
            }
        }
        Ok(())
    }

    /// Commits the loaded latches (row or single word) at the current PC.
    pub fn program(&mut self, wait_us: u32) -> Result<()> {
        let cmd = match self.algo {
            Algorithm::Midrange6Bit => six::BEGIN_INTERNAL_PROGRAMMING,
            Algorithm::Midrange8Bit => eight::BEGIN_INTERNAL_PROGRAMMING,
        };
        self.cmd(cmd)?;
        self.b.delay_us(wait_us)
    }

    /// Erases program memory, user IDs and config words, and data EEPROM
    /// unless `keep_eeprom`.
    pub fn bulk_erase(&mut self, device: &Device, keep_eeprom: bool) -> Result<()> {
        let wait = device.t_erase_us as u32;
        let eeprom = device.eeprom.is_some() && !keep_eeprom;
        match self.algo {
            Algorithm::Midrange6Bit => {
                // PC in config space makes the erase include user IDs and config.
                self.goto(CONFIG_SPACE)?;
                self.cmd(six::BULK_ERASE_PROGRAM)?;
                self.b.delay_us(wait)?;
                if eeprom {
                    self.cmd(six::BULK_ERASE_DATA)?;
                    self.b.delay_us(wait)?;
                }
            }
            Algorithm::Midrange8Bit => {
                // PC 0x8000..0x80FD: program memory, config words and user
                // IDs; PC in EEPROM space: the same plus the EEPROM.
                self.goto(if eeprom { EEPROM_SPACE } else { CONFIG_SPACE })?;
                self.cmd(eight::BULK_ERASE)?;
                self.b.delay_us(wait)?;
            }
        }
        Ok(())
    }

    /// Erases the program memory row (or, at 0x8000 on 8-bit parts, the
    /// user IDs) holding the current PC.
    pub fn row_erase(&mut self, wait_us: u32) -> Result<()> {
        self.cmd(match self.algo {
            Algorithm::Midrange6Bit => six::ROW_ERASE_PROGRAM,
            Algorithm::Midrange8Bit => eight::ROW_ERASE,
        })?;
        self.b.delay_us(wait_us)
    }

    /// Moves to data EEPROM byte `offset`. 6-bit parts address the EEPROM
    /// with the low bits of the program PC; 8-bit parts map it at 0xF000.
    pub fn goto_eeprom(&mut self, offset: u32) -> Result<()> {
        match self.algo {
            Algorithm::Midrange6Bit => self.goto(offset),
            Algorithm::Midrange8Bit => self.goto(EEPROM_SPACE + offset),
        }
    }

    /// Loads one EEPROM byte at the current address; commit with
    /// [`Self::program`].
    pub fn load_eeprom(&mut self, byte: u8) -> Result<()> {
        match self.algo {
            Algorithm::Midrange6Bit => self.cmd_data(six::LOAD_DATA_MEMORY, byte as u32),
            Algorithm::Midrange8Bit => self.cmd_data(eight::LOAD_DATA, byte as u32),
        }
    }

    /// Reads the EEPROM byte at the current address into the reply.
    pub fn read_eeprom(&mut self, then_increment: bool) -> Result<()> {
        match self.algo {
            Algorithm::Midrange6Bit => {
                self.cmd_read(six::READ_DATA_MEMORY)?;
                if then_increment {
                    self.increment()?;
                }
            }
            Algorithm::Midrange8Bit => self.read(then_increment)?,
        }
        Ok(())
    }
}
