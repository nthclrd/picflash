//! PIC16 enhanced mid-range assembler, source-compatible with the subset of
//! pic-as (XC8) used in lab code: `PROCESSOR`, `CONFIG`, `#include <xc.inc>`,
//! `PSECT`, `DS`/`DB`/`DW`, `EQU`/`SET`, `ORG`/`ALIGN`, `BANKSEL`/`PAGESEL`,
//! `MACRO`/`REPT`/`IF`, every instruction of the core, and the linker's psect
//! placement. Built from the instruction set in the datasheets and checked
//! by assembling the same sources with pic-as and comparing the HEX output.
//!
//! Part data (registers, bits, config fields, memory map) comes from the
//! embedded device database, so neither XC8 nor MPLAB X is needed.

mod expr;
mod link;
mod pre;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result};

use crate::devdb::{header_symbols, Database};
use crate::device::Device;
use crate::image::Image;
use crate::toolchain::Diagnostic;
use expr::{is_ident_char, is_ident_start, Env, Expr};
use link::{Placement, Psect};
use pre::{split_args, Item, Preprocessor};

/// Source position for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loc {
    pub file: Rc<str>,
    pub line: u32,
}

/// Result of an assembly: the image when there were no errors.
pub struct Assembly {
    pub device: Option<Device>,
    pub image: Option<Image>,
    pub diagnostics: Vec<Diagnostic>,
    /// Label and EQU values, for listings.
    pub symbols: BTreeMap<String, i64>,
    /// Psect name, class, base address, length, in address order.
    pub map: Vec<(String, String, u32, u32)>,
}

impl Assembly {
    pub fn ok(&self) -> bool {
        self.image.is_some()
    }

    /// Human-readable diagnostics, one per line, in `file:line: error: msg` form.
    pub fn log(&self) -> String {
        let mut out = String::new();
        for d in &self.diagnostics {
            let kind = if d.error { "error" } else { "warning" };
            match d.line {
                Some(l) => out.push_str(&format!("{}:{l}:: {kind}: {}\n", d.file, d.message)),
                None => out.push_str(&format!("{}:: {kind}: {}\n", d.file, d.message)),
            }
        }
        out
    }
}

/// Options that pic-as takes on its command line.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Part when the source has no `PROCESSOR` line.
    pub part: Option<String>,
    /// Fixed psect addresses (`-Wl,-pNAME=ADDR`). `reset_vec` at 0 and
    /// `isr_vec` at 4 are added when the source declares them, as in the lab
    /// Makefile.
    pub placements: Vec<(String, u32)>,
}

impl Options {
    /// Parses pic-as style extra options: `-Wl,-preset_vec=0h`, `-P16F1789`.
    pub fn parse_extra(&mut self, args: &[String]) -> Result<(), String> {
        for arg in args {
            for piece in arg.split_whitespace() {
                if let Some(p) = piece.strip_prefix("-Wl,-p").or_else(|| piece.strip_prefix("-p")) {
                    for spec in p.split(',') {
                        let (name, addr) = spec.split_once('=').ok_or(format!("option {piece} : NOM=ADRESSE attendu"))?;
                        let addr = expr::parse_number(addr)? as u32;
                        self.placements.push((name.to_owned(), addr));
                    }
                } else if let Some(cpu) = piece.strip_prefix("-mcpu=") {
                    self.part = Some(cpu.to_owned());
                } else if piece == "-xassembler-with-cpp" || piece.starts_with("-Wa,") || piece.starts_with("-mdfp=") {
                    // Always on / not needed.
                } else {
                    return Err(format!("option non supportée : {piece}"));
                }
            }
        }
        Ok(())
    }
}

/// Assembles the file at `path`.
pub fn assemble_file(path: &Path, options: &Options) -> Result<Assembly> {
    let text = std::fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))?;
    Ok(assemble(path, &text, options))
}

/// `PROCESSOR 16F1789` from the raw source, before preprocessing.
pub fn processor_of(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.split(';').next()?.trim();
        let (kw, rest) = l.split_once(char::is_whitespace)?;
        kw.eq_ignore_ascii_case("processor").then(|| rest.trim().to_owned())
    })
}

pub fn assemble(path: &Path, text: &str, options: &Options) -> Assembly {
    let file: Rc<str> = Rc::from(path.display().to_string());
    let fail = |msg: String| Assembly {
        device: None,
        image: None,
        diagnostics: vec![Diagnostic { file: file.to_string(), line: None, error: true, message: msg }],
        symbols: BTreeMap::new(),
        map: Vec::new(),
    };
    let Some(part) = processor_of(text).or_else(|| options.part.clone()) else {
        return fail("aucune puce : ajoute « PROCESSOR 16F1789 » en tête du fichier".into());
    };
    let Some(device) = Database::embedded().find(&part) else {
        return fail(format!("puce inconnue ou non supportée : {part}"));
    };
    if !device.arch.starts_with("16E") && !device.arch.eq_ignore_ascii_case("PIC14E") && device.asm.registers.is_empty() {
        return fail(format!("{} : pas de données assembleur pour cette puce", device.name));
    }

    let mut pre = Preprocessor::new(device);
    pre.run(path, text, 0);
    let mut asm = Assembler::new(device, options, text);
    for (loc, msg, error) in std::mem::take(&mut pre.errors) {
        asm.diag(&loc, msg, error);
    }
    let lines: Vec<Item> = std::mem::take(&mut pre.items);
    asm.block(&lines, 0);
    asm.finish()
}

