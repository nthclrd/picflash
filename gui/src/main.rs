//! picflash desktop interface: pick a part, drop an assembly source or a HEX
//! file, then build, program, erase, verify or read through a picflash board
//! or the built-in simulator.
//!
//! Usage: `picflash-gui [file.hex|file.asm] [--part NAME] [--asm] [--debug]`

mod debug;
mod highlight;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context as _, Result};
use eframe::egui::{self, Color32, RichText};
use picflash::device::{self, Device};
use picflash::disasm::{disassemble, Line};
use picflash::image::Image;
use picflash::checksum::checksum;
use picflash::programmer::{Ids, Mismatch, Programmer, Regions};
use picflash::sim::SimPic;
use picflash::asm;
use picflash::devdb::Database;
use picflash::toolchain::Diagnostic;
use picflash::algo::Entry;
use picflash::pickit;
use picflash::tool::{self, Choice};
use picflash::transport::Transport;

const WORDS_PER_ROW: usize = 8;
const ERASED: u16 = 0x3FFF;
const GREEN: Color32 = Color32::from_rgb(70, 200, 110);
const RED: Color32 = Color32::from_rgb(225, 85, 85);
const BLUE: Color32 = Color32::from_rgb(90, 160, 255);
const SOURCE_EXTENSIONS: &[&str] = &["asm", "s", "as"];

