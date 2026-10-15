//! picflash board (hardware/picflash-board.kicad_sch).
//!
//! ICSP lines go through SN74LVC1T45 shifters: DIR = 1 drives the target,
//! DIR = 0 reads it (and leaves the target side high impedance). DIR pins have
//! pull-downs, so every line is released at reset. MCLR has three drivers:
//! Q1 (VPP), U10 + D5 (VDD level) and Q3 (ground); none on = released.
//! VPP and the target supply are set by PWM "DACs" in their regulators'
//! feedback and checked on the ADC. Both regulators default to off.

use embassy_rp::adc::{self, Adc, Blocking, Channel};
use embassy_rp::gpio::{Flex, Level, Output, Pull};
use embassy_rp::pwm::{self, Pwm};
use embassy_rp::peripherals::USB;
use embassy_rp::{Peri, Peripherals};
use embassy_time::Timer;
use picflash_protocol::board::{self, Loop, VDD_LOOP, VPP_LOOP};
use picflash_protocol::{caps, status, store_bit, wire_bit, GROUP_GAP_US};

use crate::delay_us;

pub struct Hw<'d> {
    pgc: Flex<'d>,
    pgc_dir: Output<'d>,
    pgd: Flex<'d>,
    pgd_dir: Output<'d>,
    mclr_lv: Output<'d>,
    vpp_on: Output<'d>,
    mclr_pd: Output<'d>,
    boost_en: Output<'d>,
    vpp_pwm: Pwm<'d>,
    vt_en: Output<'d>,
    vt_pwm: Pwm<'d>,
    led_act: Output<'d>,
    led_err: Output<'d>,
    adc: Adc<'d, Blocking>,
    ch_tvdd: Channel<'d>,
    ch_vpp: Channel<'d>,
    vpp_ready: bool,
    vdd_on: bool,
}

fn pwm_config(duty: u16) -> pwm::Config {
    let mut c = pwm::Config::default();
    c.top = board::PWM_TOP;
    c.compare_a = duty;
    c
}

impl<'d> Hw<'d> {
    /// Takes the pins it needs and hands back the USB peripheral.
    pub fn new(p: Peripherals) -> (Self, Peri<'static, USB>) {
        let mut pgc = Flex::new(p.PIN_2);
        let mut pgd = Flex::new(p.PIN_4);
        pgc.set_as_input();
        pgd.set_as_input();
        pgd.set_pull(Pull::Down);
        let hw = Self {
            pgc,
            pgc_dir: Output::new(p.PIN_3, Level::Low),
            pgd,
            pgd_dir: Output::new(p.PIN_5, Level::Low),
            mclr_lv: Output::new(p.PIN_12, Level::Low),
            vpp_on: Output::new(p.PIN_13, Level::Low),
            mclr_pd: Output::new(p.PIN_14, Level::Low),
            boost_en: Output::new(p.PIN_15, Level::Low),
            vpp_pwm: Pwm::new_output_a(p.PWM_SLICE0, p.PIN_16, pwm_config(0)),
            vt_en: Output::new(p.PIN_17, Level::Low),
            vt_pwm: Pwm::new_output_a(p.PWM_SLICE1, p.PIN_18, pwm_config(0)),
            led_act: Output::new(p.PIN_20, Level::Low),
            led_err: Output::new(p.PIN_21, Level::Low),
            adc: Adc::new_blocking(p.ADC, adc::Config::default()),
            ch_tvdd: Channel::new_pin(p.PIN_26, Pull::None),
            ch_vpp: Channel::new_pin(p.PIN_27, Pull::None),
            vpp_ready: false,
            vdd_on: false,
        };
        (hw, p.USB)
    }

    pub fn caps(&self) -> u32 {
        caps::HIGH_VOLTAGE | caps::TARGET_POWER | caps::MEASURE
    }

    pub fn led(&mut self, active: bool, error: bool) {
        self.led_act.set_level(Level::from(active));
        self.led_err.set_level(Level::from(error));
    }

    // --- MCLR: break before make between its three drivers. ---------------
    fn mclr_off(&mut self) {
        self.vpp_on.set_low();
        self.mclr_pd.set_low();
        self.mclr_lv.set_low();
        delay_us(1);
    }
    fn mclr_low(&mut self) {
        self.mclr_off();
        self.mclr_pd.set_high();
    }
    fn mclr_vdd(&mut self) {
        self.mclr_off();
        self.mclr_lv.set_high();
    }
    fn mclr_vpp(&mut self) {
        self.mclr_off();
        self.vpp_on.set_high();
    }

    // --- Data lines: never let the RP2040 and a shifter drive A together. --
    fn drive_lines_low(&mut self) {
        self.pgc_dir.set_high();
        self.pgd_dir.set_high();
        self.pgc.set_low();
        self.pgc.set_as_output();
        self.pgd.set_low();
        self.pgd.set_as_output();
    }
    fn release_lines(&mut self) {
        self.pgc.set_as_input();
        self.pgd.set_as_input();
        self.pgc_dir.set_low();
        self.pgd_dir.set_low();
    }

    /// Lines released, MCLR at VDD: the target runs.
    pub fn release(&mut self) {
        self.release_lines();
        self.mclr_vdd();
    }

