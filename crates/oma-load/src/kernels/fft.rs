//! Double-precision radix-2 FFT for K2, K3 and K4 (§4.1, DA9): decimation in time on split
//! real and imaginary arrays, with the twiddles of every stage stored side by side
//! (`tw[half + j]` for the stage with `half` butterflies per block), so every stage reads
//! them in order. The butterflies are AVX-512, AVX2 with FMA or SSE2; the stages narrower
//! than a vector, and the bit reversal, are scalar.

use std::arch::x86_64::*;

use oma_ipc::load::Isa;

use crate::kernel::KernelError;
use crate::sys::memory::Region;

type Stage = unsafe fn(*mut f64, *mut f64, usize, usize, *const f64, *const f64);

pub struct Fft {
    max_n: usize,
    lanes: usize,
    stage: Stage,
    tw_re: F64Buf,
    tw_im: F64Buf,
}

/// Buffers above this many bytes come from `Region` (VirtualAlloc), below from the heap.
const REGION_ABOVE: usize = 64 << 20;

/// A zeroed f64 buffer: a `Vec` when small, a `Region` when big.
pub enum F64Buf {
    Heap(Vec<f64>),
    Region(Region),
}

impl std::ops::Deref for F64Buf {
    type Target = [f64];
    fn deref(&self) -> &[f64] {
        match self {
            Self::Heap(v) => v,
            Self::Region(r) => {
                let s = r.as_slice();
                // SAFETY: u64 and f64 have the same size and alignment, and every bit
                // pattern is a valid f64.
                unsafe { std::slice::from_raw_parts(s.as_ptr().cast(), s.len()) }
            }
        }
    }
}

impl std::ops::DerefMut for F64Buf {
    fn deref_mut(&mut self) -> &mut [f64] {
        match self {
            Self::Heap(v) => v,
            Self::Region(r) => {
                let s = r.as_mut_slice();
                // SAFETY: as in `deref`; `&mut self` makes this the only access.
                unsafe { std::slice::from_raw_parts_mut(s.as_mut_ptr().cast(), s.len()) }
            }
        }
    }
}

/// `len` zeroed f64, or `Err` when the memory is not there (never aborts). The one place
/// the FFT kernels allocate their big buffers.
pub fn alloc_f64(len: usize) -> Result<F64Buf, KernelError> {
    let bytes = len.checked_mul(8).ok_or(KernelError::Insufficient)?;
    if bytes > REGION_ABOVE {
        return Region::alloc(bytes as u64).map(F64Buf::Region);
    }
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| KernelError::Insufficient)?;
    v.resize(len, 0.0);
    Ok(F64Buf::Heap(v))
}

