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

pub const PROTOCOL_VERSION: u8 = 2;

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
    /// `[shape: u8][bits: u32 LE]` — clock out bits, see [`super::shape`].
    pub const OUT: u8 = 0x10;
    /// `[shape: u8]` — clock in bits; the value goes into the reply.
    pub const IN: u8 = 0x11;
    /// `[us: u16 LE]` — wait.
    pub const DELAY_US: u8 = 0x20;
}

/// Reply status codes.
pub mod status {
    pub const OK: u8 = 0x00;
    pub const UNKNOWN_OP: u8 = 0x01;
    pub const TRUNCATED: u8 = 0x02;
    pub const REPLY_OVERFLOW: u8 = 0x03;
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
        op::PING | op::ENTER | op::RELEASE | op::ENTER_HV => Some(1),
        op::IN => Some(2),
        op::DELAY_US => Some(3),
        op::OUT => Some(6),
        _ => None,
    }
}

/// Bytes an op adds to the reply.
pub const fn op_reply_len(opcode: u8) -> usize {
    match opcode {
        op::PING | op::IN => 4,
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
}
