//! Expressions in pic-as syntax: `0FFh`, `0x1F`, `0b101`, `17o`, `10q`, `'a'`,
//! `$`, `high x`, `low(x)`, `a mod b`, `x shl 2`, `a and b`, `~x`, `not x`,
//! `BANKMASK(x)`, `PAGEMASK(x)`, plus C-style comparisons and `defined()`
//! for `#if` lines.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Num(i64),
    Sym(String),
    /// `$`: address of the current instruction.
    Here,
    Unary(Un, Box<Expr>),
    Binary(Bin, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Un {
    Neg,
    Not,
    LogicalNot,
    High,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bin {
    Mul,
    Div,
    Mod,
    Add,
    Sub,
    Shl,
    Shr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Xor,
    Or,
    LogicalAnd,
    LogicalOr,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(i64),
    Ident(String),
    Here,
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Tok::Num(n) => write!(f, "{n}"),
            Tok::Ident(s) => write!(f, "{s}"),
            Tok::Here => write!(f, "$"),
            Tok::Op(o) => write!(f, "{o}"),
            Tok::LParen => write!(f, "("),
            Tok::RParen => write!(f, ")"),
            Tok::Comma => write!(f, ","),
        }
    }
}

/// Parses a number token such as `0FFh`, `0x1F`, `0b101`, `17o`, `42`.
pub fn parse_number(text: &str) -> Result<i64, String> {
    let lower = text.to_ascii_lowercase();
    let bad = || format!("nombre invalide : {text}");
    let radix = |digits: &str, radix: u32| i64::from_str_radix(digits, radix).map_err(|_| bad());
    if let Some(hex) = lower.strip_prefix("0x") {
        radix(hex, 16)
    } else if let Some(hex) = lower.strip_suffix('h') {
        radix(hex, 16)
    } else if let Some(bin) = lower.strip_prefix("0b").filter(|b| b.chars().all(|c| c == '0' || c == '1')) {
        radix(bin, 2)
    } else if let Some(oct) = lower.strip_suffix(['o', 'q']) {
        radix(oct, 8)
    } else if lower.ends_with(['b', 'f']) && lower[..lower.len() - 1].chars().all(|c| c.is_ascii_digit()) {
        Err(format!("labels temporaires ({text}) non supportés"))
    } else {
        radix(&lower, 10)
    }
}

pub fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '?' || c == '.'
}

pub fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '?' || c == '.' || c == '$'
}

fn tokenize(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                i += 1;
            }
            toks.push(Tok::Num(parse_number(&chars[start..i].iter().collect::<String>())?));
        } else if c == '\'' {
            let (value, len) = match (chars.get(i + 1), chars.get(i + 2), chars.get(i + 3)) {
                (Some('\\'), Some(e), Some('\'')) => {
                    let v = match e {
                        'n' => 10,
                        'r' => 13,
                        't' => 9,
                        '0' => 0,
                        other => *other as i64,
                    };
                    (v, 4)
                }
                (Some(ch), Some('\''), _) => (*ch as i64, 3),
                _ => return Err("caractère littéral mal formé".into()),
            };
            toks.push(Tok::Num(value));
            i += len;
        } else if c == '$' && !chars.get(i + 1).is_some_and(|n| is_ident_char(*n)) {
            toks.push(Tok::Here);
            i += 1;
        } else if is_ident_start(c) {
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let op = match word.to_ascii_lowercase().as_str() {
                "mod" => Some("mod"),
                "and" => Some("&"),
                "shl" => Some("<<"),
                "shr" => Some(">>"),
                "not" => Some("~"),
                "high" => Some("high"),
                "low" => Some("low"),
                _ => None,
            };
            toks.push(op.map_or(Tok::Ident(word), Tok::Op));
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            let op = ["<<", ">>", "<=", ">=", "==", "!=", "&&", "||"].into_iter().find(|o| *o == two);
            if let Some(op) = op {
                toks.push(Tok::Op(op));
                i += 2;
                continue;
            }
            let tok = match c {
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                ',' => Tok::Comma,
                '+' => Tok::Op("+"),
                '-' => Tok::Op("-"),
                '*' => Tok::Op("*"),
                '/' => Tok::Op("/"),
                '&' => Tok::Op("&"),
                '|' => Tok::Op("|"),
                '^' => Tok::Op("^"),
                '~' => Tok::Op("~"),
                '!' => Tok::Op("!"),
                '<' => Tok::Op("<"),
                '>' => Tok::Op(">"),
                '=' => Tok::Op("=="),
                other => return Err(format!("caractère inattendu « {other} »")),
            };
            toks.push(tok);
            i += 1;
        }
    }
    Ok(toks)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