impl Fft {
    /// Twiddles for every power of 2 up to `n` (a power of 2, at least 2). `Unsupported`
    /// when the CPU lacks `isa`; `Insufficient` when the table does not fit in memory.
    pub fn new(n: usize, isa: Isa) -> Result<Self, KernelError> {
        assert!(n >= 2 && n.is_power_of_two());
        let (lanes, stage): (usize, Stage) = match isa {
            Isa::Avx512 if is_x86_feature_detected!("avx512f") => (8, stage_avx512),
            Isa::Avx2 if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") => {
                (4, stage_avx2)
            }
            Isa::Sse2 => (2, stage_sse2),
            _ => return Err(KernelError::Unsupported),
        };
        let (mut tw_re_buf, mut tw_im_buf) = (alloc_f64(n)?, alloc_f64(n)?);
        // One deref, not one per element (a call per access in a debug build).
        let (tw_re, tw_im) = (&mut tw_re_buf[..], &mut tw_im_buf[..]);
        // The top stage from the formula, the others are every other entry of the one above.
        let top = n / 2;
        for j in 0..top {
            let (c, s) = cos_sin_turn(j, n);
            tw_re[top + j] = c;
            tw_im[top + j] = -s;
        }
        let mut half = top / 2;
        while half >= 1 {
            for j in 0..half {
                tw_re[half + j] = tw_re[2 * half + 2 * j];
                tw_im[half + j] = tw_im[2 * half + 2 * j];
            }
            half /= 2;
        }
        Ok(Self {
            max_n: n,
            lanes,
            stage,
            tw_re: tw_re_buf,
            tw_im: tw_im_buf,
        })
    }

    /// Test helper: breaks one twiddle of the top stage.
    #[cfg(test)]
    pub(crate) fn corrupt_for_test(&mut self) {
        let at = self.max_n / 2 + 3;
        self.tw_re[at] += 0.1;
    }

    pub fn forward(&self, re: &mut [f64], im: &mut [f64]) {
        self.forward_with(re, im, &mut || true);
    }

    pub fn inverse(&self, re: &mut [f64], im: &mut [f64]) {
        self.inverse_with(re, im, &mut || true);
    }

    /// In-place forward transform of `re.len()` points (a power of 2, at most the size
    /// given to `new`). `tick` runs after the bit reversal, every 2^20 points of it and
    /// after each stage; when it returns false the transform stops and this returns false.
    pub fn forward_with(
        &self,
        re: &mut [f64],
        im: &mut [f64],
        tick: &mut dyn FnMut() -> bool,
    ) -> bool {
        let n = re.len();
        assert!(n.is_power_of_two() && n <= self.max_n && im.len() == n);
        if n < 2 {
            return true;
        }
        let shift = usize::BITS - n.ilog2();
        for i in 0..n {
            let j = i.reverse_bits() >> shift;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
            if i & 0xF_FFFF == 0xF_FFFF && !tick() {
                return false;
            }
        }
        if !tick() {
            return false;
        }
        let mut half = 1;
        while half < n {
            let (wr, wi) = (&self.tw_re[half..2 * half], &self.tw_im[half..2 * half]);
            if half >= self.lanes {
                // SAFETY: `new` picked `stage` for an instruction set this CPU has; the
                // arrays hold `n` points, `n` is a multiple of `2 * half`, and `wr`, `wi`
                // hold `half` entries, a multiple of the vector width here.
                unsafe {
                    (self.stage)(
                        re.as_mut_ptr(),
                        im.as_mut_ptr(),
                        n,
                        half,
                        wr.as_ptr(),
                        wi.as_ptr(),
                    )
                }
            } else {
                stage_scalar(re, im, half, wr, wi);
            }
            if !tick() {
                return false;
            }
            half *= 2;
        }
        true
    }

    /// The inverse, scaled by 1/N: the forward transform with the two arrays swapped.
    pub fn inverse_with(
        &self,
        re: &mut [f64],
        im: &mut [f64],
        tick: &mut dyn FnMut() -> bool,
    ) -> bool {
        if !self.forward_with(im, re, tick) {
            return false;
        }
        // N is a power of 2, so the scale is exact.
        let scale = 1.0 / re.len() as f64;
        re.iter_mut().chain(im.iter_mut()).for_each(|v| *v *= scale);
        true
    }
}

fn stage_scalar(re: &mut [f64], im: &mut [f64], half: usize, wr: &[f64], wi: &[f64]) {
    for (r, i) in re
        .chunks_exact_mut(2 * half)
        .zip(im.chunks_exact_mut(2 * half))
    {
        for j in 0..half {
            let (xr, xi) = (r[j + half], i[j + half]);
            let tr = xr * wr[j] - xi * wi[j];
            let ti = xr * wi[j] + xi * wr[j];
            let (ur, ui) = (r[j], i[j]);
            r[j] = ur + tr;
            i[j] = ui + ti;
            r[j + half] = ur - tr;
            i[j + half] = ui - ti;
        }
    }
}

/// One stage per instruction set: the same loop with the vector width and the way the
/// complex product is made (FMA, or multiply and add) changed.
macro_rules! stage_fn {
    ($name:ident, $feat:literal, $ty:ty, $load:ident, $store:ident, $add:ident, $sub:ident,
     $w:expr, |$xr:ident, $xi:ident, $wr:ident, $wi:ident| ($tr:expr, $ti:expr)) => {
        #[target_feature(enable = $feat)]
        unsafe fn $name(
            re: *mut f64,
            im: *mut f64,
            n: usize,
            half: usize,
            wr: *const f64,
            wi: *const f64,
        ) {
            // SAFETY: the caller guarantees the feature, `n` points in each array, a
            // multiple of `2 * half` of them, `half` twiddles and `half % $w == 0`, so
            // every load and store below is inside those ranges.
            unsafe {
                let mut s = 0;
                while s < n {
                    let mut j = 0;
                    while j < half {
                        let (a, b) = (s + j, s + j + half);
                        let $xr: $ty = $load(re.add(b));
                        let $xi: $ty = $load(im.add(b));
                        let $wr: $ty = $load(wr.add(j));
                        let $wi: $ty = $load(wi.add(j));
                        let (tr, ti) = ($tr, $ti);
                        let (ur, ui) = ($load(re.add(a)), $load(im.add(a)));
                        $store(re.add(a), $add(ur, tr));
                        $store(im.add(a), $add(ui, ti));
                        $store(re.add(b), $sub(ur, tr));
                        $store(im.add(b), $sub(ui, ti));
                        j += $w;
                    }
                    s += 2 * half;
                }
            }
        }
    };
}

