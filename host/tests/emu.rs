//! Instruction-set simulator against small programs built with our own
//! assembler for PIC16F1789, and against the lab's blink program.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use picflash::asm::{assemble, assemble_file, Options};
use picflash::devdb::Database;
use picflash::emu::{Cpu, Stop, STACK_DEPTH};
use picflash::image::Image;

const C: u8 = 0x01;
const DC: u8 = 0x02;
const Z: u8 = 0x04;

/// Assembles `body`, placed at the reset vector of a PIC16F1789.
fn build(body: &str) -> (Cpu, BTreeMap<String, i64>) {
    let source = format!(
        "PROCESSOR 16F1789\n#include <xc.inc>\nPSECT reset_vec, class = CODE, delta = 2\nreset_vec:\n{body}\n"
    );
    let out = assemble(Path::new("emu_test.asm"), &source, &Options::default());
    let image = out.image.as_ref().unwrap_or_else(|| panic!("{}", out.log()));
    let cpu = Cpu::new(out.device.as_ref().unwrap(), image).unwrap();
    (cpu, out.symbols)
}

/// Builds `body` and runs it until its `sleep`.
fn exec(body: &str) -> Cpu {
    let (mut cpu, _) = build(body);
    assert_eq!(cpu.run(100_000, &BTreeSet::new()), Stop::Sleep);
    cpu
}

fn flags(cpu: &Cpu) -> u8 {
    cpu.status() & (C | DC | Z)
}

#[test]
fn power_on_state() {
    let (cpu, _) = build("nop");
    assert_eq!(cpu.pc, 0);
    assert_eq!(cpu.status(), 0x18);
    assert_eq!(cpu.bsr(), 0);
    assert_eq!(cpu.sfr_value("TRISA"), Some(0xFF));
    assert_eq!(cpu.sfr_value("ANSELB"), Some(0xFF));
    assert_eq!(cpu.sfr_value("LATA"), Some(0));
    assert_eq!(cpu.sfr_value("STKPTR"), Some(0x1F));
    assert_eq!(cpu.pins('a'), Some((0, 0xFF)));
    assert_eq!(cpu.pins('Z'), None);
}

#[test]
fn addition_flags() {
    let (mut cpu, _) = build(
        "movlw 0xFF
         addlw 0x01
         movlw 0x0F
         movwf 0x70
         movlw 0x01
         addwf 0x70, f
         movlw 0x70
         addwf 0x70, w
         sleep",
    );
    cpu.step();
    cpu.step();
    assert_eq!((cpu.w, flags(&cpu)), (0x00, C | DC | Z));
    for _ in 0..4 {
        cpu.step();
    }
    assert_eq!(cpu.read(0x70), 0x10);
    assert_eq!(flags(&cpu), DC);
    cpu.step();
    cpu.step();
    // 0x10 + 0x70 = 0x80: no carry out of either nibble or the byte.
    assert_eq!((cpu.w, flags(&cpu)), (0x80, 0));
    assert_eq!(cpu.cycles, 8);
}

#[test]
fn subtraction_borrow() {
    let (mut cpu, _) = build(
        "movlw 3
         movwf 0x71
         movlw 5
         subwf 0x71, w   ; 3 - 5
         movlw 3
         subwf 0x71, w   ; 3 - 3
         movlw 2
         subwf 0x71, w   ; 3 - 2
         movlw 1
         sublw 0         ; 0 - 1
         sleep",
    );
    let mut after = |n: usize| {
        for _ in 0..n {
            cpu.step();
        }
        (cpu.w, flags(&cpu))
    };
    // C = 0 means a borrow happened.
    assert_eq!(after(4), (0xFE, 0));
    assert_eq!(after(2), (0x00, C | DC | Z));
    assert_eq!(after(2), (0x01, C | DC));
    assert_eq!(after(2), (0xFF, 0));
}

