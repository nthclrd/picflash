//! Drop-in for MPLAB IPE's `ipecmd` command line.
//!
//! Accepts the usual switches (`-` or `/`, any case), e.g. the lab Makefile's
//! `-TPPK4 -E -M -P16F1789 -F"blink.hex" -OL`. Tools: `-TPPK4`, `-TPPK5`,
//! `-TPPKBASIC`, `-TPSNAP` (Microchip tools in MPLAB mode), `-TPPPF` (picflash
//! board), `-TPSIM` (bit-level simulator: a dry run without hardware). Any
//! other name picks whatever programmer is plugged in.
//!
//! Supported: -P -T -F -M -E -Y -GF -I -OL -L -W -OH -?. Exit code 0 on
//! success, 1 on failure, 2 on bad arguments.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use picflash::algo::Entry;
use picflash::device::{self, Device};
use picflash::image::Image;
use picflash::checksum::checksum;
use picflash::programmer::{Mismatch, Programmer, Regions};
use picflash::tool::{self, Choice, Link};

#[derive(Default)]
struct Args {
    part: Option<String>,
    tool: Choice,
    port: Option<String>,
    low_voltage: bool,
    /// `Some(None)`: -W without a voltage, the part's usual VDD.
    power: Option<Option<f64>>,
    hex: Option<PathBuf>,
    /// `Regions::ALL` for a full program (with erase), else the regions of
    /// `-MP`, `-ME`, `-MI`, `-MC`.
    program: Option<Regions>,
    erase: bool,
    verify: Option<Regions>,
    keep_eeprom: bool,
    show_checksum: bool,
    read_to: Option<PathBuf>,
    show_id: bool,
    release: bool,
}