/// Binary operators by precedence level, loosest first.
const LEVELS: &[&[(&str, Bin)]] = &[
    &[("||", Bin::LogicalOr)],
    &[("&&", Bin::LogicalAnd)],
    &[("|", Bin::Or)],
    &[("^", Bin::Xor)],
    &[("&", Bin::And)],
    &[("==", Bin::Eq), ("!=", Bin::Ne)],
    &[("<", Bin::Lt), ("<=", Bin::Le), (">", Bin::Gt), (">=", Bin::Ge)],
    &[("<<", Bin::Shl), (">>", Bin::Shr)],
    &[("+", Bin::Add), ("-", Bin::Sub)],
    &[("*", Bin::Mul), ("/", Bin::Div), ("mod", Bin::Mod)],
];

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o)) => LEVELS[level].iter().find(|(s, _)| s == o).map(|(_, b)| *b),
                _ => None,
            };
            let Some(op) = op else { return Ok(lhs) };
            self.pos += 1;
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        let op = match self.peek() {
            Some(Tok::Op("-")) => Some(Un::Neg),
            Some(Tok::Op("+")) => {
                self.pos += 1;
                return self.unary();
            }
            Some(Tok::Op("~")) => Some(Un::Not),
            Some(Tok::Op("!")) => Some(Un::LogicalNot),
            Some(Tok::Op("high")) => Some(Un::High),
            Some(Tok::Op("low")) => Some(Un::Low),
            _ => None,
        };
        if let Some(op) = op {
            self.pos += 1;
            return Ok(Expr::Unary(op, Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Here) => Ok(Expr::Here),
            Some(Tok::LParen) => {
                let e = self.binary(0)?;
                match self.next() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err("parenthèse fermante manquante".into()),
                }
            }
            Some(Tok::Ident(name)) => {
                if self.peek() == Some(&Tok::LParen) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.binary(0)?);
                            match self.next() {
                                Some(Tok::Comma) => continue,
                                Some(Tok::RParen) => break,
                                _ => return Err(format!("appel de {name} mal formé")),
                            }
                        }
                    } else {
                        self.pos += 1;
                    }
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Sym(name))
                }
            }
            Some(t) => Err(format!("« {t} » inattendu")),
            None => Err("expression incomplète".into()),
        }
    }
}

pub fn parse(text: &str) -> Result<Expr, String> {
    let toks = tokenize(text)?;
    if toks.is_empty() {
        return Err("expression vide".into());
    }
    let mut p = Parser { toks, pos: 0 };
    let e = p.binary(0)?;
    match p.peek() {
        None => Ok(e),
        Some(t) => Err(format!("« {t} » inattendu")),
    }
}

/// What an expression can refer to.
pub trait Env {
    fn symbol(&self, name: &str) -> Result<i64, String>;
    fn here(&self) -> Result<i64, String>;
}