#[test]
fn multi_byte_carry_and_shifts() {
    let cpu = exec(
        "movlw 0xFF
         movwf 0x72
         movlw 0x01
         movwf 0x73      ; 0x01FF
         movlw 1
         addwf 0x72, f
         movlw 0
         addwfc 0x73, f  ; + 1 = 0x0200
         movf 0x72, w
         movwf 0x74
         movf 0x73, w
         movwf 0x75
         movlw 1
         subwf 0x72, f
         movlw 0
         subwfb 0x73, f  ; - 1 = 0x01FF
         movlw 0x81
         movwf 0x76
         lslf 0x76, f    ; 0x02, C = 1
         rlf 0x76, f     ; 0x05, C = 0
         movlw 0x80
         movwf 0x77
         asrf 0x77, f    ; 0xC0
         lsrf 0x77, f    ; 0x60
         bsf STATUS, 0
         rrf 0x77, f     ; 0xB0, C = 0
         movlw 0x3C
         movwf 0x78
         swapf 0x78, f
         clrf STATUS     ; Z set by the instruction, TO/PD untouched
         sleep",
    );
    assert_eq!((cpu.read(0x74), cpu.read(0x75)), (0x00, 0x02));
    assert_eq!((cpu.read(0x72), cpu.read(0x73)), (0xFF, 0x01));
    assert_eq!(cpu.read(0x76), 0x05);
    assert_eq!(cpu.read(0x77), 0xB0);
    assert_eq!(cpu.read(0x78), 0xC3);
    // After sleep: nTO = 1, nPD = 0.
    assert_eq!(cpu.status(), 0x10 | Z);
}

#[test]
fn banks_and_common_ram() {
    let cpu = exec(
        "movlb 1
         movlw 0x55
         movwf 0x20      ; 0xA0
         banksel TRISA
         clrf TRISA
         movlb 0
         movlw 0x77
         movwf 0x20      ; 0x20
         movlw 0x99
         movwf 0x70      ; common RAM
         movlb 5
         movf 0x70, w
         movwf 0x71
         banksel LATA
         bsf LATA, 3
         movlb 2
         movlw 0x5A
         movwf 0x7F
         sleep",
    );
    assert_eq!(cpu.read(0xA0), 0x55);
    assert_eq!(cpu.read(0x20), 0x77);
    assert_eq!(cpu.read(0x2F0), 0x99);
    assert_eq!(cpu.read(0x71), 0x99);
    assert_eq!(cpu.read(0x7F), 0x5A);
    assert_eq!(cpu.read(0xC7F), 0x5A);
    assert_eq!(cpu.sfr_value("TRISA"), Some(0));
    assert_eq!(cpu.bsr(), 2);
    // Core registers are mirrored in every bank.
    assert_eq!(cpu.read(0x188), 2);
    assert_eq!(cpu.read(0x503), cpu.status());
    assert_eq!(cpu.pins('A'), Some((0x08, 0x00)));
    // Unimplemented locations read 0.
    assert_eq!(cpu.read(0x1F), 0);
}

#[test]
fn ports_write_latch_and_read_outputs_only() {
    let cpu = exec(
        "movlw 0xF0
         banksel TRISB
         movwf TRISB     ; RB0-3 outputs
         banksel PORTB
         movlw 0xFF
         movwf PORTB
         movf PORTB, w
         sleep",
    );
    assert_eq!(cpu.sfr_value("LATB"), Some(0xFF));
    assert_eq!(cpu.sfr_value("PORTB"), Some(0x0F));
    assert_eq!(cpu.w, 0x0F);
    assert_eq!(cpu.pins('B'), Some((0xFF, 0xF0)));
}

#[test]
fn fsr_linear_and_moviw_modes() {
    let (mut cpu, _) = build(
        "movlw 0x20
         movwf FSR0H
         movlw 0x4F
         movwf FSR0L     ; 0x204F = bank 0, 0x6F
         movlw 0x11
         movwi FSR0++    ; 0x6F
         movlw 0x22
         movwi FSR0++    ; 0x2050 = bank 1, 0xA0
         movlw 0x33
         movwi 1[FSR0]   ; 0x2052 = 0xA2
         movlw 0x44
         movwi ++FSR0    ; 0x2052 again
         movlw 0x01
         movwf FSR1H
         movlw 0x20
         movwf FSR1L     ; 0x0120 = bank 2, 0x20
         movlw 0x55
         movwi FSR1--
         moviw ++FSR1    ; 0x55
         movwf 0x70
         moviw --FSR0    ; 0x2051 = 0xA1 (still 0)
         moviw -2[FSR0]  ; 0x204F
         movwf 0x71
         movf INDF1, w
         movwf 0x72
         addfsr FSR1, -32
         addfsr FSR1, 31
         sleep",
    );
    assert_eq!(cpu.run(100, &BTreeSet::new()), Stop::Sleep);
    assert_eq!(cpu.read(0x6F), 0x11);
    assert_eq!(cpu.read(0xA0), 0x22);
    assert_eq!(cpu.read(0xA2), 0x44);
    assert_eq!(cpu.read(0x2052), 0x44);
    assert_eq!(cpu.read(0x120), 0x55);
    assert_eq!(cpu.read(0x70), 0x55);
    assert_eq!(cpu.read(0x71), 0x11);
    assert_eq!(cpu.read(0x72), 0x55);
    assert_eq!(cpu.fsr(0), 0x2051);
    assert_eq!(cpu.fsr(1), 0x0120 - 1);

    // moviw sets Z.
    let (mut cpu, _) = build("clrf 0x30\n movlw 0x30\n movwf FSR0L\n moviw FSR0++\n sleep");
    cpu.run(100, &BTreeSet::new());
    assert_eq!(cpu.status() & Z, Z);
    assert_eq!(cpu.fsr(0), 0x31);
}

