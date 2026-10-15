//! Picks and opens the programmer: picflash board, Microchip PICkit / Snap,
//! or the simulator. Shared by the CLI, `picflash-ipecmd` and the GUI.

use anyhow::{bail, Result};

use crate::algo::Entry;
use crate::board::{self, BoardLink};
use crate::device::Device;
use crate::pickit::{self, Gen4Link, PicKit, Setup};
use crate::sim::SimPic;
use crate::transport::{board_ports, SerialLink, Transport};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Choice {
    /// picflash board if one is plugged in, else a PICkit / Snap.
    #[default]
    Auto,
    Board,
    PicKit,
    Sim,
}

/// Entry mode as asked by the user; `None` picks the tool's default.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub entry: Option<Entry>,
    pub power_mv: Option<u16>,
    /// Serial port (board) or serial number (PICkit).
    pub port: Option<String>,
}

pub enum Link {
    Board(Box<BoardLink<SerialLink>>),
    PicKit(Box<Gen4Link<PicKit>>),
    Sim(Box<SimPic>),
}

impl Transport for Link {
    fn run(&mut self, script: &[u8]) -> Result<Vec<u32>> {
        match self {
            Link::Board(l) => l.run(script),
            Link::PicKit(l) => l.run(script),
            Link::Sim(l) => l.run(script),
        }
    }
}

pub struct Opened {
    pub link: Link,
    pub entry: Entry,
    /// One line for the log: tool, firmware, voltage.
    pub summary: String,
}

/// What is plugged in, for status displays.
pub fn detect() -> String {
    let mut parts: Vec<String> = board_ports().iter().map(|p| format!("carte picflash ({p})")).collect();
    parts.extend(pickit::scan().into_iter().map(|k| k.problem.unwrap_or_else(|| k.kind.name().to_owned())));
    if parts.is_empty() {
        "aucun programmateur détecté".into()
    } else {
        parts.join(", ")
    }
}

pub fn open(choice: Choice, device: &Device, opts: &Options) -> Result<Opened> {
    let choice = match choice {
        Choice::Auto if !board_ports().is_empty() => Choice::Board,
        Choice::Auto if pickit::scan().iter().any(|k| k.usable) => Choice::PicKit,
        Choice::Auto => {
            if let Some(problem) = pickit::scan().into_iter().find_map(|k| k.problem) {
                bail!("{problem}");
            }
            bail!("aucun programmateur détecté (carte picflash, PICkit 4/5/Basic ou Snap)")
        }
        c => c,
    };
    match choice {
        Choice::Board => {
            let link = SerialLink::open(opts.port.as_deref())?;
            let (link, entry, summary) = board::setup(link, device, opts.entry, opts.power_mv)?;
            Ok(Opened { link: Link::Board(Box::new(link)), entry, summary })
        }
        Choice::PicKit => {
            let mut kit = PicKit::open(opts.port.as_deref())?;
            // High voltage like MPLAB IPE when the tool can, LVP otherwise.
            let entry = opts.entry.unwrap_or(if kit.kind.has_hv() {
                Entry::HighVoltage
            } else {
                Entry::LowVoltage
            });
            kit.setup(device, Setup { entry, power_mv: opts.power_mv })?;
            let summary = format!(
                "{} (firmware {}{}), {}, cible {}{}",
                kit.kind.name(),
                kit.firmware,
                kit.serial.as_deref().map(|s| format!(", n° {s}")).unwrap_or_default(),
                entry_name(entry),
                kit.vdd_mv.map(board::volts).unwrap_or_else(|| "VDD non mesuré".into()),
                if opts.power_mv.is_some() { " fournie par l'outil" } else { "" },
            );
            let algo = device.algorithm()?;
            Ok(Opened { link: Link::PicKit(Box::new(Gen4Link::new(kit, algo))), entry, summary })
        }
        Choice::Sim => {
            let entry = opts.entry.unwrap_or_default();
            Ok(Opened {
                link: Link::Sim(Box::new(SimPic::new(device)?)),
                entry,
                summary: format!("simulateur, {}", entry_name(entry)),
            })
        }
        Choice::Auto => unreachable!(),
    }
}

pub fn entry_name(entry: Entry) -> &'static str {
    match entry {
        Entry::LowVoltage => "entrée basse tension (LVP)",
        Entry::HighVoltage => "entrée haute tension (VPP)",
    }
}
