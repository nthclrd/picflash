//! C-style preprocessing, as pic-as does with `-xassembler-with-cpp`:
//! comments, `#include`, `#define` (object and function-like), `#undef`,
//! `#ifdef`/`#ifndef`/`#if`/`#elif`/`#else`/`#endif`, `#error`.
//!
//! `#include <xc.inc>` is served from the embedded device database instead of
//! XC8's headers: it defines the part's bit aliases and emits a marker where
//! the assembler declares the part's symbols and standard psects.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::expr::{self, is_ident_char, is_ident_start, Env};
use super::Loc;
use crate::devdb::header_symbols;
use crate::device::Device;

#[derive(Debug, Clone)]
pub enum Item {
    Line { loc: Loc, text: String },
    /// `#include <xc.inc>`: declare the part's symbols and psects here.
    DeviceHeader { loc: Loc },
}

#[derive(Debug, Clone)]
struct Macro {
    params: Option<Vec<String>>,
    body: String,
}

struct Cond {
    /// Lines in this branch are kept.
    active: bool,
    /// Some branch of this #if chain was already taken.
    taken: bool,
    /// The enclosing block is active.
    outer: bool,
}

pub struct Preprocessor<'a> {
    device: &'a Device,
    macros: HashMap<String, Macro>,
    pub items: Vec<Item>,
    pub errors: Vec<(Loc, String, bool)>,
    header_included: bool,
}

const MAX_INCLUDE_DEPTH: usize = 16;

