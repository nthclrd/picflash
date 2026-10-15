//! Cycle-counting instruction-set simulator for the PIC16 enhanced mid-range
//! core (PIC12/16F1xxx and PIC16F18xxx), meant to drive a step-by-step
//! debugger.
//!
//! Semantics follow the instruction set chapters of the PIC16(L)F1789
//! (DS40001675) and PIC16F18446 datasheets:
//!
//! - 14-bit program words decoded by [`crate::disasm::decode`], 15-bit PC,
//!   16-level hardware stack (overflow/underflow stop the simulation, as
//!   with `STVREN = 1` the part would reset).
//! - Banked data memory: `bank * 0x80 + offset`, core registers
//!   (`INDF0`..`INTCON`, offsets 0x00-0x0B) and common RAM (0x70-0x7F)
//!   mirrored in every bank.
//! - FSR address space: traditional data memory below `Device::data_end`,
//!   linear GPR memory at 0x2000 (80 bytes per bank), program flash at
//!   0x8000 (low byte of each word, one extra cycle, read-only).
//! - One cycle per instruction, two for branches, calls, returns, PCL writes
//!   and taken skips.
//!
//! Peripherals are not modelled. Port writes go to the output latch (`LATx`)
//! and port reads return the latch for output pins and 0 for input pins.
//! Interrupts are never raised; `RETFIE` pops the stack, restores the shadow
//! registers and sets GIE as on silicon.

use std::collections::{BTreeSet, HashMap};

use anyhow::{bail, Result};

use crate::device::Device;
use crate::disasm::{decode, Insn};
use crate::image::Image;

/// Depth of the hardware return stack.
pub const STACK_DEPTH: usize = 16;

/// Start of linear GPR memory in the FSR address space.
const LINEAR_BASE: u16 = 0x2000;
/// Start of program flash in the FSR address space.
const FLASH_BASE: u16 = 0x8000;
/// GPR bytes per bank in linear memory (offsets 0x20-0x6F).
const GPR_PER_BANK: u16 = 80;
const PC_MASK: u16 = 0x7FFF;

// Core register offsets, the same in every bank.
const PCL: u16 = 0x02;
const STATUS: u16 = 0x03;
const FSR0L: u16 = 0x04;
const BSR: u16 = 0x08;
const WREG: u16 = 0x09;
const PCLATH: u16 = 0x0A;
const INTCON: u16 = 0x0B;
const CORE_END: u16 = 0x0C;
const COMMON_START: u16 = 0x70;

// STATUS bits.
const C: u8 = 0x01;
const DC: u8 = 0x02;
const Z: u8 = 0x04;
const NPD: u8 = 0x08;
const NTO: u8 = 0x10;
const GIE: u8 = 0x80;

/// Shadow registers restored by `RETFIE`, in core register order from STATUS.
const SHADOWS: [(&str, u16); 8] = [
    ("STATUS_SHAD", STATUS),
    ("WREG_SHAD", WREG),
    ("BSR_SHAD", BSR),
    ("PCLATH_SHAD", PCLATH),
    ("FSR0L_SHAD", FSR0L),
    ("FSR0H_SHAD", FSR0L + 1),
    ("FSR1L_SHAD", FSR0L + 2),
    ("FSR1H_SHAD", FSR0L + 3),
];

/// Why [`Cpu::step`] or [`Cpu::run`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// One instruction executed normally.
    Step,
    /// `run` reached a breakpoint; the instruction there is not executed.
    Breakpoint(u16),
    /// `run` executed its maximum number of steps.
    Limit,
    /// A `SLEEP` instruction executed. Stepping again resumes after it, as
    /// if a wake-up event had occurred.
    Sleep,
    /// The word at PC is not an instruction; nothing executed.
    Invalid(u16),
    /// A `CALL`/`CALLW` with a full stack; nothing executed.
    StackOverflow,
    /// A return with an empty stack; nothing executed.
    StackUnderflow,
}

/// What a data address holds, beyond plain RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Unimplemented,
    Ram,
    /// `PORTx`: writes go to `lat`, reads give output latches of pins whose
    /// `tris` bit is 0.
    Port { lat: Option<u16>, tris: Option<u16> },
    Stkptr,
    Tosl,
    Tosh,
}

/// Where a data access lands once mirroring and indirection are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// `INDFn` addressed directly: goes through FSRn.
    Indf(u16),
    Pcl,
    Wreg,
    /// Index into `Cpu::data`.
    Data(u16),
    /// Program flash word address.
    Flash(u16),
    /// Unimplemented: reads 0, writes ignored.
    Nothing,
}

