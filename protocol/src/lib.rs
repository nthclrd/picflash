//! Wire protocol shared by the `picflash` host tool and the programmer firmware.
//!
//! The firmware knows nothing about PIC families: it drives MCLR and shifts
//! bits on ICSPCLK/ICSPDAT. The host sends a *script* (a frame holding a list
//! of ops); the firmware runs them in order and returns one reply frame with a
//! status byte followed by every value clocked in by `IN` ops.
//!
//! Frame layout (both directions): `[len: u16 LE][payload: len bytes]`.
//! Reply payload: `[status: u8][values: u32 LE ...]`.

#![no_std]

pub const PROTOCOL_VERSION: u8 = 3;

/// Largest payload accepted in either direction.
pub const MAX_PAYLOAD: usize = 4096;

/// USB IDs. 0x1209:0x0001 is the pid.codes test ID; swap for a real one before
/// distributing boards.
pub const USB_VID: u16 = 0x1209;
pub const USB_PID: u16 = 0x0001;

/// Delay the firmware inserts after every `OUT`/`IN` group, in µs. Covers the
/// ICSP TDLY minimum (1 µs) between a command and its payload.
pub const GROUP_GAP_US: u32 = 2;

/// Operations understood by the firmware.
pub mod op {
    /// Reply carries `PROTOCOL_VERSION` as one value.
    pub const PING: u8 = 0x01;
    /// Drive ICSPCLK, ICSPDAT and MCLR low (program mode entry). No arguments.
    pub const ENTER: u8 = 0x02;
    /// Release ICSPCLK/ICSPDAT and raise MCLR (target runs). No arguments.
    pub const RELEASE: u8 = 0x03;
    /// Drive ICSPCLK/ICSPDAT low and raise MCLR to VPP (high-voltage program
    /// mode entry, no key). No arguments. Boards without a VPP supply answer
    /// `UNKNOWN_OP`.
    pub const ENTER_HV: u8 = 0x04;
    /// Reply carries the board's [`super::caps`] bits as one value.
    pub const CAPS: u8 = 0x05;
    /// `[shape: u8][bits: u32 LE]` — clock out bits, see [`super::shape`].
    pub const OUT: u8 = 0x10;
    /// `[shape: u8]` — clock in bits; the value goes into the reply.
    pub const IN: u8 = 0x11;
    /// `[us: u16 LE]` — wait.
    pub const DELAY_US: u8 = 0x20;
    /// `[mv: u16 LE]` — regulate VPP to `mv` (0 = off). MCLR is not switched
    /// to VPP until `ENTER_HV`.
    pub const SET_VPP: u8 = 0x30;
    /// `[mv: u16 LE]` — supply the target at `mv` (0 = off). Refused if the
    /// target is already powered.
    pub const SET_VDD: u8 = 0x31;
    /// Reply carries `target VDD mV | VPP mV << 16` as one value.
    pub const MEASURE: u8 = 0x32;
}

/// Board capability bits returned by `CAPS`.
pub mod caps {
    /// VPP supply and `ENTER_HV`.
    pub const HIGH_VOLTAGE: u32 = 1 << 0;
    /// `SET_VDD` can power the target.
    pub const TARGET_POWER: u32 = 1 << 1;
    /// `MEASURE` reports real voltages.
    pub const MEASURE: u32 = 1 << 2;
}

/// Reply status codes.
pub mod status {
    pub const OK: u8 = 0x00;
    pub const UNKNOWN_OP: u8 = 0x01;
    pub const TRUNCATED: u8 = 0x02;
    pub const REPLY_OVERFLOW: u8 = 0x03;
    /// `SET_VPP`/`SET_VDD` value outside what the board can produce.
    pub const VOLTAGE_RANGE: u8 = 0x04;
    /// VPP did not reach the requested level (supply turned off).
    pub const VPP_FAIL: u8 = 0x05;
    /// Target VDD did not reach the requested level (supply turned off).
    pub const VDD_FAIL: u8 = 0x06;
    /// `SET_VDD` refused: the target already has its own supply.
    pub const EXTERNAL_POWER: u8 = 0x07;
    /// `ENTER_HV` before a successful `SET_VPP`.
    pub const NO_VPP: u8 = 0x08;
}