#[derive(Debug, Clone)]
enum SymDef {
    /// Absolute value (EQU of a constant, SET, header symbol).
    Value(i64),
    /// EQU of an expression involving labels, resolved after linking.
    Expr(Expr),
    /// Label: psect index and offset.
    Label(usize, u32),
}

/// One emitted unit inside a psect.
#[derive(Debug, Clone)]
enum Emit {
    Insn { mnemonic: String, args: Vec<String> },
    /// `DW` / `DB` values, one word each (DB stores the low byte).
    Words { values: Vec<Expr>, mask: u16 },
}

#[derive(Debug, Clone)]
struct Stmt {
    loc: Loc,
    psect: usize,
    offset: u32,
    emit: Emit,
}

struct MacroDef {
    params: Vec<String>,
    locals: Vec<String>,
    body: Vec<Item>,
}

struct Cond {
    active: bool,
    taken: bool,
    outer: bool,
}

struct Assembler<'a> {
    device: &'a Device,
    symbols: HashMap<String, SymDef>,
    /// Upper-cased names declared as SET: substituted at use.
    set_names: HashMap<String, ()>,
    psects: Vec<Psect>,
    current: Option<usize>,
    stmts: Vec<Stmt>,
    config: BTreeMap<u32, u16>,
    config_used: bool,
    macros: HashMap<String, Rc<MacroDef>>,
    conds: Vec<Cond>,
    local_counter: u32,
    placements: Vec<(String, u32)>,
    diagnostics: Vec<Diagnostic>,
    ended: bool,
}

const MAX_MACRO_DEPTH: usize = 32;

fn upper(s: &str) -> String {
    s.to_ascii_uppercase()
}

/// Splits `word rest` on the first whitespace.
fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim()),
        None => (s, ""),
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(is_ident_start) && chars.all(is_ident_char)
}

impl<'a> Assembler<'a> {
    fn new(device: &'a Device, options: &Options, source: &str) -> Self {
        let mut placements = options.placements.clone();
        for (name, addr) in [("reset_vec", 0u32), ("isr_vec", 4)] {
            let declared = crate::toolchain::vector_options(source)
                .iter()
                .any(|o| o.contains(&format!("-p{name}=")));
            if declared && !placements.iter().any(|(n, _)| n == name) {
                placements.push((name.to_owned(), addr));
            }
        }
        let mut config = BTreeMap::new();
        for word in &device.config {
            config.insert(word.address, word.default);
        }
        Self {
            device,
            symbols: HashMap::new(),
            set_names: HashMap::new(),
            psects: Vec::new(),
            current: None,
            stmts: Vec::new(),
            config,
            config_used: false,
            macros: HashMap::new(),
            conds: Vec::new(),
            local_counter: 0,
            placements,
            diagnostics: Vec::new(),
            ended: false,
        }
    }

    fn diag(&mut self, loc: &Loc, msg: impl Into<String>, error: bool) {
        self.diagnostics.push(Diagnostic {
            file: loc.file.to_string(),
            line: Some(loc.line),
            error,
            message: msg.into(),
        });
    }

    fn err(&mut self, loc: &Loc, msg: impl Into<String>) {
        self.diag(loc, msg, true);
    }

    fn wide_bsr(&self) -> bool {
        self.device.data_end > 0x1000
    }

    /// Processes a block of lines; nested MACRO/REPT bodies are collected
    /// up to their matching ENDM.
    fn block(&mut self, items: &[Item], depth: usize) {
        let mut i = 0;
        while i < items.len() {
            if self.ended {
                return;
            }
            let (loc, text) = match &items[i] {
                Item::DeviceHeader { loc } => {
                    if self.conds.last().is_none_or(|c| c.active) {
                        self.device_header(loc);
                    }
                    i += 1;
                    continue;
                }
                Item::Line { loc, text } => (loc.clone(), text.clone()),
            };
            i += 1;
            let (label, rest) = self.take_label(&text);
            let (word, args) = split_word(rest);
            let (second, after_second) = split_word(args);
            let kw = upper(word);

            // Conditionals are tracked even inside inactive blocks.
            match kw.as_str() {
                "IF" | "ELSIF" | "ELSE" | "ENDIF" => {
                    self.conditional(&kw, args, &loc);
                    continue;
                }
                _ => {}
            }
            if !self.conds.last().is_none_or(|c| c.active) {
                // Skip macro bodies wholesale so their ENDM does not confuse us.
                if upper(second) == "MACRO" || kw == "REPT" || kw == "IRP" || kw == "IRPC" {
                    i = self.body_end(items, i, &loc).1;
                }
                continue;
            }

            if upper(second) == "MACRO" && label.is_none() {
                let (end, next) = self.body_end(items, i, &loc);
                let body = items[i..end].to_vec();
                i = next;
                self.define_macro(word, after_second, body, &loc);
                continue;
            }
            if kw == "REPT" {
                let (end, next) = self.body_end(items, i, &loc);
                let body = items[i..end].to_vec();
                i = next;
                match self.const_eval(args) {
                    Ok(n) if (0..=10_000).contains(&n) => {
                        if depth >= MAX_MACRO_DEPTH {
                            self.err(&loc, "REPT trop imbriqués");
                            continue;
                        }
                        for _ in 0..n {
                            self.block(&body, depth + 1);
                        }
                    }
                    Ok(n) => self.err(&loc, format!("REPT {n} : nombre de répétitions invalide")),
                    Err(e) => self.err(&loc, format!("REPT : {e}")),
                }
                continue;
            }

            if let Some(label) = &label {
                self.define_label(label, &loc);
            }
            if word.is_empty() {
                continue;
            }

            // name EQU / SET / = value
            let skw = upper(second);
            if label.is_none() && (skw == "EQU" || skw == "SET" || second == "=") && is_ident(word) {
                self.assign(word, &skw, after_second, &loc);
                continue;
            }

            if let Some(m) = self.macros.get(&kw).cloned() {
                if depth >= MAX_MACRO_DEPTH {
                    self.err(&loc, format!("macro {word} : récursion trop profonde"));
                    continue;
                }
                let body = self.expand_macro(&m, args, &loc);
                self.block(&body, depth + 1);
                continue;
            }
            self.statement(&kw, word, args, &loc);
        }
    }

