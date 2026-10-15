//! Builds assembly sources with Microchip's `pic-as` (part of XC8, installed
//! separately by the user) for the part selected in picflash.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::device::{version_key, Device};

/// Install roots searched for `xc8/v*/pic-as/bin/pic-as`.
fn xc8_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications/microchip/xc8"),
        PathBuf::from("/opt/microchip/xc8"),
    ];
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(var) {
            roots.push(Path::new(&dir).join("Microchip").join("xc8"));
        }
    }
    roots
}

/// Finds `pic-as`: `PICFLASH_PIC_AS`, then `PATH`, then the newest XC8 install.
pub fn find_pic_as() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "pic-as.exe" } else { "pic-as" };
    if let Some(path) = std::env::var_os("PICFLASH_PIC_AS") {
        return Some(PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("PATH") {
        if let Some(hit) = std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.is_file()) {
            return Some(hit);
        }
    }
    let mut found: Vec<PathBuf> = xc8_roots()
        .iter()
        .flat_map(|root| std::fs::read_dir(root).into_iter().flatten().flatten())
        .map(|e| e.path().join("pic-as").join("bin").join(exe))
        .filter(|p| p.is_file())
        .collect();
    // .../xc8/<version>/pic-as/bin/pic-as
    found.sort_by_key(|p| version_key(p.parent().unwrap().parent().unwrap().parent().unwrap()));
    found.pop()
}

/// One compiler diagnostic, e.g. `blink.asm:6:: error: (876) syntax error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    pub line: Option<u32>,
    pub error: bool,
    pub message: String,
}

pub fn parse_diagnostics(log: &str) -> Vec<Diagnostic> {
    log.lines()
        .filter_map(|l| {
            let (place, rest) = l.split_once(":: ")?;
            let (error, message) = if let Some(m) = rest.strip_prefix("error: ") {
                (true, m)
            } else {
                (false, rest.strip_prefix("warning: ")?)
            };
            let (file, line) = match place.rsplit_once(':') {
                Some((f, n)) => (f.to_owned(), n.parse().ok()),
                None => (place.to_owned(), None),
            };
            Some(Diagnostic { file, line, error, message: message.to_owned() })
        })
        .collect()
}

/// Linker placement for the conventional `reset_vec`/`isr_vec` psects, only
/// for those the source declares (pic-as rejects options for undefined ones).
pub fn vector_options(source: &str) -> Vec<String> {
    let declares = |name: &str| {
        source.lines().any(|l| {
            let mut words = l.split(|c: char| c.is_whitespace() || c == ',');
            let first = words.find(|w| !w.is_empty());
            first.is_some_and(|w| w.eq_ignore_ascii_case("psect"))
                && words.find(|w| !w.is_empty()) == Some(name)
        })
    };
    let mut opts = Vec::new();
    if declares("reset_vec") {
        opts.push("-Wl,-preset_vec=0h".to_owned());
    }
    if declares("isr_vec") {
        opts.push("-Wl,-pisr_vec=4h".to_owned());
    }
    opts
}

pub struct Build {
    /// The HEX file, when the build succeeded.
    pub hex: Option<PathBuf>,
    /// Command line and compiler output.
    pub log: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// Assembles `source` for `device` into `out_dir`. `extra` options go after
/// the automatic ones.
pub fn assemble(
    pic_as: &Path,
    source: &Path,
    device: &Device,
    extra: &[String],
    out_dir: &Path,
) -> Result<Build> {
    let pack = device
        .pack_dir
        .as_ref()
        .context("dossier du device pack inconnu pour cette puce")?;
    let dfp = pack.join("xc8");
    if !dfp.is_dir() {
        bail!("le pack {} n'a pas de support XC8 ({})", device.name, dfp.display());
    }
    let text = std::fs::read_to_string(source)
        .with_context(|| format!("lecture de {}", source.display()))?;
    let stem = source.file_stem().context("nom de fichier invalide")?;
    std::fs::create_dir_all(out_dir)?;
    let out = out_dir.join(stem);

    let cpu = device.name.strip_prefix("PIC").unwrap_or(&device.name);
    let mut cmd = Command::new(pic_as);
    cmd.arg(format!("-mdfp={}", dfp.display()))
        .arg(format!("-mcpu={cpu}"))
        .arg("-xassembler-with-cpp")
        .arg("-Wa,-a")
        .args(vector_options(&text))
        .args(extra)
        .arg(source)
        .arg("-o")
        .arg(&out)
        .current_dir(source.parent().unwrap_or(Path::new(".")));
    let shown = format!("{} {}", pic_as.display(), cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    let output = cmd.output().with_context(|| format!("lancement de {}", pic_as.display()))?;

    let mut log = shown + "\n";
    log.push_str(&String::from_utf8_lossy(&output.stdout));
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    let hex = out.with_extension("hex");
    Ok(Build {
        hex: (output.status.success() && hex.is_file()).then_some(hex),
        diagnostics: parse_diagnostics(&log),
        log,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pic_as_diagnostics() {
        let log = "bad.asm:6:: error: (876) syntax error\n\
                   ::: error: (450) psect \"reset_vec\" was never defined, or is local\n\
                   x.asm:3:: warning: (2) something\n\
                   16F1789 Memory Summary:";
        let d = parse_diagnostics(log);
        assert_eq!(d.len(), 3);
        assert_eq!((d[0].file.as_str(), d[0].line, d[0].error), ("bad.asm", Some(6), true));
        assert_eq!(d[0].message, "(876) syntax error");
        assert_eq!(d[1].line, None);
        assert!(!d[2].error);
    }

    #[test]
    fn vector_options_follow_declared_psects() {
        let blink = include_str!("../tests/fixtures/blink.asm");
        assert_eq!(vector_options(blink), ["-Wl,-preset_vec=0h", "-Wl,-pisr_vec=4h"]);
        assert!(vector_options("PSECT code,class=CODE,delta=2\n").is_empty());
        assert_eq!(vector_options("  psect reset_vec, class=CODE\n"), ["-Wl,-preset_vec=0h"]);
    }
}