stage_fn!(
    stage_avx512,
    "avx512f",
    __m512d,
    _mm512_loadu_pd,
    _mm512_storeu_pd,
    _mm512_add_pd,
    _mm512_sub_pd,
    8,
    |xr, xi, wr, wi| (
        _mm512_fmsub_pd(xr, wr, _mm512_mul_pd(xi, wi)),
        _mm512_fmadd_pd(xr, wi, _mm512_mul_pd(xi, wr))
    )
);
stage_fn!(
    stage_avx2,
    "avx2,fma",
    __m256d,
    _mm256_loadu_pd,
    _mm256_storeu_pd,
    _mm256_add_pd,
    _mm256_sub_pd,
    4,
    |xr, xi, wr, wi| (
        _mm256_fmsub_pd(xr, wr, _mm256_mul_pd(xi, wi)),
        _mm256_fmadd_pd(xr, wi, _mm256_mul_pd(xi, wr))
    )
);
stage_fn!(
    stage_sse2,
    "sse2",
    __m128d,
    _mm_loadu_pd,
    _mm_storeu_pd,
    _mm_add_pd,
    _mm_sub_pd,
    2,
    |xr, xi, wr, wi| (
        _mm_sub_pd(_mm_mul_pd(xr, wr), _mm_mul_pd(xi, wi)),
        _mm_add_pd(_mm_mul_pd(xr, wi), _mm_mul_pd(xi, wr))
    )
);

/// `(cos, sin)` of the angle `2π k / n` (`n` a power of 2, `k < n`). Plain multiplications
/// and additions after an exact reduction to an octant, so the bits are the same on every
/// CPU and C runtime.
fn cos_sin_turn(k: usize, n: usize) -> (f64, f64) {
    let quadrant = 4 * k / n;
    let r = 4 * k - quadrant * n;
    // Within the quadrant the angle is (π/2) r/n; past π/4 use the complement.
    let (num, swap) = if 2 * r > n { (n - r, true) } else { (r, false) };
    let x = std::f64::consts::FRAC_PI_2 * (num as f64 / n as f64);
    let x2 = x * x;
    // Nested (Horner) Taylor series; x <= π/4, so 13 terms are far below 1e-16.
    let (mut s, mut c) = (1.0, 1.0);
    for t in (1..=13).rev() {
        let a = (2 * t) as f64;
        c = 1.0 - x2 / ((a - 1.0) * a) * c;
        s = 1.0 - x2 / (a * (a + 1.0)) * s;
    }
    let (sin, cos) = (x * s, c);
    let (c0, s0) = if swap { (sin, cos) } else { (cos, sin) };
    match quadrant {
        0 => (c0, s0),
        1 => (-s0, c0),
        2 => (-c0, -s0),
        _ => (s0, -c0),
    }
}

