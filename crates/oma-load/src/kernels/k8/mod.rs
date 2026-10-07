//! K8, crypto, checksum, compression and sort (§4.1, DA9): AES-128 (AES-NI) over 64 KiB,
//! SHA-256 (SHA-NI or scalar) of the same 64 KiB, carry-less multiplications (PCLMULQDQ)
//! of its words, CRC32C of it, an LZ77 round trip of 256 KiB of text from a 256-word
//! dictionary, and a quicksort of 65 536 `u32`. The instruction set (`ctx.isa`) does not
//! change the work.
//!
//! **The digest.** Each step gives a 64-bit result; the digest is the xor of one mixed
//! value per step (`mix(step number, result)`). A step the CPU cannot do (AES without
//! AES-NI) contributes nothing, so the other steps' share of the digest is the same on any
//! CPU. SHA-NI and the scalar SHA-256 give the same result, and the CLMUL and CRC32C
//! software fallbacks too, so only a missing AES-NI changes the digest, and the reference
//! is computed by the same code on the same machine. The LZ round trip and the sort check
//! themselves and give `Check::Mismatch` when they fail.

mod aes;
mod clmul;
pub(crate) mod lz;
pub(crate) mod sha256;
pub(crate) mod sort;

use std::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::{Check, Kernel, KernelError, KernelFactory, RefFailure, WorkerCtx};
use crate::kernels::k1::crc::{crc32c, crc32c_u64};
use crate::rng::Xoshiro256ss;

const BUF_BYTES: usize = 64 * 1024;
pub(crate) const TEXT_BYTES: usize = 256 * 1024;
const SORT_LEN: usize = 65_536;
/// One CLMUL product in this many is checked against the software one.
const CLMUL_CHECK_EVERY: usize = 64;
const KEY: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];

pub(crate) fn mix(step: u64, r: u64) -> u64 {
    let mut z = r ^ step.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn fold128(v: u128) -> u64 {
    mix(1, v as u64) ^ mix(2, (v >> 64) as u64)
}

pub(crate) fn fold_bytes(b: &[u8]) -> u64 {
    b.chunks(8).enumerate().fold(0, |acc, (i, c)| {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        acc ^ mix(i as u64, u64::from_le_bytes(w))
    })
}

fn has_aes() -> bool {
    is_x86_feature_detected!("aes") && is_x86_feature_detected!("sse2")
}

pub(crate) fn has_sha() -> bool {
    is_x86_feature_detected!("sha")
        && is_x86_feature_detected!("ssse3")
        && is_x86_feature_detected!("sse4.1")
}

fn has_clmul() -> bool {
    is_x86_feature_detected!("pclmulqdq") && is_x86_feature_detected!("sse2")
}

/// 256 words of 4–10 lowercase letters, and `TEXT_BYTES` of them chosen by a walk where
/// each word is followed by one of two words, so the text is compressible.
pub(crate) fn make_text(rng: &mut Xoshiro256ss) -> Vec<u8> {
    let dict: Vec<Vec<u8>> = (0..256)
        .map(|_| {
            let n = 4 + (rng.next_u64() % 7) as usize;
            (0..n).map(|_| b'a' + (rng.next_u64() % 26) as u8).collect()
        })
        .collect();
    let mut text = Vec::with_capacity(TEXT_BYTES + 16);
    let mut cur = 0usize;
    while text.len() < TEXT_BYTES {
        text.extend_from_slice(&dict[cur]);
        text.push(b' ');
        cur = (cur * 5 + 1 + (rng.next_u64() & 1) as usize) % 256;
    }
    text.truncate(TEXT_BYTES);
    text
}

pub(crate) struct K8 {
    buf: Vec<u8>,
    text: Vec<u8>,
    sort_src: Vec<u32>,
    sort_work: Vec<u32>,
    aes: bool,
    sha: bool,
    clmul: bool,
    /// Test hook: (word of `buf`, bit) flipped once halfway through, before SHA-256.
    #[cfg(test)]
    pub(crate) flip: Option<(usize, u32)>,
}

impl K8 {
    pub(crate) fn new(ctx: &WorkerCtx) -> Result<Self, KernelError> {
        let mut rng = Xoshiro256ss::new(ctx.seed);
        let mut buf = vec![0u8; BUF_BYTES];
        buf.chunks_exact_mut(8)
            .for_each(|c| c.copy_from_slice(&rng.next_u64().to_le_bytes()));
        let text = make_text(&mut rng);
        let sort_src: Vec<u32> = (0..SORT_LEN).map(|_| rng.next_u64() as u32).collect();
        Ok(Self {
            buf,
            text,
            sort_work: vec![0; SORT_LEN],
            sort_src,
            aes: has_aes(),
            sha: has_sha(),
            clmul: has_clmul(),
            #[cfg(test)]
            flip: None,
        })
    }
}

impl Kernel for K8 {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        let bump = || {
            beat.fetch_add(1, Ordering::Relaxed);
        };
        let mut digest = 0u64;

        // Step 0, AES-128 chain; skipped without AES-NI.
        if self.aes {
            // SAFETY: AES-NI and SSE2 were detected in `new`.
            let c = unsafe { aes::chain(&KEY, &self.buf) };
            digest ^= mix(0, fold_bytes(&c));
        }
        bump();

        #[cfg(test)]
        if let Some((word, bit)) = self.flip.take() {
            self.buf[word * 8 + (bit / 8) as usize] ^= 1 << (bit % 8);
        }

        // Step 1, SHA-256.
        digest ^= mix(1, fold_bytes(&sha256::sha256(&self.buf, self.sha)));
        bump();

        // Step 2, carry-less products, checked against the software ones.
        let acc = if self.clmul {
            // SAFETY: PCLMULQDQ and SSE2 were detected in `new`.
            match unsafe { clmul::accumulate_hw(&self.buf, CLMUL_CHECK_EVERY) } {
                Ok(a) => a,
                Err((expected, actual)) => {
                    return Check::Mismatch {
                        expected: expected as u64,
                        actual: actual as u64,
                    }
                }
            }
        } else {
            clmul::accumulate_sw(&self.buf)
        };
        digest ^= mix(2, fold128(acc));
        bump();

        // Step 3, CRC32C (the hardware instruction when there is one).
        let crc = self.buf.chunks_exact(8).fold(0xFFFF_FFFFu32, |c, w| {
            crc32c_u64(c, u64::from_le_bytes(w.try_into().unwrap()))
        });
        digest ^= mix(3, u64::from(crc));
        bump();

        // Step 4, LZ77 round trip.
        let packed = lz::compress(&self.text);
        bump();
        match lz::decompress(&packed, TEXT_BYTES) {
            Ok(out) if out == self.text => {}
            Ok(out) => {
                return Check::Mismatch {
                    expected: u64::from(crc32c(&self.text)),
                    actual: u64::from(crc32c(&out)),
                }
            }
            Err(_) => {
                return Check::Mismatch {
                    expected: u64::from(crc32c(&self.text)),
                    actual: 0,
                }
            }
        }
        digest ^= mix(4, u64::from(crc32c(&packed)) | (packed.len() as u64) << 32);
        bump();

        // Step 5, quicksort: sorted, with the same sum.
        self.sort_work.copy_from_slice(&self.sort_src);
        sort::sort(&mut self.sort_work);
        let sum = |v: &[u32]| v.iter().map(|&x| u64::from(x)).sum::<u64>();
        let (want, got) = (sum(&self.sort_src), sum(&self.sort_work));
        if want != got || self.sort_work.windows(2).any(|w| w[0] > w[1]) {
            return Check::Mismatch {
                expected: want,
                actual: got,
            };
        }
        let crc = self.sort_work.chunks_exact(2).fold(0xFFFF_FFFFu32, |c, p| {
            crc32c_u64(c, u64::from(p[0]) | u64::from(p[1]) << 32)
        });
        digest ^= mix(5, u64::from(crc) | got << 32);
        bump();

        Check::Digest(digest)
    }
}

