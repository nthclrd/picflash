//! Step-by-step debugger on the instruction-set simulator: run, pause, step,
//! step over/out, breakpoints, core registers, stack, port pins and RAM.

use std::collections::BTreeSet;
use std::time::Instant;

use eframe::egui::{self, Color32, RichText};
use picflash::device::Device;
use picflash::disasm::{Insn, Line};
use picflash::emu::{Cpu, Stop};
use picflash::image::Image;

use crate::{highlight, GREEN, RED};

const CURRENT: Color32 = Color32::from_rgba_premultiplied(60, 50, 10, 60);
/// Instructions per frame when running flat out.
const FAST_STEPS: u64 = 2_000_000;

pub struct Debugger {
    pub cpu: Cpu,
    pub breakpoints: BTreeSet<u16>,
    running: bool,
    /// Run at the chip's real speed instead of as fast as possible.
    real_time: bool,
    /// Oscillator frequency; one instruction cycle is 4 clocks.
    fosc_mhz: f32,
    last_frame: Option<Instant>,
    /// Why the last run stopped, for the status line.
    status: String,
    follow: bool,
    ram_bank: u8,
    /// Just resumed: a breakpoint on the current PC must not stop at once.
    resuming: bool,
}

impl Debugger {
    pub fn new(device: &Device, image: &Image) -> anyhow::Result<Self> {
        Ok(Self {
            cpu: Cpu::new(device, image)?,
            breakpoints: BTreeSet::new(),
            running: false,
            real_time: true,
            fosc_mhz: 32.0,
            last_frame: None,
            status: "prêt : PC = 0x0000".into(),
            follow: true,
            ram_bank: 0,
            resuming: false,
        })
    }

    fn describe(&mut self, stop: Stop) {
        self.status = match stop {
            Stop::Step | Stop::Limit => format!("PC = {:#06x}", self.cpu.pc),
            Stop::Breakpoint(pc) => format!("point d'arrêt à {pc:#06x}"),
            Stop::Sleep => "SLEEP exécuté".into(),
            Stop::Invalid(word) => format!("instruction invalide {word:#06x} à {:#06x}", self.cpu.pc),
            Stop::StackOverflow => "débordement de pile (16 niveaux)".into(),
            Stop::StackUnderflow => "pile vide (return sans call)".into(),
        };
        if !matches!(stop, Stop::Limit) {
            self.running = false;
        }
        self.follow = true;
    }

    fn step(&mut self) {
        let stop = self.cpu.step();
        self.describe(stop);
    }

    /// Runs a call to completion, or steps any other instruction.
    fn step_over(&mut self) {
        let is_call = matches!(
            self.cpu.insn_at(self.cpu.pc),
            Insn::Jump { op: "call", .. } | Insn::Plain("callw")
        );
        if !is_call {
            return self.step();
        }
        let depth = self.cpu.stack.len();
        let stop = self.cpu.step();
        if !matches!(stop, Stop::Step) {
            return self.describe(stop);
        }
        self.until(|cpu| cpu.stack.len() <= depth);
    }

    /// Runs until the current function returns.
    fn step_out(&mut self) {
        let depth = self.cpu.stack.len();
        if depth == 0 {
            self.status = "pas dans une sous-routine".into();
            return;
        }
        self.until(|cpu| cpu.stack.len() < depth);
    }

    fn until(&mut self, done: impl Fn(&Cpu) -> bool) {
        for _ in 0..50_000_000u64 {
            if done(&self.cpu) {
                return self.describe(Stop::Step);
            }
            if self.breakpoints.contains(&self.cpu.pc) {
                return self.describe(Stop::Breakpoint(self.cpu.pc));
            }
            let stop = self.cpu.step();
            if !matches!(stop, Stop::Step) {
                return self.describe(stop);
            }
        }
        self.status = "toujours pas revenu après 50 M instructions".into();
    }

    /// Advances a running program by one frame's worth of cycles.
    fn tick(&mut self, ctx: &egui::Context) {
        if !self.running {
            self.last_frame = None;
            return;
        }
        let now = Instant::now();
        let stop = if self.real_time {
            // Cycles due since the last frame, capped so a stalled frame
            // does not run a huge burst.
            let elapsed = self.last_frame.map_or(0.0, |p| now.duration_since(p).as_secs_f64());
            let due = (elapsed * self.fosc_mhz as f64 * 1e6 / 4.0) as u64;
            self.run_cycles(due.min(FAST_STEPS))
        } else {
            self.run_cycles(FAST_STEPS)
        };
        self.last_frame = Some(now);
        self.describe(stop);
        self.follow = false;
        ctx.request_repaint();
    }

