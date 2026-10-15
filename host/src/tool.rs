//! Picks and opens the programmer: picflash board, Microchip PICkit / Snap,
//! or the simulator. Shared by the CLI, `picflash-ipecmd` and the GUI.

use anyhow::{bail, Result};

use crate::algo::Entry;
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
    Board(SerialLink),
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
    let boards = board_ports();
    let kits = pickit::scan();
    let mut parts: Vec<String> = boards.iter().map(|p| format!("carte picflash ({p})")).collect();
    for k in &kits {
        match &k.problem {
            None => parts.push(k.kind.name().to_owned()),
            Some(p) => parts.push(p.clone()),
        }
    }
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
            if opts.entry == Some(Entry::HighVoltage) {
                bail!("la carte picflash n'a pas de VPP : seule l'entrée basse tension (LVP) est possible");
            }
            if opts.power_mv.is_some() {
                bail!("la carte picflash n'alimente pas la cible");
            }
            let link = SerialLink::open(opts.port.as_deref())?;
            Ok(Opened {
                link: Link::Board(link),
                entry: Entry::LowVoltage,
                summary: "carte picflash, entrée basse tension".into(),
            })
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
            let vdd = match kit.vdd_mv {
                Some(mv) => format!("{:.2} V", mv as f64 / 1000.0),
                None => "VDD non mesuré".into(),
            };
            let summary = format!(
                "{} (firmware {}{}), {}, cible {vdd}{}",
                kit.kind.name(),
                kit.firmware,
                kit.serial.as_deref().map(|s| format!(", n° {s}")).unwrap_or_default(),
                entry_name(entry),
                if opts.power_mv.is_some() { " fournie par l'outil" } else { "" },
            );
            let algo = device.algorithm()?;
            Ok(Opened { link: Link::PicKit(Box::new(Gen4Link::new(kit, algo))), entry, summary })
        }
        Choice::Sim => Ok(Opened {
            link: Link::Sim(Box::new(SimPic::new(device)?)),
            entry: opts.entry.unwrap_or_default(),
            summary: format!("simulateur, {}", entry_name(opts.entry.unwrap_or_default())),
        }),
        Choice::Auto => unreachable!(),
    }
}

pub fn entry_name(entry: Entry) -> &'static str {
    match entry {
        Entry::LowVoltage => "entrée basse tension (LVP)",
        Entry::HighVoltage => "entrée haute tension (VPP)",
    }
}