#[test]
fn program_memory_through_fsr() {
    let (mut cpu, symbols) = build(
        "movlw high(table) | 0x80
         movwf FSR0H
         movlw low(table)
         movwf FSR0L
         moviw FSR0++
         movwf 0x70
         moviw FSR0++
         movwf INDF0     ; ignored: flash is read-only
         sleep
table:   retlw 0x42
         dw 0x3FA5",
    );
    for _ in 0..4 {
        cpu.step();
    }
    let before = cpu.cycles;
    cpu.step();
    assert_eq!(cpu.cycles - before, 2, "flash access costs one extra cycle");
    assert_eq!(cpu.w, 0x42);
    cpu.run(100, &BTreeSet::new());
    assert_eq!(cpu.read(0x70), 0x42);
    assert_eq!(cpu.w, 0xA5);
    let table = symbols["table"] as u16;
    assert_eq!(cpu.read(0x8000 | table), 0x42);
}

#[test]
fn calls_returns_and_computed_gotos() {
    let (mut cpu, _) = build(
        "movlw 2
         call table
         movwf 0x70
         movlw 0
         call table
         movwf 0x71
         movlp high(getval)
         movlw low(getval)
         callw
         movwf 0x72
         movlp high(table2)
         movlw 1
         call table2
         movwf 0x73
         movlw 5
         bra skip
         movlw 6
skip:    movwf 0x74
         sleep
table:   brw
         retlw 10
         retlw 20
         retlw 30
getval:  retlw 0x33
table2:  addwf PCL, f
         retlw 0xA0
         retlw 0xA1",
    );
    // The first call: movlw (1) + call (2) + brw (2) + retlw (2).
    for _ in 0..4 {
        cpu.step();
    }
    assert_eq!(cpu.cycles, 7);
    assert_eq!(cpu.w, 30);
    assert_eq!(cpu.run(100, &BTreeSet::new()), Stop::Sleep);
    assert_eq!(cpu.read(0x70), 30);
    assert_eq!(cpu.read(0x71), 10);
    assert_eq!(cpu.read(0x72), 0x33);
    assert_eq!(cpu.read(0x73), 0xA1);
    assert_eq!(cpu.read(0x74), 5);
    assert!(cpu.stack.is_empty());
}

#[test]
fn stack_limits() {
    let (mut cpu, symbols) = build("rec: call rec");
    assert_eq!(cpu.run(1000, &BTreeSet::new()), Stop::StackOverflow);
    assert_eq!(cpu.stack.len(), STACK_DEPTH);
    assert_eq!(cpu.pc, symbols["rec"] as u16);
    assert_eq!(cpu.sfr_value("STKPTR"), Some(15));
    assert_eq!(cpu.sfr_value("TOSL"), Some(1));

    let (mut cpu, _) = build("return");
    assert_eq!(cpu.step(), Stop::StackUnderflow);
    assert_eq!(cpu.pc, 0);
}

#[test]
fn nested_delay_cycles() {
    const N: u64 = 5;
    const M: u64 = 10;
    let (mut cpu, symbols) = build(&format!(
        "call delay
done:    sleep
delay:   movlw {N}
         movwf 0x70
outer:   movlw {M}
         movwf 0x71
inner:   decfsz 0x71, f
         goto inner
         decfsz 0x70, f
         goto outer
         return"
    ));
    let done = symbols["done"] as u16;
    assert_eq!(cpu.run(10_000, &BTreeSet::from([done])), Stop::Breakpoint(done));
    // Inner loop: M - 1 times decfsz + goto (3), then a taken skip (2).
    let inner = 3 * (M - 1) + 2;
    let outer_again = 2 + inner + 1 + 2;
    let outer_last = 2 + inner + 2;
    let delay = 2 + (N - 1) * outer_again + outer_last + 2;
    assert_eq!(cpu.cycles, 2 + delay);
}

