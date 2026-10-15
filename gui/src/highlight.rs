//! Syntax colors for PIC16 assembly, shared by the disassembly and the
//! assembly source views.

use eframe::egui::{text::LayoutJob, Color32, FontId, TextFormat};

pub const BRANCH: Color32 = Color32::from_rgb(232, 120, 160);
pub const SKIP: Color32 = Color32::from_rgb(235, 180, 70);
pub const BANK: Color32 = Color32::from_rgb(80, 200, 190);
pub const LITERAL_OP: Color32 = Color32::from_rgb(120, 200, 110);
pub const NUMBER: Color32 = Color32::from_rgb(240, 150, 90);
pub const REGISTER: Color32 = Color32::from_rgb(180, 140, 245);
pub const LABEL: Color32 = Color32::from_rgb(90, 160, 255);
pub const COMMENT: Color32 = Color32::from_rgb(120, 130, 140);
pub const DIRECTIVE: Color32 = Color32::from_rgb(200, 120, 230);

fn mnemonic_color(op: &str, text: Color32) -> Color32 {
    match op.to_ascii_lowercase().as_str() {
        "goto" | "call" | "bra" | "brw" | "callw" | "return" | "retfie" | "retlw" | "reset" => {
            BRANCH
        }
        "btfsc" | "btfss" | "decfsz" | "incfsz" => SKIP,
        "movlb" | "movlp" | "banksel" | "pagesel" => BANK,
        "movlw" | "addlw" | "sublw" | "andlw" | "iorlw" | "xorlw" => LITERAL_OP,
        _ => text,
    }
}

fn is_directive(word: &str) -> bool {
    let w = word.to_ascii_uppercase();
    word.starts_with('#')
        || matches!(
            w.as_str(),
            "PROCESSOR" | "CONFIG" | "PSECT" | "DS" | "DB" | "DW" | "EQU" | "END" | "ORG"
                | "GLOBAL" | "SET" | "MACRO" | "ENDM" | "IF" | "ELSE" | "ENDIF" | "DABS"
        )
}

fn operand_color(token: &str, text: Color32, dim: Color32) -> Color32 {
    let lower = token.to_ascii_lowercase();
    let digits = token.strip_prefix('-').unwrap_or(token);
    if digits.starts_with(|c: char| c.is_ascii_digit()) {
        NUMBER
    } else if lower == "f" || lower == "w" {
        dim
    } else if lower.contains("fsr") {
        BANK
    } else if token.starts_with("L_") || token.starts_with("sub_") || token == "reset_vec" || token == "isr" {
        LABEL
    } else if token.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        REGISTER
    } else {
        text
    }
}

fn push(job: &mut LayoutJob, s: &str, font: &FontId, color: Color32) {
    job.append(s, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
}

/// Colors one instruction (`mnemonic operands`) or source line
/// (`label: mnemonic operands ; comment`).
pub fn line(source: &str, font: &FontId, text: Color32, dim: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let (code, comment) = match source.find(';') {
        Some(i) => (&source[..i], Some(&source[i..])),
        None => (source, None),
    };

    // Split into whitespace and token runs, keeping everything.
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut in_space = None;
    for (i, c) in code.char_indices() {
        let space = c.is_whitespace() || c == ',';
        if in_space != Some(space) {
            if i > start {
                tokens.push(&code[start..i]);
            }
            start = i;
            in_space = Some(space);
        }
    }
    if start < code.len() {
        tokens.push(&code[start..]);
    }

    let mut seen_op = false;
    for token in tokens {
        if token.trim_matches(|c: char| c.is_whitespace() || c == ',').is_empty() {
            push(&mut job, token, font, dim);
        } else if token.ends_with(':') && !seen_op {
            push(&mut job, token, font, LABEL);
        } else if !seen_op {
            seen_op = true;
            let color = if is_directive(token) { DIRECTIVE } else { mnemonic_color(token, text) };
            push(&mut job, token, font, color);
        } else {
            push(&mut job, token, font, operand_color(token, text, dim));
        }
    }
    if let Some(comment) = comment {
        push(&mut job, comment, font, COMMENT);
    }
    job
}
