//! Disassembler for the PIC16 enhanced mid-range core (PIC12/16F1xxx and
//! PIC16F18xxx: 14-bit words, 49 instructions). Encodings checked against
//! pic-as listings for PIC16F1789 (32 banks) and PIC16F18446 (64 banks); the
//! two cores differ only in MOVLB.
//!
//! Output uses MPASM syntax with register names from the device pack, e.g.
//! `movwf OSCCON`, `incfsz 0x70, f`, `bsf PORTA, 0`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Result};

use crate::device::Device;

/// One decoded instruction word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insn {
    /// No operands: `nop`, `return`, `clrw`...
    Plain(&'static str),
    /// File register with optional destination (`movwf`/`clrf` have none).
    File { op: &'static str, f: u8, to_f: Option<bool> },
    Bit { op: &'static str, f: u8, bit: u8 },
    Literal { op: &'static str, k: u8 },
    /// `call`/`goto`: 11-bit address within the PCLATH page.
    Jump { op: &'static str, k: u16 },
    /// `bra`: signed offset from the next instruction.
    Bra(i16),
    Movlb(u8),
    Movlp(u8),
    Addfsr { fsr: u8, k: i8 },
    /// `moviw`/`movwi` with pre/post increment/decrement (mode 0..3).
    Indirect { op: &'static str, fsr: u8, mode: u8 },
    /// `moviw`/`movwi` with signed offset.
    Indexed { op: &'static str, fsr: u8, k: i8 },
    Tris(u8),
    Invalid(u16),
}

fn sign(value: u16, bits: u32) -> i16 {
    let shift = 16 - bits;
    ((value << shift) as i16) >> shift
}

/// Decodes one word. `wide_bsr` selects the 64-bank MOVLB encoding.
pub fn decode(word: u16, wide_bsr: bool) -> Insn {
    let w = word & 0x3FFF;
    let f = (w & 0x7F) as u8;
    let to_f = Some(w & 0x80 != 0);
    let file = |op| Insn::File { op, f, to_f };
    match w >> 12 {
        0b00 => match (w >> 8) & 0xF {
            0x0 if w & 0x80 != 0 => Insn::File { op: "movwf", f, to_f: None },
            0x0 => special(w, wide_bsr),
            0x1 if w & 0x80 != 0 => Insn::File { op: "clrf", f, to_f: None },
            0x1 if wide_bsr && w & 0x40 != 0 => Insn::Movlb((w & 0x3F) as u8),
            0x1 => Insn::Plain("clrw"),
            0x2 => file("subwf"),
            0x3 => file("decf"),
            0x4 => file("iorwf"),
            0x5 => file("andwf"),
            0x6 => file("xorwf"),
            0x7 => file("addwf"),
            0x8 => file("movf"),
            0x9 => file("comf"),
            0xA => file("incf"),
            0xB => file("decfsz"),
            0xC => file("rrf"),
            0xD => file("rlf"),
            0xE => file("swapf"),
            _ => file("incfsz"),
        },
        0b01 => {
            let op = ["bcf", "bsf", "btfsc", "btfss"][((w >> 10) & 3) as usize];
            Insn::Bit { op, f, bit: ((w >> 7) & 7) as u8 }
        }
        0b10 => {
            let op = if w & 0x800 == 0 { "call" } else { "goto" };
            Insn::Jump { op, k: w & 0x7FF }
        }
        _ => {
            let k = (w & 0xFF) as u8;
            let fsr = ((w >> 6) & 1) as u8;
            let k6 = sign(w & 0x3F, 6) as i8;
            match (w >> 8) & 0xF {
                0x0 => Insn::Literal { op: "movlw", k },
                0x1 if w & 0x80 == 0 => Insn::Addfsr { fsr, k: k6 },
                0x1 => Insn::Movlp((w & 0x7F) as u8),
                0x2 | 0x3 => Insn::Bra(sign(w & 0x1FF, 9)),
                0x4 => Insn::Literal { op: "retlw", k },
                0x5 => file("lslf"),
                0x6 => file("lsrf"),
                0x7 => file("asrf"),
                0x8 => Insn::Literal { op: "iorlw", k },
                0x9 => Insn::Literal { op: "andlw", k },
                0xA => Insn::Literal { op: "xorlw", k },
                0xB => file("subwfb"),
                0xC => Insn::Literal { op: "sublw", k },
                0xD => file("addwfc"),
                0xE => Insn::Literal { op: "addlw", k },
                _ => {
                    let op = if w & 0x80 == 0 { "moviw" } else { "movwi" };
                    Insn::Indexed { op, fsr, k: k6 }
                }
            }
        }
    }
}

/// Words 0x0000..0x007F that are not `movwf`.
fn special(w: u16, wide_bsr: bool) -> Insn {
    match w {
        0x00 => Insn::Plain("nop"),
        0x01 => Insn::Plain("reset"),
        0x08 => Insn::Plain("return"),
        0x09 => Insn::Plain("retfie"),
        0x0A => Insn::Plain("callw"),
        0x0B => Insn::Plain("brw"),
        0x10..=0x17 => Insn::Indirect { op: "moviw", fsr: ((w >> 2) & 1) as u8, mode: (w & 3) as u8 },
        0x18..=0x1F => Insn::Indirect { op: "movwi", fsr: ((w >> 2) & 1) as u8, mode: (w & 3) as u8 },
        0x20..=0x3F if !wide_bsr => Insn::Movlb((w & 0x1F) as u8),
        0x62 => Insn::Plain("option"),
        0x63 => Insn::Plain("sleep"),
        0x64 => Insn::Plain("clrwdt"),
        0x65..=0x67 => Insn::Tris((w & 7) as u8),
        _ => Insn::Invalid(w),
    }
}

/// One line of disassembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub addr: u32,
    pub word: u16,
    /// Label placed on this address, if it is a vector or branch target.
    pub label: Option<String>,
    pub text: String,
}

/// What is known about a register value at an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Known {
    Unreached,
    Value(u32),
    Unknown,
}

impl Known {
    fn join(self, other: Known) -> Known {
        match (self, other) {
            (Known::Unreached, x) | (x, Known::Unreached) => x,
            (a, b) if a == b => a,
            _ => Known::Unknown,
        }
    }

    fn value(self) -> Option<u32> {
        match self {
            Known::Value(v) => Some(v),
            _ => None,
        }
    }
}

/// Context used to format one instruction.
#[derive(Clone, Copy)]
struct Context {
    bank: Option<u32>,
    pclath: Option<u32>,
}

/// Disassembles every program word present in `flash`, in address order.
pub fn disassemble(flash: &BTreeMap<u32, u16>, device: &Device) -> Result<Vec<Line>> {
    if device.arch != "16Exxx" {
        bail!("désassembleur : architecture {} non supportée (PIC16 enhanced uniquement)", device.arch);
    }
    let wide = device.data_end > 0x1000;
    let labels = labels(flash, wide);
    let banks = bank_flow(flash, wide);

    let mut lines = Vec::with_capacity(flash.len());
    let mut pclath = None;
    let mut prev = None;
    for (&addr, &word) in flash {
        let label = labels.get(&addr).cloned();
        if label.is_some() || prev.is_none_or(|p| p + 1 != addr) {
            pclath = None;
        }
        let insn = decode(word, wide);
        let ctx = Context { bank: banks.get(&addr).and_then(|k| k.value()), pclath };
        let text = format_insn(insn, addr, ctx, &labels, device);
        match insn {
            Insn::Movlp(k) => pclath = Some(k as u32),
            Insn::Jump { op: "call", .. } | Insn::Plain("callw") => pclath = None,
            _ => {}
        }
        lines.push(Line { addr, word, label, text });
        prev = Some(addr);
    }
    Ok(lines)
}

/// Where control can go after an instruction.
enum Flow {
    Next,
    /// Skip instructions: next or the one after.
    Skip,
    Jump(u32),
    Call(u32),
    Stop,
}

fn flow(insn: Insn, addr: u32, pclath: Option<u32>) -> Flow {
    match insn {
        Insn::Jump { op: "goto", k } => Flow::Jump(jump_target(addr, k, pclath)),
        Insn::Jump { k, .. } => Flow::Call(jump_target(addr, k, pclath)),
        Insn::Bra(k) => Flow::Jump(bra_target(addr, k)),
        Insn::Plain("return" | "retfie" | "reset" | "brw") | Insn::Literal { op: "retlw", .. } => {
            Flow::Stop
        }
        Insn::Bit { op: "btfsc" | "btfss", .. } | Insn::File { op: "decfsz" | "incfsz", .. } => {
            Flow::Skip
        }
        _ => Flow::Next,
    }
}

/// True if `insn` changes BSR.
fn writes_bsr(insn: Insn) -> bool {
    match insn {
        Insn::Movlb(_) => true,
        Insn::File { f: 0x08, to_f, .. } => to_f != Some(false),
        _ => false,
    }
}

/// True if the subroutine at `entry` can return without touching BSR.
fn preserves_bsr(
    flash: &BTreeMap<u32, u16>,
    wide: bool,
    entry: u32,
    memo: &mut BTreeMap<u32, bool>,
) -> bool {
    if let Some(&known) = memo.get(&entry) {
        return known;
    }
    memo.insert(entry, false); // recursion is treated as clobbering
    let mut seen = BTreeSet::new();
    let mut todo = vec![entry];
    let mut ok = true;
    while let Some(addr) = todo.pop() {
        let Some(&word) = flash.get(&addr) else { continue };
        if !seen.insert(addr) {
            continue;
        }
        let insn = decode(word, wide);
        if writes_bsr(insn) || insn == Insn::Plain("callw") {
            ok = false;
            break;
        }
        match flow(insn, addr, None) {
            Flow::Next => todo.push(addr + 1),
            Flow::Skip => todo.extend([addr + 1, addr + 2]),
            Flow::Jump(t) => todo.push(t),
            Flow::Call(t) => {
                if !preserves_bsr(flash, wide, t, memo) {
                    ok = false;
                    break;
                }
                todo.push(addr + 1);
            }
            Flow::Stop => {}
        }
    }
    memo.insert(entry, ok);
    ok
}

/// BSR value at each address, joined over every path that reaches it.
/// BSR is 0 out of reset; the interrupt vector starts unknown.
fn bank_flow(flash: &BTreeMap<u32, u16>, wide: bool) -> BTreeMap<u32, Known> {
    let mut state: BTreeMap<u32, Known> = BTreeMap::new();
    let mut memo = BTreeMap::new();
    let mut todo = Vec::new();
    let reach = |state: &mut BTreeMap<u32, Known>, todo: &mut Vec<u32>, addr: u32, k: Known| {
        if !flash.contains_key(&addr) {
            return;
        }
        let old = state.get(&addr).copied().unwrap_or(Known::Unreached);
        let new = old.join(k);
        if new != old {
            state.insert(addr, new);
            todo.push(addr);
        }
    };
    reach(&mut state, &mut todo, 0x0000, Known::Value(0));
    reach(&mut state, &mut todo, 0x0004, Known::Unknown);

    while let Some(addr) = todo.pop() {
        let insn = decode(flash[&addr], wide);
        let before = state[&addr];
        let after = match insn {
            Insn::Movlb(k) => Known::Value(k as u32),
            Insn::File { op: "clrf", f: 0x08, .. } => Known::Value(0),
            i if writes_bsr(i) => Known::Unknown,
            _ => before,
        };
        match flow(insn, addr, None) {
            Flow::Next => reach(&mut state, &mut todo, addr + 1, after),
            Flow::Skip => {
                reach(&mut state, &mut todo, addr + 1, after);
                reach(&mut state, &mut todo, addr + 2, after);
            }
            Flow::Jump(t) => reach(&mut state, &mut todo, t, after),
            Flow::Call(t) => {
                reach(&mut state, &mut todo, t, after);
                let back = if preserves_bsr(flash, wide, t, &mut memo) { after } else { Known::Unknown };
                reach(&mut state, &mut todo, addr + 1, back);
            }
            Flow::Stop => {}
        }
    }
    // Code never reached from a vector (e.g. tables) keeps no bank.
    state
}

fn jump_target(addr: u32, k: u16, pclath: Option<u32>) -> u32 {
    // Without a known PCLATH, assume the current 2K page.
    let page = pclath.map_or(addr & 0x7800, |p| (p & 0x78) << 8);
    page | k as u32
}

fn bra_target(addr: u32, k: i16) -> u32 {
    (addr as i64 + 1 + k as i64) as u32
}

fn labels(flash: &BTreeMap<u32, u16>, wide: bool) -> BTreeMap<u32, String> {
    let mut calls = BTreeSet::new();
    let mut jumps = BTreeSet::new();
    let mut pclath = None;
    for (&addr, &word) in flash {
        match decode(word, wide) {
            Insn::Movlp(k) => pclath = Some(k as u32),
            Insn::Jump { op, k } => {
                let target = jump_target(addr, k, pclath);
                if op == "call" {
                    calls.insert(target);
                } else {
                    jumps.insert(target);
                }
            }
            Insn::Bra(k) => {
                jumps.insert(bra_target(addr, k));
            }
            _ => {}
        }
    }
    let mut labels = BTreeMap::new();
    for t in jumps {
        labels.insert(t, format!("L_{t:04X}"));
    }
    for t in calls {
        labels.insert(t, format!("sub_{t:04X}"));
    }
    labels.insert(0x0000, "reset_vec".into());
    if flash.contains_key(&0x0004) {
        labels.insert(0x0004, "isr".into());
    }
    labels
}

/// Name of file register `f` given the current bank, or hex.
fn register(f: u8, ctx: Context, device: &Device) -> String {
    let f = f as u32;
    let addr = match (f, ctx.bank) {
        // Core registers and common RAM appear in every bank.
        (0x00..=0x0B, _) => Some(f),
        (0x70..=0x7F, _) => None,
        (_, Some(bank)) => Some(bank * 0x80 + f),
        _ => None,
    };
    addr.and_then(|a| device.sfrs.get(&a).cloned())
        .unwrap_or_else(|| format!("0x{f:02X}"))
}

fn format_insn(
    insn: Insn,
    addr: u32,
    ctx: Context,
    labels: &BTreeMap<u32, String>,
    device: &Device,
) -> String {
    let target = |t: u32| labels.get(&t).cloned().unwrap_or_else(|| format!("0x{t:04X}"));
    let fsr_mode = |fsr: u8, mode: u8| match mode {
        0 => format!("++FSR{fsr}"),
        1 => format!("--FSR{fsr}"),
        2 => format!("FSR{fsr}++"),
        _ => format!("FSR{fsr}--"),
    };
    match insn {
        Insn::Plain(op) => op.to_owned(),
        Insn::File { op, f, to_f: None } => format!("{op} {}", register(f, ctx, device)),
        Insn::File { op, f, to_f: Some(to_f) } => {
            let d = if to_f { "f" } else { "w" };
            format!("{op} {}, {d}", register(f, ctx, device))
        }
        Insn::Bit { op, f, bit } => format!("{op} {}, {bit}", register(f, ctx, device)),
        Insn::Literal { op, k } => format!("{op} 0x{k:02X}"),
        Insn::Jump { op, k } => format!("{op} {}", target(jump_target(addr, k, ctx.pclath))),
        Insn::Bra(k) => format!("bra {}", target(bra_target(addr, k))),
        Insn::Movlb(k) => format!("movlb {k}"),
        Insn::Movlp(k) => format!("movlp 0x{k:02X}"),
        Insn::Addfsr { fsr, k } => format!("addfsr FSR{fsr}, {k}"),
        Insn::Indirect { op, fsr, mode } => format!("{op} {}", fsr_mode(fsr, mode)),
        Insn::Indexed { op, fsr, k } => format!("{op} {k}[FSR{fsr}]"),
        Insn::Tris(f) => format!("tris {f}"),
        Insn::Invalid(w) => format!("dw 0x{w:04X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::tests::pic16f1789;
    use crate::image::Image;

    /// Opcodes assembled by pic-as, shared by both cores (operands are
    /// printed in MPASM syntax here).
    const COMMON: &[(u16, &str)] = &[
        (0x0103, "clrw"),
        (0x0010, "moviw ++FSR0"),
        (0x0017, "moviw FSR1--"),
        (0x3F85, "movwi 5[FSR0]"),
        (0x3F7D, "moviw -3[FSR1]"),
        (0x317D, "addfsr FSR1, -3"),
        (0x3188, "movlp 0x08"),
        (0x000B, "brw"),
        (0x000A, "callw"),
        (0x35A0, "lslf 0x20, f"),
        (0x3721, "asrf 0x21, w"),
        (0x3BA2, "subwfb 0x22, f"),
        (0x3D23, "addwfc 0x23, w"),
        (0x0001, "reset"),
        (0x0063, "sleep"),
        (0x0064, "clrwdt"),
        (0x3455, "retlw 0x55"),
        (0x3C01, "sublw 0x01"),
        (0x3E02, "addlw 0x02"),
        (0x3A03, "xorlw 0x03"),
        (0x3904, "andlw 0x04"),
        (0x3805, "iorlw 0x05"),
        (0x09A4, "comf 0x24, f"),
        (0x0E25, "swapf 0x25, w"),
        (0x0CA6, "rrf 0x26, f"),
        (0x0D27, "rlf 0x27, w"),
        (0x1BA8, "btfsc 0x28, 7"),
        (0x1CA9, "btfss 0x29, 1"),
        (0x0BAA, "decfsz 0x2A, f"),
        (0x022B, "subwf 0x2B, w"),
        (0x0000, "nop"),
    ];

    fn text(word: u16, wide: bool) -> String {
        let device = pic16f1789();
        let ctx = Context { bank: None, pclath: None };
        format_insn(decode(word, wide), 0x100, ctx, &BTreeMap::new(), &device)
    }

    #[test]
    fn matches_pic_as_encodings() {
        for wide in [false, true] {
            for &(word, expected) in COMMON {
                assert_eq!(text(word, wide), expected, "{word:#06x} wide={wide}");
            }
        }
        // The only difference between the 32- and 64-bank cores.
        assert_eq!(text(0x0023, false), "movlb 3");
        assert_eq!(text(0x003F, false), "movlb 31");
        assert_eq!(text(0x0143, true), "movlb 3");
        assert_eq!(text(0x017F, true), "movlb 63");
        assert_eq!(text(0x0023, true), "dw 0x0023");
        assert_eq!(text(0x0143, false), "clrw");
        // bra start, at 0x0009 jumping back to 0x0000.
        assert_eq!(
            format_insn(decode(0x33F6, false), 9, Context { bank: None, pclath: None }, &BTreeMap::new(), &pic16f1789()),
            "bra 0x0000"
        );
    }

    #[test]
    fn blink_reads_like_the_source() {
        let mut device = pic16f1789();
        for (addr, name) in [(0x08, "BSR"), (0x0C, "PORTA"), (0x8C, "TRISA"), (0x99, "OSCCON"), (0x18C, "ANSELA")] {
            device.sfrs.insert(addr, name.into());
        }
        let image = Image::parse_hex(include_str!("../tests/blink.hex"), &device).unwrap();
        let lines = disassemble(&image.flash, &device).unwrap();
        let find = |addr: u32| lines.iter().find(|l| l.addr == addr).unwrap();

        assert_eq!(find(0x00).label.as_deref(), Some("reset_vec"));
        assert_eq!(find(0x00).text, "goto L_0010");
        assert_eq!(find(0x04).text, "retfie");
        assert_eq!(find(0x05).label.as_deref(), Some("sub_0005"));
        assert_eq!(find(0x05).text, "movlw 0xF8");
        assert_eq!(find(0x09).text, "incfsz 0x70, f");
        assert_eq!(find(0x0A).text, "goto L_0009");
        assert_eq!(find(0x0F).text, "return");
        // banksel OSCCON; movlw 0xf8; movwf OSCCON; banksel ANSELA; clrf ANSELA...
        let start: Vec<&str> = (0x10..=0x17).map(|a| find(a).text.as_str()).collect();
        assert_eq!(
            start,
            ["movlb 1", "movlw 0xF8", "movwf OSCCON", "movlb 3", "clrf ANSELA", "movlb 1", "clrf TRISA", "clrf BSR"]
        );
        assert_eq!(find(0x18).label.as_deref(), Some("L_0018"));
        assert_eq!(find(0x18).text, "bsf PORTA, 0");
        assert_eq!(find(0x19).text, "call sub_0005");
        assert_eq!(find(0x1C).text, "goto L_0018");
    }
}