/// Processor state of one enhanced mid-range part.
#[derive(Debug, Clone)]
pub struct Cpu {
    /// Address of the next instruction (15 bits).
    pub pc: u16,
    pub w: u8,
    /// Return addresses, innermost last. At most [`STACK_DEPTH`] entries.
    pub stack: Vec<u16>,
    /// Instruction cycles (Fosc/4) since the last [`Cpu::reset`].
    pub cycles: u64,
    /// Banked data memory, indexed by absolute banked address. Core
    /// registers and common RAM live in bank 0.
    data: Vec<u8>,
    kind: Vec<Kind>,
    flash: Vec<u16>,
    wide_bsr: bool,
    /// SFR addresses by name.
    names: HashMap<String, u16>,
}

impl Cpu {
    /// Loads `image` into a part and applies the power-on reset state.
    /// General purpose RAM starts cleared (it is undefined on silicon).
    pub fn new(device: &Device, image: &Image) -> Result<Self> {
        if !device.arch.starts_with("16E") {
            bail!("{}: architecture {} is not PIC16 enhanced mid-range", device.name, device.arch);
        }
        if !(0x80..=0x2000).contains(&device.data_end) || !device.data_end.is_multiple_of(0x80) {
            bail!("{}: unexpected data memory size {:#x}", device.name, device.data_end);
        }
        if device.flash_words == 0 || device.flash_words > 0x8000 {
            bail!("{}: unexpected flash size {} words", device.name, device.flash_words);
        }
        let data_end = device.data_end as u16;
        let mut kind = vec![Kind::Unimplemented; data_end as usize];
        let mut mark = |from: u32, to: u32| {
            for addr in from..to.min(data_end as u32) {
                kind[addr as usize] = Kind::Ram;
            }
        };
        for &(from, to) in &device.gpr {
            mark(from, to);
        }
        mark(0, CORE_END as u32);
        mark(COMMON_START as u32, 0x80);
        let mut names = HashMap::new();
        for (&addr, name) in &device.sfrs {
            if addr < data_end as u32 {
                mark(addr, addr + 1);
                names.insert(name.clone(), addr as u16);
            }
        }
        for (name, &addr) in &names {
            let special = match name.as_str() {
                "STKPTR" => Kind::Stkptr,
                "TOSL" => Kind::Tosl,
                "TOSH" => Kind::Tosh,
                _ => match name.strip_prefix("PORT") {
                    Some(x) if x.len() == 1 => Kind::Port {
                        lat: names.get(&format!("LAT{x}")).copied(),
                        tris: names.get(&format!("TRIS{x}")).copied(),
                    },
                    _ => continue,
                },
            };
            kind[addr as usize] = special;
        }

        let mut flash = vec![0x3FFF; device.flash_words as usize];
        for (&addr, &word) in &image.flash {
            match flash.get_mut(addr as usize) {
                Some(slot) => *slot = word & 0x3FFF,
                None => bail!("word address {addr:#06x} is outside {} program memory", device.name),
            }
        }

        let mut cpu = Cpu {
            pc: 0,
            w: 0,
            stack: Vec::with_capacity(STACK_DEPTH),
            cycles: 0,
            data: vec![0; data_end as usize],
            kind,
            flash,
            wide_bsr: device.data_end > 0x1000,
            names,
        };
        cpu.reset();
        Ok(cpu)
    }

    /// Resets like MCLR: PC, stack, cycle count and every SFR go back to
    /// their reset values (`STATUS = 0x18`, `TRISx`/`ANSELx = 0xFF`, others
    /// 0). W and general purpose RAM keep their contents.
    pub fn reset(&mut self) {
        self.pc = 0;
        self.stack.clear();
        self.cycles = 0;
        for (name, &addr) in &self.names {
            let value = if name.starts_with("TRIS") || name.starts_with("ANSEL") { 0xFF } else { 0 };
            self.data[addr as usize] = value;
        }
        for addr in 0..CORE_END {
            self.data[addr as usize] = 0;
        }
        self.data[STATUS as usize] = NTO | NPD;
    }

    /// True for 64-bank parts, whose `MOVLB` takes 6 bits.
    pub fn wide_bsr(&self) -> bool {
        self.wide_bsr
    }

