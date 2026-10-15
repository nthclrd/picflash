//! Session on the picflash board (firmware protocol v3): version and
//! capability check, target supply, VDD measurement, VPP, and power-off when
//! the session ends.

use anyhow::{bail, Result};
use picflash_protocol::{board, caps, op, PROTOCOL_VERSION};

use crate::algo::Entry;
use crate::device::Device;
use crate::transport::Transport;

/// The board link, turning off what the session switched on when dropped.
pub struct BoardLink<T: Transport> {
    pub link: T,
    vpp_on: bool,
    vdd_on: bool,
}

impl<T: Transport> Transport for BoardLink<T> {
    fn run(&mut self, script: &[u8]) -> Result<Vec<u32>> {
        self.link.run(script)
    }
}

impl<T: Transport> Drop for BoardLink<T> {
    fn drop(&mut self) {
        let mut s = Vec::new();
        if self.vpp_on {
            s.extend_from_slice(&[op::SET_VPP, 0, 0]);
        }
        if self.vdd_on {
            s.extend_from_slice(&[op::SET_VDD, 0, 0]);
        }
        if !s.is_empty() {
            let _ = self.link.run(&s);
        }
    }
}

fn mv16(op: u8, mv: u16) -> [u8; 3] {
    let [lo, hi] = mv.to_le_bytes();
    [op, lo, hi]
}

pub(crate) fn volts(mv: u16) -> String {
    format!("{:.2} V", mv as f64 / 1000.0)
}

/// Prepares the board for `device`. `entry`/`power_mv` as asked by the user.
pub fn setup<T: Transport>(
    link: T,
    device: &Device,
    entry: Option<Entry>,
    power_mv: Option<u16>,
) -> Result<(BoardLink<T>, Entry, String)> {
    let mut b = BoardLink { link, vpp_on: false, vdd_on: false };
    let info = b.run(&[op::PING, op::CAPS]);
    let (version, caps) = match info.as_deref() {
        Ok([v, c]) => (*v, *c),
        // Firmware v2 does not know CAPS.
        _ => bail!(
            "firmware de la carte trop ancien : reflashe picflash-firmware.uf2 (protocole v{PROTOCOL_VERSION})"
        ),
    };
    if version != PROTOCOL_VERSION as u32 {
        bail!("firmware de la carte en protocole v{version}, picflash attend v{PROTOCOL_VERSION} : reflashe la carte");
    }
    let hv = caps & caps::HIGH_VOLTAGE != 0;
    let entry = entry.unwrap_or(if hv { Entry::HighVoltage } else { Entry::LowVoltage });
    if entry == Entry::HighVoltage && !hv {
        bail!("cette carte n'a pas de VPP (prototype Pico) : seule l'entrée basse tension (LVP) est possible");
    }
    let p = &device.power;

    if let Some(mv) = power_mv {
        if caps & caps::TARGET_POWER == 0 {
            bail!("cette carte (prototype Pico) n'alimente pas la cible");
        }
        if !board::VDD_RANGE_MV.contains(&mv) {
            bail!(
                "la carte fournit {} à {}, pas {}",
                volts(*board::VDD_RANGE_MV.start()),
                volts(*board::VDD_RANGE_MV.end()),
                volts(mv)
            );
        }
        if p.vdd_max_mv != 0 && !(p.vdd_min_mv..=p.vdd_max_mv).contains(&mv) {
            bail!("{} accepte {} à {}, pas {}", device.name, volts(p.vdd_min_mv), volts(p.vdd_max_mv), volts(mv));
        }
        b.run(&mv16(op::SET_VDD, mv))?;
        b.vdd_on = true;
    }

    let vdd = if caps & caps::MEASURE != 0 {
        let vdd = (b.run(&[op::MEASURE])?[0] & 0xFFFF) as u16;
        if vdd < board::POWERED_MV {
            bail!(
                "cible non alimentée (VDD mesuré {}) : alimente la carte cible ou demande l'alimentation par la carte",
                volts(vdd)
            );
        }
        if entry == Entry::LowVoltage && p.lvp_min_mv != 0 && vdd < p.lvp_min_mv {
            bail!(
                "VDD {} sous le minimum de {} pour l'entrée basse tension ({})",
                volts(vdd),
                device.name,
                volts(p.lvp_min_mv)
            );
        }
        Some(vdd)
    } else {
        None
    };

    if entry == Entry::HighVoltage {
        if !board::VPP_RANGE_MV.contains(&p.vpp_mv) {
            bail!("{} : VPP {} hors de la plage de la carte (6,8-13,8 V)", device.name, volts(p.vpp_mv));
        }
        b.run(&mv16(op::SET_VPP, p.vpp_mv))?;
        b.vpp_on = true;
    }

    let summary = format!(
        "carte picflash (protocole v{version}), {}{}, cible {}{}",
        crate::tool::entry_name(entry),
        if entry == Entry::HighVoltage { format!(" à {}", volts(p.vpp_mv)) } else { String::new() },
        vdd.map(volts).unwrap_or_else(|| "VDD non mesuré".into()),
        if b.vdd_on { " fournie par la carte" } else { "" },
    );
    Ok((b, entry, summary))
}