    /// Runs until `cycles` more instruction cycles have elapsed (most
    /// instructions take 1, jumps and skips 2), or something stops it.
    fn run_cycles(&mut self, cycles: u64) -> Stop {
        let target = self.cpu.cycles + cycles;
        loop {
            let left = target.saturating_sub(self.cpu.cycles);
            if left == 0 {
                return Stop::Limit;
            }
            // `Cpu::run` never stops on its first instruction; only skip a
            // breakpoint there right after « Continuer ».
            if !std::mem::take(&mut self.resuming) && self.breakpoints.contains(&self.cpu.pc) {
                return Stop::Breakpoint(self.cpu.pc);
            }
            match self.cpu.run(left.div_ceil(2), &self.breakpoints) {
                Stop::Limit => {}
                stop => return stop,
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lines: &[Line], device: &Device) {
        self.tick(ui.ctx());
        ui.horizontal(|ui| {
            if self.running {
                if ui.button("⏸ Pause").clicked() {
                    self.running = false;
                    self.follow = true;
                    self.status = format!("pause à {:#06x}", self.cpu.pc);
                }
            } else if ui.button("▶ Continuer").on_hover_text("F5").clicked()
                || ui.input(|i| i.key_pressed(egui::Key::F5))
            {
                // Step off a breakpoint first so Continuer does not stop at once.
                self.running = true;
                self.resuming = true;
                self.last_frame = None;
            }
            ui.add_enabled_ui(!self.running, |ui| {
                if ui.button("Pas").on_hover_text("F11 : une instruction").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::F11))
                {
                    self.step();
                }
                if ui.button("Pas par-dessus").on_hover_text("F10 : exécute un call jusqu'au retour").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::F10))
                {
                    self.step_over();
                }
                if ui.button("Sortir").on_hover_text("Jusqu'au return de la sous-routine").clicked() {
                    self.step_out();
                }
            });
            if ui.button("Reset").on_hover_text("MCLR : PC = 0, registres à leur valeur de reset").clicked() {
                self.cpu.reset();
                self.running = false;
                self.follow = true;
                self.status = "reset".into();
            }
            ui.separator();
            ui.checkbox(&mut self.real_time, "Temps réel");
            ui.add_enabled(
                self.real_time,
                egui::DragValue::new(&mut self.fosc_mhz).range(0.031..=64.0).speed(0.5).suffix(" MHz"),
            )
            .on_hover_text("Fréquence de l'oscillateur (Fosc). Le simulateur ne modélise pas l'oscillateur.");
            ui.separator();
            let time_ms = self.cpu.cycles as f64 * 4.0 / (self.fosc_mhz as f64 * 1e3);
            ui.label(RichText::new(format!("{} cycles · {time_ms:.3} ms", self.cpu.cycles)).monospace());
            ui.weak(&self.status);
        });
        ui.separator();

        egui::Panel::right("debug_state")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.state(ui, device));
            });
        egui::CentralPanel::default().show(ui, |ui| self.code(ui, lines));
    }

    fn code(&mut self, ui: &mut egui::Ui, lines: &[Line]) {
        if lines.is_empty() {
            ui.weak("Rien à déboguer : charge un .hex ou compile un .asm.");
            return;
        }
        ui.weak("Clique dans la marge pour poser ou retirer un point d'arrêt.");
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
        let dim = ui.visuals().weak_text_color();
        let text = ui.visuals().text_color();
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let current = lines.iter().position(|l| l.addr == self.cpu.pc as u32);
        let mut area = egui::ScrollArea::vertical().auto_shrink(false);
        if let (true, Some(index)) = (self.follow && !self.running, current) {
            // Keep the current line in view, a few rows from the top.
            area = area.vertical_scroll_offset((index.saturating_sub(6)) as f32 * (row_height + ui.spacing().item_spacing.y));
            self.follow = false;
        }
        area.show_rows(ui, row_height, lines.len(), |ui, range| {
            for line in &lines[range] {
                let addr = line.addr as u16;
                let is_pc = addr == self.cpu.pc;
                let response = ui.horizontal(|ui| {
                    let bp = self.breakpoints.contains(&addr);
                    let (rect, click) = ui.allocate_exact_size(egui::vec2(16.0, row_height), egui::Sense::click());
                    if bp {
                        ui.painter().circle_filled(rect.center(), 5.0, RED);
                    } else if click.hovered() {
                        ui.painter().circle_stroke(rect.center(), 5.0, egui::Stroke::new(1.0, RED));
                    }
                    if click.clicked() && !self.breakpoints.remove(&addr) {
                        self.breakpoints.insert(addr);
                    }
                    let arrow = if is_pc { "▶" } else { " " };
                    ui.label(RichText::new(arrow).monospace().color(GREEN));
                    ui.label(RichText::new(format!("{:04X}  {:04X}", line.addr, line.word)).monospace().color(dim));
                    highlight::label_column(ui, line.label.as_deref(), row_height);
                    ui.label(highlight::line(&line.text, &font, text, dim));
                });
                if is_pc {
                    ui.painter().rect_filled(response.response.rect, 2.0, CURRENT);
                }
            }
        });
    }

    fn state(&mut self, ui: &mut egui::Ui, device: &Device) {
        let cpu = &self.cpu;
        ui.strong("Cœur");
        let status = cpu.status();
        egui::Grid::new("core").num_columns(2).striped(true).show(ui, |ui| {
            let mut row = |k: &str, v: String| {
                ui.label(k);
                ui.monospace(v);
                ui.end_row();
            };
            row("PC", format!("{:04X}", cpu.pc));
            row("W", format!("{:02X}  ({})", cpu.w, cpu.w));
            row("BSR", format!("{}", cpu.bsr()));
            row("PCLATH", format!("{:02X}", cpu.pclath()));
            row("FSR0", format!("{:04X}", cpu.fsr(0)));
            row("FSR1", format!("{:04X}", cpu.fsr(1)));
            let flag = |bit: u8, name: &str| {
                if status & (1 << bit) != 0 {
                    name.to_owned()
                } else {
                    name.to_lowercase()
                }
            };
            row(
                "STATUS",
                format!("{status:02X}  {} {} {} {} {}", flag(4, "TO"), flag(3, "PD"), flag(2, "Z"), flag(1, "DC"), flag(0, "C")),
            );
        });

        ui.add_space(6.0);
        ui.strong(format!("Pile ({}/16)", cpu.stack.len()));
        if cpu.stack.is_empty() {
            ui.weak("vide");
        }
        for (depth, ret) in cpu.stack.iter().rev().enumerate() {
            ui.monospace(format!("{depth:>2}  → {ret:04X}"));
        }

        ui.add_space(6.0);
        ui.strong("Broches (LAT, sortie si TRIS = 0)");
        for port in ['A', 'B', 'C', 'D', 'E'] {
            let Some((lat, tris)) = cpu.pins(port) else { continue };
            ui.horizontal(|ui| {
                ui.monospace(format!("R{port}"));
                for bit in (0..8).rev() {
                    let output = tris & (1 << bit) == 0;
                    let high = lat & (1 << bit) != 0;
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    let center = rect.center();
                    if output {
                        let color = if high { GREEN } else { Color32::from_gray(60) };
                        ui.painter().circle_filled(center, 6.0, color);
                    } else {
                        ui.painter().circle_stroke(center, 6.0, egui::Stroke::new(1.0, Color32::from_gray(90)));
                    }
                    response.on_hover_text(format!(
                        "R{port}{bit} : {}{}",
                        if output { "sortie " } else { "entrée" },
                        if output { if high { "haute" } else { "basse" } } else { "" }
                    ));
                }
            });
        }

        ui.add_space(6.0);
        ui.strong("Registres");
        egui::Grid::new("sfrs").num_columns(2).striped(true).show(ui, |ui| {
            for name in ["PORTA", "LATA", "TRISA", "ANSELA", "PORTB", "LATB", "TRISB", "PORTC", "LATC", "TRISC", "OSCCON", "INTCON", "OPTION_REG"] {
                if let Some(v) = cpu.sfr_value(name) {
                    ui.label(name);
                    ui.monospace(format!("{v:02X}  {v:08b}"));
                    ui.end_row();
                }
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.strong("RAM banque");
            let banks = if device.data_end > 0x1000 { 63 } else { 31 };
            ui.add(egui::DragValue::new(&mut self.ram_bank).range(0..=banks));
        });
        let base = self.ram_bank as u16 * 0x80;
        for row in (0..0x80u16).step_by(8) {
            let bytes: Vec<String> = (0..8).map(|i| format!("{:02X}", self.cpu.read(base + row + i))).collect();
            ui.monospace(format!("{:04X}  {}", base + row, bytes.join(" ")));
        }
    }
}