    /// Program word at `addr`; addresses past the end of flash wrap around.
    pub fn flash_word(&self, addr: u16) -> u16 {
        self.flash[addr as usize % self.flash.len()]
    }

    /// The instruction at `addr`, as [`Cpu::step`] would decode it.
    pub fn insn_at(&self, addr: u16) -> Insn {
        decode(self.flash_word(addr), self.wide_bsr)
    }

    pub fn bsr(&self) -> u8 {
        self.data[BSR as usize]
    }

    pub fn status(&self) -> u8 {
        self.data[STATUS as usize]
    }

    pub fn pclath(&self) -> u8 {
        self.data[PCLATH as usize]
    }

    /// FSR0 (`n = 0`) or FSR1 (`n = 1`) as a 16-bit address.
    pub fn fsr(&self, n: usize) -> u16 {
        let low = (FSR0L + 2 * (n as u16 & 1)) as usize;
        u16::from_le_bytes([self.data[low], self.data[low + 1]])
    }

    fn set_fsr(&mut self, n: usize, value: u16) {
        let low = (FSR0L + 2 * (n as u16 & 1)) as usize;
        let [lo, hi] = value.to_le_bytes();
        self.data[low] = lo;
        self.data[low + 1] = hi;
    }

    /// Reads an address of the FSR space: banked data memory (`bank * 0x80 +
    /// offset`, core registers and common RAM mirrored), linear memory at
    /// 0x2000 or program flash at 0x8000. `INDFn` reads go through FSRn and
    /// `PORTx` reads give the pin levels. Unimplemented locations read 0.
    pub fn read(&self, addr: u16) -> u8 {
        self.load(self.resolve(addr))
    }

    /// Writes an address of the FSR space, with the same mapping as
    /// [`Cpu::read`]. Writing PCL jumps to `PCLATH:value`; `PORTx` writes go
    /// to `LATx`; flash and unimplemented locations ignore writes.
    pub fn write(&mut self, addr: u16, value: u8) {
        let target = self.resolve(addr);
        self.store(target, value);
    }

    /// Value of the SFR called `name` (e.g. `"LATA"`), as [`Cpu::read`]
    /// returns it.
    pub fn sfr_value(&self, name: &str) -> Option<u8> {
        self.names.get(name).map(|&addr| self.read(addr))
    }

    /// Output latch (`LATx`, or `PORTx` without a latch register) and `TRISx`
    /// of port `port` (`'A'`, `'B'`...). A pin drives its latch bit when its
    /// TRIS bit is 0.
    pub fn pins(&self, port: char) -> Option<(u8, u8)> {
        let x = port.to_ascii_uppercase();
        let latch = self.names.get(&format!("LAT{x}")).or_else(|| self.names.get(&format!("PORT{x}")))?;
        let tris = self.names.get(&format!("TRIS{x}")).map_or(0, |&a| self.data[a as usize]);
        Some((self.data[*latch as usize], tris))
    }

    /// Maps an FSR-space address to what it holds. `INDFn` stays symbolic.
    fn target(&self, addr: u16) -> Target {
        let data_end = self.data.len() as u16;
        if addr >= FLASH_BASE {
            return Target::Flash(addr - FLASH_BASE);
        }
        let banked = if addr < data_end {
            addr
        } else if addr >= LINEAR_BASE {
            let n = addr - LINEAR_BASE;
            let banked = (n / GPR_PER_BANK) * 0x80 + 0x20 + n % GPR_PER_BANK;
            if banked >= data_end {
                return Target::Nothing;
            }
            banked
        } else {
            return Target::Nothing;
        };
        let offset = banked & 0x7F;
        match offset {
            0x00 | 0x01 => Target::Indf(offset),
            PCL => Target::Pcl,
            WREG => Target::Wreg,
            _ if !(CORE_END..COMMON_START).contains(&offset) => Target::Data(offset),
            _ if self.kind[banked as usize] == Kind::Unimplemented => Target::Nothing,
            _ => Target::Data(banked),
        }
    }

    /// Like [`Cpu::target`], following `INDFn` through FSRn. Indirectly
    /// addressing INDF itself reads 0 and writes nothing.
    fn resolve(&self, addr: u16) -> Target {
        match self.target(addr) {
            Target::Indf(n) => match self.target(self.fsr(n as usize)) {
                Target::Indf(_) => Target::Nothing,
                t => t,
            },
            t => t,
        }
    }