/// A digest of the bit patterns of `re` then `im`, four interleaved chains so it costs
/// little next to the transform.
pub fn digest(re: &[f64], im: &[f64]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut h = [1u64, 2, 3, 4].map(|i| 0xCBF2_9CE4_8422_2325 ^ i.wrapping_mul(K));
    for part in [re, im] {
        let chunks = part.chunks_exact(4);
        let rest = chunks.remainder();
        for c in chunks {
            for (h, v) in h.iter_mut().zip(c) {
                *h = (*h ^ v.to_bits()).wrapping_mul(K).rotate_left(31);
            }
        }
        for v in rest {
            h[0] = (h[0] ^ v.to_bits()).wrapping_mul(K).rotate_left(31);
        }
    }
    crate::verify::digest_words(&h)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::rng::Xoshiro256ss;

    /// The instruction sets this machine has, saying which are skipped.
    pub(crate) fn available() -> Vec<Isa> {
        [Isa::Avx512, Isa::Avx2, Isa::Sse2]
            .into_iter()
            .filter(|&isa| match Fft::new(2, isa) {
                Err(KernelError::Unsupported) => {
                    eprintln!("skipped: {isa:?} not available");
                    false
                }
                _ => true,
            })
            .collect()
    }

    fn data(n: usize, seed: u64) -> (Vec<f64>, Vec<f64>) {
        let mut rng = Xoshiro256ss::new(seed);
        let mut v = |_| (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64 - 0.5;
        ((0..n).map(&mut v).collect(), (0..n).map(&mut v).collect())
    }

    #[test]
    fn fft_matches_a_naive_dft_for_n_16_and_64() {
        for isa in available() {
            for n in [16usize, 64] {
                let (xr, xi) = data(n, 5);
                let (mut re, mut im) = (xr.clone(), xi.clone());
                Fft::new(n, isa).unwrap().forward(&mut re, &mut im);
                for k in 0..n {
                    let (mut sr, mut si) = (0.0, 0.0);
                    for t in 0..n {
                        let a = -2.0 * std::f64::consts::PI * ((k * t) % n) as f64 / n as f64;
                        sr += xr[t] * a.cos() - xi[t] * a.sin();
                        si += xr[t] * a.sin() + xi[t] * a.cos();
                    }
                    assert!(
                        (re[k] - sr).abs() < 1e-12 && (im[k] - si).abs() < 1e-12,
                        "{isa:?} n={n} k={k}"
                    );
                }
            }
        }
    }

    #[test]
    fn fft_round_trip_within_tolerance_for_each_isa() {
        for isa in available() {
            // Large enough for every stage to run vectorised.
            let n = 4096;
            let (xr, xi) = data(n, 9);
            let (mut re, mut im) = (xr.clone(), xi.clone());
            let fft = Fft::new(n, isa).unwrap();
            fft.forward(&mut re, &mut im);
            fft.inverse(&mut re, &mut im);
            for i in 0..n {
                assert!(
                    (re[i] - xr[i]).abs() < 1e-12 && (im[i] - xi[i]).abs() < 1e-12,
                    "{isa:?}"
                );
            }
        }
    }

    #[test]
    fn fft_dc_sum_check() {
        for isa in available() {
            let n = 1024;
            let (xr, xi) = data(n, 3);
            let (mut re, mut im) = (xr.clone(), xi.clone());
            Fft::new(n, isa).unwrap().forward(&mut re, &mut im);
            assert!((re[0] - xr.iter().sum::<f64>()).abs() < 1e-9);
            assert!((im[0] - xi.iter().sum::<f64>()).abs() < 1e-9);
        }
    }

    #[test]
    fn twiddles_are_exact_at_the_axes_and_accurate_elsewhere() {
        assert_eq!(cos_sin_turn(0, 8).0, 1.0);
        assert_eq!(cos_sin_turn(2, 8).0.abs(), 0.0);
        for k in 0..64 {
            let a = 2.0 * std::f64::consts::PI * k as f64 / 64.0;
            let (c, s) = cos_sin_turn(k, 64);
            assert!(
                (c - a.cos()).abs() < 1e-15 && (s - a.sin()).abs() < 1e-15,
                "{k}"
            );
        }
    }

    #[test]
    fn tick_false_stops_the_transform() {
        let (mut re, mut im) = data(64, 1);
        let fft = Fft::new(64, Isa::Sse2).unwrap();
        let mut calls = 0;
        assert!(!fft.forward_with(&mut re, &mut im, &mut || {
            calls += 1;
            calls < 3
        }));
        assert_eq!(calls, 3);
    }

    #[test]
    fn big_buffers_come_from_a_region() {
        let mut big = alloc_f64(REGION_ABOVE / 8 + 8).unwrap();
        assert!(matches!(big, F64Buf::Region(_)));
        assert!(big.iter().all(|&x| x == 0.0));
        let last = big.len() - 1;
        big[last] = 1.5;
        assert_eq!(big[last], 1.5);
        assert!(matches!(alloc_f64(1024).unwrap(), F64Buf::Heap(_)));
    }
}