#[test]
fn reset_sleep_and_invalid() {
    let (mut cpu, _) = build("movlw 1\n movwf 0x70\n clrwdt\n sleep\n reset");
    assert_eq!(cpu.run(100, &BTreeSet::new()), Stop::Sleep);
    assert_eq!(cpu.status() & 0x18, 0x10);
    assert_eq!(cpu.step(), Stop::Step); // reset
    assert_eq!(cpu.pc, 0);
    assert_eq!(cpu.status(), 0x18);
    assert_eq!(cpu.read(0x70), 1, "RAM survives a reset");

    let device = Database::embedded().find("PIC16F1789").unwrap();
    let image = Image { flash: BTreeMap::from([(0, 0x0002)]), ..Image::default() };
    let mut cpu = Cpu::new(device, &image).unwrap();
    assert_eq!(cpu.step(), Stop::Invalid(0x0002));
    assert_eq!((cpu.pc, cpu.cycles), (0, 0));

    let too_big = Image { flash: BTreeMap::from([(0x4000, 0)]), ..Image::default() };
    assert!(Cpu::new(device, &too_big).is_err());
}

/// The lab program: blinks RA0 with a software delay.
#[test]
fn blink_toggles_ra0() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let out = assemble_file(&dir.join("fixtures/blink.asm"), &Options::default()).unwrap();
    let image = out.image.as_ref().unwrap_or_else(|| panic!("{}", out.log()));
    let device = out.device.as_ref().unwrap();
    let hex = Image::parse_hex(include_str!("blink.hex"), device).unwrap();
    assert_eq!(image.flash, hex.flash);
    let mut cpu = Cpu::new(device, image).unwrap();

    let ra0 = |cpu: &Cpu| cpu.pins('A').unwrap().0 & 1;
    let mut toggles = Vec::new();
    let mut last = ra0(&cpu);
    for _ in 0..3_000_000u32 {
        assert_eq!(cpu.step(), Stop::Step);
        if ra0(&cpu) != last {
            last = ra0(&cpu);
            toggles.push((last, cpu.cycles));
            if toggles.len() == 3 {
                break;
            }
        }
    }
    assert_eq!(toggles.iter().map(|t| t.0).collect::<Vec<_>>(), [1, 0, 1]);
    assert_eq!(cpu.sfr_value("TRISA"), Some(0));
    assert_eq!(cpu.sfr_value("ANSELA"), Some(0));
    assert_eq!(cpu.sfr_value("OSCCON"), Some(0xF8));

    // delay: 4 setup cycles; 2048 passes of 256 incfsz (255 x 3 + 2);
    // counter_h: 2040 x 3 + 8 x 2; counter_hh: 7 x 3 + 2; return 2.
    let delay = 4 + 2048 * (255 * 3 + 2) + 2040 * 3 + 8 * 2 + 7 * 3 + 2 + 2;
    // call (2) + delay, then bcf (1); then call + delay + goto (2) + bsf.
    assert_eq!(toggles[1].1 - toggles[0].1, 2 + delay + 1);
    assert_eq!(toggles[2].1 - toggles[1].1, 2 + delay + 2 + 1);

    // A breakpoint on `loop: bsf PORTA, 0` stops before it executes.
    let mut cpu = Cpu::new(device, image).unwrap();
    let lp = out.symbols["loop"] as u16;
    let breakpoints = BTreeSet::from([lp]);
    assert_eq!(cpu.run(100, &breakpoints), Stop::Breakpoint(lp));
    assert_eq!(cpu.pc, lp);
    assert_eq!(ra0(&cpu), 0);
    // Resuming runs a whole period and comes back to it.
    assert_eq!(cpu.run(5_000_000, &breakpoints), Stop::Breakpoint(lp));
    assert_eq!(ra0(&cpu), 0);
    assert_eq!(cpu.run(1, &breakpoints), Stop::Limit);
    assert_eq!(ra0(&cpu), 1);
}

#[test]
fn retfie_restores_shadows_and_sets_gie() {
    let cpu = exec(
        "call isr
         sleep
isr:     banksel WREG_SHAD
         movlw 0x42
         movwf WREG_SHAD
         movlw 3
         movwf BSR_SHAD
         movlw 0
         retfie",
    );
    assert_eq!(cpu.w, 0x42);
    assert_eq!(cpu.bsr(), 3);
    assert_eq!(cpu.sfr_value("INTCON").map(|v| v & 0x80), Some(0x80));
}