    /// Finds the ENDM matching the MACRO/REPT just before `start`. Returns
    /// (body end, index after ENDM).
    fn body_end(&mut self, items: &[Item], start: usize, loc: &Loc) -> (usize, usize) {
        let mut nest = 0;
        for (j, item) in items.iter().enumerate().skip(start) {
            let Item::Line { text, .. } = item else { continue };
            let (_, rest) = self.take_label(text);
            let (w, a) = split_word(rest);
            let w = upper(w);
            let second = upper(split_word(a).0);
            if second == "MACRO" || w == "REPT" || w == "IRP" || w == "IRPC" {
                nest += 1;
            } else if w == "ENDM" {
                if nest == 0 {
                    return (j, j + 1);
                }
                nest -= 1;
            }
        }
        self.err(loc, "ENDM manquant");
        (items.len(), items.len())
    }

    /// Splits a leading `label:` off a line.
    fn take_label<'t>(&self, text: &'t str) -> (Option<String>, &'t str) {
        let t = text.trim_start();
        let end = t.find(|c: char| !is_ident_char(c)).unwrap_or(t.len());
        if end > 0 && t.starts_with(is_ident_start) && t[end..].starts_with(':') && !t[end..].starts_with("::") {
            return (Some(t[..end].to_owned()), t[end + 1..].trim_start());
        }
        (None, t)
    }

    fn conditional(&mut self, kw: &str, args: &str, loc: &Loc) {
        let active = self.conds.last().is_none_or(|c| c.active);
        match kw {
            "IF" => {
                let cond = active && self.cond_value(args, loc);
                self.conds.push(Cond { active: cond, taken: cond, outer: active });
            }
            "ELSIF" => {
                let Some(top) = self.conds.last() else {
                    return self.err(loc, "ELSIF sans IF");
                };
                let (outer, taken) = (top.outer, top.taken);
                let cond = outer && !taken && self.cond_value(args, loc);
                let top = self.conds.last_mut().unwrap();
                top.active = cond;
                top.taken |= cond;
            }
            "ELSE" => match self.conds.last_mut() {
                Some(top) => {
                    top.active = top.outer && !top.taken;
                    top.taken = true;
                }
                None => self.err(loc, "ELSE sans IF"),
            },
            _ => {
                if self.conds.pop().is_none() {
                    self.err(loc, "ENDIF sans IF");
                }
            }
        }
    }

    fn cond_value(&mut self, args: &str, loc: &Loc) -> bool {
        match self.const_eval(args) {
            Ok(v) => v != 0,
            Err(e) => {
                self.err(loc, format!("IF : {e}"));
                false
            }
        }
    }

    fn define_macro(&mut self, name: &str, params: &str, body: Vec<Item>, loc: &Loc) {
        let params: Vec<String> = split_args(params).into_iter().filter(|p| !p.is_empty()).collect();
        let mut locals = Vec::new();
        let mut kept = Vec::new();
        for item in body {
            if let Item::Line { text, .. } = &item {
                let (w, a) = split_word(text);
                if upper(w) == "LOCAL" {
                    locals.extend(split_args(a));
                    continue;
                }
            }
            kept.push(item);
        }
        let key = upper(name);
        if is_instruction(&key) || is_directive(&key) {
            self.err(loc, format!("{name} est un mot réservé, pas un nom de macro"));
            return;
        }
        self.macros.insert(key, Rc::new(MacroDef { params, locals, body: kept }));
    }

    fn expand_macro(&mut self, m: &MacroDef, args: &str, loc: &Loc) -> Vec<Item> {
        let values = split_args(args);
        if values.len() > m.params.len() {
            self.err(loc, format!("macro : {} arguments pour {} paramètres", values.len(), m.params.len()));
        }
        let mut subst: HashMap<String, String> = HashMap::new();
        for (k, p) in m.params.iter().enumerate() {
            subst.insert(p.clone(), values.get(k).cloned().unwrap_or_default());
        }
        for l in &m.locals {
            self.local_counter += 1;
            subst.insert(l.clone(), format!("{l}__{:04}", self.local_counter));
        }
        m.body
            .iter()
            .map(|item| match item {
                Item::Line { text, .. } => Item::Line { loc: loc.clone(), text: replace_words(text, &subst) },
                other => other.clone(),
            })
            .collect()
    }

    fn device_header(&mut self, loc: &Loc) {
        let header = header_symbols(&self.device.asm);
        for (name, value) in header.equ {
            self.symbols.entry(name).or_insert(SymDef::Value(value));
        }
        // The header's psect lines use its own #defines (SPACE_DATA...).
        let defines: HashMap<String, String> = header.defines.into_iter().collect();
        let psects = self.device.asm.psects.clone();
        for line in psects {
            let (_, args) = split_word(&line);
            self.psect(&replace_words(args, &defines), loc);
        }
    }

    fn define_label(&mut self, name: &str, loc: &Loc) {
        let Some(p) = self.current else {
            return self.err(loc, format!("label {name} hors de tout PSECT"));
        };
        if self.symbols.contains_key(name) {
            return self.err(loc, format!("symbole {name} déjà défini"));
        }
        let offset = self.psects[p].size;
        self.symbols.insert(name.to_owned(), SymDef::Label(p, offset));
    }

    fn assign(&mut self, name: &str, kw: &str, value: &str, loc: &Loc) {
        let parsed = match expr::parse(value) {
            Ok(e) => self.substitute_sets(e),
            Err(e) => return self.err(loc, format!("{name} : {e}")),
        };
        if kw == "EQU" {
            if self.symbols.contains_key(name) {
                return self.err(loc, format!("symbole {name} déjà défini"));
            }
            let def = match expr::eval(&parsed, &ConstEnv(self)) {
                Ok(v) => SymDef::Value(v),
                Err(_) => SymDef::Expr(parsed),
            };
            self.symbols.insert(name.to_owned(), def);
        } else {
            if matches!(self.symbols.get(name), Some(def) if !self.set_names.contains_key(name) && !matches!(def, SymDef::Value(_))) {
                return self.err(loc, format!("symbole {name} déjà défini"));
            }
            match expr::eval(&parsed, &ConstEnv(self)) {
                Ok(v) => {
                    self.symbols.insert(name.to_owned(), SymDef::Value(v));
                    self.set_names.insert(name.to_owned(), ());
                }
                Err(e) => self.err(loc, format!("SET {name} : {e}")),
            }
        }
    }

    /// Replaces SET symbols by their current value, since later SETs change them.
    fn substitute_sets(&self, e: Expr) -> Expr {
        match e {
            Expr::Sym(ref s) if self.set_names.contains_key(s) => match self.symbols.get(s) {
                Some(SymDef::Value(v)) => Expr::Num(*v),
                _ => e,
            },
            Expr::Unary(op, x) => Expr::Unary(op, Box::new(self.substitute_sets(*x))),
            Expr::Binary(op, a, b) => {
                Expr::Binary(op, Box::new(self.substitute_sets(*a)), Box::new(self.substitute_sets(*b)))
            }
            Expr::Call(name, args) => Expr::Call(name, args.into_iter().map(|a| self.substitute_sets(a)).collect()),
            other => other,
        }
    }

    /// Evaluates with constants only (no labels): for DS, ORG, IF, REPT, SET.
    fn const_eval(&self, text: &str) -> Result<i64, String> {
        let e = expr::parse(text)?;
        expr::eval(&e, &ConstEnv(self))
    }

    fn psect(&mut self, args: &str, loc: &Loc) {
        let parts = split_args(args);
        let Some(name) = parts.first().filter(|n| is_ident(n)) else {
            return self.err(loc, "PSECT : nom attendu");
        };
        let idx = match self.psects.iter().position(|p| &p.name == name) {
            Some(i) => i,
            None => {
                self.psects.push(Psect::new(name));
                self.psects.len() - 1
            }
        };
        for flag in &parts[1..] {
            let (key, value) = match flag.split_once('=') {
                Some((k, v)) => (k.trim().to_ascii_lowercase(), Some(v.trim())),
                None => (flag.trim().to_ascii_lowercase(), None),
            };
            let num = match value.map(|v| self.const_eval(v)) {
                Some(Ok(n)) => Ok(n as u32),
                Some(Err(e)) => Err(e),
                None => Err(format!("{key}= attend une valeur")),
            };
            let p = &mut self.psects[idx];
            let res: Result<(), String> = match key.as_str() {
                "class" => {
                    p.class = value.map(upper);
                    Ok(())
                }
                "delta" => num.and_then(|d| match d {
                    1 | 2 => {
                        p.delta = d;
                        Ok(())
                    }
                    _ => Err(format!("delta={d} non supporté")),
                }),
                "space" => num.map(|s| p.space = Some(s)),
                "reloc" => num.map(|r| p.reloc = r.max(1)),
                "abs" => {
                    p.abs = true;
                    Ok(())
                }
                "ovrld" | "noexec" | "local" | "global" | "pure" | "bit" | "inline" | "keep" | "optim"
                | "merge" | "split" | "with" | "limit" | "size" | "lowdata" | "highdata" | "nodata" => Ok(()),
                _ => Err(format!("option de PSECT inconnue : {flag}")),
            };
            if let Err(e) = res {
                self.err(loc, e);
            }
        }
        self.current = Some(idx);
    }

    fn here(&self) -> Option<(usize, u32)> {
        self.current.map(|p| (p, self.psects[p].size))
    }

    fn statement(&mut self, kw: &str, word: &str, args: &str, loc: &Loc) {
        match kw {
            "PROCESSOR" => {
                if Database::embedded().find(args).map(|d| &d.name) != Some(&self.device.name) {
                    self.err(loc, format!("PROCESSOR {args} différent de la puce assemblée ({})", self.device.name));
                }
            }
            "PSECT" => self.psect(args, loc),
            "CONFIG" => self.config_directive(args, loc),
            "END" => self.ended = true,
            "GLOBAL" | "EXTRN" | "PUBLIC" | "SIGNAT" | "FNCALL" | "FNROOT" | "FNCONF" | "FNSIZE" | "FNARG"
            | "FNADDR" | "FNBREAK" | "FNINDIR" | "FNSIZE?" | "OPT" | "TITLE" | "SUBTITLE" | "LIST" | "NOLIST"
            | "PAGE" | "SPACE" | "RADIX" | "EXPAND" | "NOEXPAND" => {}
            "MESSG" | "MESSAGE" => self.diag(loc, args.trim_matches('"').to_owned(), false),
            "ERROR" => self.err(loc, args.trim_matches('"').to_owned()),
            "ENDM" => self.err(loc, "ENDM sans MACRO ni REPT"),
            "DS" | "DABS" => {
                let Some((p, _)) = self.here() else { return self.err(loc, "DS hors de tout PSECT") };
                match self.const_eval(args) {
                    Ok(n) if n >= 0 => self.psects[p].size += n as u32,
                    Ok(n) => self.err(loc, format!("DS {n} : taille négative")),
                    Err(e) => self.err(loc, format!("DS : {e}")),
                }
            }
            "ORG" | "ALIGN" => {
                let Some((p, size)) = self.here() else { return self.err(loc, format!("{word} hors de tout PSECT")) };
                match self.const_eval(args) {
                    Ok(n) if kw == "ORG" && n >= 0 => self.psects[p].size = n as u32,
                    Ok(n) if kw == "ALIGN" && n > 0 => {
                        let n = n as u32;
                        self.psects[p].size = size.div_ceil(n) * n;
                        self.psects[p].reloc = self.psects[p].reloc.max(n);
                    }
                    Ok(n) => self.err(loc, format!("{word} {n} : valeur invalide")),
                    Err(e) => self.err(loc, format!("{word} : {e}")),
                }
            }
            "DW" | "DB" | "DDW" => {
                let mask = if kw == "DB" { 0xFF } else { 0xFFFF };
                let mut values = Vec::new();
                for a in split_args(args) {
                    if kw == "DB" && a.starts_with('"') && a.ends_with('"') && a.len() >= 2 {
                        values.extend(a[1..a.len() - 1].bytes().map(|b| Expr::Num(b as i64)));
                        continue;
                    }
                    match expr::parse(&a) {
                        Ok(e) => values.push(self.substitute_sets(e)),
                        Err(e) => return self.err(loc, format!("{word} : {e}")),
                    }
                }
                self.emit(Emit::Words { values, mask }, loc);
            }
            _ if is_instruction(kw) => {
                let args = if args.trim().is_empty() { Vec::new() } else { split_args(args) };
                self.emit(Emit::Insn { mnemonic: kw.to_owned(), args }, loc);
            }
            _ => self.err(loc, format!("instruction ou directive inconnue : {word}")),
        }
    }

    fn emit(&mut self, emit: Emit, loc: &Loc) {
        let Some((p, offset)) = self.here() else {
            return self.err(loc, "code hors de tout PSECT (ajoute « PSECT code »)");
        };
        let words = match &emit {
            Emit::Insn { .. } => 1,
            Emit::Words { values, .. } => values.len() as u32,
        };
        let psect = &mut self.psects[p];
        // Program memory psects count in words whatever delta says.
        psect.has_code = true;
        psect.size += words;
        self.stmts.push(Stmt { loc: loc.clone(), psect: p, offset, emit });
    }

    fn config_directive(&mut self, args: &str, loc: &Loc) {
        self.config_used = true;
        for setting in split_args(args) {
            let Some((name, value)) = setting.split_once('=') else {
                self.err(loc, format!("CONFIG : « {setting} » n'est pas de la forme CHAMP = VALEUR"));
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            let Some(field) = self.device.config_fields.iter().find(|f| f.name.eq_ignore_ascii_case(name)) else {
                self.err(loc, format!("CONFIG : champ inconnu {name} pour {}", self.device.name));
                continue;
            };
            let v = match field.values.iter().find(|(n, _)| n.eq_ignore_ascii_case(value)) {
                Some((_, v)) => *v as i64,
                None => match self.const_eval(value) {
                    Ok(v) => v,
                    Err(_) => {
                        let names: Vec<_> = field.values.iter().map(|(n, _)| n.as_str()).collect();
                        self.err(loc, format!("CONFIG {name} : valeur {value} inconnue ({})", names.join(", ")));
                        continue;
                    }
                },
            };
            let mask = ((1u32 << field.width) - 1) as u16;
            if v < 0 || v as u32 > mask as u32 {
                self.err(loc, format!("CONFIG {name} = {v} : hors limites"));
                continue;
            }
            let word = self.config.entry(field.address).or_insert(0x3FFF);
            *word = (*word & !(mask << field.shift)) | ((v as u16) << field.shift);
        }
    }

    fn finish(mut self) -> Assembly {
        if !self.conds.is_empty() {
            self.diagnostics.push(Diagnostic { file: String::new(), line: None, error: true, message: "IF sans ENDIF".into() });
        }
        // Link.
        let placement = link::place(self.device, &mut self.psects, &self.placements);
        let mut map = Vec::new();
        match placement {
            Ok(Placement { warnings }) => {
                for w in warnings {
                    self.diagnostics.push(Diagnostic { file: String::new(), line: None, error: false, message: w });
                }
            }
            Err(errors) => {
                for e in errors {
                    self.diagnostics.push(Diagnostic { file: String::new(), line: None, error: true, message: e });
                }
            }
        }
        for p in &self.psects {
            if let Some(base) = p.base {
                if p.size > 0 {
                    map.push((p.name.clone(), p.class.clone().unwrap_or_default(), base, p.size));
                }
            }
        }
        map.sort_by_key(|m| (m.0.starts_with("udata") as u8, m.2));

        // Resolve symbols.
        let mut symbols = BTreeMap::new();
        let names: Vec<String> = self.symbols.keys().cloned().collect();
        for name in names {
            if let Ok(v) = (LinkEnv { asm: &self, here: None, depth: 0 }).symbol(&name) {
                symbols.insert(name, v);
            }
        }

        // Encode.
        let mut image = Image::default();
        let mut words: BTreeMap<u32, (u16, Loc)> = BTreeMap::new();
        let stmts = std::mem::take(&mut self.stmts);
        for stmt in &stmts {
            let p = &self.psects[stmt.psect];
            let Some(base) = p.base else { continue };
            let addr = base + stmt.offset;
            let env = LinkEnv { asm: &self, here: Some(addr as i64), depth: 0 };
            let encoded = match &stmt.emit {
                Emit::Insn { mnemonic, args } => encode(mnemonic, args, &env, addr, self.wide_bsr()).map(|w| vec![w]),
                Emit::Words { values, mask } => values
                    .iter()
                    .map(|v| expr::eval(v, &env).map(|x| (x as u16) & mask))
                    .collect(),
            };
            match encoded {
                Ok(ws) => {
                    for (i, w) in ws.into_iter().enumerate() {
                        let a = addr + i as u32;
                        if let Some((_, other)) = words.insert(a, (w, stmt.loc.clone())) {
                            let msg = format!("adresse {a:#06x} déjà utilisée (ligne {})", other.line);
                            self.diag(&stmt.loc, msg, true);
                        }
                    }
                }
                Err(e) => self.diag(&stmt.loc, e, true),
            }
        }
        for (addr, (word, loc)) in words {
            // Program words are 14 bits; pic-as keeps DW's upper bits in the
            // HEX, the chip drops them.
            let word = word & 0x3FFF;
            if addr < self.device.flash_words {
                image.flash.insert(addr, word);
            } else if self.device.user_ids.contains(&addr) || self.device.config.iter().any(|c| c.address == addr) {
                image.config.insert(addr, word);
            } else {
                self.diag(&loc, format!("adresse {addr:#06x} hors de la mémoire programme de {}", self.device.name), true);
            }
        }
        if self.config_used {
            for (addr, value) in &self.config {
                image.config.insert(*addr, *value);
            }
        }

        let failed = self.diagnostics.iter().any(|d| d.error);
        Assembly {
            device: Some(self.device.clone()),
            image: (!failed).then_some(image),
            diagnostics: self.diagnostics,
            symbols,
            map,
        }
    }
}

