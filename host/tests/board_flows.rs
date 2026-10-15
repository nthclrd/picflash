//! Board session (firmware protocol v3) against a model of the firmware:
//! power ops are handled here, ICSP ops go to the simulated PIC.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{bail, Result};
use picflash::algo::Entry;
use picflash::board;
use picflash::device::Device;
use picflash::image::Image;
use picflash::programmer::Programmer;
use picflash::sim::SimPic;
use picflash::transport::Transport;
use picflash_protocol::{caps, op, op_len, PROTOCOL_VERSION};

struct FakeBoard {
    pic: SimPic,
    caps: u32,
    version: u32,
    /// Target supply from the target's own board, mV (0 = none).
    external_mv: u16,
    vdd_mv: u16,
    vpp_mv: u16,
    /// Every power op seen: (op, mV).
    log: Vec<(u8, u16)>,
}

impl FakeBoard {
    fn new(device: &Device, caps: u32, external_mv: u16) -> Self {
        Self { pic: SimPic::new(device).unwrap(), caps, version: PROTOCOL_VERSION as u32, external_mv, vdd_mv: 0, vpp_mv: 0, log: vec![] }
    }
    fn target_mv(&self) -> u16 {
        self.external_mv.max(self.vdd_mv)
    }
}

impl Transport for FakeBoard {
    fn run(&mut self, script: &[u8]) -> Result<Vec<u32>> {
        let mut out = vec![];
        let mut i = 0;
        while i < script.len() {
            let code = script[i];
            let len = op_len(code).unwrap();
            let arg = |k: usize| script[i + k];
            let mv = || u16::from_le_bytes([arg(1), arg(2)]);
            match code {
                op::PING => out.push(self.version),
                op::CAPS => out.push(self.caps),
                op::MEASURE => out.push(self.target_mv() as u32 | (self.vpp_mv as u32) << 16),
                op::SET_VPP => {
                    self.log.push((code, mv()));
                    if mv() != 0 && self.caps & caps::HIGH_VOLTAGE == 0 {
                        bail!("unknown op");
                    }
                    self.vpp_mv = mv();
                }
                op::SET_VDD => {
                    self.log.push((code, mv()));
                    if mv() != 0 && self.vdd_mv == 0 && self.external_mv > 1000 {
                        bail!("carte : la cible est déjà alimentée");
                    }
                    self.vdd_mv = mv();
                }
                op::ENTER_HV if self.vpp_mv == 0 => bail!("carte : entrée haute tension sans VPP réglé"),
                _ => {
                    if self.target_mv() < 1000 {
                        bail!("PIC sans alimentation");
                    }
                    out.extend(self.pic.run(&script[i..i + len])?);
                }
            }
            i += len;
        }
        Ok(out)
    }
}

fn device() -> Device {
    let mut d = Device::from_pic_xml(include_str!("fixtures/sample.PIC")).unwrap();
    d.power.vpp_mv = 9000;
    d.power.vdd_min_mv = 1800;
    d.power.vdd_max_mv = 5500;
    d.power.lvp_min_mv = 1800;
    d
}

const FULL: u32 = caps::HIGH_VOLTAGE | caps::TARGET_POWER | caps::MEASURE;

fn write_and_verify(link: &mut impl Transport, d: &Device, entry: Entry) {
    let image = Image::parse_hex(include_str!("blink.hex"), d).unwrap();
    let mut prog = Programmer::new(link, d).unwrap();
    prog.entry = entry;
    prog.check_id().unwrap();
    prog.erase().unwrap();
    prog.write(&image, false).unwrap();
    assert!(prog.verify(&image).unwrap().is_empty());
    prog.release().unwrap();
}

#[test]
fn high_voltage_on_externally_powered_target() {
    let d = device();
    let (mut link, entry, summary) = board::setup(FakeBoard::new(&d, FULL, 5000), &d, None, None).unwrap();
    assert_eq!(entry, Entry::HighVoltage, "VPP by default, like MPLAB IPE");
    assert!(summary.contains("9.00 V") && summary.contains("5.00 V"), "{summary}");
    write_and_verify(&mut link, &d, entry);
    assert_eq!(link.link.log, [(op::SET_VPP, 9000)]);
}