    /// Resolves an indirect (FSR) address.
    fn resolve_indirect(&self, addr: u16) -> Target {
        match self.target(addr) {
            Target::Indf(_) => Target::Nothing,
            t => t,
        }
    }

    fn load(&self, target: Target) -> u8 {
        match target {
            Target::Indf(_) | Target::Nothing => 0,
            Target::Pcl => self.pc as u8,
            Target::Wreg => self.w,
            Target::Flash(addr) => self.flash_word(addr) as u8,
            Target::Data(i) => match self.kind[i as usize] {
                Kind::Port { lat, tris } => {
                    let latch = self.data[lat.unwrap_or(i) as usize];
                    let tris = tris.map_or(0, |t| self.data[t as usize]);
                    latch & !tris
                }
                Kind::Stkptr => (self.stack.len() as u8).wrapping_sub(1) & 0x1F,
                Kind::Tosl => self.stack.last().map_or(0, |&a| a as u8),
                Kind::Tosh => self.stack.last().map_or(0, |&a| (a >> 8) as u8),
                Kind::Ram | Kind::Unimplemented => self.data[i as usize],
            },
        }
    }

    /// Stores `value`; returns true if this wrote PCL (a jump).
    fn store(&mut self, target: Target, value: u8) -> bool {
        match target {
            Target::Indf(_) | Target::Nothing | Target::Flash(_) => {}
            Target::Pcl => {
                self.pc = (u16::from(self.pclath()) << 8 | u16::from(value)) & PC_MASK;
                return true;
            }
            Target::Wreg => self.w = value,
            Target::Data(i) => {
                let slot = match self.kind[i as usize] {
                    Kind::Port { lat, .. } => lat.unwrap_or(i),
                    // The stack registers are read-only in this model.
                    Kind::Stkptr | Kind::Tosl | Kind::Tosh => return false,
                    Kind::Ram | Kind::Unimplemented => i,
                };
                let value = match slot {
                    // nTO and nPD are read-only.
                    STATUS => self.status() & (NTO | NPD) | value & (C | DC | Z),
                    BSR => value & if self.wide_bsr { 0x3F } else { 0x1F },
                    PCLATH => value & 0x7F,
                    _ => value,
                };
                self.data[slot as usize] = value;
            }
        }
        false
    }

    fn set_status(&mut self, mask: u8, bits: u8) {
        let status = &mut self.data[STATUS as usize];
        *status = *status & !mask | bits & mask;
    }

    fn carry(&self) -> u8 {
        self.status() & C
    }

    fn push(&mut self, addr: u16) {
        self.stack.push(addr & PC_MASK);
    }

    fn pop(&mut self) -> u16 {
        self.stack.pop().unwrap_or(0)
    }

