//! picflash firmware for the RP2040: runs bit-level ICSP scripts sent by the
//! host over USB CDC. All PIC family knowledge lives in the host tool.
//!
//! Wiring (target powered at 3.3 V from the Pico):
//!   GP2 -> ICSPCLK    GP3 <-> ICSPDAT    GP4 -> MCLR
//!   3V3(OUT) -> VDD   GND -> VSS
//! A 5 V target needs level shifting: 3.3 V is below its ICSP input high level.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Flex, Level, Output, Pull};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config};
use panic_halt as _;
use picflash_protocol::{
    decode_shape, op, op_len, op_reply_len, status, store_bit, wire_bit, GROUP_GAP_US,
    MAX_PAYLOAD, PROTOCOL_VERSION, USB_PID, USB_VID,
};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

const PACKET: usize = 64;
/// Core clock cycles per microsecond at embassy-rp's default 125 MHz.
const CYCLES_PER_US: u32 = 125;

type Class<'d> = CdcAcmClass<'d, Driver<'d, USB>>;

/// ICSP lines. All are released (inputs) outside programming so the target's
/// application can use RB6/RB7.
struct Icsp<'d> {
    clk: Flex<'d>,
    dat: Flex<'d>,
    mclr: Flex<'d>,
}

fn delay_us(us: u32) {
    cortex_m::asm::delay(us * CYCLES_PER_US);
}

impl<'d> Icsp<'d> {
    fn release(&mut self) {
        self.clk.set_as_input();
        self.dat.set_as_input();
        self.mclr.set_high();
        self.mclr.set_as_output();
    }

    fn enter(&mut self) {
        self.clk.set_low();
        self.clk.set_as_output();
        self.dat.set_low();
        self.dat.set_as_output();
        self.mclr.set_low();
        self.mclr.set_as_output();
    }

    /// Clocks out `count` bits. The target latches on the falling edge.
    fn shift_out(&mut self, bits: u32, count: u32, msb_first: bool) {
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
    fn shift_in(&mut self, count: u32, msb_first: bool) -> u32 {
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
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut config = Config::new(USB_VID, USB_PID);
    config.manufacturer = Some("picflash");
    config.product = Some("picflash PIC programmer");
    config.serial_number = Some("0001");
    config.max_power = 100;
    config.max_packet_size_0 = PACKET as u8;

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];
    let mut state = State::new();
    let mut builder = Builder::new(
        Driver::new(p.USB, Irqs),
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [],
        &mut control_buf,
    );
    let mut class = CdcAcmClass::new(&mut builder, &mut state, PACKET as u16);
    let mut usb = builder.build();

    let mut icsp = Icsp {
        clk: Flex::new(p.PIN_2),
        dat: Flex::new(p.PIN_3),
        mclr: Flex::new(p.PIN_4),
    };
    icsp.dat.set_pull(Pull::Down);
    icsp.release();
    let mut led = Output::new(p.PIN_25, Level::Low);

    let app = async {
        loop {
            class.wait_connection().await;
            let _ = serve(&mut class, &mut icsp, &mut led).await;
            icsp.release();
            led.set_low();
        }
    };
    join(usb.run(), app).await;
}

async fn serve(
    class: &mut Class<'_>,
    icsp: &mut Icsp<'_>,
    led: &mut Output<'_>,
) -> Result<(), EndpointError> {
    let mut rx = [0u8; MAX_PAYLOAD + 2];
    let mut tx = [0u8; MAX_PAYLOAD + 2];
    loop {
        let reply_len = match recv_frame(class, &mut rx).await? {
            Some(len) => {
                led.set_high();
                run_script(icsp, &rx[2..2 + len], &mut tx[2..]).await
            }
            None => {
                tx[2] = status::TRUNCATED;
                1
            }
        };
        tx[..2].copy_from_slice(&(reply_len as u16).to_le_bytes());
        send(class, &tx[..2 + reply_len]).await?;
        led.set_low();
    }
}

/// Reads one `[len][payload]` frame into `buf`. `None` if the length is bad.
async fn recv_frame(class: &mut Class<'_>, buf: &mut [u8]) -> Result<Option<usize>, EndpointError> {
    let mut packet = [0u8; PACKET];
    let mut have = 0;
    loop {
        if have >= 2 {
            let len = u16::from_le_bytes([buf[0], buf[1]]) as usize;
            if len == 0 || len > MAX_PAYLOAD {
                return Ok(None);
            }
            if have >= len + 2 {
                return Ok(Some(len));
            }
        }
        let n = class.read_packet(&mut packet).await?;
        let n = n.min(buf.len() - have);
        buf[have..have + n].copy_from_slice(&packet[..n]);
        have += n;
    }
}

async fn send(class: &mut Class<'_>, data: &[u8]) -> Result<(), EndpointError> {
    for chunk in data.chunks(PACKET) {
        class.write_packet(chunk).await?;
    }
    // A full last packet needs a zero-length packet to end the transfer.
    if data.len().is_multiple_of(PACKET) {
        class.write_packet(&[]).await?;
    }
    Ok(())
}

/// Runs `script`, writing `[status][values...]` into `reply`. Returns its length.
async fn run_script(icsp: &mut Icsp<'_>, script: &[u8], reply: &mut [u8]) -> usize {
    reply[0] = status::OK;
    let mut out = 1;
    let mut i = 0;
    while i < script.len() {
        let code = script[i];
        let error = match op_len(code) {
            None => Some(status::UNKNOWN_OP),
            Some(len) if i + len > script.len() => Some(status::TRUNCATED),
            Some(_) if out + op_reply_len(code) > MAX_PAYLOAD => Some(status::REPLY_OVERFLOW),
            Some(_) => None,
        };
        if let Some(error) = error {
            reply[0] = error;
            return 1;
        }
        let len = op_len(code).unwrap_or(1);
        let arg = &script[i + 1..i + len];
        match code {
            op::PING => {
                reply[out..out + 4].copy_from_slice(&(PROTOCOL_VERSION as u32).to_le_bytes());
                out += 4;
            }
            op::ENTER => icsp.enter(),
            op::RELEASE => icsp.release(),
            op::OUT | op::IN => {
                let Some((count, msb_first)) = decode_shape(arg[0]) else {
                    reply[0] = status::UNKNOWN_OP;
                    return 1;
                };
                if code == op::OUT {
                    let bits = u32::from_le_bytes([arg[1], arg[2], arg[3], arg[4]]);
                    icsp.shift_out(bits, count, msb_first);
                } else {
                    let value = icsp.shift_in(count, msb_first);
                    reply[out..out + 4].copy_from_slice(&value.to_le_bytes());
                    out += 4;
                }
            }
            op::DELAY_US => {
                Timer::after_micros(u16::from_le_bytes([arg[0], arg[1]]) as u64).await
            }
            // ENTER_HV needs a VPP supply this board does not have.
            _ => {
                reply[0] = status::UNKNOWN_OP;
                return 1;
            }
        }
        i += len;
    }
    out
}