    pub fn enter(&mut self) {
        self.drive_lines_low();
        self.mclr_low();
    }

    pub fn enter_hv(&mut self) -> Result<(), u8> {
        if !self.vpp_ready {
            return Err(status::NO_VPP);
        }
        self.drive_lines_low();
        self.mclr_vpp();
        Ok(())
    }

    pub fn shift_out(&mut self, bits: u32, count: u32, msb_first: bool) {
        self.pgd_dir.set_high();
        self.pgd.set_as_output();
        for i in 0..count {
            self.pgd.set_level(Level::from(wire_bit(bits, count, msb_first, i)));
            self.pgc.set_high();
            delay_us(1);
            self.pgc.set_low();
            delay_us(1);
        }
        delay_us(GROUP_GAP_US);
    }

    pub fn shift_in(&mut self, count: u32, msb_first: bool) -> u32 {
        self.pgd.set_as_input();
        self.pgd_dir.set_low();
        delay_us(1);
        let mut value = 0;
        for i in 0..count {
            self.pgc.set_high();
            delay_us(1);
            value = store_bit(value, count, msb_first, i, self.pgd.is_high());
            self.pgc.set_low();
            delay_us(1);
        }
        delay_us(GROUP_GAP_US);
        value
    }

    // --- Analog. -----------------------------------------------------------
    fn read(&mut self, vpp: bool) -> u16 {
        let ch = if vpp { &mut self.ch_vpp } else { &mut self.ch_tvdd };
        let mut sum = 0u32;
        for _ in 0..8 {
            sum += self.adc.blocking_read(ch).unwrap_or(0) as u32;
        }
        (sum / 8) as u16
    }
    fn tvdd_mv(&mut self) -> u16 {
        board::tvdd_mv(self.read(false))
    }
    fn vpp_mv(&mut self) -> u16 {
        board::vpp_mv(self.read(true))
    }

    /// `target VDD | VPP << 16`, in mV.
    pub fn measure(&mut self) -> u32 {
        self.tvdd_mv() as u32 | (self.vpp_mv() as u32) << 16
    }

    /// Closes the loop on one regulator; `false` if it never got in range.
    async fn regulate(&mut self, vpp: bool, l: Loop, mv: u16, tolerance: i32) -> bool {
        let mut duty = l.duty_for(mv);
        for _ in 0..25 {
            let measured = if vpp { self.vpp_mv() } else { self.tvdd_mv() };
            if (measured as i32 - mv as i32).abs() <= tolerance {
                return true;
            }
            duty = l.correct(duty, measured, mv);
            if vpp {
                self.vpp_pwm.set_config(&pwm_config(duty));
            } else {
                self.vt_pwm.set_config(&pwm_config(duty));
            }
            // RC filter (10k, 1 µF): ~5 τ.
            Timer::after_millis(50).await;
        }
        false
    }

    pub async fn set_vpp(&mut self, mv: u16) -> Result<(), u8> {
        self.vpp_ready = false;
        if self.vpp_on.is_set_high() {
            self.mclr_low();
        }
        if mv == 0 {
            self.boost_en.set_low();
            return Ok(());
        }
        if !board::VPP_RANGE_MV.contains(&mv) {
            self.boost_en.set_low();
            return Err(status::VOLTAGE_RANGE);
        }
        // Set the DAC first: with PWM at 0 the boost would start at 13.9 V.
        self.vpp_pwm.set_config(&pwm_config(VPP_LOOP.duty_for(mv)));
        Timer::after_millis(60).await;
        self.boost_en.set_high();
        Timer::after_millis(10).await;
        if self.regulate(true, VPP_LOOP, mv, board::VPP_TOLERANCE_MV).await {
            self.vpp_ready = true;
            Ok(())
        } else {
            self.boost_en.set_low();
            Err(status::VPP_FAIL)
        }
    }

    pub async fn set_vdd(&mut self, mv: u16) -> Result<(), u8> {
        if mv == 0 {
            self.vt_en.set_low();
            self.vdd_on = false;
            return Ok(());
        }
        if !board::VDD_RANGE_MV.contains(&mv) {
            return Err(status::VOLTAGE_RANGE);
        }
        if !self.vdd_on && self.tvdd_mv() > board::POWERED_MV {
            return Err(status::EXTERNAL_POWER);
        }
        self.vt_pwm.set_config(&pwm_config(VDD_LOOP.duty_for(mv)));
        Timer::after_millis(60).await;
        self.vt_en.set_high();
        self.vdd_on = true;
        Timer::after_millis(5).await;
        if self.regulate(false, VDD_LOOP, mv, board::VDD_TOLERANCE_MV).await {
            Ok(())
        } else {
            self.vt_en.set_low();
            self.vdd_on = false;
            Err(status::VDD_FAIL)
        }
    }

    /// Host gone: everything off, MCLR and lines released.
    pub fn shutdown(&mut self) {
        self.mclr_off();
        self.release_lines();
        self.boost_en.set_low();
        self.vpp_ready = false;
        self.vt_en.set_low();
        self.vdd_on = false;
        self.led(false, false);
    }
}