const HELP: &str = "\
picflash-ipecmd — remplaçant d'ipecmd (carte picflash, PICkit 4/5/Basic, Snap)

  -P<part>     puce, avec ou sans préfixe PIC (-P16F1789)
  -TP<outil>   PK4, PK5, PKBASIC, SNAP, PPF (carte picflash), SIM (simulateur) ;
               autre nom = programmateur branché
  -TS<id>      n° de série du PICkit, ou port série de la carte
  -F<fichier>  fichier HEX
  -M           programmer tout (efface d'abord)
  -MP -ME -MI -MC  programmer seulement mémoire programme / EEPROM / user IDs /
               config, sans effacement global (cumulables : -MPE)
  -E           effacer (efface aussi l'EEPROM, même avec -Z)
  -Y[PEIC]     vérifier contre le fichier HEX (tout, ou les zones données)
  -Z[plage]    préserver l'EEPROM pendant la programmation
  -K           afficher le checksum du fichier HEX
  -GF<fichier> lire la puce vers un fichier HEX
  -I           afficher le device ID
  -OL          relâcher le reset à la fin
  -L           entrée basse tension (LVP) ; sinon haute tension sur PICkit 4/5
  -W[volts]    alimenter la cible depuis le PICkit 4/5 (-W, -W3.3, -W5)
  -OH          accepté
  -?           cette aide";

fn parse(argv: impl Iterator<Item = String>) -> Result<Args> {
    let mut args = Args::default();
    for raw in argv {
        let Some(body) = raw.strip_prefix('-').or_else(|| raw.strip_prefix('/')) else {
            bail!("argument inattendu : {raw}");
        };
        let upper = body.to_ascii_uppercase();
        let value = |n: usize| body[n..].trim_matches('"').to_owned();
        match upper.as_bytes() {
            [b'?'] | [b'H'] => {
                println!("{HELP}");
                std::process::exit(0);
            }
            [b'P', ..] => args.part = Some(value(1)),
            [b'T', b'P', ..] => {
                args.tool = match &upper[2..] {
                    "SIM" | "PSIM" => Choice::Sim,
                    "PF" | "PPF" => Choice::Board,
                    "PK4" | "PK5" | "PKBASIC" | "SNAP" => Choice::PicKit,
                    _ => Choice::Auto,
                }
            }
            [b'T', b'S', ..] => args.port = Some(value(2)),
            [b'T'] => {}
            [b'F', ..] => args.hex = Some(PathBuf::from(value(1))),
            [b'M'] => args.program = Some(Regions::ALL),
            [b'M', ..] => args.program = Some(merge(args.program.unwrap_or(Regions::NONE), regions(&upper[1..], &raw)?)),
            [b'E'] => args.erase = true,
            [b'Y'] => args.verify = Some(Regions::ALL),
            [b'Y', ..] => args.verify = Some(merge(args.verify.unwrap_or(Regions::NONE), regions(&upper[1..], &raw)?)),
            [b'Z', ..] => args.keep_eeprom = true,
            [b'K'] => args.show_checksum = true,
            [b'G', b'F', ..] => args.read_to = Some(PathBuf::from(value(2))),
            [b'G', ..] => bail!("-{body} : seule la lecture vers fichier (-GF<fichier>) est supportée"),
            [b'I'] => args.show_id = true,
            [b'O', b'L'] => args.release = true,
            [b'L'] => args.low_voltage = true,
            [b'O', b'H'] => {}
            [b'W'] => args.power = Some(None),
            [b'W', ..] => {
                let v: f64 = value(1).parse().with_context(|| format!("tension invalide : {raw}"))?;
                args.power = Some(Some(v));
            }
            [b'J'] => bail!("-J : l'entrée haute tension n'est pas encore supportée"),
            _ => bail!("option inconnue : {raw} (voir -?)"),
        }
    }
    Ok(args)
}

/// Region letters of `-M`/`-Y`: P program, E EEPROM, I user IDs, C config.
fn regions(letters: &str, raw: &str) -> Result<Regions> {
    let mut r = Regions::NONE;
    for c in letters.chars() {
        match c {
            'P' => r.program = true,
            'E' => r.eeprom = true,
            'I' => r.ids = true,
            'C' => r.config = true,
            'B' | 'A' => bail!("{raw} : pas de mémoire boot/auxiliaire sur les PIC16"),
            _ => bail!("{raw} : zone inconnue (plages d'adresses non supportées)"),
        }
    }
    Ok(r)
}

fn merge(a: Regions, b: Regions) -> Regions {
    Regions {
        program: a.program || b.program,
        eeprom: a.eeprom || b.eeprom,
        ids: a.ids || b.ids,
        config: a.config || b.config,
    }
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("erreur : {e:#}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => {
            println!("Operation Succeeded");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erreur : {e:#}");
            println!("Operation Failed");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<()> {
    let part = args.part.as_deref().context("-P<part> manquant")?;
    let device = device::load(&device::default_pack_roots(), part)?;
    let image = match &args.hex {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("lecture de {}", path.display()))?;
            Some(Image::parse_hex(&text, &device)?)
        }
        None => None,
    };
    if (args.program.is_some() || args.verify.is_some() || args.show_checksum) && image.is_none() {
        bail!("-M/-Y/-K demandent un fichier HEX (-F)");
    }
    if let (true, Some(image)) = (args.show_checksum, &image) {
        println!("Checksum : 0x{:04X}", checksum(image, &device));
    }
    if args.tool == Choice::Auto
        && args.program.is_none()
        && args.verify.is_none()
        && !args.erase
        && args.read_to.is_none()
        && !args.show_id
        && !args.release
    {
        // Only a checksum or nothing was asked: no programmer needed.
        return Ok(());
    }

    let power_mv = args.power.map(|v| match v {
        Some(volts) => (volts * 1000.0).round() as u16,
        // The part's usual supply: 5 V parts at 5 V, LF parts at 3.3 V.
        None if device.power.vdd_max_mv >= 5000 => 5000,
        None => 3300,
    });
    let opts = tool::Options {
        entry: args.low_voltage.then_some(Entry::LowVoltage),
        power_mv,
        port: args.port.clone(),
    };
    let tool::Opened { mut link, entry, summary } = tool::open(args.tool, &device, &opts)
        .context("programmateur introuvable (simule avec -TPSIM)")?;
    println!("{} : {summary}", device.name);
    run_on(&mut link, entry, &device, image.as_ref(), args)?;
    if let Link::Sim(pic) = &link {
        println!("Durée estimée sur carte : {:.2} s", pic.now_us as f64 / 1e6);
    }
    Ok(())
}

fn run_on(
    link: &mut Link,
    entry: Entry,
    device: &Device,
    image: Option<&Image>,
    args: &Args,
) -> Result<()> {
    let mut prog = Programmer::new(link, device)?;
    prog.entry = entry;
    prog.ping()?;
    let ids = prog.check_id()?;
    if args.show_id || args.program.is_some() || args.erase {
        println!("Device ID {:#06x}, révision {:#06x}", ids.device_id, ids.revision_id);
    }

    // Same order as ipecmd: erase, program, verify, read, release. -E wins
    // over -Z: an explicit erase clears the EEPROM too.
    let full = args.program == Some(Regions::ALL);
    if args.erase && !full {
        println!("Erasing...");
        prog.erase()?;
        println!("Erase successful");
    }
    prog.keep_eeprom = args.keep_eeprom && !args.erase;
    if let (Some(regions), Some(image)) = (args.program, image) {
        println!("Programming...");
        if full {
            prog.write(image, false)?;
        } else {
            prog.write_regions(image, regions, false)?;
        }
        println!(
            "Program succeeded ({} mots, {} mots de config, {} octets EEPROM)",
            image.flash.len(),
            image.config.len(),
            image.eeprom.len()
        );
    }
    if let (Some(regions), Some(image)) = (args.verify.or(args.program), image) {
        println!("Verifying...");
        report(&prog.verify_regions(image, regions)?)?;
    }
    if let Some(path) = &args.read_to {
        println!("Reading...");
        let dump = prog.read()?;
        std::fs::write(path, dump.to_hex()?)?;
        println!("Read successful : {}", path.display());
    }
    if args.release {
        prog.release()?;
        println!("Reset relâché");
    }
    Ok(())
}

fn report(bad: &[Mismatch]) -> Result<()> {
    if bad.is_empty() {
        println!("Verify successful");
        return Ok(());
    }
    for m in bad.iter().take(10) {
        eprintln!("  {:#06x} : attendu {:#06x}, lu {:#06x}", m.address, m.expected, m.found);
    }
    bail!("Verify failed ({} mots)", bad.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        parse(list.iter().map(|s| s.to_string())).unwrap()
    }

    #[test]
    fn lab_makefile_flash_args() {
        let a = args(&["-TPPK4", "-E", "-M", "-P16F1789", "-F\"blink.hex\"", "-OL"]);
        assert_eq!(a.part.as_deref(), Some("16F1789"));
        assert_eq!(a.hex, Some(PathBuf::from("blink.hex")));
        assert!(a.erase && a.program == Some(Regions::ALL) && a.release);
        assert_eq!(a.tool, Choice::PicKit);
    }

    #[test]
    fn low_voltage_and_power() {
        let a = args(&["-TPPKBASIC", "-L", "-W3.3"]);
        assert!(a.low_voltage);
        assert_eq!(a.power, Some(Some(3.3)));
        assert_eq!(args(&["-W"]).power, Some(None));
        assert_eq!(args(&["-TPPPF"]).tool, Choice::Board);
    }

    #[test]
    fn simulator_and_slash_switches() {
        let a = args(&["/tpsim", "/p16f18446", "/fx.hex", "/m", "/y", "/gfdump.hex"]);
        assert!(a.tool == Choice::Sim && a.program == Some(Regions::ALL) && a.verify == Some(Regions::ALL));
        assert_eq!(a.read_to, Some(PathBuf::from("dump.hex")));
    }

    #[test]
    fn region_switches() {
        let a = args(&["-MP", "-MC", "-YE", "-Z", "-K"]);
        assert_eq!(a.program, Some(Regions { program: true, config: true, ..Regions::NONE }));
        assert_eq!(a.verify, Some(Regions { eeprom: true, ..Regions::NONE }));
        assert!(a.keep_eeprom && a.show_checksum);
        assert_eq!(args(&["-MPEIC"]).program, Some(Regions::ALL));
        assert!(parse(["-MP0-1FF".to_string()].into_iter()).is_err());
    }
}