/// Removes `;`, `//` and `/* */` comments, keeping quoted text.
/// `in_block` carries an open `/*` across lines.
fn strip_comments(line: &str, in_block: &mut bool) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if *in_block {
            if c == '*' && chars.get(i + 1) == Some(&'/') {
                *in_block = false;
                i += 2;
                out.push(' ');
            } else {
                i += 1;
            }
            continue;
        }
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.get(i + 1) {
                    out.push(*n);
                    i += 1;
                }
            } else if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                out.push(c);
            }
            ';' => break,
            '/' if chars.get(i + 1) == Some(&'/') => break,
            '/' if chars.get(i + 1) == Some(&'*') => {
                *in_block = true;
                i += 1;
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// Splits `a, (b, c), d` on top-level commas.
pub fn split_args(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in text.chars() {
        if let Some(q) = quote {
            current.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                current.push(c);
            }
            '(' | '[' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => args.push(std::mem::take(&mut current).trim().to_owned()),
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() || !args.is_empty() {
        args.push(current.trim().to_owned());
    }
    args
}

impl<'a> Preprocessor<'a> {
    pub fn new(device: &'a Device) -> Self {
        let mut macros = HashMap::new();
        // Chip select, as XC8 defines it (_16F1789).
        let chip = device.name.strip_prefix("PIC").unwrap_or(&device.name);
        macros.insert(format!("_{chip}"), Macro { params: None, body: "1".into() });
        Self { device, macros, items: Vec::new(), errors: Vec::new(), header_included: false }
    }

    fn error(&mut self, loc: &Loc, msg: impl Into<String>) {
        self.errors.push((loc.clone(), msg.into(), true));
    }

    /// Expands macros in `text`; `active` guards against recursion.
    fn expand(&self, text: &str, active: &mut Vec<String>) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        let mut quote: Option<char> = None;
        while i < chars.len() {
            let c = chars[i];
            if let Some(q) = quote {
                out.push(c);
                if c == q {
                    quote = None;
                }
                i += 1;
                continue;
            }
            if c == '\'' || c == '"' {
                quote = Some(c);
                out.push(c);
                i += 1;
                continue;
            }
            // Numbers like 0FFh must not be read as identifiers.
            if c.is_ascii_digit() {
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    out.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            if !is_ident_start(c) {
                out.push(c);
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            let Some(m) = self.macros.get(&name).filter(|_| !active.contains(&name)) else {
                out.push_str(&name);
                continue;
            };
            let body = match &m.params {
                None => m.body.clone(),
                Some(params) => {
                    let mut j = i;
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if chars.get(j) != Some(&'(') {
                        out.push_str(&name);
                        continue;
                    }
                    let mut depth = 0;
                    let mut end = j;
                    while end < chars.len() {
                        match chars[end] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        end += 1;
                    }
                    let inner: String = chars[j + 1..end.min(chars.len())].iter().collect();
                    i = (end + 1).min(chars.len());
                    let args = split_args(&inner);
                    let mut subst = HashMap::new();
                    for (k, p) in params.iter().enumerate() {
                        subst.insert(p.as_str(), args.get(k).cloned().unwrap_or_default());
                    }
                    replace_idents(&m.body, &subst)
                }
            };
            active.push(name);
            out.push_str(&self.expand(&body, active));
            active.pop();
        }
        out
    }

    fn eval_if(&self, text: &str, loc: &Loc) -> Result<bool, String> {
        // defined(X) / defined X before macro expansion.
        let mut resolved = String::new();
        let mut rest = text;
        while let Some(pos) = rest.find("defined") {
            resolved.push_str(&rest[..pos]);
            let after = rest[pos + 7..].trim_start();
            let (name, tail) = if let Some(inner) = after.strip_prefix('(') {
                let close = inner.find(')').ok_or("defined( sans parenthèse fermante")?;
                (inner[..close].trim(), &inner[close + 1..])
            } else {
                let end = after.find(|c: char| !is_ident_char(c)).unwrap_or(after.len());
                (&after[..end], &after[end..])
            };
            resolved.push_str(if self.macros.contains_key(name) { "1" } else { "0" });
            rest = tail;
        }
        resolved.push_str(rest);
        let expanded = self.expand(&resolved, &mut Vec::new());
        struct Zero;
        impl Env for Zero {
            fn symbol(&self, _: &str) -> Result<i64, String> {
                Ok(0)
            }
            fn here(&self) -> Result<i64, String> {
                Err("$ interdit dans #if".into())
            }
        }
        let _ = loc;
        Ok(expr::eval(&expr::parse(&expanded)?, &Zero)? != 0)
    }

    /// Preprocesses `text` (the contents of `path`) into `self.items`.
    pub fn run(&mut self, path: &Path, text: &str, depth: usize) {
        let file: Rc<str> = Rc::from(path.display().to_string());
        let mut conds: Vec<Cond> = Vec::new();
        let mut in_block = false;
        let mut pending = String::new();
        let mut pending_line = 0;
        for (n, raw) in text.lines().enumerate() {
            let line_no = n as u32 + 1;
            // Backslash continuation.
            if pending.is_empty() {
                pending_line = line_no;
            }
            if let Some(head) = raw.strip_suffix('\\') {
                pending.push_str(head);
                pending.push(' ');
                continue;
            }
            pending.push_str(raw);
            let joined = std::mem::take(&mut pending);
            let loc = Loc { file: file.clone(), line: pending_line };
            let line = strip_comments(&joined, &mut in_block);
            let active = conds.last().is_none_or(|c| c.active);
            let trimmed = line.trim();

            if let Some(directive) = trimmed.strip_prefix('#') {
                let directive = directive.trim_start();
                let (word, rest) = directive.split_once(char::is_whitespace).unwrap_or((directive, ""));
                let rest = rest.trim();
                match word {
                    "ifdef" | "ifndef" | "if" => {
                        let cond = if !active {
                            false
                        } else if word == "if" {
                            match self.eval_if(rest, &loc) {
                                Ok(v) => v,
                                Err(e) => {
                                    self.error(&loc, format!("#if : {e}"));
                                    false
                                }
                            }
                        } else {
                            self.macros.contains_key(rest) == (word == "ifdef")
                        };
                        conds.push(Cond { active: active && cond, taken: cond, outer: active });
                    }
                    "elif" => {
                        let Some(top) = conds.last() else {
                            self.error(&loc, "#elif sans #if");
                            continue;
                        };
                        let (outer, taken) = (top.outer, top.taken);
                        let cond = outer && !taken && self.eval_if(rest, &loc).unwrap_or(false);
                        let top = conds.last_mut().unwrap();
                        top.active = cond;
                        top.taken |= cond;
                    }
                    "else" => match conds.last_mut() {
                        Some(top) => {
                            top.active = top.outer && !top.taken;
                            top.taken = true;
                        }
                        None => self.error(&loc, "#else sans #if"),
                    },
                    "endif" => {
                        if conds.pop().is_none() {
                            self.error(&loc, "#endif sans #if");
                        }
                    }
                    _ if !active => {}
                    "define" => self.define(rest, &loc),
                    "undef" => {
                        self.macros.remove(rest);
                    }
                    "include" => self.include(rest, path, &loc, depth),
                    "error" => self.error(&loc, format!("#error {rest}")),
                    "warning" => self.errors.push((loc.clone(), format!("#warning {rest}"), false)),
                    "pragma" | "line" => {}
                    other => self.error(&loc, format!("directive inconnue : #{other}")),
                }
                continue;
            }
            if !active || trimmed.is_empty() {
                continue;
            }
            let text = self.expand(&line, &mut Vec::new());
            self.items.push(Item::Line { loc, text });
        }
        if !conds.is_empty() {
            let loc = Loc { file, line: text.lines().count() as u32 };
            self.error(&loc, "#if sans #endif");
        }
    }

    fn define(&mut self, rest: &str, loc: &Loc) {
        let name_end = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
        let name = rest[..name_end].to_owned();
        if name.is_empty() {
            self.error(loc, "#define sans nom");
            return;
        }
        let after = &rest[name_end..];
        let m = if let Some(params) = after.strip_prefix('(') {
            let Some(close) = params.find(')') else {
                self.error(loc, "#define : ')' manquante");
                return;
            };
            let names = params[..close].split(',').map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
            Macro { params: Some(names.collect()), body: params[close + 1..].trim().to_owned() }
        } else {
            Macro { params: None, body: after.trim().to_owned() }
        };
        self.macros.insert(name, m);
    }

    fn include(&mut self, rest: &str, from: &Path, loc: &Loc, depth: usize) {
        if let Some(name) = rest.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
            match name.to_ascii_lowercase().as_str() {
                "xc.inc" | "pic.inc" => self.device_header(loc),
                _ => self.error(loc, format!("en-tête <{name}> inconnu (seul <xc.inc> est fourni)")),
            }
            return;
        }
        let Some(name) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
            self.error(loc, format!("#include mal formé : {rest}"));
            return;
        };
        if depth >= MAX_INCLUDE_DEPTH {
            self.error(loc, "trop d'#include imbriqués");
            return;
        }
        let path: PathBuf = from.parent().unwrap_or(Path::new(".")).join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => self.run(&path, &text, depth + 1),
            Err(e) => self.error(loc, format!("{} : {e}", path.display())),
        }
    }

    fn device_header(&mut self, loc: &Loc) {
        if self.header_included {
            return;
        }
        self.header_included = true;
        for flag in ["_XC_INC_", "_PIC_INC_"] {
            self.macros.insert(flag.into(), Macro { params: None, body: String::new() });
        }
        for (name, value) in header_symbols(&self.device.asm).defines {
            self.macros.entry(name).or_insert(Macro { params: None, body: value });
        }
        self.items.push(Item::DeviceHeader { loc: loc.clone() });
    }
}

/// Replaces whole identifiers per `subst`.
fn replace_idents(body: &str, subst: &HashMap<&str, String>) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if is_ident_start(chars[i]) {
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            out.push_str(subst.get(word.as_str()).map_or(&word, |s| s));
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_keep_quoted_semicolons() {
        let mut block = false;
        assert_eq!(strip_comments("movlw ';' ; comment", &mut block).trim(), "movlw ';'");
        assert_eq!(strip_comments("a /* b", &mut block).trim(), "a");
        assert!(block);
        assert_eq!(strip_comments("c */ d // e", &mut block).trim(), "d");
        assert!(!block);
    }

    #[test]
    fn args_split_on_top_level_commas() {
        assert_eq!(split_args("a, (b, c), 'x,'"), ["a", "(b, c)", "'x,'"]);
        assert_eq!(split_args("class = CODE, delta = 2"), ["class = CODE", "delta = 2"]);
    }
}