/// Symbols known before linking: header and EQU/SET constants.
struct ConstEnv<'b, 'a>(&'b Assembler<'a>);

impl Env for ConstEnv<'_, '_> {
    fn symbol(&self, name: &str) -> Result<i64, String> {
        match self.0.symbols.get(name) {
            Some(SymDef::Value(v)) => Ok(*v),
            Some(_) => Err(format!("{name} n'est connu qu'après l'édition de liens")),
            None => Err(format!("symbole inconnu : {name}")),
        }
    }
    fn here(&self) -> Result<i64, String> {
        Err("$ n'est pas une constante ici".into())
    }
}

struct LinkEnv<'b, 'a> {
    asm: &'b Assembler<'a>,
    here: Option<i64>,
    depth: usize,
}

impl Env for LinkEnv<'_, '_> {
    fn symbol(&self, name: &str) -> Result<i64, String> {
        match self.asm.symbols.get(name) {
            Some(SymDef::Value(v)) => Ok(*v),
            Some(SymDef::Label(p, off)) => self.asm.psects[*p]
                .base
                .map(|b| (b + off) as i64)
                .ok_or_else(|| format!("{name} : PSECT {} non placé", self.asm.psects[*p].name)),
            Some(SymDef::Expr(e)) => {
                if self.depth > 32 {
                    return Err(format!("{name} : définition circulaire"));
                }
                expr::eval(e, &LinkEnv { asm: self.asm, here: None, depth: self.depth + 1 })
            }
            None => Err(format!("symbole inconnu : {name}")),
        }
    }
    fn here(&self) -> Result<i64, String> {
        self.here.ok_or_else(|| "$ interdit ici".into())
    }
}

