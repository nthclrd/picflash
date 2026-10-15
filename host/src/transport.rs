//! Script batching and the USB serial link to the programmer board.

use std::io::{Read, Write};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use picflash_protocol::{op, op_reply_len, shape, status, MAX_PAYLOAD, USB_PID, USB_VID};

/// Runs one script frame on the target and returns the values clocked in.
pub trait Transport {
    fn run(&mut self, script: &[u8]) -> Result<Vec<u32>>;
}

/// Accumulates bit-level ops and sends them in frames that fit
/// [`MAX_PAYLOAD`]. Target state persists between frames, so splitting is safe.
pub struct Batch<'a, T: Transport + ?Sized> {
    link: &'a mut T,
    script: Vec<u8>,
    reply_len: usize,
    reads: Vec<u32>,
}

impl<'a, T: Transport + ?Sized> Batch<'a, T> {
    pub fn new(link: &'a mut T) -> Self {
        Self {
            link,
            script: Vec::with_capacity(MAX_PAYLOAD),
            reply_len: 1,
            reads: Vec::new(),
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Result<()> {
        let extra_reply = op_reply_len(bytes[0]);
        if self.script.len() + bytes.len() > MAX_PAYLOAD
            || self.reply_len + extra_reply > MAX_PAYLOAD
        {
            self.flush()?;
        }
        self.script.extend_from_slice(bytes);
        self.reply_len += extra_reply;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        if !self.script.is_empty() {
            let words = self.link.run(&self.script)?;
            self.reads.extend(words);
            self.script.clear();
            self.reply_len = 1;
        }
        Ok(())
    }

    pub fn ping(&mut self) -> Result<()> {
        self.push(&[op::PING])
    }
    /// Drives ICSPCLK, ICSPDAT and MCLR low.
    pub fn enter(&mut self) -> Result<()> {
        self.push(&[op::ENTER])
    }
    /// Drives ICSPCLK/ICSPDAT low and MCLR to VPP.
    pub fn enter_hv(&mut self) -> Result<()> {
        self.push(&[op::ENTER_HV])
    }
    /// Releases the ICSP lines and raises MCLR.
    pub fn release(&mut self) -> Result<()> {
        self.push(&[op::RELEASE])
    }
    /// Clocks out the low `count` bits of `bits`.
    pub fn out(&mut self, bits: u32, count: u32, msb_first: bool) -> Result<()> {
        let [a, b, c, d] = bits.to_le_bytes();
        self.push(&[op::OUT, shape(count, msb_first), a, b, c, d])
    }
    /// Clocks in `count` bits; the value lands in the reads.
    pub fn input(&mut self, count: u32, msb_first: bool) -> Result<()> {
        self.push(&[op::IN, shape(count, msb_first)])
    }
    pub fn delay_us(&mut self, us: u32) -> Result<()> {
        // Waits longer than one op can hold are split.
        let mut left = us;
        while left > 0 {
            let chunk = left.min(u16::MAX as u32) as u16;
            let [lo, hi] = chunk.to_le_bytes();
            self.push(&[op::DELAY_US, lo, hi])?;
            left -= chunk as u32;
        }
        Ok(())
    }

    /// Sends anything pending and returns every value read so far.
    pub fn finish(mut self) -> Result<Vec<u32>> {
        self.flush()?;
        Ok(self.reads)
    }
}

pub struct SerialLink {
    port: Box<dyn serialport::SerialPort>,
}

impl SerialLink {
    /// Opens `path`, or the first port with the programmer's USB IDs.
    pub fn open(path: Option<&str>) -> Result<Self> {
        let path = match path {
            Some(p) => p.to_owned(),
            None => find_port()?,
        };
        let port = serialport::new(&path, 115_200)
            .timeout(Duration::from_secs(5))
            .open()
            .with_context(|| format!("cannot open {path}"))?;
        Ok(Self { port })
    }
}

/// Serial ports of connected picflash boards (matched by USB ID).
pub fn board_ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| match &p.port_type {
            serialport::SerialPortType::UsbPort(usb) => usb.vid == USB_VID && usb.pid == USB_PID,
            _ => false,
        })
        .map(|p| p.port_name)
        // macOS lists both tty.* and cu.*; cu.* does not wait for DCD.
        .filter(|name| !name.contains("/tty."))
        .collect()
}

fn find_port() -> Result<String> {
    match board_ports().into_iter().next() {
        Some(port) => Ok(port),
        None => bail!("no programmer found ({USB_VID:04x}:{USB_PID:04x}); pass --port"),
    }
}

impl Transport for SerialLink {
    fn run(&mut self, script: &[u8]) -> Result<Vec<u32>> {
        let mut frame = Vec::with_capacity(script.len() + 2);
        frame.extend_from_slice(&(script.len() as u16).to_le_bytes());
        frame.extend_from_slice(script);
        self.port.write_all(&frame)?;
        self.port.flush()?;

        let mut len = [0u8; 2];
        self.port.read_exact(&mut len).context("no reply from programmer")?;
        let len = u16::from_le_bytes(len) as usize;
        if len == 0 || len > MAX_PAYLOAD {
            bail!("bad reply length {len}");
        }
        let mut payload = vec![0u8; len];
        self.port.read_exact(&mut payload)?;
        match payload[0] {
            status::OK => {}
            status::UNKNOWN_OP => bail!("carte : opération inconnue (firmware trop ancien ?)"),
            status::TRUNCATED => bail!("carte : script tronqué"),
            status::REPLY_OVERFLOW => bail!("carte : réponse trop longue"),
            status::VOLTAGE_RANGE => bail!("carte : tension demandée hors de sa plage"),
            status::VPP_FAIL => bail!("carte : VPP n'atteint pas la tension demandée (MCLR en court-circuit ?)"),
            status::VDD_FAIL => bail!("carte : l'alimentation de la cible n'atteint pas la tension demandée (court-circuit ou trop de courant ?)"),
            status::EXTERNAL_POWER => bail!("carte : la cible est déjà alimentée, n'active pas l'alimentation par la carte"),
            status::NO_VPP => bail!("carte : entrée haute tension sans VPP réglé"),
            s => bail!("programmer: status {s:#04x}"),
        }
        Ok(payload[1..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect())
    }
}
