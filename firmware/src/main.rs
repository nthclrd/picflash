//! picflash firmware for the RP2040: runs bit-level ICSP scripts sent by the
//! host over USB CDC. All PIC family knowledge lives in the host tool; the
//! firmware drives the lines and, on the picflash board, VPP and the target
//! supply (see `hw_board.rs`).
//!
//! Build for the board (default) or a Pi Pico prototype:
//!   cargo build --release
//!   cargo build --release --no-default-features --features pico

#![no_std]
#![no_main]

#[cfg(all(feature = "board", feature = "pico"))]
compile_error!("choose one of the `board` and `pico` features");

#[cfg_attr(feature = "board", path = "hw_board.rs")]
#[cfg_attr(feature = "pico", path = "hw_pico.rs")]
mod hw;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config};
use hw::Hw;
use panic_halt as _;
use picflash_protocol::{
    decode_shape, op, op_len, op_reply_len, status, MAX_PAYLOAD, PROTOCOL_VERSION, USB_PID, USB_VID,
};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

const PACKET: usize = 64;
/// Core clock cycles per microsecond at embassy-rp's default 125 MHz.
const CYCLES_PER_US: u32 = 125;

type Class<'d> = CdcAcmClass<'d, Driver<'d, USB>>;

pub(crate) fn delay_us(us: u32) {
    cortex_m::asm::delay(us * CYCLES_PER_US);
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let (mut hw, usb_periph) = Hw::new(embassy_rp::init(Default::default()));
    let usb_driver = Driver::new(usb_periph, Irqs);

    let mut config = Config::new(USB_VID, USB_PID);
    config.manufacturer = Some("picflash");
    config.product = Some("picflash PIC programmer");
    config.serial_number = Some("0001");
    config.max_power = 250;
    config.max_packet_size_0 = PACKET as u8;

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];
    let mut state = State::new();
    let mut builder = Builder::new(
        usb_driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [],
        &mut control_buf,
    );
    let mut class = CdcAcmClass::new(&mut builder, &mut state, PACKET as u16);
    let mut usb = builder.build();

    let app = async {
        loop {
            class.wait_connection().await;
            let _ = serve(&mut class, &mut hw).await;
            hw.shutdown();
        }
    };
    join(usb.run(), app).await;
}

async fn serve(class: &mut Class<'_>, hw: &mut Hw<'_>) -> Result<(), EndpointError> {
    let mut rx = [0u8; MAX_PAYLOAD + 2];
    let mut tx = [0u8; MAX_PAYLOAD + 2];
    loop {
        let reply_len = match recv_frame(class, &mut rx).await? {
            Some(len) => {
                hw.led(true, false);
                run_script(hw, &rx[2..2 + len], &mut tx[2..]).await
            }
            None => {
                tx[2] = status::TRUNCATED;
                1
            }
        };
        tx[..2].copy_from_slice(&(reply_len as u16).to_le_bytes());
        send(class, &tx[..2 + reply_len]).await?;
        hw.led(false, tx[2] != status::OK);
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
async fn run_script(hw: &mut Hw<'_>, script: &[u8], reply: &mut [u8]) -> usize {
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
        let u16_arg = || u16::from_le_bytes([arg[0], arg[1]]);
        let mut value = None;
        let result = match code {
            op::PING => {
                value = Some(PROTOCOL_VERSION as u32);
                Ok(())
            }
            op::CAPS => {
                value = Some(hw.caps());
                Ok(())
            }
            op::MEASURE => {
                value = Some(hw.measure());
                Ok(())
            }
            op::ENTER => {
                hw.enter();
                Ok(())
            }
            op::ENTER_HV => hw.enter_hv(),
            op::RELEASE => {
                hw.release();
                Ok(())
            }
            op::OUT | op::IN => match decode_shape(arg[0]) {
                None => Err(status::UNKNOWN_OP),
                Some((count, msb_first)) => {
                    if code == op::OUT {
                        let bits = u32::from_le_bytes([arg[1], arg[2], arg[3], arg[4]]);
                        hw.shift_out(bits, count, msb_first);
                    } else {
                        value = Some(hw.shift_in(count, msb_first));
                    }
                    Ok(())
                }
            },
            op::DELAY_US => {
                Timer::after_micros(u16_arg() as u64).await;
                Ok(())
            }
            op::SET_VPP => hw.set_vpp(u16_arg()).await,
            op::SET_VDD => hw.set_vdd(u16_arg()).await,
            _ => Err(status::UNKNOWN_OP),
        };
        if let Err(e) = result {
            reply[0] = e;
            return 1;
        }
        if let Some(v) = value {
            reply[out..out + 4].copy_from_slice(&v.to_le_bytes());
            out += 4;
        }
        i += len;
    }
    out
}