fn replace_words(text: &str, subst: &HashMap<String, String>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if is_ident_start(chars[i]) {
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            let w: String = chars[start..i].iter().collect();
            out.push_str(subst.get(&w).map_or(&w, |s| s));
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

const FILE_OPS: &[(&str, u16)] = &[
    ("SUBWF", 0x0200),
    ("DECF", 0x0300),
    ("IORWF", 0x0400),
    ("ANDWF", 0x0500),
    ("XORWF", 0x0600),
    ("ADDWF", 0x0700),
    ("MOVF", 0x0800),
    ("COMF", 0x0900),
    ("INCF", 0x0A00),
    ("DECFSZ", 0x0B00),
    ("RRF", 0x0C00),
    ("RLF", 0x0D00),
    ("SWAPF", 0x0E00),
    ("INCFSZ", 0x0F00),
    ("LSLF", 0x3500),
    ("LSRF", 0x3600),
    ("ASRF", 0x3700),
    ("SUBWFB", 0x3B00),
    ("ADDWFC", 0x3D00),
];

const LITERAL_OPS: &[(&str, u16)] = &[
    ("MOVLW", 0x3000),
    ("RETLW", 0x3400),
    ("IORLW", 0x3800),
    ("ANDLW", 0x3900),
    ("XORLW", 0x3A00),
    ("SUBLW", 0x3C00),
    ("ADDLW", 0x3E00),
];

const PLAIN_OPS: &[(&str, u16)] = &[
    ("NOP", 0x0000),
    ("RESET", 0x0001),
    ("RETURN", 0x0008),
    ("RETFIE", 0x0009),
    ("CALLW", 0x000A),
    ("BRW", 0x000B),
    ("OPTION", 0x0062),
    ("SLEEP", 0x0063),
    ("CLRWDT", 0x0064),
    ("CLRW", 0x0103),
];

const OTHER_OPS: &[&str] = &[
    "MOVWF", "CLRF", "BCF", "BSF", "BTFSC", "BTFSS", "CALL", "GOTO", "BRA", "MOVLB", "MOVLP", "ADDFSR", "MOVIW",
    "MOVWI", "TRIS", "BANKSEL", "PAGESEL",
];

pub fn is_instruction(kw: &str) -> bool {
    FILE_OPS.iter().any(|(n, _)| *n == kw)
        || LITERAL_OPS.iter().any(|(n, _)| *n == kw)
        || PLAIN_OPS.iter().any(|(n, _)| *n == kw)
        || OTHER_OPS.contains(&kw)
}

fn is_directive(kw: &str) -> bool {
    matches!(
        kw,
        "PROCESSOR" | "PSECT" | "CONFIG" | "END" | "DS" | "DB" | "DW" | "ORG" | "ALIGN" | "EQU" | "SET" | "MACRO"
            | "ENDM" | "REPT" | "IF" | "ELSIF" | "ELSE" | "ENDIF" | "GLOBAL" | "LOCAL"
    )
}

fn value(arg: &str, env: &LinkEnv) -> Result<i64, String> {
    expr::eval(&expr::parse(arg)?, env)
}

fn arg<'s>(args: &'s [String], i: usize, op: &str) -> Result<&'s str, String> {
    args.get(i).map(String::as_str).ok_or_else(|| format!("{op} : opérande {} manquant", i + 1))
}

