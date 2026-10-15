//! Psect placement, following what the pic-as linker does for PIC16
//! enhanced mid-range parts (observed on its link maps):
//!
//! - psects named in `-p` options sit at their address;
//! - other psects are allocated in declaration order inside their class
//!   ranges (CODE: one range per 2K page; BANKn, COMMON, RAM: data banks);
//! - program memory classes are stacked right after the highest fixed psect
//!   placed at a non-zero address, or from the top of the first page down
//!   when there is none (a lone `reset_vec` at 0 does not count);
//! - data classes are stacked from the bottom of their range up;
//! - classes are filled in the linker's `-A` order (CODE first, BANKn
//!   before the catch-all RAM); within a class, largest psect first, ties
//!   in declaration order.

use crate::device::Device;

#[derive(Debug, Clone)]
pub struct Psect {
    pub name: String,
    pub class: Option<String>,
    pub space: Option<u32>,
    pub reloc: u32,
    pub abs: bool,
    /// Holds instructions or DW/DB data.
    pub has_code: bool,
    /// Current length (words for program memory, bytes for data).
    pub size: u32,
    pub base: Option<u32>,
}

impl Psect {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            class: None,
            space: None,
            reloc: 1,
            abs: false,
            has_code: false,
            size: 0,
            base: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Space {
    Program,
    Data,
}

/// Ranges (end exclusive) of a linker class.
fn class_ranges(device: &Device, class: &str) -> Option<(Space, Vec<(u32, u32)>)> {
    let flash = device.flash_words;
    let chunks = |size: u32| (0..flash).step_by(size as usize).map(|s| (s, (s + size).min(flash))).collect();
    let bank_of = |r: &(u32, u32)| r.0 >> 7;
    Some(match class {
        "CODE" => (Space::Program, chunks(0x800)),
        "STRCODE" => (Space::Program, vec![(0, flash)]),
        "ENTRY" | "CONST" | "STRING" => (Space::Program, chunks(0x100)),
        "COMMON" => (Space::Data, device.common_ram.into_iter().collect()),
        "RAM" => (Space::Data, device.gpr.clone()),
        _ => {
            let bank: u32 = class.strip_prefix("BANK")?.parse().ok()?;
            let ranges: Vec<_> = device.gpr.iter().filter(|r| bank_of(r) == bank).copied().collect();
            if ranges.is_empty() {
                return None;
            }
            (Space::Data, ranges)
        }
    })
}

/// Position of a class in pic-as's `-A` options, which sets allocation order.
fn class_rank(class: &str) -> u32 {
    match class {
        "CODE" => 0,
        "STRCODE" => 1,
        "STRING" => 2,
        "CONST" => 3,
        "ENTRY" => 4,
        "COMMON" => 5,
        "RAM" => 1000,
        _ => class.strip_prefix("BANK").and_then(|b| b.parse::<u32>().ok()).map_or(2000, |b| 6 + b),
    }
}

fn align(addr: u32, to: u32) -> u32 {
    addr.div_ceil(to) * to
}

/// Lowest free `reloc`-aligned address in `from..hi` that fits `size`.
fn fit_up(from: u32, hi: u32, size: u32, reloc: u32, taken: impl Fn(u32, u32) -> bool) -> Option<u32> {
    let mut a = align(from, reloc);
    while a + size <= hi {
        if !taken(a, a + size) {
            return Some(a);
        }
        a = align(a + 1, reloc);
    }
    None
}

/// Highest free `reloc`-aligned address in `lo..hi` that fits `size`.
fn fit_down(lo: u32, hi: u32, size: u32, reloc: u32, taken: impl Fn(u32, u32) -> bool) -> Option<u32> {
    if hi - lo < size {
        return None;
    }
    let mut a = (hi - size) / reloc * reloc;
    while a >= lo {
        if !taken(a, a + size) {
            return Some(a);
        }
        if a < reloc {
            return None;
        }
        a -= reloc;
    }
    None
}

pub fn place(device: &Device, psects: &mut [Psect], fixed: &[(String, u32)]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    // Occupied intervals per space.
    let mut used: Vec<(Space, u32, u32, String)> = Vec::new();

    let space_of = |p: &Psect| -> Space {
        if let Some((space, _)) = p.class.as_deref().and_then(|c| class_ranges(device, c)) {
            return space;
        }
        match p.space {
            Some(1) => Space::Data,
            _ if p.has_code => Space::Program,
            _ => Space::Data,
        }
    };

    for (name, addr) in fixed {
        let Some(p) = psects.iter_mut().find(|p| &p.name == name) else {
            errors.push(format!("psect {name} placé par option mais jamais défini"));
            continue;
        };
        p.base = Some(*addr);
    }
    for p in psects.iter_mut().filter(|p| p.abs && p.base.is_none()) {
        p.base = Some(0);
    }
    for p in psects.iter().filter(|p| p.base.is_some() && p.size > 0) {
        let base = p.base.unwrap();
        used.push((space_of(p), base, base + p.size, p.name.clone()));
    }

    // Program memory cursor: end of the highest fixed psect at a non-zero address.
    let mut cursor: Option<u32> = psects
        .iter()
        .filter(|p| p.base.is_some_and(|b| b > 0) && p.size > 0 && space_of(p) == Space::Program)
        .max_by_key(|p| p.base)
        .map(|p| p.base.unwrap() + p.size);

    let mut order: Vec<usize> = (0..psects.len()).collect();
    order.sort_by_key(|&i| (psects[i].class.as_deref().map_or(3000, class_rank), std::cmp::Reverse(psects[i].size)));
    for i in order {
        let p = &psects[i];
        if p.base.is_some() || p.size == 0 {
            continue;
        }
        let Some(class) = p.class.clone() else {
            errors.push(format!("psect {} : pas de class= ni d'adresse, impossible à placer", p.name));
            continue;
        };
        let Some((space, ranges)) = class_ranges(device, &class) else {
            errors.push(format!("psect {} : classe {class} inconnue pour {}", p.name, device.name));
            continue;
        };
        let (size, reloc) = (p.size, p.reloc.max(1));
        let taken = |start: u32, end: u32| used.iter().any(|(s, a, b, _)| *s == space && start < *b && *a < end);
        let placed = match (space, cursor) {
            (Space::Program, Some(c)) => {
                // Bottom-up from the cursor, then the remaining ranges.
                let mut order = ranges.iter().filter(|r| r.1 > c).chain(ranges.iter().filter(|r| r.1 <= c));
                order.find_map(|&(lo, hi)| fit_up(if (lo..hi).contains(&c) { c } else { lo }, hi, size, reloc, taken))
            }
            (Space::Program, None) => ranges.iter().find_map(|&(lo, hi)| fit_down(lo, hi, size, reloc, taken)),
            (Space::Data, _) => ranges.iter().find_map(|&(lo, hi)| fit_up(lo, hi, size, reloc, taken)),
        };
        match placed {
            Some(a) => {
                psects[i].base = Some(a);
                used.push((space, a, a + size, psects[i].name.clone()));
                if space == Space::Program && cursor.is_some() {
                    cursor = Some(a + size);
                }
            }
            None => errors.push(format!(
                "psect {} ({size} {}) ne tient pas dans la classe {class}",
                psects[i].name,
                if space == Space::Program { "mots" } else { "octets" }
            )),
        }
    }

    // Fixed psects that collide.
    used.sort_by_key(|u| (u.0 == Space::Data, u.1));
    for w in used.windows(2) {
        if w[0].0 == w[1].0 && w[1].1 < w[0].2 {
            errors.push(format!("les psects {} et {} se chevauchent ({:#06x})", w[0].3, w[1].3, w[1].1));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