pub(crate) struct K8Factory;

/// The known vectors on this machine, for the paths it has (FIPS-197 C.1, SHA-256 "abc",
/// CRC32C "123456789"); a failure is a defect of the code, not of a core.
pub(crate) fn check_vectors() -> Result<(), String> {
    const PLAIN: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    const CIPHER: [u8; 16] = [
        0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4, 0xc5,
        0x5a,
    ];
    if has_aes() {
        // SAFETY: AES-NI and SSE2 were just detected.
        if unsafe { aes::encrypt_block(&KEY, &PLAIN) } != CIPHER {
            return Err("AES-128 FIPS-197 vector".into());
        }
    }
    let abc = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22,
        0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00,
        0x15, 0xad,
    ];
    if sha256::sha256(b"abc", has_sha()) != abc {
        return Err("SHA-256 \"abc\" vector".into());
    }
    if crc32c(b"123456789") != 0xE306_9283 {
        return Err("CRC32C \"123456789\" vector".into());
    }
    Ok(())
}

impl KernelFactory for K8Factory {
    fn reference(&self, ctx: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        Some((|| {
            check_vectors().map_err(RefFailure::Invalid)?;
            let mut k = K8::new(ctx)?;
            match k.iterate(&AtomicU64::new(0)) {
                Check::Digest(d) => Ok(vec![d]),
                _ => Err(RefFailure::Invalid(
                    "the K8 self-checks failed in the reference".into(),
                )),
            }
        })())
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(K8::new(ctx)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{PhaseShared, ThreadBudget};
    use oma_ipc::load::{DataSize, Isa};
    use std::sync::Arc;

    fn ctx(seed: u64) -> WorkerCtx {
        WorkerCtx {
            isa: Isa::Sse2,
            size: DataSize::L2,
            budget: ThreadBudget {
                l1d: 32 * 1024,
                l2_thread: 256 * 1024,
                l3_share: 1024 * 1024,
                ram_per_thread: 1 << 20,
            },
            seed,
            worker: 0,
            workers: 1,
            patterns: Vec::new(),
            shared: Arc::new(PhaseShared::default()),
        }
    }

    fn digest(k: &mut K8) -> u64 {
        match k.iterate(&AtomicU64::new(0)) {
            Check::Digest(d) => d,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn aes_fips197_vector() {
        if !has_aes() {
            eprintln!("skipped: aes not available");
            return;
        }
        check_vectors().unwrap();
    }

    #[test]
    fn sha256_abc_vector_with_and_without_sha_ni() {
        let hex = |d: [u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let want = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(hex(sha256::sha256(b"abc", false)), want);
        // Two-block message of the FIPS-180-4 examples.
        let two = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        let want2 = "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1";
        assert_eq!(hex(sha256::sha256(two, false)), want2);
        if has_sha() {
            assert_eq!(hex(sha256::sha256(b"abc", true)), want);
            assert_eq!(hex(sha256::sha256(two, true)), want2);
            let k = K8::new(&ctx(1)).unwrap();
            assert_eq!(sha256::sha256(&k.buf, true), sha256::sha256(&k.buf, false));
        } else {
            eprintln!("skipped: sha not available");
        }
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn clmul_matches_software() {
        assert_eq!(clmul::mul_sw(0b101, 0b11), 0b1111);
        let k = K8::new(&ctx(2)).unwrap();
        if has_clmul() {
            // SAFETY: PCLMULQDQ and SSE2 were just detected.
            let hw = unsafe { clmul::accumulate_hw(&k.buf, 1) };
            assert_eq!(hw, Ok(clmul::accumulate_sw(&k.buf)));
        } else {
            eprintln!("skipped: pclmulqdq not available");
        }
    }

    #[test]
    fn lz_round_trip_and_compresses() {
        let mut rng = Xoshiro256ss::new(3);
        let text = make_text(&mut rng);
        let packed = lz::compress(&text);
        assert!(packed.len() < text.len() / 2, "{} bytes", packed.len());
        assert_eq!(lz::decompress(&packed, TEXT_BYTES).unwrap(), text);
        // Incompressible data (long literal runs) and tiny inputs round trip too.
        let noise: Vec<u8> = (0..2000).map(|_| rng.next_u64() as u8).collect();
        for data in [&noise[..], b"", b"abc", b"aaaaaaaaaaaaaaaaaaaaaaaa"] {
            assert_eq!(lz::decompress(&lz::compress(data), 4096).unwrap(), data);
        }
    }

    #[test]
    fn lz_rejects_a_corrupted_stream_without_panic() {
        let mut rng = Xoshiro256ss::new(4);
        let text = make_text(&mut rng);
        let packed = lz::compress(&text[..4096]);
        // Every truncation and every single-byte corruption is an error or a different
        // output, never a panic.
        for cut in 0..packed.len() {
            let _ = lz::decompress(&packed[..cut], TEXT_BYTES);
        }
        for i in 0..packed.len() {
            let mut bad = packed.clone();
            bad[i] ^= 0xFF;
            let _ = lz::decompress(&bad, TEXT_BYTES);
        }
        // Offset beyond the output, output over the limit, trailing bytes.
        assert!(lz::decompress(&[0, 5, 0, 9, 0, 0], 100).is_err());
        assert!(lz::decompress(&[1, b'a', 1, 0, 255, 0, 0], 100).is_err());
        assert!(lz::decompress(&[0, 0, 0, 7], 100).is_err());
        assert!(lz::decompress(&[], 100).is_err());
    }

    #[test]
    fn sort_sorts_and_keeps_the_sum() {
        let mut rng = Xoshiro256ss::new(5);
        for n in [0, 1, 15, 16, 17, 1000, SORT_LEN] {
            let src: Vec<u32> = (0..n).map(|_| rng.next_u64() as u32 % 1000).collect();
            let mut v = src.clone();
            sort::sort(&mut v);
            let mut want = src;
            want.sort_unstable();
            assert_eq!(v, want, "n = {n}");
        }
    }

    #[test]
    fn k8_digest_is_deterministic() {
        let mut a = K8::new(&ctx(7)).unwrap();
        let first = digest(&mut a);
        assert_eq!(first, digest(&mut a), "again");
        let mut other = ctx(7);
        other.worker = 3;
        other.workers = 4;
        assert_eq!(first, digest(&mut K8::new(&other).unwrap()));
        assert_ne!(first, digest(&mut K8::new(&ctx(8)).unwrap()));
        assert_eq!(K8Factory.reference(&ctx(7)), Some(Ok(vec![first])));
    }

    #[test]
    fn k8_bit_flip_is_a_mismatch() {
        let reference = digest(&mut K8::new(&ctx(9)).unwrap());
        let mut k = K8::new(&ctx(9)).unwrap();
        k.flip = Some((100, 5));
        assert_ne!(digest(&mut k), reference);
    }

    #[test]
    fn k8_iteration_time() {
        let mut k = K8::new(&ctx(1)).unwrap();
        let t = std::time::Instant::now();
        for _ in 0..5 {
            digest(&mut k);
        }
        eprintln!("K8 iteration: {:?}", t.elapsed() / 5);
    }
}