    /// Executes the instruction at PC.
    pub fn step(&mut self) -> Stop {
        let word = self.flash_word(self.pc);
        let insn = decode(word, self.wide_bsr);
        let pushes = matches!(insn, Insn::Jump { op: "call", .. } | Insn::Plain("callw"));
        let pops = matches!(insn, Insn::Plain("return" | "retfie") | Insn::Literal { op: "retlw", .. });
        if pushes && self.stack.len() >= STACK_DEPTH {
            return Stop::StackOverflow;
        }
        if pops && self.stack.is_empty() {
            return Stop::StackUnderflow;
        }
        if let Insn::Invalid(w) = insn {
            return Stop::Invalid(w);
        }

        let here = self.pc;
        let next = (here + 1) & PC_MASK;
        self.pc = next;
        let mut cycles = 1;
        let mut stop = Stop::Step;
        match insn {
            Insn::Plain(op) => match op {
                "nop" => {}
                "reset" => {
                    let total = self.cycles;
                    self.reset();
                    self.cycles = total;
                }
                "return" => {
                    self.pc = self.pop();
                    cycles = 2;
                }
                "retfie" => {
                    self.pc = self.pop();
                    self.restore_shadows();
                    self.data[INTCON as usize] |= GIE;
                    cycles = 2;
                }
                "callw" => {
                    self.push(next);
                    self.pc = (u16::from(self.pclath()) << 8 | u16::from(self.w)) & PC_MASK;
                    cycles = 2;
                }
                "brw" => {
                    self.pc = next.wrapping_add(u16::from(self.w)) & PC_MASK;
                    cycles = 2;
                }
                "clrw" => {
                    self.w = 0;
                    self.set_status(Z, Z);
                }
                "option" => self.write_named("OPTION_REG"),
                "sleep" => {
                    self.set_status(NTO | NPD, NTO);
                    stop = Stop::Sleep;
                }
                "clrwdt" => self.set_status(NTO | NPD, NTO | NPD),
                _ => {
                    self.pc = here;
                    return Stop::Invalid(word);
                }
            },
            Insn::File { op, f, to_f } => {
                let target = self.file_target(f, &mut cycles);
                let v = self.load(target);
                let w = self.w;
                let c = self.carry();
                let zero = |r: u8| if r == 0 { Z } else { 0 };
                let arith = |(r, flags): (u8, u8)| (r, C | DC | Z, flags, false);
                // Result, STATUS bits affected, their new values, skip.
                let (result, mask, flags, skip) = match op {
                    "addwf" => arith(add(v, w, 0)),
                    "addwfc" => arith(add(v, w, c)),
                    "subwf" => arith(add(v, !w, 1)),
                    "subwfb" => arith(add(v, !w, c)),
                    "andwf" => (v & w, Z, zero(v & w), false),
                    "iorwf" => (v | w, Z, zero(v | w), false),
                    "xorwf" => (v ^ w, Z, zero(v ^ w), false),
                    "comf" => (!v, Z, zero(!v), false),
                    "decf" => (v.wrapping_sub(1), Z, zero(v.wrapping_sub(1)), false),
                    "incf" => (v.wrapping_add(1), Z, zero(v.wrapping_add(1)), false),
                    "movf" => (v, Z, zero(v), false),
                    "decfsz" => (v.wrapping_sub(1), 0, 0, v == 1),
                    "incfsz" => (v.wrapping_add(1), 0, 0, v == 0xFF),
                    "rlf" => (v << 1 | c, C, v >> 7, false),
                    "rrf" => (v >> 1 | c << 7, C, v & 1, false),
                    "lslf" => (v << 1, C | Z, v >> 7 | zero(v << 1), false),
                    "lsrf" => (v >> 1, C | Z, v & 1 | zero(v >> 1), false),
                    "asrf" => {
                        let r = v >> 1 | v & 0x80;
                        (r, C | Z, v & 1 | zero(r), false)
                    }
                    "swapf" => (v.rotate_left(4), 0, 0, false),
                    "movwf" => (w, 0, 0, false),
                    "clrf" => (0, Z, Z, false),
                    _ => {
                        self.pc = here;
                        return Stop::Invalid(word);
                    }
                };
                if to_f.unwrap_or(true) {
                    if self.store(target, result) {
                        cycles = 2;
                    }
                } else {
                    self.w = result;
                }
                // Flags win over a write of the same bits to STATUS.
                self.set_status(mask, flags);
                if skip {
                    self.skip(&mut cycles);
                }
            }
            Insn::Bit { op, f, bit } => {
                let target = self.file_target(f, &mut cycles);
                let v = self.load(target);
                let m = 1u8 << bit;
                match op {
                    "bcf" | "bsf" => {
                        let value = if op == "bsf" { v | m } else { v & !m };
                        if self.store(target, value) {
                            cycles = 2;
                        }
                    }
                    "btfsc" if v & m == 0 => self.skip(&mut cycles),
                    "btfss" if v & m != 0 => self.skip(&mut cycles),
                    _ => {}
                }
            }
            Insn::Literal { op, k } => {
                let w = self.w;
                let logic = |r: u8| (r, Z, if r == 0 { Z } else { 0 });
                let arith = |(r, flags): (u8, u8)| (r, C | DC | Z, flags);
                let (result, mask, flags) = match op {
                    "movlw" => (k, 0, 0),
                    "retlw" => {
                        self.pc = self.pop();
                        cycles = 2;
                        (k, 0, 0)
                    }
                    "addlw" => arith(add(k, w, 0)),
                    "sublw" => arith(add(k, !w, 1)),
                    "andlw" => logic(k & w),
                    "iorlw" => logic(k | w),
                    "xorlw" => logic(k ^ w),
                    _ => {
                        self.pc = here;
                        return Stop::Invalid(word);
                    }
                };
                self.w = result;
                self.set_status(mask, flags);
            }
            Insn::Jump { op, k } => {
                if op == "call" {
                    self.push(next);
                }
                self.pc = (u16::from(self.pclath() & 0x78) << 8 | k) & PC_MASK;
                cycles = 2;
            }
            Insn::Bra(k) => {
                self.pc = next.wrapping_add_signed(k) & PC_MASK;
                cycles = 2;
            }
            Insn::Movlb(k) => self.data[BSR as usize] = k,
            Insn::Movlp(k) => self.data[PCLATH as usize] = k & 0x7F,
            Insn::Addfsr { fsr, k } => {
                let n = fsr as usize;
                self.set_fsr(n, self.fsr(n).wrapping_add_signed(i16::from(k)));
            }
            Insn::Indirect { op, fsr, mode } => {
                let n = fsr as usize;
                let step: i16 = if mode & 1 == 0 { 1 } else { -1 };
                let before = self.fsr(n);
                let after = before.wrapping_add_signed(step);
                // Modes 0/1 are pre-increment/decrement, 2/3 post.
                let addr = if mode < 2 { after } else { before };
                self.set_fsr(n, after);
                self.move_indirect(op, addr, &mut cycles);
            }
            Insn::Indexed { op, fsr, k } => {
                let addr = self.fsr(fsr as usize).wrapping_add_signed(i16::from(k));
                self.move_indirect(op, addr, &mut cycles);
            }
            Insn::Tris(f) => {
                let port = char::from(b'A' + f.saturating_sub(5));
                self.write_named(&format!("TRIS{port}"));
            }
            Insn::Invalid(w) => {
                self.pc = here;
                return Stop::Invalid(w);
            }
        }
        self.cycles += cycles;
        stop
    }