/// Records every script, readable after the link is dropped.
struct Rec(Rc<RefCell<Vec<Vec<u8>>>>, FakeBoard);
impl Transport for Rec {
    fn run(&mut self, s: &[u8]) -> Result<Vec<u32>> {
        self.0.borrow_mut().push(s.to_vec());
        self.1.run(s)
    }
}

#[test]
fn powers_the_target_then_switches_everything_off() {
    let d = device();
    let log = Rc::new(RefCell::new(vec![]));
    let rec = Rec(log.clone(), FakeBoard::new(&d, FULL, 0));
    let (mut link, entry, summary) = board::setup(rec, &d, Some(Entry::HighVoltage), Some(3300)).unwrap();
    assert!(summary.contains("fournie par la carte") && summary.contains("3.30 V"), "{summary}");
    // Order: identify, supply on, check VDD, then VPP.
    assert_eq!(
        log.borrow()[..4],
        [vec![op::PING, op::CAPS], vec![op::SET_VDD, 0xE4, 0x0C], vec![op::MEASURE], vec![op::SET_VPP, 0x28, 0x23]]
    );
    write_and_verify(&mut link, &d, entry);
    drop(link);
    assert_eq!(log.borrow().last().unwrap(), &[op::SET_VPP, 0, 0, op::SET_VDD, 0, 0]);
}

#[test]
fn nothing_to_switch_off_on_external_lvp() {
    let d = device();
    let log = Rc::new(RefCell::new(vec![]));
    let (link, _, _) =
        board::setup(Rec(log.clone(), FakeBoard::new(&d, FULL, 3300)), &d, Some(Entry::LowVoltage), None).unwrap();
    drop(link);
    assert!(log.borrow().iter().all(|s| s[0] != op::SET_VPP && s[0] != op::SET_VDD));
}

#[test]
fn refusals() {
    let d = device();
    let err = |r: Result<(board::BoardLink<FakeBoard>, Entry, String)>| format!("{:#}", r.err().expect("should fail"));

    // Board refuses to power an already powered target.
    let e = err(board::setup(FakeBoard::new(&d, FULL, 5000), &d, None, Some(3300)));
    assert!(e.contains("déjà alimentée"), "{e}");
    // Nothing powers the target.
    let e = err(board::setup(FakeBoard::new(&d, FULL, 0), &d, None, None));
    assert!(e.contains("non alimentée"), "{e}");
    // Out of the board's range.
    let e = err(board::setup(FakeBoard::new(&d, FULL, 0), &d, None, Some(5000)));
    assert!(e.contains("4.80 V"), "{e}");
    // LVP below the part's threshold.
    let e = err(board::setup(FakeBoard::new(&d, FULL, 1500), &d, Some(Entry::LowVoltage), None));
    assert!(e.contains("basse tension"), "{e}");
    // Pico prototype: no VPP, no supply.
    let e = err(board::setup(FakeBoard::new(&d, 0, 3300), &d, Some(Entry::HighVoltage), None));
    assert!(e.contains("pas de VPP"), "{e}");
    let e = err(board::setup(FakeBoard::new(&d, 0, 3300), &d, None, Some(3300)));
    assert!(e.contains("n'alimente pas"), "{e}");
    // Old firmware.
    let mut old = FakeBoard::new(&d, FULL, 3300);
    old.version = 2;
    let e = err(board::setup(old, &d, None, None));
    assert!(e.contains("reflashe"), "{e}");
    // VPP of the part outside the board's pump range.
    let mut hot = device();
    hot.power.vpp_mv = 14500;
    let e = err(board::setup(FakeBoard::new(&hot, FULL, 3300), &hot, Some(Entry::HighVoltage), None));
    assert!(e.contains("hors de la plage"), "{e}");
}

#[test]
fn pico_defaults_to_lvp_without_measuring() {
    let d = device();
    let (mut link, entry, summary) = board::setup(FakeBoard::new(&d, 0, 3300), &d, None, None).unwrap();
    assert_eq!(entry, Entry::LowVoltage);
    assert!(summary.contains("non mesuré"), "{summary}");
    write_and_verify(&mut link, &d, entry);
}
