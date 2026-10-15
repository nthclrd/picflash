use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use picflash::device;
use picflash::image::Image;
use picflash::algo::Entry;
use picflash::checksum::checksum;
use picflash::programmer::{Programmer, Regions};
use picflash::tool::{self, Choice};

#[derive(Parser)]
#[command(
    version,
    about = "Program PIC microcontrollers through a picflash board, a PICkit 4/5/Basic or a Snap"
)]
struct Cli {
    /// Programmer: auto (picflash board, else PICkit/Snap), board, pickit or sim.
    #[arg(long, global = true, default_value = "auto")]
    tool: ToolArg,
    /// Serial port of the board, or serial number of the PICkit.
    #[arg(long, global = true)]
    port: Option<String>,
    /// Low-voltage entry ("MCHP" key, needs LVP = ON). Default on the board
    /// and the PICkit Basic / Snap.
    #[arg(long, global = true, conflicts_with = "hv")]
    lv: bool,
    /// High-voltage entry (VPP on MCLR). Default on PICkit 4/5.
    #[arg(long, global = true)]
    hv: bool,
    /// Power the target from the PICkit 4/5 at this voltage, e.g. 3.3 or 5.
    #[arg(long, global = true, value_name = "VOLTS")]
    power: Option<f64>,
    /// Target part.
    #[arg(short = 'P', long, global = true, default_value = "PIC16F1789")]
    part: String,
    /// Device pack directory (default: PICFLASH_PACKS, then MPLAB X installs).
    #[arg(long, global = true)]
    packs: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ToolArg {
    Auto,
    Board,
    Pickit,
    Sim,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum RegionArg {
    Program,
    Eeprom,
    Ids,
    Config,
}

fn to_regions(list: &[RegionArg]) -> Regions {
    let mut r = Regions::NONE;
    for a in list {
        match a {
            RegionArg::Program => r.program = true,
            RegionArg::Eeprom => r.eeprom = true,
            RegionArg::Ids => r.ids = true,
            RegionArg::Config => r.config = true,
        }
    }
    r
}

#[derive(Subcommand)]
enum Command {
    /// Show the part's programming parameters from the device pack (offline).
    Info,
    /// Disassemble a HEX file (offline).
    Disasm { hex: PathBuf },
    /// Device checksum of a HEX file, as MPLAB IPE shows it (offline).
    Checksum { hex: PathBuf },
    /// Assemble a pic-as source file into a HEX file (offline, no XC8 needed).
    Asm {
        source: PathBuf,
        /// Output HEX file (default: source with .hex).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Psect placement as with pic-as, e.g. -Wl,-pmytable=100h.
        #[arg(long = "link", allow_hyphen_values = true)]
        link: Vec<String>,
        /// Print where each psect was placed.
        #[arg(long)]
        map: bool,
    },
    /// Rebuild the embedded device database from installed device packs.
    #[command(hide = true)]
    DbBuild { out: PathBuf },
    /// List parts found in the device packs, grouped by programming spec (offline).
    Parts {
        /// Print every part, not just the per-spec summary.
        #[arg(long)]
        all: bool,
    },
    /// Check the link to the programmer.
    Ping,
    /// Read device and revision IDs.
    Id,
    /// Bulk erase the whole chip.
    Erase {
        /// Leave the data EEPROM as it is.
        #[arg(long)]
        keep_eeprom: bool,
    },
    /// Erase, program and verify a HEX file, then run it.
    Write {
        hex: PathBuf,
        #[arg(long)]
        no_verify: bool,
        /// Keep MCLR low after programming.
        #[arg(long)]
        hold_reset: bool,
        /// Allow config that disables LVP in low-voltage mode (the chip then
        /// needs high voltage).
        #[arg(long)]
        allow_lvp_off: bool,
        /// Leave the data EEPROM as it is (bytes in the HEX are still written).
        #[arg(long)]
        keep_eeprom: bool,
        /// Write only these regions, without a bulk erase: any of
        /// program,eeprom,ids,config.
        #[arg(long, value_delimiter = ',')]
        only: Vec<RegionArg>,
    },
    /// Compare chip contents with a HEX file.
    Verify { hex: PathBuf },
    /// Dump the chip to a HEX file.
    Read { out: PathBuf },
    /// Release reset so the target runs.
    Run,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let roots = match &cli.packs {
        Some(dir) => vec![dir.clone()],
        None => device::default_pack_roots(),
    };
    if let Command::DbBuild { out } = &cli.command {
        let db = picflash::devdb::build(&roots)?;
        let bytes = db.encode()?;
        std::fs::write(out, &bytes)?;
        println!(
            "{} parts, {:.2} Mo, depuis {}",
            db.parts.len(),
            bytes.len() as f64 / 1e6,
            db.sources.join(", ")
        );
        return Ok(());
    }
    if let Command::Asm { source, out, link, map } = &cli.command {
        return assemble(source, out.as_ref(), link, &cli.part, *map);
    }
    if let Command::Parts { all } = cli.command {
        // Device packs only when asked for; the embedded database otherwise.
        return match &cli.packs {
            Some(_) => list_parts(&roots, all),
            None => list_embedded(all),
        };
    }
    let device = &device::load(&roots, &cli.part)?;
    let load = |path: &PathBuf| -> Result<Image> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        Image::parse_hex(&text, device)
    };
    match &cli.command {
        Command::Info => {
            print_info(device);
            return Ok(());
        }
        Command::Disasm { hex } => return print_disasm(device, &load(hex)?),
        Command::Checksum { hex } => {
            println!("checksum 0x{:04X}", checksum(&load(hex)?, device));
            return Ok(());
        }
        _ => {}
    }
    device.algorithm()?;

    let choice = match cli.tool {
        ToolArg::Auto => Choice::Auto,
        ToolArg::Board => Choice::Board,
        ToolArg::Pickit => Choice::PicKit,
        ToolArg::Sim => Choice::Sim,
    };
    let opts = tool::Options {
        entry: match (cli.lv, cli.hv) {
            (true, _) => Some(Entry::LowVoltage),
            (_, true) => Some(Entry::HighVoltage),
            _ => None,
        },
        power_mv: cli.power.map(|v| (v * 1000.0).round() as u16),
        port: cli.port.clone(),
    };
    let tool::Opened { mut link, entry, summary } = tool::open(choice, device, &opts)?;
    eprintln!("{summary}");
    let mut prog = Programmer::new(&mut link, device)?;
    prog.entry = entry;
    prog.ping()?;

    match cli.command {
        Command::Info
        | Command::Parts { .. }
        | Command::Disasm { .. }
        | Command::Checksum { .. }
        | Command::Asm { .. }
        | Command::DbBuild { .. } => {
            unreachable!()
        }
        Command::Ping => println!("programmer OK"),
        Command::Id => {
            let ids = prog.read_ids()?;
            let name = if ids.device_id & device.device_id_mask == device.device_id {
                device.name.as_str()
            } else {
                "unknown"
            };
            println!(
                "device ID {:#06x} ({name}), revision {:#06x}",
                ids.device_id, ids.revision_id
            );
        }
        Command::Erase { keep_eeprom } => {
            prog.check_id()?;
            prog.keep_eeprom = keep_eeprom;
            prog.erase()?;
            println!("erased");
        }
        Command::Write { hex, no_verify, hold_reset, allow_lvp_off, keep_eeprom, only } => {
            let image = load(&hex)?;
            prog.check_id()?;
            prog.keep_eeprom = keep_eeprom;
            let regions = if only.is_empty() { Regions::ALL } else { to_regions(&only) };
            let start = Instant::now();
            if only.is_empty() {
                prog.write(&image, allow_lvp_off)?;
            } else {
                prog.write_regions(&image, regions, allow_lvp_off)?;
            }
            println!(
                "programmed {} words + {} config words + {} EEPROM bytes in {:.2?} (checksum 0x{:04X})",
                image.flash.len(),
                image.config.len(),
                image.eeprom.len(),
                start.elapsed(),
                checksum(&image, device)
            );
            if !no_verify {
                report(&prog.verify_regions(&image, regions)?)?;
            }
            if !hold_reset {
                prog.release()?;
            }
        }
        Command::Verify { hex } => {
            let image = load(&hex)?;
            prog.check_id()?;
            report(&prog.verify(&image)?)?;
            prog.release()?;
        }
        Command::Read { out } => {
            prog.check_id()?;
            let image = prog.read()?;
            std::fs::write(&out, image.to_hex()?)?;
            prog.release()?;
            println!(
                "wrote {} ({} words used, {} EEPROM bytes, checksum 0x{:04X})",
                out.display(),
                image.flash.len(),
                image.eeprom.len(),
                checksum(&image, device)
            );
        }
        Command::Run => {
            prog.release()?;
            println!("target running");
        }
    }
    Ok(())
}

fn list_embedded(all: bool) -> Result<()> {
    use std::collections::BTreeMap;

    let db = picflash::devdb::Database::embedded();
    let mut specs: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for d in &db.parts {
        specs.entry(&d.prog_spec).or_default().push(&d.name);
    }
    for (spec, names) in &specs {
        println!("DS{spec:<10} {:>4} parts", names.len());
        if all {
            println!("    {}", names.join(" "));
        }
    }
    println!("{} parts intégrées ({} specs), données de {}", db.parts.len(), specs.len(), db.sources.join(", "));
    Ok(())
}

fn list_parts(roots: &[PathBuf], all: bool) -> Result<()> {
    use std::collections::BTreeMap;

    let parts = device::scan(roots);
    if parts.is_empty() {
        bail!("no device packs found; pass --packs");
    }
    // spec -> (arch, part names)
    let mut specs: BTreeMap<String, (String, std::collections::BTreeSet<String>)> = BTreeMap::new();
    let mut failed = 0;
    for (path, dev) in &parts {
        match dev {
            Ok(d) => {
                let entry = specs.entry(d.prog_spec.clone()).or_default();
                entry.0.clone_from(&d.arch);
                entry.1.insert(d.name.clone());
            }
            Err(e) => {
                failed += 1;
                if all {
                    eprintln!("  {}: {e:#}", path.display());
                }
            }
        }
    }
    let mut ranked: Vec<_> = specs.into_iter().collect();
    ranked.sort_by_key(|(_, (_, names))| std::cmp::Reverse(names.len()));
    let mut supported = 0;
    for (spec, (arch, names)) in &ranked {
        let probe = device::Device {
            prog_spec: spec.clone(),
            ..device::Device::default()
        };
        let ok = probe.algorithm().is_ok();
        if ok {
            supported += names.len();
        }
        let mark = if ok { "yes" } else { "-" };
        println!("DS{spec:<10} {arch:<8} {:>4} parts  supported: {mark}", names.len());
        if all {
            println!("    {}", names.iter().cloned().collect::<Vec<_>>().join(" "));
        }
    }
    println!(
        "{} parts, {} specs, {supported} supported, {failed} unreadable",
        ranked.iter().map(|(_, (_, n))| n.len()).sum::<usize>(),
        ranked.len()
    );
    Ok(())
}

fn assemble(source: &std::path::Path, out: Option<&PathBuf>, link: &[String], part: &str, map: bool) -> Result<()> {
    let mut options = picflash::asm::Options { part: Some(part.to_owned()), ..Default::default() };
    options.parse_extra(link).map_err(anyhow::Error::msg)?;
    let result = picflash::asm::assemble_file(source, &options)?;
    eprint!("{}", result.log());
    if map {
        for (name, class, base, len) in &result.map {
            println!("  {name:<16} {class:<8} {base:#06x}  {len:>5}");
        }
    }
    let Some(image) = &result.image else {
        bail!("assemblage échoué");
    };
    let out = out.cloned().unwrap_or_else(|| source.with_extension("hex"));
    std::fs::write(&out, image.to_hex()?).with_context(|| format!("écriture de {}", out.display()))?;
    let device = result.device.as_ref().unwrap();
    println!(
        "{} : {} mots de programme sur {}, {} mots de config -> {}",
        device.name,
        image.flash.len(),
        device.flash_words,
        image.config.len(),
        out.display()
    );
    Ok(())
}

fn print_disasm(device: &device::Device, image: &Image) -> Result<()> {
    let mut prev = None;
    for line in picflash::disasm::disassemble(&image.flash, device)? {
        if prev.is_some_and(|p: u32| p + 1 != line.addr) {
            println!();
        }
        if let Some(label) = &line.label {
            println!("{label}:");
        }
        println!("    {:04X}  {:04X}    {}", line.addr, line.word, line.text);
        prev = Some(line.addr);
    }
    if !image.config.is_empty() {
        println!();
        for (addr, word) in &image.config {
            println!("    {addr:04X}  {word:04X}    ; config");
        }
    }
    Ok(())
}

fn print_info(d: &device::Device) {
    println!("{} (arch {}, programming spec DS{})", d.name, d.arch, d.prog_spec);
    match d.algorithm() {
        Ok(a) => println!("  algorithm   {a:?}"),
        Err(e) => println!("  algorithm   none: {e}"),
    }
    println!("  device ID   {:#06x} mask {:#06x}", d.device_id, d.device_id_mask);
    println!("  flash       {:#06x} words, rows of {}", d.flash_words, d.row_words);
    println!("  user IDs    {:#06x}..{:#06x}", d.user_ids.start, d.user_ids.end);
    for c in &d.config {
        println!("  config      {:#06x} mask {:#06x}", c.address, c.mask);
    }
    if let Some((addr, bit)) = d.lvp_fuse {
        println!("  LVP fuse    {addr:#06x} bit {}", bit.trailing_zeros());
    }
    if let Some(r) = &d.eeprom {
        println!("  EEPROM      {:#06x}..{:#06x}", r.start, r.end);
    }
    println!(
        "  waits       erase {} us, row {} us, config {} us",
        d.t_erase_us, d.t_program_us, d.t_config_us
    );
}

fn report(bad: &[picflash::programmer::Mismatch]) -> Result<()> {
    if bad.is_empty() {
        println!("verify OK");
        return Ok(());
    }
    for m in bad.iter().take(10) {
        eprintln!(
            "  {:#06x}: expected {:#06x}, read {:#06x}",
            m.address, m.expected, m.found
        );
    }
    bail!("verify failed at {} words", bad.len())
}