fn main() -> eframe::Result {
    let mut file = None;
    let mut part = "PIC16F1789".to_owned();
    let mut asm = false;
    let mut debug = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--part" | "-P" => part = args.next().unwrap_or(part),
            "--asm" => asm = true,
            "--debug" => debug = true,
            _ => file = Some(arg),
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("picflash")
            .with_inner_size([1060.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            // Without an explicit icon, eframe swaps in the egui logo at startup.
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../../packaging/icons/picflash.png"
                ))
                .expect("bundled icon is a valid PNG"),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "picflash",
        options,
        Box::new(move |cc| {
            let mut app = App::new(&cc.egui_ctx, part);
            if let Some(file) = file {
                app.open_file(Path::new(&file));
            }
            if asm {
                app.format = Format::Asm;
            }
            if debug {
                app.format = Format::Debug;
            }
            Ok(Box::new(app))
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Simulator,
    Board,
    PicKit,
}

/// Entry mode picked in the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryChoice {
    /// High voltage on PICkit 4/5, low voltage elsewhere.
    Auto,
    Low,
    High,
}

impl EntryChoice {
    fn entry(self) -> Option<Entry> {
        match self {
            EntryChoice::Auto => None,
            EntryChoice::Low => Some(Entry::LowVoltage),
            EntryChoice::High => Some(Entry::HighVoltage),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Id,
    Erase,
    Program,
    Verify,
    Read,
    Run,
}

impl Job {
    fn label(self) -> &'static str {
        match self {
            Job::Id => "Lecture de l'ID",
            Job::Erase => "Effacement",
            Job::Program => "Programmation",
            Job::Verify => "Vérification",
            Job::Read => "Lecture",
            Job::Run => "Relâchement du reset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    File,
    Chip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Source,
    Hex,
    Asm,
    Eeprom,
    Debug,
}

/// Options a job needs from the window.
#[derive(Clone, Copy)]
struct JobOpts {
    verify_after: bool,
    allow_lvp_off: bool,
    keep_eeprom: bool,
    regions: Regions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Info,
    Ok,
    Error,
}

enum Msg {
    Log(Level, String),
    /// Programmers found by the background scan.
    Tools { ports: Vec<String>, boards: Vec<String>, kits: Vec<pickit::Found> },
    Ids(Ids),
    Dump(Image),
    Built { hex: Option<PathBuf>, part: Option<String>, log: String, diagnostics: Vec<Diagnostic>, then_program: bool },
    Done,
}

struct App {
    ctx: egui::Context,
    roots: Vec<PathBuf>,
    part: String,
    device: Option<Device>,
    parts: Vec<String>,
    target: Target,
    /// Empty means auto-detect by USB ID.
    port: String,
    ports: Vec<String>,
    /// Ports whose USB ID is a picflash board.
    boards: Vec<String>,
    /// Connected PICkit / Snap tools.
    kits: Vec<pickit::Found>,
    entry: EntryChoice,
    /// Supply the target from the PICkit 4/5.
    power: bool,
    power_volts: f32,
    /// Simulated chip for the loaded part; keeps its contents between jobs.
    sim: Arc<Mutex<Option<SimPic>>>,
    hex_path: String,
    /// Assembly source being edited outside and built here.
    source_path: String,
    source_lines: Vec<String>,
    /// Compiler messages by 1-based source line.
    source_errors: BTreeMap<u32, String>,
    /// Extra assembler options (`-Wl,-pNAME=ADDR`).
    build_options: String,
    image: Option<Image>,
    chip: Option<Image>,
    ids: Option<Ids>,
    view: View,
    format: Format,
    /// Disassembly of `image` and `chip`, rebuilt when either changes.
    asm_file: Vec<Line>,
    asm_chip: Vec<Line>,
    verify_after: bool,
    allow_lvp_off: bool,
    keep_eeprom: bool,
    /// Regions written by « Programmer »; all of them means a full erase first.
    regions: Regions,
    config_open: bool,
    /// The image was edited in the window (config editor) and not saved.
    edited: bool,
    debugger: Option<debug::Debugger>,
    log: Vec<(Level, String)>,
    busy: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl App {
    fn new(ctx: &egui::Context, part: String) -> Self {
        let (tx, rx) = channel();
        let mut app = Self {
            ctx: ctx.clone(),
            roots: device::default_pack_roots(),
            part,
            device: None,
            parts: Vec::new(),
            target: Target::Simulator,
            port: String::new(),
            ports: Vec::new(),
            boards: Vec::new(),
            kits: Vec::new(),
            entry: EntryChoice::Auto,
            power: false,
            power_volts: 5.0,
            sim: Arc::new(Mutex::new(None)),
            hex_path: String::new(),
            source_path: String::new(),
            source_lines: Vec::new(),
            source_errors: BTreeMap::new(),
            build_options: String::new(),
            image: None,
            chip: None,
            ids: None,
            view: View::File,
            format: Format::Hex,
            asm_file: Vec::new(),
            asm_chip: Vec::new(),
            verify_after: true,
            allow_lvp_off: false,
            keep_eeprom: false,
            regions: Regions::ALL,
            config_open: false,
            edited: false,
            debugger: None,
            log: Vec::new(),
            busy: false,
            tx,
            rx,
        };
        app.refresh_ports();
        if !app.boards.is_empty() {
            app.target = Target::Board;
        } else if app.kits.iter().any(|k| k.usable) {
            app.target = Target::PicKit;
        }
        app.watch_tools();
        app.load_device();
        app.scan_parts();
        app.info(format!("assembleur intégré, {} puces embarquées", app.parts.len()));
        app
    }

    /// Opens a dropped or chosen file: assembly sources are built, HEX files loaded.
    fn open_file(&mut self, path: &Path) {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "hex" {
            self.hex_path = path.display().to_string();
            self.load_hex();
        } else if SOURCE_EXTENSIONS.contains(&ext.as_str()) {
            self.source_path = path.display().to_string();
            self.load_source();
            self.format = Format::Source;
            self.build(false);
        } else {
            self.error(format!("{} : seuls les .asm/.s et .hex sont acceptés", path.display()));
        }
    }

    fn load_source(&mut self) {
        self.source_errors.clear();
        match std::fs::read_to_string(&self.source_path) {
            Ok(text) => self.source_lines = text.lines().map(str::to_owned).collect(),
            Err(e) => {
                self.source_lines.clear();
                self.error(format!("lecture de {} : {e}", self.source_path));
            }
        }
    }

    /// Assembles the source in the background, then optionally programs it.
    fn build(&mut self, then_program: bool) {
        if self.source_path.is_empty() {
            self.error("aucun fichier .asm : dépose-le dans la fenêtre ou utilise « Ouvrir… »");
            return;
        }
        // Re-read so edits made in another editor are picked up.
        self.load_source();
        let source = PathBuf::from(&self.source_path);
        let mut options = asm::Options { part: Some(self.part.clone()), ..Default::default() };
        let extra: Vec<String> = self.build_options.split_whitespace().map(str::to_owned).collect();
        if let Err(e) = options.parse_extra(&extra) {
            self.error(e);
            return;
        }
        let out_dir = std::env::temp_dir().join("picflash-build");
        let (ctx, tx) = (self.ctx.clone(), self.tx.clone());
        self.busy = true;
        let name = source.file_name().map_or_else(|| source.display().to_string(), |n| n.to_string_lossy().into_owned());
        self.info(format!("Compilation de {name}…"));
        thread::spawn(move || {
            let msg = match assemble_to(&source, &options, &out_dir) {
                Ok((hex, part, log, diagnostics)) => Msg::Built { hex, part, log, diagnostics, then_program },
                Err(e) => Msg::Built {
                    hex: None,
                    part: None,
                    log: format!("{e:#}"),
                    diagnostics: Vec::new(),
                    then_program: false,
                },
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn built(
        &mut self,
        hex: Option<PathBuf>,
        part: Option<String>,
        log: String,
        diagnostics: Vec<Diagnostic>,
        then_program: bool,
    ) {
        self.busy = false;
        // The source's PROCESSOR line wins over the selected part.
        if let Some(part) = part.filter(|p| *p != self.part) {
            self.info(format!("puce du source : {part}"));
            self.part = part;
            self.load_device();
        }
        let source_name = Path::new(&self.source_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for d in &diagnostics {
            let level = if d.error { Level::Error } else { Level::Info };
            let place = d.line.map_or(String::new(), |l| format!("ligne {l} : "));
            self.log.push((level, format!("{place}{}", d.message)));
            let ours = Path::new(&d.file).file_name().is_some_and(|f| f.to_string_lossy() == source_name);
            if let (true, Some(line)) = (ours, d.line) {
                self.source_errors.entry(line).or_insert_with(|| d.message.clone());
            }
        }
        match hex {
            Some(hex) => {
                self.log.push((Level::Ok, "compilation réussie".into()));
                self.hex_path = hex.display().to_string();
                self.load_hex();
                if then_program {
                    self.start(Job::Program);
                }
            }
            None => {
                if diagnostics.is_empty() {
                    self.log.push((Level::Error, log));
                }
                self.log.push((Level::Error, "compilation échouée".into()));
                self.format = Format::Source;
            }
        }
    }

    fn info(&mut self, text: impl Into<String>) {
        self.log.push((Level::Info, text.into()));
    }

    fn error(&mut self, text: impl Into<String>) {
        self.log.push((Level::Error, text.into()));
    }

    /// Lists the parts of the embedded database.
    fn scan_parts(&mut self) {
        self.parts = Database::embedded().parts.iter().map(|d| d.name.clone()).collect();
        self.parts.sort();
    }

    fn refresh_ports(&mut self) {
        self.ports = serial_ports();
        self.boards = picflash::transport::board_ports();
        self.kits = pickit::scan();
    }

    /// Rescans connected programmers every 2 s so plugging one in shows up.
    fn watch_tools(&self) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        thread::spawn(move || loop {
            thread::sleep(std::time::Duration::from_secs(2));
            let msg = Msg::Tools { ports: serial_ports(), boards: picflash::transport::board_ports(), kits: pickit::scan() };
            if tx.send(msg).is_err() {
                break;
            }
            ctx.request_repaint();
        });
    }

    fn refresh_asm(&mut self) {
        let lines = |image: Option<&Image>| match (image, &self.device) {
            (Some(image), Some(device)) => disassemble(&image.flash, device).unwrap_or_default(),
            _ => Vec::new(),
        };
        self.asm_file = lines(self.image.as_ref());
        self.debugger = None;
        self.asm_chip = lines(self.chip.as_ref());
    }

    fn load_device(&mut self) {
        self.ids = None;
        self.chip = None;
        match device::load(&self.roots, &self.part) {
            Ok(d) => {
                *self.sim.lock().unwrap() = SimPic::new(&d).ok();
                match d.algorithm() {
                    Ok(a) => self.info(format!("{} chargé ({a:?}, spec DS{})", d.name, d.prog_spec)),
                    Err(e) => self.error(format!("{e}")),
                }
                self.device = Some(d);
                self.refresh_asm();
                if !self.hex_path.is_empty() {
                    self.load_hex();
                }
            }
            Err(e) => {
                self.device = None;
                self.error(format!("{e:#}"));
            }
        }
    }

    fn load_hex(&mut self) {
        let Some(device) = &self.device else { return };
        let result = std::fs::read_to_string(&self.hex_path)
            .with_context(|| format!("lecture de {}", self.hex_path))
            .and_then(|text| Image::parse_hex(&text, device));
        match result {
            Ok(image) => {
                let name = Path::new(&self.hex_path)
                    .file_name()
                    .map_or_else(|| self.hex_path.clone(), |n| n.to_string_lossy().into_owned());
                let msg = format!(
                    "{name} chargé : {} mots programme, {} mots config",
                    image.flash.len(),
                    image.config.len()
                );
                self.image = Some(image);
                self.edited = false;
                self.view = View::File;
                self.refresh_asm();
                self.info(msg);
            }
            Err(e) => {
                self.image = None;
                self.refresh_asm();
                self.error(format!("{e:#}"));
            }
        }
    }

    fn start(&mut self, job: Job) {
        let Some(device) = self.device.clone() else {
            self.error("aucune puce chargée");
            return;
        };
        if matches!(job, Job::Program | Job::Verify) && self.image.is_none() {
            self.error("charge un fichier .hex d'abord");
            return;
        }
        self.busy = true;
        let ctx = self.ctx.clone();
        let tx = self.tx.clone();
        let sim = self.sim.clone();
        let image = self.image.clone();
        let target = self.target;
        let opts = JobOpts {
            verify_after: self.verify_after,
            allow_lvp_off: self.allow_lvp_off,
            keep_eeprom: self.keep_eeprom,
            regions: self.regions,
        };
        let tool_opts = tool::Options {
            entry: self.entry.entry(),
            power_mv: self.power.then(|| (self.power_volts * 1000.0).round() as u16),
            // Only the board takes a port; the PICkit is found by USB.
            port: (target == Target::Board && !self.port.is_empty()).then(|| self.port.clone()),
        };
        self.info(format!("{}…", job.label()));
        thread::spawn(move || {
            let send = |level, text: String| {
                let _ = tx.send(Msg::Log(level, text));
            };
            let result = match target {
                Target::Simulator => {
                    let mut guard = sim.lock().unwrap();
                    match guard.as_mut() {
                        Some(pic) => {
                            let t0 = pic.now_us;
                            let entry = tool_opts.entry.unwrap_or_default();
                            let r = run_job(job, pic, entry, &device, image.as_ref(), opts, &tx);
                            let dt = (pic.now_us - t0) as f64 / 1e6;
                            if r.is_ok() && job != Job::Run {
                                send(Level::Info, format!("durée estimée sur carte : {dt:.2} s"));
                            }
                            r
                        }
                        None => Err(anyhow::anyhow!("pas de simulateur pour cette puce")),
                    }
                }
                Target::Board | Target::PicKit => {
                    let choice = if target == Target::Board { Choice::Board } else { Choice::PicKit };
                    tool::open(choice, &device, &tool_opts).and_then(|o| {
                        send(Level::Info, o.summary);
                        let mut link = o.link;
                        run_job(job, &mut link, o.entry, &device, image.as_ref(), opts, &tx)
                    })
                }
            };
            if let Err(e) = result {
                send(Level::Error, format!("{e:#}"));
            }
            let _ = tx.send(Msg::Done);
            ctx.request_repaint();
        });
    }

    fn poll(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Log(level, text) => self.log.push((level, text)),
                Msg::Tools { ports, boards, kits } => {
                    let fresh_kit = kits.iter().any(|k| k.usable) && !self.kits.iter().any(|k| k.usable);
                    let fresh_board = !boards.is_empty() && self.boards.is_empty();
                    self.ports = ports;
                    self.boards = boards;
                    self.kits = kits;
                    // Follow a programmer that was just plugged in, unless busy.
                    if !self.busy {
                        if fresh_board {
                            self.target = Target::Board;
                        } else if fresh_kit && self.target != Target::Board {
                            self.target = Target::PicKit;
                        }
                    }
                }
                Msg::Ids(ids) => self.ids = Some(ids),
                Msg::Dump(image) => {
                    self.chip = Some(image);
                    self.view = View::Chip;
                    self.refresh_asm();
                }
                Msg::Built { hex, part, log, diagnostics, then_program } => {
                    self.built(hex, part, log, diagnostics, then_program)
                }
                Msg::Done => self.busy = false,
            }
        }
    }
}

/// Serial ports, without the macOS `/dev/tty.*` duplicates of `/dev/cu.*`.
fn serial_ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .filter(|n| !n.contains("/tty."))
        .collect()
}

/// The connected chip's ID matches the loaded part (or the part has no ID).
fn id_matches(device: &Device, ids: Ids) -> bool {
    device.device_id_mask == 0 || ids.device_id & device.device_id_mask == device.device_id
}

fn run_job(
    job: Job,
    link: &mut dyn Transport,
    entry: Entry,
    device: &Device,
    image: Option<&Image>,
    opts: JobOpts,
    tx: &Sender<Msg>,
) -> Result<()> {
    let log = |level, text: String| {
        let _ = tx.send(Msg::Log(level, text));
    };
    let mut prog = Programmer::new(link, device)?;
    prog.entry = entry;
    prog.keep_eeprom = opts.keep_eeprom;
    prog.ping()?;
    match job {
        Job::Id => {
            let ids = prog.read_ids()?;
            let _ = tx.send(Msg::Ids(ids));
            let (level, what) = if id_matches(device, ids) {
                (Level::Ok, device.name.as_str())
            } else {
                (Level::Error, "puce inattendue")
            };
            log(
                level,
                format!("ID {:#06x} ({what}), révision {:#06x}", ids.device_id, ids.revision_id),
            );
        }
        Job::Erase => {
            let _ = tx.send(Msg::Ids(prog.check_id()?));
            prog.erase()?;
            log(Level::Ok, "puce effacée".into());
        }
        Job::Program => {
            let image = image.context("aucun fichier .hex")?;
            let _ = tx.send(Msg::Ids(prog.check_id()?));
            if opts.regions == Regions::ALL {
                prog.write(image, opts.allow_lvp_off)?;
            } else {
                prog.write_regions(image, opts.regions, opts.allow_lvp_off)?;
            }
            log(
                Level::Ok,
                format!(
                    "programmé : {} mots + {} config + {} octets EEPROM, checksum {:04X}",
                    image.flash.len(),
                    image.config.len(),
                    image.eeprom.len(),
                    checksum(image, device)
                ),
            );
            if opts.verify_after {
                report(&prog.verify_regions(image, opts.regions)?, &log);
            }
            prog.release()?;
        }
        Job::Verify => {
            let image = image.context("aucun fichier .hex")?;
            let _ = tx.send(Msg::Ids(prog.check_id()?));
            report(&prog.verify(image)?, &log);
            prog.release()?;
        }
        Job::Read => {
            let _ = tx.send(Msg::Ids(prog.check_id()?));
            let dump = prog.read()?;
            log(
                Level::Ok,
                format!(
                    "lu : {} mots non vides, {} octets EEPROM, checksum {:04X}",
                    dump.flash.len(),
                    dump.eeprom.len(),
                    checksum(&dump, device)
                ),
            );
            let _ = tx.send(Msg::Dump(dump));
            prog.release()?;
        }
        Job::Run => {
            prog.release()?;
            log(Level::Ok, "reset relâché, la cible tourne".into());
        }
    }
    Ok(())
}

/// The assembly source with syntax colors; lines with compiler errors are red.
fn source_view(ui: &mut egui::Ui, lines: &[String], errors: &BTreeMap<u32, String>) {
    if lines.is_empty() {
        ui.weak("Aucun source : dépose un .asm dans la fenêtre.");
        return;
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let dim = ui.visuals().weak_text_color();
    let text = ui.visuals().text_color();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    egui::ScrollArea::both()
        .auto_shrink(false)
        .show_rows(ui, row_height, lines.len(), |ui, range| {
            for i in range {
                let number = i as u32 + 1;
                ui.horizontal(|ui| {
                    let error = errors.get(&number);
                    let gutter = RichText::new(format!("{number:>4}")).monospace();
                    ui.label(if error.is_some() { gutter.color(RED).strong() } else { gutter.color(dim) });
                    let response = ui.label(highlight::line(&lines[i], &font, text, dim));
                    if let Some(message) = error {
                        ui.painter().rect_filled(
                            response.rect.expand(1.0),
                            2.0,
                            Color32::from_rgba_unmultiplied(225, 85, 85, 40),
                        );
                        ui.label(RichText::new(format!("← {message}")).color(RED).monospace());
                    }
                });
            }
        });
}

/// One instruction per row: address, word, label, mnemonic. Words that differ
/// from the other view are red.
fn asm_view(ui: &mut egui::Ui, lines: &[Line], other: Option<&Image>) {
    if lines.is_empty() {
        ui.weak("Rien à désassembler (architecture non supportée ou mémoire vide).");
        return;
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let dim = ui.visuals().weak_text_color();
    let text = ui.visuals().text_color();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show_rows(ui, row_height, lines.len(), |ui, range| {
            for line in &lines[range] {
                let differs = other.is_some_and(|o| {
                    o.flash.get(&line.addr).copied().unwrap_or(ERASED) != line.word
                });
                ui.horizontal(|ui| {
                    let head = format!("{:04X}  {:04X}", line.addr, line.word);
                    let head = RichText::new(head).monospace();
                    ui.label(if differs { head.color(RED) } else { head.color(dim) })
                        .on_hover_text(if differs { "différent de l'autre vue" } else { "" });
                    highlight::label_column(ui, line.label.as_deref(), row_height);
                    ui.label(highlight::line(&line.text, &font, text, dim));
                });
            }
        });
}

/// Data EEPROM, 16 bytes per row; bytes that differ from the other view are red.
fn eeprom_view(ui: &mut egui::Ui, d: &Device, shown: &Image, other: Option<&Image>) {
    let size = d.eeprom.as_ref().map_or(0, |r| r.len() as u32);
    let base = d.eeprom.as_ref().map_or(0, |r| r.start);
    let dim = ui.visuals().weak_text_color();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for row in (0..size).step_by(16) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(format!("{:04X}", base + row)).monospace().color(dim));
                let mut ascii = String::new();
                for offset in row..(row + 16).min(size) {
                    let byte = shown.eeprom.get(&offset).copied().unwrap_or(0xFF);
                    let differs = other.is_some_and(|o| o.eeprom.get(&offset).copied().unwrap_or(0xFF) != byte);
                    let text = RichText::new(format!("{byte:02X}")).monospace();
                    ui.label(if differs { text.color(RED) } else if byte == 0xFF { text.color(dim) } else { text });
                    ascii.push(if byte.is_ascii_graphic() { byte as char } else { '.' });
                }
                ui.label(RichText::new(ascii).monospace().color(dim));
            });
        }
    });
}

fn report(bad: &[Mismatch], log: &impl Fn(Level, String)) {
    if bad.is_empty() {
        log(Level::Ok, "vérification OK".into());
        return;
    }
    for m in bad.iter().take(8) {
        log(
            Level::Error,
            format!("  {:#06x} : attendu {:#06x}, lu {:#06x}", m.address, m.expected, m.found),
        );
    }
    log(Level::Error, format!("vérification échouée sur {} mots", bad.len()));
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        self.handle_drop(ui.ctx());
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            self.toolbar(ui);
            ui.add_space(6.0);
        });
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(150.0)
            .show(ui, |ui| self.log_panel(ui));
        egui::Panel::left("details")
            .resizable(true)
            .default_size(270.0)
            .show(ui, |ui| self.details(ui));
        egui::CentralPanel::default().show(ui, |ui| self.memory(ui));
        self.config_window(ui.ctx());
    }
}