    /// Runs up to `max_steps` instructions. Stops before executing an
    /// instruction whose address is in `breakpoints`, except the first one
    /// (so that resuming from a breakpoint makes progress).
    pub fn run(&mut self, max_steps: u64, breakpoints: &BTreeSet<u16>) -> Stop {
        let mut steps = 0;
        loop {
            if steps > 0 && breakpoints.contains(&self.pc) {
                return Stop::Breakpoint(self.pc);
            }
            if steps == max_steps {
                return Stop::Limit;
            }
            match self.step() {
                Stop::Step => steps += 1,
                other => return other,
            }
        }
    }

    /// Target of file register `f` in the current bank, following INDFn.
    /// Accesses to program flash cost one more cycle.
    fn file_target(&self, f: u8, cycles: &mut u64) -> Target {
        let target = self.resolve(u16::from(self.bsr()) << 7 | u16::from(f));
        if matches!(target, Target::Flash(_)) {
            *cycles += 1;
        }
        target
    }

    /// `moviw` (to W, sets Z) or `movwi` (from W) at FSR address `addr`.
    fn move_indirect(&mut self, op: &str, addr: u16, cycles: &mut u64) {
        let target = self.resolve_indirect(addr);
        if matches!(target, Target::Flash(_)) {
            *cycles += 1;
        }
        if op == "moviw" {
            self.w = self.load(target);
            let z = if self.w == 0 { Z } else { 0 };
            self.set_status(Z, z);
        } else if self.store(target, self.w) {
            *cycles += 1;
        }
    }

    fn skip(&mut self, cycles: &mut u64) {
        self.pc = (self.pc + 1) & PC_MASK;
        *cycles = 2;
    }

    /// Copies W into the SFR `name` (`OPTION`, `TRIS`), if the part has it.
    fn write_named(&mut self, name: &str) {
        if let Some(&addr) = self.names.get(name) {
            self.data[addr as usize] = self.w;
        }
    }

    fn restore_shadows(&mut self) {
        for (name, core) in SHADOWS {
            if let Some(&addr) = self.names.get(name) {
                let value = self.data[addr as usize];
                match core {
                    WREG => self.w = value,
                    STATUS => self.set_status(C | DC | Z, value),
                    _ => {
                        self.store(Target::Data(core), value);
                    }
                }
            }
        }
    }
}

/// `a + b + carry_in` and the C, DC, Z bits it produces. Subtractions pass
/// `!b` with carry in 1 (no borrow), so C = 1 means "no borrow".
fn add(a: u8, b: u8, carry_in: u8) -> (u8, u8) {
    let sum = u16::from(a) + u16::from(b) + u16::from(carry_in);
    let half = (a & 0x0F) + (b & 0x0F) + carry_in;
    let result = sum as u8;
    let mut flags = 0;
    if sum > 0xFF {
        flags |= C;
    }
    if half > 0x0F {
        flags |= DC;
    }
    if result == 0 {
        flags |= Z;
    }
    (result, flags)
}