/// Encodes a bit count (1..=32) and bit order into the `OUT`/`IN` shape byte:
/// bit 7 = MSb first, bits 0..5 = count - 1.
pub const fn shape(count: u32, msb_first: bool) -> u8 {
    assert!(count >= 1 && count <= 32);
    (count - 1) as u8 | if msb_first { 0x80 } else { 0 }
}

/// Inverse of [`shape`]: `(count, msb_first)`. `None` if the count is over 32.
pub const fn decode_shape(shape: u8) -> Option<(u32, bool)> {
    let count = (shape & 0x3F) as u32 + 1;
    if count > 32 {
        None
    } else {
        Some((count, shape & 0x80 != 0))
    }
}

/// Bytes an op occupies in a script, opcode included. `None` if unknown.
pub const fn op_len(opcode: u8) -> Option<usize> {
    match opcode {
        op::PING | op::ENTER | op::RELEASE | op::ENTER_HV | op::CAPS | op::MEASURE => Some(1),
        op::IN => Some(2),
        op::DELAY_US | op::SET_VPP | op::SET_VDD => Some(3),
        op::OUT => Some(6),
        _ => None,
    }
}

/// Bytes an op adds to the reply.
pub const fn op_reply_len(opcode: u8) -> usize {
    match opcode {
        op::PING | op::IN | op::CAPS | op::MEASURE => 4,
        _ => 0,
    }
}

/// Bit `i` of a `count`-bit transfer of `bits`, in wire order.
pub const fn wire_bit(bits: u32, count: u32, msb_first: bool, i: u32) -> bool {
    let pos = if msb_first { count - 1 - i } else { i };
    bits >> pos & 1 != 0
}

/// Accumulates bit `i` (wire order) of a `count`-bit transfer into `value`.
pub const fn store_bit(value: u32, count: u32, msb_first: bool, i: u32, bit: bool) -> u32 {
    let pos = if msb_first { count - 1 - i } else { i };
    value | (bit as u32) << pos
}

/// Analog side of the picflash board (hardware/): ADC scaling and the PWM
/// "DACs" injected into the regulators' feedback nodes. Pure arithmetic so the
/// host can test it; the firmware runs it.
pub mod board {
    /// PWM counter top: duty is `0..=PWM_TOP + 1` counts.
    pub const PWM_TOP: u16 = 4095;
    const FULL: i32 = PWM_TOP as i32 + 1;
    /// ADC full scale (12 bits, 3.3 V reference).
    const ADC_FULL: u32 = 4095;
    const VREF_MV: u32 = 3300;

    /// VPP the board can regulate (MT3608 with R40/R41/R42+R43).
    pub const VPP_RANGE_MV: core::ops::RangeInclusive<u16> = 6800..=13800;
    /// Target VDD the board can supply (TLV75801 below 5 V USB).
    pub const VDD_RANGE_MV: core::ops::RangeInclusive<u16> = 1500..=4800;
    /// Above this the target counts as powered.
    pub const POWERED_MV: u16 = 1000;
    pub const VPP_TOLERANCE_MV: i32 = 150;
    pub const VDD_TOLERANCE_MV: i32 = 60;

    /// Regulator output against DAC voltage: `out = a - b * dac`, with
    /// `a` in mV and `b` in mV per volt of DAC, from the feedback networks.
    #[derive(Clone, Copy)]
    pub struct Loop {
        pub a_mv: i32,
        pub b_mv_per_v: i32,
    }
    /// VPP: 0.6 V FB, 200k/10k, DAC through 82k + 10k.
    pub const VPP_LOOP: Loop = Loop { a_mv: 13904, b_mv_per_v: 2174 };
    /// Target VDD: 0.55 V FB, 56k/10k, DAC through 15k + 10k.
    pub const VDD_LOOP: Loop = Loop { a_mv: 4862, b_mv_per_v: 2240 };

