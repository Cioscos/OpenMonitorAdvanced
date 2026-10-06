/******************************************************************************
 * FIRESTARTER - A Processor Stress Test Utility
 * Copyright (C) 2020 TU Dresden, Center for Information Services and High
 * Performance Computing
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/\>.
 *
 * Contact: daniel.hackenberg@tu-dresden.de
 *****************************************************************************/

// Adapted from FIRESTARTER (https://github.com/tud-zih-energy/FIRESTARTER), commit 927ae17e55f3f90f7575f6a68630a366fde9c94e, include/firestarter/X86/Platform/SkylakeSPConfig.hpp and include/firestarter/X86/Platform/HaswellConfig.hpp: the instruction groups and the 1536 lines.
// Adapted from FIRESTARTER (https://github.com/tud-zih-energy/FIRESTARTER), commit 927ae17e55f3f90f7575f6a68630a366fde9c94e, src/firestarter/Payload/PayloadSettings.cpp: generateSequence and getNumberOfSequenceRepetitions.
// Modified for OpenMonitor Advanced: Rust intrinsics generated at build time instead of asmjit at run time; accumulators reset every block; L3/RAM items dropped.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.

// The instruction groups of K1 (DA8) and the parser that turns them into the unrolled
// sequence. `build.rs` reads this file with `include!`, so it holds only plain items and
// `//` comments (no inner attributes).

/// AVX-512 groups, from `SkylakeSPConfig.hpp` without the L3 and RAM items.
pub const AVX512_GROUPS: &str = "REG:140,L1_L:40,L2_L:70,L2_S:4";

/// AVX2+FMA groups, from `HaswellConfig.hpp` without the L3 and RAM items; SSE2 uses them
/// too.
pub const AVX2_GROUPS: &str = "REG:40,L1_LS:90,L2_LS:9";

/// The sequence repeats as many whole times as fit in this many lines.
pub const LINES: usize = 1536;

/// The groups the generator knows.
pub const ITEMS: [&str; 6] = ["REG", "L1_L", "L1_LS", "L2_L", "L2_S", "L2_LS"];

/// Parses `ITEM:VAL,ITEM:VAL,...`; every VAL is at least 1.
pub fn parse_groups(text: &str) -> Result<Vec<(&str, usize)>, String> {
    text.split(',')
        .map(|group| {
            let (item, val) = group
                .split_once(':')
                .ok_or_else(|| format!("{group:?}: expected ITEM:VAL"))?;
            if !ITEMS.contains(&item) {
                return Err(format!("{item:?}: unknown instruction group"));
            }
            match val.parse::<usize>() {
                Ok(0) => Err(format!("{group:?}: VAL must be at least 1")),
                Ok(n) => Ok((item, n)),
                Err(_) => Err(format!("{group:?}: VAL is not a number")),
            }
        })
        .collect()
}

/// FIRESTARTER's `generateSequence`: the first group VAL times, then each later group's
/// items spread among the ones already there.
pub fn sequence<'a>(groups: &[(&'a str, usize)]) -> Vec<&'a str> {
    let Some((&(first, count), rest)) = groups.split_first() else {
        return Vec::new();
    };
    let mut seq = vec![first; count];
    for &(item, val) in rest {
        for i in 0..val {
            // FIRESTARTER floors the quotient in `float`; the operands are small integers,
            // so the integer division gives the same position.
            seq.insert(1 + i * (seq.len() + val - i) / val, item);
        }
    }
    seq
}

/// The sequence of `text`, repeated `LINES / len` times (`getNumberOfSequenceRepetitions`).
pub fn unroll(text: &str) -> Result<Vec<&str>, String> {
    let seq = sequence(&parse_groups(text)?);
    if seq.len() > LINES {
        return Err(format!("{text:?}: longer than {LINES} lines"));
    }
    Ok(seq.repeat(LINES / seq.len()))
}