pub fn eval(e: &Expr, env: &dyn Env) -> Result<i64, String> {
    Ok(match e {
        Expr::Num(n) => *n,
        Expr::Sym(s) => env.symbol(s)?,
        Expr::Here => env.here()?,
        Expr::Unary(op, x) => {
            let v = eval(x, env)?;
            match op {
                Un::Neg => v.wrapping_neg(),
                Un::Not => !v,
                Un::LogicalNot => (v == 0) as i64,
                Un::High => (v >> 8) & 0xFF,
                Un::Low => v & 0xFF,
            }
        }
        Expr::Binary(op, a, b) => {
            let (a, b) = (eval(a, env)?, eval(b, env)?);
            match op {
                Bin::Mul => a.wrapping_mul(b),
                Bin::Div | Bin::Mod if b == 0 => return Err("division par zéro".into()),
                Bin::Div => a / b,
                Bin::Mod => a % b,
                Bin::Add => a.wrapping_add(b),
                Bin::Sub => a.wrapping_sub(b),
                Bin::Shl => a.wrapping_shl(b as u32),
                Bin::Shr => a.wrapping_shr(b as u32),
                Bin::Lt => (a < b) as i64,
                Bin::Le => (a <= b) as i64,
                Bin::Gt => (a > b) as i64,
                Bin::Ge => (a >= b) as i64,
                Bin::Eq => (a == b) as i64,
                Bin::Ne => (a != b) as i64,
                Bin::And => a & b,
                Bin::Xor => a ^ b,
                Bin::Or => a | b,
                Bin::LogicalAnd => (a != 0 && b != 0) as i64,
                Bin::LogicalOr => (a != 0 || b != 0) as i64,
            }
        }
        Expr::Call(name, args) => {
            let arg = |i: usize| -> Result<i64, String> {
                let x = args.get(i).ok_or_else(|| format!("{name} attend un argument"))?;
                eval(x, env)
            };
            match name.to_ascii_uppercase().as_str() {
                "BANKMASK" => arg(0)? & 0x7F,
                "PAGEMASK" => arg(0)? & 0x7FF,
                "HIGH" => (arg(0)? >> 8) & 0xFF,
                "LOW" => arg(0)? & 0xFF,
                _ => return Err(format!("fonction inconnue : {name}")),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoSyms;
    impl Env for NoSyms {
        fn symbol(&self, name: &str) -> Result<i64, String> {
            match name {
                "table" => Ok(0x17),
                _ => Err(format!("symbole inconnu : {name}")),
            }
        }
        fn here(&self) -> Result<i64, String> {
            Ok(0x12)
        }
    }

    fn v(text: &str) -> i64 {
        eval(&parse(text).unwrap(), &NoSyms).unwrap()
    }

    /// Values pic-as produced for the same expressions (retlw operand).
    #[test]
    fn matches_pic_as() {
        assert_eq!(v("100 mod 7"), 2);
        assert_eq!(v("~1 & 0xFF"), 0xFE);
        assert_eq!(v("not 1 & 0xFF"), 0xFE);
        assert_eq!(v("0x0F and 0x3"), 3);
        assert_eq!(v("1 shl 3"), 8);
        assert_eq!(v("16 shr 2"), 4);
        assert_eq!(v("5 ^ 1"), 4);
        assert_eq!(v("1 | 2"), 3);
        assert_eq!(v("2 > 1"), 1);
        assert_eq!(v("high 0x1234"), 0x12);
        assert_eq!(v("low 0x1234"), 0x34);
        assert_eq!(v("high(0x1234)"), 0x12);
        assert_eq!(v("-1 & 0xFF"), 0xFF);
        assert_eq!(v("'a'"), 0x61);
        assert_eq!(v("10q"), 8);
        assert_eq!(v("17o"), 15);
        assert_eq!(v("0b101"), 5);
        assert_eq!(v("0FFh"), 255);
        assert_eq!(v("7Fh"), 127);
        assert_eq!(v("(1 << 4) | 3"), 0x13);
        assert_eq!(v("high(table)"), 0);
        assert_eq!(v("low table"), 0x17);
        assert_eq!(v("high($)"), 0);
        assert_eq!(v("BANKMASK(0x99)"), 0x19);
    }

    #[test]
    fn rejects_what_pic_as_rejects() {
        assert!(parse("100 % 7").is_err());
        assert!(parse("1010b").is_err(), "temporary label reference");
    }
}