    impl Loop {
        /// PWM duty (counts) expected to give `mv`.
        pub const fn duty_for(self, mv: u16) -> u16 {
            let dac_mv = (self.a_mv - mv as i32) * 1000 / self.b_mv_per_v;
            clamp_duty(dac_mv * FULL / VREF_MV as i32)
        }
        /// Next duty after measuring `measured` while aiming at `target`
        /// (more duty = lower output).
        pub const fn correct(self, duty: u16, measured: u16, target: u16) -> u16 {
            let err = measured as i32 - target as i32;
            let per_full = self.b_mv_per_v * VREF_MV as i32 / 1000;
            clamp_duty(duty as i32 + err * FULL / per_full)
        }
    }

    const fn clamp_duty(d: i32) -> u16 {
        if d < 0 {
            0
        } else if d > FULL {
            FULL as u16
        } else {
            d as u16
        }
    }

    const fn scale(raw: u16, num: u32, den: u32) -> u16 {
        let mv = raw as u32 * VREF_MV / ADC_FULL * num / den;
        if mv > u16::MAX as u32 { u16::MAX } else { mv as u16 }
    }
    /// ADC0: target VDD through 100k/68k.
    pub const fn tvdd_mv(raw: u16) -> u16 {
        scale(raw, 168, 68)
    }
    /// ADC1: VPP through 100k/22k.
    pub const fn vpp_mv(raw: u16) -> u16 {
        scale(raw, 122, 22)
    }
    /// ADC2: USB 5 V through 10k/10k.
    pub const fn vbus_mv(raw: u16) -> u16 {
        scale(raw, 2, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_roundtrip() {
        for count in 1..=32 {
            for msb in [false, true] {
                assert_eq!(decode_shape(shape(count, msb)), Some((count, msb)));
            }
        }
        assert_eq!(decode_shape(0x3F), None);
    }

    #[test]
    fn wire_order() {
        // 0b100110, LSb first: 0,1,1,0,0,1.
        let lsb: [bool; 6] = core::array::from_fn(|i| wire_bit(0b100110, 6, false, i as u32));
        assert_eq!(lsb, [false, true, true, false, false, true]);
        let msb: [bool; 8] = core::array::from_fn(|i| wire_bit(0x80, 8, true, i as u32));
        assert!(msb[0] && !msb[1..].iter().any(|&b| b));
    }

    #[test]
    fn store_inverts_wire() {
        for msb in [false, true] {
            let mut v = 0;
            for i in 0..24 {
                v = store_bit(v, 24, msb, i, wire_bit(0xA5C3E1, 24, msb, i));
            }
            assert_eq!(v, 0xA5C3E1);
        }
    }

    #[test]
    fn op_lengths() {
        assert_eq!(op_len(op::OUT), Some(6));
        assert_eq!(op_len(0xFF), None);
        assert_eq!(op_reply_len(op::IN), 4);
    }

    #[test]
    fn board_loops() {
        use board::*;
        // Model of the board: output = a - b * dac.
        let out = |l: Loop, duty: u16| l.a_mv - l.b_mv_per_v * (duty as i32 * 3300 / 4096) / 1000;
        for (l, range) in [(VPP_LOOP, VPP_RANGE_MV), (VDD_LOOP, VDD_RANGE_MV)] {
            for target in [*range.start(), 9000.min(*range.end()), *range.end()] {
                let d = l.duty_for(target);
                assert!((out(l, d) - target as i32).abs() < 20, "{target}");
            }
        }
        // A 10 % gain error converges in a few steps.
        let mut d = VPP_LOOP.duty_for(9000);
        for _ in 0..8 {
            let real = 13904 - 2174 * 11 / 10 * (d as i32 * 3300 / 4096) / 1000;
            d = VPP_LOOP.correct(d, real as u16, 9000);
        }
        let real = 13904 - 2174 * 11 / 10 * (d as i32 * 3300 / 4096) / 1000;
        assert!((real - 9000).abs() < VPP_TOLERANCE_MV);
        // Dividers: 4095 counts = 3.3 V at the pin.
        assert_eq!(tvdd_mv(4095), 8152);
        assert_eq!(vpp_mv(2048), 9150);
        assert_eq!(vbus_mv(3103), 5000);
    }
}
