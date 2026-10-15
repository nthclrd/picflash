//! Raspberry Pi Pico prototype: target powered at 3.3 V by the Pico.
//!   GP2 -> ICSPCLK    GP3 <-> ICSPDAT    GP4 -> MCLR
//!   3V3(OUT) -> VDD   GND -> VSS
//! No VPP, no target supply control, no measurement. A 5 V target needs level
//! shifting: 3.3 V is below its ICSP input high level.

use embassy_rp::gpio::{Flex, Level, Output, Pull};
use embassy_rp::peripherals::USB;
use embassy_rp::{Peri, Peripherals};
use picflash_protocol::{status, store_bit, wire_bit, GROUP_GAP_US};

use crate::delay_us;

pub struct Hw<'d> {
    clk: Flex<'d>,
    dat: Flex<'d>,
    mclr: Flex<'d>,
    led: Output<'d>,
}

impl<'d> Hw<'d> {
    /// Takes the pins it needs and hands back the USB peripheral.
    pub fn new(p: Peripherals) -> (Self, Peri<'static, USB>) {
        let mut hw = Self {
            clk: Flex::new(p.PIN_2),
            dat: Flex::new(p.PIN_3),
            mclr: Flex::new(p.PIN_4),
            led: Output::new(p.PIN_25, Level::Low),
        };
        hw.dat.set_pull(Pull::Down);
        hw.release();
        (hw, p.USB)
    }

    pub fn caps(&self) -> u32 {
        0
    }

    pub fn led(&mut self, active: bool, _error: bool) {
        self.led.set_level(Level::from(active));
    }

    pub fn release(&mut self) {
        self.clk.set_as_input();
        self.dat.set_as_input();
        self.mclr.set_high();
        self.mclr.set_as_output();
    }

    pub fn enter(&mut self) {
        self.clk.set_low();
        self.clk.set_as_output();
        self.dat.set_low();
        self.dat.set_as_output();
        self.mclr.set_low();
        self.mclr.set_as_output();
    }

    pub fn enter_hv(&mut self) -> Result<(), u8> {
        Err(status::UNKNOWN_OP)
    }

    /// Clocks out `count` bits. The target latches on the falling edge.
    pub fn shift_out(&mut self, bits: u32, count: u32, msb_first: bool) {
        self.dat.set_as_output();
        for i in 0..count {
            self.dat.set_level(Level::from(wire_bit(bits, count, msb_first, i)));
            self.clk.set_high();
            delay_us(1);
            self.clk.set_low();
            delay_us(1);
        }
        delay_us(GROUP_GAP_US);
    }

    /// Clocks in `count` bits. The target drives on the rising edge; sample
    /// just before the falling edge.
    pub fn shift_in(&mut self, count: u32, msb_first: bool) -> u32 {
        self.dat.set_as_input();
        let mut value = 0;
        for i in 0..count {
            self.clk.set_high();
            delay_us(1);
            value = store_bit(value, count, msb_first, i, self.dat.is_high());
            self.clk.set_low();
            delay_us(1);
        }
        delay_us(GROUP_GAP_US);
        value
    }

    pub fn measure(&mut self) -> u32 {
        0
    }

    /// Only "off" is accepted, so the host's clean-up works on every board.
    pub async fn set_vpp(&mut self, mv: u16) -> Result<(), u8> {
        if mv == 0 { Ok(()) } else { Err(status::UNKNOWN_OP) }
    }

    pub async fn set_vdd(&mut self, mv: u16) -> Result<(), u8> {
        if mv == 0 { Ok(()) } else { Err(status::UNKNOWN_OP) }
    }

    pub fn shutdown(&mut self) {
        self.release();
        self.led(false, false);
    }
}
