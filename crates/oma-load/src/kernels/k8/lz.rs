//! A small LZ77 compressor and its decoder.
//!
//! Stream: a sequence of `[literal count u8][literals][offset u16 LE][length u8]`. A length
//! of 0 is a null reference (it continues a literal run longer than 255); an offset of 0
//! ends the stream and has no length byte. A reference copies `length` (4..=255) bytes from
//! `offset` bytes back, byte by byte (it may overlap). The match finder is a hash table of
//! 4096 entries over the next 4 bytes.

const MIN_MATCH: usize = 4;
const HASH_SIZE: usize = 4096;

fn hash(b: &[u8]) -> usize {
    let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    (v.wrapping_mul(2_654_435_761) >> 20) as usize
}

pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut table = [usize::MAX; HASH_SIZE];
    let (mut i, mut lit_start) = (0, 0);
    while i + MIN_MATCH <= data.len() {
        let h = hash(&data[i..]);
        let cand = table[h];
        table[h] = i;
        let mut len = 0;
        if cand != usize::MAX && i - cand <= u16::MAX as usize {
            while len < 255 && i + len < data.len() && data[cand + len] == data[i + len] {
                len += 1;
            }
        }
        if len >= MIN_MATCH {
            emit_literals(&mut out, &data[lit_start..i]);
            out.extend_from_slice(&((i - cand) as u16).to_le_bytes());
            out.push(len as u8);
            let end = (i + len).min(data.len() - (MIN_MATCH - 1));
            for j in i + 1..end {
                table[hash(&data[j..])] = j;
            }
            i += len;
            lit_start = i;
        } else {
            i += 1;
        }
    }
    emit_literals(&mut out, &data[lit_start..]);
    out.extend_from_slice(&[0, 0]);
    out
}

/// Writes the literals as runs of at most 255; a full run is followed by a null
/// reference, and the caller writes the real reference (or the end) after the last run.
fn emit_literals(out: &mut Vec<u8>, mut lits: &[u8]) {
    while lits.len() > 255 {
        out.push(255);
        out.extend_from_slice(&lits[..255]);
        out.extend_from_slice(&[1, 0, 0]);
        lits = &lits[255..];
    }
    out.push(lits.len() as u8);
    out.extend_from_slice(lits);
}

/// Decodes `stream`; `Err` for any malformed stream or output over `max_out`.
pub fn decompress(stream: &[u8], max_out: usize) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::new();
    let mut p = 0;
    loop {
        let n = *stream.get(p).ok_or("truncated")? as usize;
        let lits = stream.get(p + 1..p + 1 + n).ok_or("truncated literals")?;
        if out.len() + n > max_out {
            return Err("output too long");
        }
        out.extend_from_slice(lits);
        p += 1 + n;
        let off = stream.get(p..p + 2).ok_or("truncated offset")?;
        let off = u16::from_le_bytes([off[0], off[1]]) as usize;
        p += 2;
        if off == 0 {
            return if p == stream.len() {
                Ok(out)
            } else {
                Err("trailing bytes")
            };
        }
        let len = *stream.get(p).ok_or("truncated length")? as usize;
        p += 1;
        if len == 0 {
            continue;
        }
        if len < MIN_MATCH || off > out.len() || out.len() + len > max_out {
            return Err("bad reference");
        }
        for _ in 0..len {
            out.push(out[out.len() - off]);
        }
    }
}