/// `FSR0`/`FSR1` (any case) as 0/1.
fn fsr_index(text: &str) -> Option<u16> {
    match text.trim().to_ascii_uppercase().as_str() {
        "FSR0" => Some(0),
        "FSR1" => Some(1),
        _ => None,
    }
}

fn signed6(v: i64, op: &str) -> Result<u16, String> {
    if !(-32..=31).contains(&v) {
        return Err(format!("{op} : {v} n'est pas une valeur signée sur 6 bits (-32..31)"));
    }
    Ok((v as u16) & 0x3F)
}

fn encode(op: &str, args: &[String], env: &LinkEnv, addr: u32, wide_bsr: bool) -> Result<u16, String> {
    let lower = op.to_ascii_lowercase();
    let count = |n: usize| -> Result<(), String> {
        if args.len() > n {
            Err(format!("{lower} : trop d'opérandes"))
        } else {
            Ok(())
        }
    };
    if let Some((_, base)) = PLAIN_OPS.iter().find(|(n, _)| *n == op) {
        count(0)?;
        return Ok(*base);
    }
    if let Some((_, base)) = LITERAL_OPS.iter().find(|(n, _)| *n == op) {
        count(1)?;
        let k = value(arg(args, 0, &lower)?, env)?;
        return Ok(base | (k as u16 & 0xFF));
    }
    if let Some((_, base)) = FILE_OPS.iter().find(|(n, _)| *n == op) {
        count(2)?;
        let f = value(arg(args, 0, &lower)?, env)? as u16 & 0x7F;
        let d = match args.get(1).map(|s| s.trim().to_ascii_uppercase()) {
            None => 1,
            Some(d) if d == "F" => 1,
            Some(d) if d == "W" => 0,
            Some(d) => match value(&d, env)? {
                0 => 0,
                1 => 1,
                v => return Err(format!("{lower} : destination {v} invalide (w, f, 0 ou 1)")),
            },
        };
        return Ok(base | (d << 7) | f);
    }
    match op {
        "MOVWF" | "CLRF" => {
            count(1)?;
            let f = value(arg(args, 0, &lower)?, env)? as u16 & 0x7F;
            Ok(if op == "MOVWF" { 0x0080 } else { 0x0180 } | f)
        }
        "BCF" | "BSF" | "BTFSC" | "BTFSS" => {
            count(2)?;
            let f = value(arg(args, 0, &lower)?, env)? as u16 & 0x7F;
            let b = value(arg(args, 1, &lower)?, env)?;
            if !(0..=7).contains(&b) {
                return Err(format!("{lower} : bit {b} hors de 0..7"));
            }
            let base = match op {
                "BCF" => 0x1000,
                "BSF" => 0x1400,
                "BTFSC" => 0x1800,
                _ => 0x1C00,
            };
            Ok(base | ((b as u16) << 7) | f)
        }
        "CALL" | "GOTO" => {
            count(1)?;
            let k = value(arg(args, 0, &lower)?, env)? as u16 & 0x7FF;
            Ok(if op == "CALL" { 0x2000 } else { 0x2800 } | k)
        }
        "BRA" => {
            count(1)?;
            let target = value(arg(args, 0, &lower)?, env)?;
            let offset = target - (addr as i64 + 1);
            if !(-256..=255).contains(&offset) {
                return Err(format!("bra : cible trop loin ({offset} mots, limite -256..255)"));
            }
            Ok(0x3200 | (offset as u16 & 0x1FF))
        }
        "MOVLB" | "BANKSEL" => {
            count(1)?;
            let v = value(arg(args, 0, &lower)?, env)?;
            let bank = if op == "BANKSEL" { v >> 7 } else { v };
            Ok(if wide_bsr { 0x0140 | (bank as u16 & 0x3F) } else { 0x0020 | (bank as u16 & 0x1F) })
        }
        "MOVLP" | "PAGESEL" => {
            count(1)?;
            let v = value(arg(args, 0, &lower)?, env)?;
            let page = if op == "PAGESEL" { v >> 8 } else { v };
            Ok(0x3180 | (page as u16 & 0x7F))
        }
        "ADDFSR" => {
            count(2)?;
            let n = fsr_index(arg(args, 0, &lower)?).ok_or("addfsr : FSR0 ou FSR1 attendu")?;
            let k = signed6(value(arg(args, 1, &lower)?, env)?, &lower)?;
            Ok(0x3100 | (n << 6) | k)
        }
        "MOVIW" | "MOVWI" => {
            count(1)?;
            let text = arg(args, 0, &lower)?.trim().to_owned();
            let store = op == "MOVWI";
            if let Some((k, rest)) = text.split_once('[') {
                let n = fsr_index(rest.trim_end_matches(']')).ok_or(format!("{lower} : k[FSR0] ou k[FSR1] attendu"))?;
                let k = signed6(value(k, env)?, &lower)?;
                return Ok(0x3F00 | if store { 0x80 } else { 0 } | (n << 6) | k);
            }
            let (mode, reg) = if let Some(r) = text.strip_prefix("++") {
                (0, r)
            } else if let Some(r) = text.strip_prefix("--") {
                (1, r)
            } else if let Some(r) = text.strip_suffix("++") {
                (2, r)
            } else if let Some(r) = text.strip_suffix("--") {
                (3, r)
            } else {
                (4, text.as_str())
            };
            let n = fsr_index(reg).ok_or(format!("{lower} : ++FSRn, --FSRn, FSRn++, FSRn-- ou k[FSRn] attendu"))?;
            if mode == 4 {
                // Plain FSRn means 0[FSRn].
                return Ok(0x3F00 | if store { 0x80 } else { 0 } | (n << 6));
            }
            Ok(if store { 0x0018 } else { 0x0010 } | (n << 2) | mode)
        }
        "TRIS" => {
            count(1)?;
            let f = value(arg(args, 0, &lower)?, env)?;
            if !(5..=7).contains(&f) {
                return Err(format!("tris : registre {f} invalide (5..7)"));
            }
            Ok(0x0060 | f as u16)
        }
        _ => Err(format!("instruction inconnue : {lower}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(src: &str) -> Assembly {
        assemble(Path::new("test.asm"), src, &Options::default())
    }

    #[test]
    fn reports_errors_with_lines() {
        let a = build("PROCESSOR 16F1789\n#include <xc.inc>\nPSECT code\nstart:\n  bogus 4\n  addfsr FSR0, 40\n");
        assert!(!a.ok());
        let lines: Vec<_> = a.diagnostics.iter().filter(|d| d.error).map(|d| d.line).collect();
        assert_eq!(lines, [Some(5), Some(6)], "{}", a.log());
    }

    #[test]
    fn needs_a_part() {
        let a = build("PSECT code\n nop\n");
        assert!(!a.ok());
        assert!(a.log().contains("PROCESSOR"));
    }

    #[test]
    fn macros_and_conditionals() {
        let a = build(
            "PROCESSOR 16F1789\n#include <xc.inc>\n\
             PSECT reset_vec, class=CODE, delta=2\n\
             blink MACRO port, bit\n LOCAL again\n again: bsf port, bit\n bra again\n ENDM\n\
             N SET 2\n\
             IF N == 2\n blink PORTA, 1\n ELSE\n nop\n ENDIF\n\
             REPT 2\n nop\n ENDM\n",
        );
        assert!(a.ok(), "{}", a.log());
        let flash: Vec<u16> = a.image.unwrap().flash.values().copied().collect();
        assert_eq!(flash, [0x148C, 0x33FE, 0x0000, 0x0000]);
    }
}