impl App {
    /// Files dropped on the window; shows a hint while they hover.
    fn handle_drop(&mut self, ctx: &egui::Context) {
        let (hovering, dropped) = ctx.input(|i| {
            let paths: Vec<PathBuf> = i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect();
            (!i.raw.hovered_files.is_empty(), paths)
        });
        if hovering {
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop"));
            let painter = ctx.layer_painter(layer);
            let rect = ctx.content_rect();
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Dépose un .asm (compilé ici) ou un .hex",
                egui::FontId::proportional(24.0),
                Color32::WHITE,
            );
        }
        if !self.busy {
            for path in dropped {
                self.open_file(&path);
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Puce");
            let edit = ui.add(egui::TextEdit::singleline(&mut self.part).desired_width(150.0));
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.part = self.part.trim().to_ascii_uppercase();
                self.load_device();
            }
            let label = format!("{} supportées", self.parts.len());
            let mut picked = None;
            egui::ComboBox::from_id_salt("parts")
                .selected_text(label)
                .height(400.0)
                .show_ui(ui, |ui| {
                    let query = self.part.trim().to_ascii_uppercase();
                    let matching = self.parts.iter().filter(|n| n.contains(&query));
                    // An exact or unknown name would hide everything: fall back to all.
                    let list: Vec<&String> = if matching.clone().count() > 1 {
                        matching.collect()
                    } else {
                        self.parts.iter().collect()
                    };
                    for name in list {
                        if ui.selectable_label(*name == self.part, name.as_str()).clicked() {
                            picked = Some(name.clone());
                        }
                    }
                });
            if let Some(name) = picked {
                self.part = name;
                self.load_device();
            }

            ui.separator();
            ui.label("Cible");
            ui.selectable_value(&mut self.target, Target::Simulator, "Simulateur");
            ui.selectable_value(&mut self.target, Target::Board, "Carte picflash");
            ui.selectable_value(&mut self.target, Target::PicKit, "PICkit / Snap");
            if self.target == Target::Board {
                let shown = if self.port.is_empty() { "auto".to_owned() } else { self.port.clone() };
                egui::ComboBox::from_id_salt("port").selected_text(shown).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.port, String::new(), "auto (1209:0001)");
                    for p in &self.ports {
                        ui.selectable_value(&mut self.port, p.clone(), p.as_str());
                    }
                });
                if ui.button("↻").on_hover_text("Rafraîchir les ports").clicked() {
                    self.refresh_ports();
                }
            }
            if self.target == Target::PicKit && ui.button("↻").on_hover_text("Rechercher les PICkit").clicked() {
                self.refresh_ports();
            }
            let kit = self.kits.iter().find(|k| k.usable);
            let kit_problem = self.kits.iter().find_map(|k| k.problem.clone());
            let (dot, tip) = match self.target {
                Target::Simulator => (BLUE, "PIC simulé bit à bit".to_owned()),
                Target::Board if !self.boards.is_empty() => (GREEN, "port USB détecté".to_owned()),
                Target::Board => (RED, "aucune carte détectée".to_owned()),
                Target::PicKit => match (kit, kit_problem) {
                    (Some(k), _) => (GREEN, format!("{} détecté", k.kind.name())),
                    (None, Some(p)) => (RED, p),
                    (None, None) => (RED, "aucun PICkit / Snap détecté".to_owned()),
                },
            };
            let (rect, response) = ui.allocate_exact_size([14.0, 14.0].into(), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, dot);
            response.on_hover_text(&tip);
            if self.target == Target::PicKit {
                ui.label(RichText::new(match kit {
                    Some(k) => k.kind.name().to_owned(),
                    None => "non détecté".to_owned(),
                }).weak());
            }
        });

        ui.horizontal(|ui| {
            ui.label("Entrée");
            let shown = match self.entry {
                EntryChoice::Auto => "auto",
                EntryChoice::Low => "basse tension (LVP)",
                EntryChoice::High => "haute tension (VPP)",
            };
            egui::ComboBox::from_id_salt("entry").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.entry, EntryChoice::Auto, "auto")
                    .on_hover_text("Haute tension sur carte picflash et PICkit 4/5 (comme MPLAB IPE), basse tension sinon");
                ui.selectable_value(&mut self.entry, EntryChoice::Low, "basse tension (LVP)")
                    .on_hover_text("Clé « MCHP », MCLR à 0 V : demande LVP = ON dans la config");
                ui.selectable_value(&mut self.entry, EntryChoice::High, "haute tension (VPP)")
                    .on_hover_text("MCLR à VPP (9 V pour PIC16F1xxx) : marche même si LVP = OFF (carte picflash, PICkit 4/5)");
            });
            if matches!(self.target, Target::PicKit | Target::Board) {
                ui.separator();
                ui.checkbox(&mut self.power, "Alimenter la cible")
                    .on_hover_text("L'outil fournit VDD ; laisse décoché si la carte cible a sa propre alimentation");
                ui.add_enabled_ui(self.power, |ui| {
                    egui::ComboBox::from_id_salt("vdd")
                        .selected_text(format!("{:.1} V", self.power_volts))
                        .show_ui(ui, |ui| {
                            let max = if self.target == Target::Board { 4.8f32 } else { 5.0 };
                            for v in [1.8f32, 3.3, max] {
                                ui.selectable_value(&mut self.power_volts, v, format!("{v:.1} V"));
                            }
                        });
                });
            }
        });

        ui.horizontal(|ui| {
            ui.label("ASM  ");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.source_path)
                    .desired_width(420.0)
                    .hint_text("dépose un .asm dans la fenêtre"),
            );
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.load_source();
                self.format = Format::Source;
            }
            if ui.button("Ouvrir…").clicked() {
                let file = rfd::FileDialog::new().add_filter("Assembleur", SOURCE_EXTENSIONS).pick_file();
                if let Some(path) = file {
                    self.open_file(&path);
                }
            }
            let can_build = !self.busy && !self.source_path.is_empty();
            ui.add_enabled_ui(can_build, |ui| {
                if ui.button("Compiler").clicked() {
                    self.build(false);
                }
                if ui.button("Compiler + programmer").clicked() {
                    self.build(true);
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.build_options)
                    .desired_width(150.0)
                    .hint_text("options de link"),
            )
            .on_hover_text("Placement de psects comme avec pic-as, ex. -Wl,-pmyvec=8h");
        });

        ui.horizontal(|ui| {
            ui.label("HEX  ");
            let edit = ui.add(egui::TextEdit::singleline(&mut self.hex_path).desired_width(420.0));
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.load_hex();
            }
            if ui.button("Parcourir…").clicked() {
                let file = rfd::FileDialog::new().add_filter("Intel HEX", &["hex"]).pick_file();
                if let Some(path) = file {
                    self.hex_path = path.display().to_string();
                    self.load_hex();
                }
            }
            let saving_chip = self.view == View::Chip && self.chip.is_some();
            let save_tip = if saving_chip { "Enregistrer le contenu lu dans la puce" } else { "Enregistrer le fichier (avec la config modifiée)" };
            if ui.add_enabled(self.image.is_some() || self.chip.is_some(), egui::Button::new("Enregistrer…")).on_hover_text(save_tip).clicked() {
                self.save_hex();
            }
            if let (Some(image), Some(d)) = (&self.image, &self.device) {
                let edited = if self.edited { " (modifié)" } else { "" };
                ui.label(RichText::new(format!("checksum {:04X}{edited}", checksum(image, d))).monospace())
                    .on_hover_text("Checksum du fichier, calculé comme MPLAB IPE");
            }
            ui.separator();
            ui.checkbox(&mut self.verify_after, "Vérifier après écriture");
            ui.checkbox(&mut self.allow_lvp_off, "Autoriser LVP off").on_hover_text(
                "Une config qui coupe LVP rend la puce injoignable sans haute tension",
            );
        });

        ui.horizontal(|ui| {
            ui.label("Zones");
            let all = self.regions == Regions::ALL;
            let shown = if all {
                "tout (effacement complet)".to_owned()
            } else {
                let r = self.regions;
                let names: Vec<&str> = [(r.program, "programme"), (r.eeprom, "EEPROM"), (r.ids, "IDs"), (r.config, "config")]
                    .into_iter()
                    .filter_map(|(on, n)| on.then_some(n))
                    .collect();
                if names.is_empty() { "aucune".to_owned() } else { names.join(" + ") }
            };
            ui.menu_button(shown, |ui| {
                ui.label("« Programmer » écrit :");
                ui.checkbox(&mut self.regions.program, "Mémoire programme");
                ui.checkbox(&mut self.regions.eeprom, "EEPROM");
                ui.checkbox(&mut self.regions.ids, "User IDs");
                ui.checkbox(&mut self.regions.config, "Configuration");
                ui.separator();
                ui.weak("Tout coché : effacement complet puis écriture.\nSinon : seulement ces zones, sans effacement global\n(lignes effacées une à une, config : bits 1 → 0 seulement).");
                if ui.button("Tout").clicked() {
                    self.regions = Regions::ALL;
                }
            });
            ui.checkbox(&mut self.keep_eeprom, "Préserver l'EEPROM")
                .on_hover_text("L'effacement ne touche pas l'EEPROM ; les octets EEPROM du fichier sont quand même écrits");
            ui.separator();
            if ui.add_enabled(self.device.is_some(), egui::Button::new("Config…")).on_hover_text("Éditer les bits de configuration du fichier").clicked() {
                self.config_open = true;
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let enabled = !self.busy && self.device.is_some();
            ui.add_enabled_ui(enabled, |ui| {
                let jobs = [
                    (Job::Program, "Programmer", "Effacer, écrire, vérifier, lancer"),
                    (Job::Erase, "Effacer", "Bulk erase"),
                    (Job::Verify, "Vérifier", "Comparer la puce au fichier"),
                    (Job::Read, "Lire", "Lire toute la puce"),
                    (Job::Id, "ID", "Lire device et revision ID"),
                    (Job::Run, "Lancer", "Relâcher MCLR"),
                ];
                for (job, text, tip) in jobs {
                    let button =
                        egui::Button::new(RichText::new(text).size(15.0)).min_size([96.0, 30.0].into());
                    if ui.add(button).on_hover_text(tip).clicked() {
                        self.start(job);
                    }
                }
            });
            if self.busy {
                ui.spinner();
            }
        });
    }

    fn details(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let Some(d) = &self.device else {
                ui.label("Aucune puce chargée.");
                return;
            };
            ui.heading(&d.name);
            let algo = d
                .algorithm()
                .map_or_else(|_| "non supporté".to_owned(), |a| format!("{a:?}"));
            egui::Grid::new("info").num_columns(2).striped(true).show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(k);
                    ui.monospace(v);
                    ui.end_row();
                };
                row("Spec", format!("DS{}", d.prog_spec));
                row("Algorithme", algo);
                row("Device ID", format!("{:#06x}", d.device_id));
                row("Flash", format!("{} mots", d.flash_words));
                row("Ligne", format!("{} mots", d.row_words));
                if let Some((addr, bit)) = d.lvp_fuse {
                    row("LVP", format!("{addr:#06x} bit {}", bit.trailing_zeros()));
                }
                row("Effacement", format!("{} µs", d.t_erase_us));
                row("Écriture", format!("{} µs", d.t_program_us));
                row("Config", format!("{} µs", d.t_config_us));
            });

            ui.add_space(8.0);
            ui.strong("Puce connectée");
            match self.ids {
                Some(ids) => {
                    let ok = id_matches(d, ids);
                    let text = format!("ID {:#06x}  rév {:#06x}", ids.device_id, ids.revision_id);
                    ui.label(RichText::new(text).monospace().color(if ok { GREEN } else { RED }));
                }
                None => {
                    ui.weak("lance « ID » pour lire la puce");
                }
            }

            ui.add_space(8.0);
            ui.strong("User IDs et configuration");
            egui::Grid::new("config").num_columns(3).striped(true).show(ui, |ui| {
                ui.weak("adresse");
                ui.weak("fichier");
                ui.weak("puce");
                ui.end_row();
                let addrs = d.user_ids.clone().chain(d.config.iter().map(|c| c.address));
                for addr in addrs {
                    let file = self.image.as_ref().and_then(|i| i.config.get(&addr).copied());
                    let chip = self.chip.as_ref().and_then(|i| i.config.get(&addr).copied());
                    let fmt = |w: Option<u16>| w.map_or("—".to_owned(), |w| format!("{w:04X}"));
                    ui.monospace(format!("{addr:04X}"));
                    ui.monospace(fmt(file));
                    let differs = matches!((file, chip), (Some(a), Some(b)) if a != b);
                    let text = RichText::new(fmt(chip)).monospace();
                    ui.label(if differs { text.color(RED) } else { text });
                    ui.end_row();
                }
            });
        });
    }

    fn memory(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Mémoire programme");
            ui.selectable_value(&mut self.view, View::File, "Fichier");
            ui.add_enabled_ui(self.chip.is_some(), |ui| {
                ui.selectable_value(&mut self.view, View::Chip, "Puce");
            });
            ui.separator();
            ui.add_enabled_ui(!self.source_lines.is_empty(), |ui| {
                ui.selectable_value(&mut self.format, Format::Source, "Source");
            });
            ui.selectable_value(&mut self.format, Format::Hex, "Hex");
            ui.selectable_value(&mut self.format, Format::Asm, "Désassemblé");
            ui.add_enabled_ui(self.device.as_ref().is_some_and(|d| d.eeprom.is_some()), |ui| {
                ui.selectable_value(&mut self.format, Format::Eeprom, "EEPROM");
            });
            ui.separator();
            ui.selectable_value(&mut self.format, Format::Debug, "Débogueur")
                .on_hover_text("Exécuter le programme pas à pas sur un PIC simulé");
            if self.chip.is_some() && self.image.is_some() {
                ui.weak("rouge = différent de l'autre vue");
            }
        });
        ui.separator();

        if self.format == Format::Source {
            source_view(ui, &self.source_lines, &self.source_errors);
            return;
        }
        if self.format == Format::Debug {
            self.debug_view(ui);
            return;
        }
        let Some(d) = &self.device else { return };
        let (shown, other) = match self.view {
            View::File => (self.image.as_ref(), self.chip.as_ref()),
            View::Chip => (self.chip.as_ref(), self.image.as_ref()),
        };
        let Some(shown) = shown else {
            ui.weak(match self.view {
                View::File => "Aucun fichier .hex chargé.",
                View::Chip => "Lance « Lire » pour voir le contenu de la puce.",
            });
            return;
        };

        if self.format == Format::Eeprom {
            eeprom_view(ui, d, shown, other);
            return;
        }

        if self.format == Format::Asm {
            let lines = match self.view {
                View::File => &self.asm_file,
                View::Chip => &self.asm_chip,
            };
            asm_view(ui, lines, other);
            return;
        }

        let rows = (d.flash_words as usize).div_ceil(WORDS_PER_ROW);
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let dim = ui.visuals().weak_text_color();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show_rows(ui, row_height, rows, |ui, range| {
                for row in range {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let base = (row * WORDS_PER_ROW) as u32;
                        ui.label(RichText::new(format!("{base:04X}")).monospace().color(dim));
                        for addr in base..base + WORDS_PER_ROW as u32 {
                            let word = shown.flash.get(&addr).copied().unwrap_or(ERASED);
                            let differs = other.is_some_and(|o| {
                                o.flash.get(&addr).copied().unwrap_or(ERASED) != word
                            });
                            let text = RichText::new(format!("{word:04X}")).monospace();
                            ui.label(if differs {
                                text.color(RED)
                            } else if word == ERASED {
                                text.color(dim)
                            } else {
                                text
                            });
                        }
                    });
                }
            });
    }

    fn debug_view(&mut self, ui: &mut egui::Ui) {
        let (Some(device), Some(image)) = (&self.device, &self.image) else {
            ui.weak("Charge un .hex ou compile un .asm pour le déboguer.");
            return;
        };
        if self.debugger.is_none() {
            match debug::Debugger::new(device, image) {
                Ok(d) => self.debugger = Some(d),
                Err(e) => {
                    ui.colored_label(RED, format!("{e:#}"));
                    return;
                }
            }
        }
        if let Some(dbg) = &mut self.debugger {
            dbg.ui(ui, &self.asm_file, device);
        }
    }

    fn save_hex(&mut self) {
        let chip = self.view == View::Chip && self.chip.is_some();
        let image = if chip { self.chip.as_ref() } else { self.image.as_ref() };
        let Some(image) = image else { return };
        let name = if chip {
            format!("{}-lu.hex", self.part.to_ascii_lowercase())
        } else {
            Path::new(&self.hex_path).file_name().map_or("programme.hex".into(), |n| n.to_string_lossy().into_owned())
        };
        let Some(path) = rfd::FileDialog::new().add_filter("Intel HEX", &["hex"]).set_file_name(name).save_file() else {
            return;
        };
        match image.to_hex().map(|text| std::fs::write(&path, text)) {
            Ok(Ok(())) => {
                if !chip {
                    self.hex_path = path.display().to_string();
                    self.edited = false;
                }
                self.info(format!("enregistré : {}", path.display()));
            }
            Ok(Err(e)) => self.error(format!("{}: {e}", path.display())),
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    /// Config word editor: one combo box per named field.
    fn config_window(&mut self, ctx: &egui::Context) {
        if !self.config_open {
            return;
        }
        let mut open = true;
        let mut changed = false;
        egui::Window::new("Bits de configuration").open(&mut open).default_width(520.0).show(ctx, |ui| {
            let (Some(d), Some(image)) = (&self.device, &mut self.image) else {
                ui.label("Charge d'abord un .hex ou compile un .asm.");
                return;
            };
            egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                for (n, word) in d.config.iter().enumerate() {
                    let value = image.config.get(&word.address).copied().unwrap_or(word.default & 0x3FFF);
                    ui.horizontal(|ui| {
                        ui.strong(format!("CONFIG{}", n + 1));
                        ui.monospace(format!("{:04X} = {value:04X}", word.address));
                    });
                    egui::Grid::new(("cfg", word.address)).num_columns(2).striped(true).show(ui, |ui| {
                        for field in d.config_fields.iter().filter(|f| f.address == word.address) {
                            let mask = ((1u32 << field.width) - 1) as u16;
                            let current = (value >> field.shift) & mask;
                            ui.label(&field.name);
                            let mut picked = current;
                            if field.values.is_empty() {
                                ui.add(egui::DragValue::new(&mut picked).range(0..=mask).hexadecimal(1, false, true));
                            } else {
                                let shown = field
                                    .values
                                    .iter()
                                    .find(|(_, v)| *v == current)
                                    .map_or_else(|| format!("{current:#x}"), |(name, _)| name.clone());
                                egui::ComboBox::from_id_salt((word.address, &field.name)).selected_text(shown).show_ui(ui, |ui| {
                                    for (name, v) in &field.values {
                                        ui.selectable_value(&mut picked, *v, name);
                                    }
                                });
                            }
                            if picked != current {
                                let new = (value & !(mask << field.shift)) | (picked & mask) << field.shift;
                                image.config.insert(word.address, new);
                                changed = true;
                            }
                            ui.end_row();
                        }
                    });
                    ui.add_space(6.0);
                }
            });
            ui.weak("Les changements s'appliquent au fichier chargé : « Enregistrer… » pour les garder.");
        });
        if changed {
            self.edited = true;
            self.debugger = None;
        }
        self.config_open = open;
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Journal");
            if ui.small_button("Effacer").clicked() {
                self.log.clear();
            }
        });
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (level, text) in &self.log {
                    let color = match level {
                        Level::Info => ui.visuals().text_color(),
                        Level::Ok => GREEN,
                        Level::Error => RED,
                    };
                    ui.label(RichText::new(text).monospace().color(color));
                }
            });
    }
}

/// Assembles `source` and writes `<out_dir>/<stem>.hex`. Returns the HEX
/// path on success, the part named by the source, the log and diagnostics.
#[allow(clippy::type_complexity)]
fn assemble_to(
    source: &Path,
    options: &asm::Options,
    out_dir: &Path,
) -> anyhow::Result<(Option<PathBuf>, Option<String>, String, Vec<Diagnostic>)> {
    let result = asm::assemble_file(source, options)?;
    let part = result.device.as_ref().map(|d| d.name.clone());
    let hex = match &result.image {
        Some(image) => {
            std::fs::create_dir_all(out_dir)?;
            let stem = source.file_stem().unwrap_or_default();
            let path = out_dir.join(stem).with_extension("hex");
            std::fs::write(&path, image.to_hex()?)?;
            Some(path)
        }
        None => None,
    };
    Ok((hex, part, result.log(), result.diagnostics))
}

