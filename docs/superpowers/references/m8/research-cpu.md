# CPU stress test and CPU benchmark: prior art and design input for OpenMonitor Advanced

Research date: 2026-10-06. Scope: how existing tools stress the CPU / caches / memory controller, how they detect errors, licenses, Windows (non-admin) techniques, and how CPU benchmarks compute scores. Every claim carries a source URL. Items marked **[unverified]** come from general knowledge or from forum-grade sources and should be double-checked before they go into a spec. Where an official document could not be fetched (OCCT docs, Prime95 `stress.txt`, y-cruncher stress-test page) I say so.

Constraints that shape everything below: Rust (toolchain 1.90.0) + .NET service, GPL-3.0-or-later, no admin, vendor neutral, x86-64 only, runtime ISA detection.

---

## 0. Executive summary

1. **All serious CPU stress tools are "deterministic work + verify". The verification is what separates a stability test from a heater.** Prime95 (FFT round-off + SUMINP/SUMOUT + known residues), y-cruncher (modular identity mod 2^61-1, "Coefficient is too large" check), Linpack (scaled residual of Ax-b), FIRESTARTER (CRC32 hash of vector registers compared between neighbouring threads), OpenDCDiag (golden value recomputed on every thread), stress-ng `--verify` (per-method checksums), stressapptest (CRC while copying), memtest (write/read-back patterns).
2. **The normal test should also verify.** Silent corruption happens on stock parts too (Google "mercurial cores", Meta/Intel SDE papers, Intel Raptor Lake Vmin-shift degradation). Verification is nearly free if designed in (a hash per iteration). A "normal" run that reports PASS while the CPU computes wrong answers is a false assurance. Normal and OC tests should therefore share the same verified kernels; they differ in kernel selection, load shape, per-core cycling, duration and how strict the stop condition is.
3. **Most sensitive tests found by overclockers:** y-cruncher N63 and VT3 (integer NTT / vector transform), Prime95 Small FFT with AVX2/AVX-512, OCCT (variable load, extreme, large dataset), per-core cycling (CoreCycler, OCCT core cycling) for Curve Optimizer / undervolt. Single-core boost-clock testing finds errors that all-core tests do not (all-core runs at lower clock).
4. **Everything we need runs in user mode without admin:** thread pinning (`SetThreadGroupAffinity`, CPU Sets), P/E core detection (`GetSystemCpuSetInformation.EfficiencyClass`), CCD/L3 grouping (`LastLevelCacheIndex`), AVX-512 intrinsics (stable since Rust 1.89), and reading WHEA events (the System log is readable by Interactive Users by default).
5. **What we cannot do without admin/driver:** reading MCA banks/MSRs directly, testing all physical RAM, guaranteed large pages (needs `SeLockMemoryPrivilege`). Mitigation: WHEA events via the event log, and a crash journal that survives BSOD/reboot.
6. **Benchmarks** (Geekbench, Cinebench, CPU-Z, PassMark, 7-Zip) are all fixed-work or fixed-time kernels, scored relative to a baseline or reported as an op rate. Our benchmark can reuse the verified stress kernels as workloads (every run self-checks, like the 7-Zip benchmark).

---

## 1. Tool-by-tool findings

### 1.1 Prime95 / mprime (GIMPS)

**What it does.** Runs the Lucas-Lehmer / PRP FFT multiplication code (the `gwnum` library, IBDWT FFTs, hand-written assembly for SSE2/AVX/FMA3/AVX-512). The torture test runs known-answer FFT multiplications in a loop on all (or selected) workers.
- Source: https://www.mersenne.org/various/math.php, https://en.wikipedia.org/wiki/Prime95

**Torture test types** (UI options since 29.8 are "based on cache sizes", plus options for a weaker test):
- *Smallest FFTs*: tests L1/L2, high power/heat/CPU stress, little RAM traffic.
- *Small FFTs*: FFT range sized to fit L2 (some descriptions say L1/L2/L3), "maximum FPU stress", RAM not tested.
- *Large FFTs (in-place)*: FFTs larger than cache, run in place so the same RAM region is hit repeatedly; stresses memory controller and RAM, high heat/power.
- *Blend*: FFT sizes from very small to very large, not in-place, cycles through RAM, auto-picks "memory to use" to occupy nearly all RAM. Tests everything including the memory path.
- *Custom*: min/max FFT size (K), memory to use (0 = in-place), time per FFT size (default 15 min per size since v21.2), threads/hyper-threading.
- Sources: https://www.playtool.com/pages/prime95/prime95.html, https://www.mersenne.org/download/whatsnew_298b3.txt, https://www.mersenne.org/download/whatsnew_293.txt
- Exact FFT-size ranges per mode are in `stress.txt`; WebFetch could not retrieve them (the fetched copy was truncated). Not needed: we choose our own sizes from the detected cache sizes.

**ISA.** SSE2, AVX, AVX2/FMA3 and AVX-512 FFT kernels (AVX-512 added in 29.6, multithreaded add/sub for AVX-512 FFTs in 29.8). Users disable levels with `CpuSupportsAVX512F=0`, `CpuSupportsAVX2=0`, `CpuSupportsFMA3=0` in `prime.txt` and in the torture dialog. CoreCycler documents "SSE", "AVX", "AVX2" modes. Sources: https://www.mersenne.org/download/whatsnew_298b3.txt, https://github.com/sp00n/corecycler

**Error detection.**
- Round-off check: the maximum convolution error must stay below 0.4 (error if above 0.40, results incorrect above 0.49). "ROUND OFF > 0.40" message. Optional "round off checking" costs a few percent. Source: https://www.mersenne.org/download/readme.txt, https://www.mersenne.org/various/math.php
- Sum-of-inputs vs sum-of-outputs: "SUMINP != SUMOUT". This is the classic FFT invariant (DC term of the transform equals the sum of the inputs). Source: https://www.mersenne.org/download/readme.txt
- On either error: "Possible hardware failure", the worker waits 5 min then restarts from the last checkpoint. In the torture test any such error is reported as a failure (messages like "FATAL ERROR: Rounding was ..., expected less than 0.4" in results.txt). Source: readme.txt, playtool page.
- Torture test compares against known-correct values ("the torture test compares results against known correct values; a mismatch indicates a hardware problem"). Source: https://www.mersenne.org/download/stress.txt
- Real work only: Gerbicz error check (PRP), Jacobi check (LL), residues, double-checking. Not relevant for a stress test but shows the hierarchy of checks. Source: https://www.mersenne.org/various/math.php, https://www.mersenne.org/download/whatsnew_298b3.txt
- Round-off checks have false positives; the readme says some correct results are flagged ("Disregard last error..."). Design consequence: prefer **exact** (integer) verification wherever possible so there are no thresholds.

**Duration.** "Between 6 and 24 hours" (stress.txt), "run it for a day if you overclocked" (readme). Source: https://www.mersenne.org/download/stress.txt

**Hyper-threading advice.** One instance of Small FFT plus one of in-place Large FFT on the two threads of a core maximizes stress while limiting heat (playtool page). 29.1 changed the config to "cores per worker", HT optional.

**License.** Freeware; most source available; **not free software** (EULA with a prize clause: GIMPS claims prize if a prime is found). `gwnum` is the same. Not GPL compatible: study the algorithms only, never copy code. Sources: https://en.wikipedia.org/wiki/Prime95 (search results also cite the EULA text), https://rieselprime.de/ziki/Prime95
- Open alternative to study: **Mlucas** (Ernst Mayer), GPL-2.0-or-later for most files, FFT-based LL/PRP with round-off checks, runs on x86/ARM. https://mlucas.sourceforge.io , https://rieselprime.de/ziki/Mlucas

### 1.2 y-cruncher (Alexander Yee)

**What it does.** Computes pi etc. with multi-threaded big-number code (Chudnovsky, large multiplication via several algorithms). The **Component Stress Tester** runs the multiplication building blocks on all selected logical cores. Notorious for stressing the memory subsystem and AVX units; the author states it "exposes instabilities that other applications and stress-tests do not". Sources: https://numberworld.org/y-cruncher/ , https://numberworld.org/y-cruncher/faq.html

**Stress test components** (names from the stress-tester spec and version history):
| Test | What it is | Notes |
|---|---|---|
| BKT | Basecase + Karatsuba multiplication | scalar integer |
| BBP | BBP digit extraction | CPU-only, in cache, "unaffected by memory bottlenecks" (added 0.7.4) |
| SFT | small in-cache FFT | in cache |
| FFT | floating-point FFT (FFTv4/SFTv4 are the newer implementation) | FP |
| N32 / N64 -> N63 | 32/64-bit small-primes NTT; N63 is the ground-up rewrite | integer NTT, considered the test that "finds errors nothing else finds" |
| HNT | hybrid NTT (removed in 0.8.1) | integer + FP |
| VST -> VT3 | Vector-Scalable Transform (VT3 is VST reimplemented, since 0.8.1) | mixed integer/FP; the community says VT3 is the stronger one, N63 "rarely fails if VT3 passes" |
| SNT / SVT | small in-cache versions of N63 / VT3 | |
| C17 | "Cannon Lake 2017" (AVX2+ only; removed in 0.8.2) | |
- Sources: https://github.com/Mysticial/y-cruncher-GUI/blob/master/ParameterSpecs/StressTester.md , https://www.numberworld.org/y-cruncher/version_history.html , https://www.numberworld.org/y-cruncher/internals/multiplication.html , https://github.com/sp00n/corecycler/discussions/71
- Parameters: memory (0 to huge, divided among threads, local NUMA or global interleaved), minimum duration per test (s), logical core selection by id, "stop on error" flag. Source: StressTester.md above.
- Binaries per ISA: 13-HSW (AVX2/FMA3), 17-SKX (AVX-512), 17-ZN1, 18-CNL (AVX-512 IFMA/VBMI), 19-ZN2 "Kagari", 04-P4P (SSE), etc. Source: https://numberworld.org/y-cruncher/internals/arch-optimizations.html

**Error detection.**
- Large multiplications are checked with **a modular identity over a 64-bit prime (mod 2^61-1, a Mersenne prime, chosen for speed)**. Source: https://numberworld.org/y-cruncher/internals/multiplication.html
- FFT-based multiplication has the "Coefficient is too large" check: a hardware error anywhere except the final carry-out "has an extremely high probability of completely corrupting some coefficients". Source: https://numberworld.org/y-cruncher/faq.html
- In the stress tester a soft error shows "Error detected" with the **logical core number**, which is how overclockers find weak cores. Source: version history above.
- y-cruncher can also detect and sometimes correct minor errors in real computations; BBP produces validation files. Source: https://numberworld.org/y-cruncher/version_history.html
- Community note: when Prime95 Small FFT passes but y-cruncher at large sizes fails, suspect RAM / IMC / Infinity Fabric rather than the cores; watch WHEA events in parallel (HWiNFO). Source: https://forums.anandtech.com/threads/y-cruncher-for-stability-test.2588309/

**Duration.** Community: one loop of the full component test is a good sign; "N63 and VT3 for an hour or more" for a sensitive pass. CoreCycler defaults per core are about 3 min; "quick" configs use 20 s per test. Source: https://github.com/sp00n/corecycler/discussions/71, search summary at https://www.overclock.net/threads/great-new-method-for-determining-stability-of-x86-chips.1799435/ (page itself was not fetchable).

**License.** Closed-source freeware ("license agreement required"); only `PublicLibs`, `DigitViewer`, `DigitViewer2` and Launcher are BSD-3; everything else is "restricted, look and study only". Not reusable. Source: https://github.com/Mysticial/y-cruncher

### 1.3 OCCT (OCBASE)

**Documentation caveat.** The official OCCT docs could not be fetched; the details below come from guides and secondary pages. OCCT is proprietary, so internals are inferred.

**CPU test options.** Data set Small / Medium / Large (small = fits in cache, hottest, finds core instability fastest; large = L1+L2+L3 and RAM, finds IMC/voltage-regulation issues), Mode Normal / Extreme (extreme adds power draw), Load type **Steady** (constant) or **Variable** (load changes quickly, stresses VRM transients), Instruction set auto/SSE/AVX/AVX2/AVX-512, thread count, "Core cycling" (since OCCT 8.0: switch core as fast as every 150 ms). Recommended by the developers for fairly reliable error detection: Large data set, Extreme, Variable, Auto instruction set, Auto threads, 1 hour. Sources: https://rtech.support/guides/how-to-use-occt/ , https://videocardz.com/newz/occt-8-0-0-now-comes-with-core-cycling-feature-for-advanced-cpu-testing , https://mundobytes.com/en/How-to-perform-stability-tests-with-OCCT-on-CPU--GPU--RAM--and-PSU/ , https://www.tomshardware.com/uk/reviews/stress-test-cpu-pc-guide,5461-2.html
- "Steady load" was added to prevent operands switching, for cooling tests. Source: search summary https://c2055.cloudnet.se/en/occt-cpu-test.html
- Also: Linpack mode (version 2021, "physical and virtual" threads, 2048 MB), CPU+RAM mode, Memory test (90-95 % of RAM), Power test (CPU+GPU), error code "4 = calculation error detected". OCCT also reports WHEA, throttling, computation errors. Sources: rtech.support guide above, https://www.onecomputerguy.com/occt-error-detected/ , https://www.occt.pro/ **[unverified]** for the exact WHEA integration.
- Reported behaviour: OCCT finds Curve Optimizer errors faster than Prime95 ("2 h OCCT ~ 24 h Prime95" is anecdotal). Source: https://techenclave.com/t/occt-vs-orthos-vs-prime95/135205
- Recent changelog: v17.1.4 fixed "false positives on recent AMD processors" in the Linpack test. Lesson: floating-point residual thresholds produce false positives on new microarchitectures. Source: https://www.ocbase.com/download
- **License:** proprietary freeware; Personal edition is non-commercial only. Not reusable.

### 1.4 Linpack family (Intel Linpack, LinX, IntelBurnTest, Linpack Xtreme)

- Solves a dense linear system Ax = b by LU factorization. Verification: regenerate A and b, then compute scaled residuals; HPL uses three residuals (r_n = ||Ax-b||_inf / (n eps ||A||_1) etc.) and the result passes if all are below 16. Source: https://arxiv.org/pdf/0806.4907 and search summary on residual formulas.
- Tools show GFLOPS and a "residual" per run; instability shows as residual mismatch between runs or a failure. IntelBurnTest/LinX/OCCT-Linpack use old (2012) Intel binaries; **Linpack Xtreme** (updated) added `/residualcheck` (default on AMD) and fixed false positives and Ryzen crashes. The bootable Linux version is said to be more sensitive than Windows. Sources: https://forums.anandtech.com/threads/new-linpack-stress-test-released.2553490 , https://techpowerup.com/forums/goto/post?id=3902308
- Characteristics: AVX/FMA heavy, very high power and temperature; with large N (e.g. 2 GB+) also memory-bandwidth bound. Single-run result checks only the final answer; errors can be masked if they cancel, but in practice detection is good.
- **License:** Intel MKL Linpack binaries are proprietary. Open building blocks: HPL (BSD-style), LAPACK/OpenBLAS (BSD-3), and in Rust `matrixmultiply` (MIT/Apache-2.0, v0.3.11) and `gemm` (MIT, v0.19.0; x86-v4/AVX-512 feature was removed in 0.19). Sources: https://crates.io/api/v1/crates/matrixmultiply , https://crates.io/api/v1/crates/gemm

**Useful cheap-verification idea (general knowledge, not from a fetched source):** Freivalds' algorithm checks a matrix product C = A*B in O(n^2): pick a random vector r and verify A*(B*r) == C*r. With small-integer inputs the arithmetic is exact in f64, so the check has no tolerance and no false positives. **[unverified as used by stress tools; standard CS result]**

### 1.5 AIDA64 System Stability Test (Finalwire, commercial)

- Selectable stressors: CPU, FPU, Cache, System memory, Local disks, GPU(s); can be toggled while running; shows temperatures, fan speeds, voltages, power, clocks, throttling graph. "FPU only" gives the highest heat/power. "CPU + FPU + Cache" is the whole-CPU test. Source: https://www.aida64.com/user-manual/tools/stability-test
- The official manual does not describe instruction sets or the error-detection mechanism; failures show as a "Hardware failure" warning. Mechanism undocumented; closed source, not reusable.
- Benchmarks it contains (CPU Queen, PhotoWorxx, Zlib, AES, SHA3, FPU Julia/Mandel/SinJulia) are reported as MB/s, MPixel/s, kPixel/s. **[unverified]**

### 1.6 CoreCycler (sp00n, Windows PowerShell)

- Launches **one stress-test worker pinned to one core at a time** (Prime95, y-cruncher, Linpack, or AIDA64) and moves to the next core after a configurable time; error detection comes from the wrapped tool (Prime95 `results.txt`, y-cruncher output, Linpack via CPU usage counters). Rationale: single-core load boosts to the highest frequency; all-core tests do not expose Curve Optimizer (CO) / PBO undervolt instability. Source: https://github.com/sp00n/corecycler
- Defaults/recommendations: Prime95 without AVX and AVX2 and "Huge" FFTs gives least heat and highest boost; one thread per physical core by default (2 optional); AVX2 with 720K or 1344K FFTs is "a popular choice"; SSE + Huge for "low-load" stress (finds crashes during load changes); SSE + All FFTs as final validation; y-cruncher modes 04-P4P and 19-ZN2 for Ryzen; tests "BKT, BBP, SFT, SNT, SVT, FFT, N63, VT3"; **per-core runtime 3 min; 12 h per core for a "12-hour stable" claim (12-core CPU = 144 h)**. Sources: https://github.com/sp00n/corecycler , https://github.com/sp00n/corecycler/discussions/71
- **License: CC BY-NC-SA.** Non-commercial and share-alike, not compatible with GPL-3.0. Ideas only; do not copy code.
- OCCT has the same idea as a built-in option (core cycling).

### 1.7 stress-ng (Colin King, Linux; GPL-2.0-or-later)

- 390+ stressors; 100+ CPU methods selected by `--cpu-method` (`--cpu-method list` prints them). Examples: `fft` (4096-sample FFT), `matrixprod` (128x128 double matrix product; "good mix of memory, cache and floating-point, probably the best CPU method to make a CPU run hot"), `double`, `float`, `int64`, `crc16`, `collatz`, `fibonacci`, `prime`, `sieve`, plus `vecmath`/`vecfp`/`cache`/`memcpy`/`bsearch`/`qsort` stressors. `--cpu-load N` with `--cpu-load-slice` gives a duty-cycled load. Sources: https://github.com/ColinIanKing/stress-ng , https://raw.githubusercontent.com/ColinIanKing/stress-ng/master/stress-ng.1
- `--verify`: "sanity check the computations or memory contents and report with the `fail` tag"; implemented per method by embedding expected results: e.g. collatz expects exactly 1348 steps, sieve expects 10000 primes, gcd checksum constant, sqrt compares to the original numbers; some methods (fft) verify only implicitly. Not available on all tests. Source: https://raw.githubusercontent.com/ColinIanKing/stress-ng/master/stress-cpu.c
- **License: GPL-2.0-or-later** (file header "either version 2 ... or (at your option) any later version") -> compatible with our GPL-3.0-or-later; we can port methods. Windows is not a first-class target.

### 1.8 FIRESTARTER (TU Dresden ZIH, GPL-3.0-or-later)

- "Processor stress test utility" designed to maximize power draw. Generates machine code at runtime for the detected microarchitecture from **instruction groups** such as `REG:4,L1_L:2,L2_L:1` (ratio of register-only FMA work to loads/stores hitting L1/L2/L3/RAM), auto-tunes with NSGA-II (power vs IPC). Payloads: SSE2, AVX, FMA, FMA4, AVX-512, Zen-FMA. Runs on Linux, **Windows** and macOS. Duty-cycle option `-l` percent with `-p` period (default 100 ms). Source: https://github.com/tud-zih-energy/FIRESTARTER , https://arxiv.org/pdf/2108.01470
- **Operand data matters for power**: operands that become +/-inf, NaN or 0 make FMAs trivial and cut power; FIRESTARTER 2 fixed a bug where register values accumulated to inf. Design consequence for our kernels: keep operands bounded and non-trivial (e.g. alternating add/sub, normalised values). Source: arXiv paper above.
- **Error detection** (`--error-detection`, requires >= 2 threads, not combinable with partial load or optimization): each thread hashes the contents of its vector registers (CRC32 instruction, SSE4.2) and compares it, for the same iteration counter, against its left and right neighbour thread; communication is a 16-byte cmpxchg16b slot per pair; a mismatch sets an `Error` flag. Sources: https://raw.githubusercontent.com/tud-zih-energy/FIRESTARTER/master/src/firestarter/Firestarter.cpp (header comment on CRC32), https://github.com/tud-zih-energy/FIRESTARTER/blob/master/include/firestarter/ErrorDetectionStruct.hpp (via `gh api`)
- The paper also adds a feature to flush register contents to a file so users can verify SIMD correctness on overclocked parts.
- **This is the best open design to copy for the "max power + verification" mode**: no precomputed golden value is needed because threads run identical deterministic streams and cross-check each other. Weakness: if all threads are wrong in the same way (e.g. a shared-resource fault) it is not noticed; and with only one thread nothing is checked.

### 1.9 OpenDCDiag / Sandstone (Intel, Apache-2.0)

- Open-source CPU defect-detection framework from the Intel Data Center Diagnostic Tool lineage. Tests: `eigen_gemm`, `eigen_svd`, `eigen_sparse`, `zstd`, `zlib`, `openssl_sha`, `ifs` (Intel in-field scan, needs kernel support), `mce_check`, `smi_count`. Also a Windows sysdeps layer. Sources: https://github.com/opendcdiag/opendcdiag , https://www.intel.com/content/www/us/en/developer/articles/news/open-data-center-diagnostic-project.html , repo tree via `gh api repos/opendcdiag/opendcdiag/git/trees/main?recursive=1`
- Verification pattern: during `test_init` compute a **golden value on the main thread**, then each hardware thread recomputes the same value in `TEST_LOOP` and compares (`memcmp_or_fail`); failures re-run to decide whether the fault is thread-specific or systemic; the RNG seed is printed so failures can be replayed (`-s`). Source: https://github.com/opendcdiag/opendcdiag/blob/main/docs/writing_tests.md
- Intel's SDE paper: causes are voltage, frequency and temperature margins; strategies: in-field testing, content design, **cross-core comparison**, repeated iterations with varied parameters; content that stresses memory, precision-sensitive operations and state kept across iterations finds more. Sources: https://cdrdv2-public.intel.com/788204/788204%20Data%20Center%20Silent%20Data%20Errors%20Tech%20Paper%20Rev1-1.pdf
- Real-world SDC: Google "Cores that don't count" (mercurial cores, a few per several thousand machines, silent miscomputation, e.g. a core that corrupted encryption so only it could decrypt), Meta/SIGARCH article (about 1 fault per thousand devices; one report notes the wrong chip is blamed up to 60 % of the time). Sources: https://research.google/pubs/cores-that-dont-count/ , https://www.sigarch.org/silent-data-corruption-at-scale/
- **License Apache-2.0**: compatible with GPL-3.0; code could be adapted (keep NOTICE/attribution).

### 1.10 stressapptest (Google, Apache-2.0)

- Userspace memory-interface test: allocates ~85 % of RAM, threads copy randomly chosen blocks, **CRC computed while copying** to verify; also disk I/O threads; data patterns chosen to be electrically stressful; "-W" more CPU stress mode. Source: https://android.googlesource.com/platform/external/stressapptest/+/refs/heads/android12-d1-release/README.md and search summary
- Good model for a user-mode RAM+IMC test that checks data in flight. Apache-2.0.

### 1.11 RAM testers (just enough to decide on a RAM sub-mode)

- **MemTest86 / Memtest86+** (boot-time, the only way to test nearly all physical RAM): address walking, own address, moving inversions (1/0, 8-bit, 32-bit, random), block move, modulo-20 (not fooled by caches), random sequences (64-bit, 128-bit SIMD), bit fade, hammer test (row-hammer disturbance). Source: https://memtest86.com/tech_individual-test-descr.html ; Memtest86+ is GPL-2.0, memtester GPL-2.0-only (not compatible with a GPL-3.0-or-later work: study only). Sources: https://metadata.ftp-master.debian.org/changelogs/main/m/memtester/stable_copyright , https://www.memtest.org/
- **HCI MemTest, TestMem5 (+ configs by 1usmus / anta777), Karhu RAM Test**: Windows user-mode testers; they allocate most free RAM (HCI: leave ~2 GB for Windows), write/verify patterns with many threads; community favourite for DDR5 overclock validation; user-mode so cannot cover all physical memory. HCI and Karhu are closed, TM5 is freeware. Source: https://www.hardwareluxx.de/community/threads/speichertestprogramme-2026.1195136/ (search summary), https://jisaku.com/glossary/memory-overclock-bios-stability-test-tm5-1usmus-anta777
- Conclusion for us: a **basic** RAM/IMC sub-mode (multi-threaded march/random-pattern + CRC-while-copying over a large heap) is feasible and valuable because "Large FFT / y-cruncher" style tests already catch most IMC instability, but we should say clearly it is not a replacement for MemTest86/TM5/Karhu. Big-page access requires `SeLockMemoryPrivilege` (not granted to standard users by default); without it use normal 4 KB pages (TLB-miss heavy, fine for stress). Source: https://learn.microsoft.com/en-us/windows/win32/memory/large-page-support

### 1.12 Vendor tools

- Intel Processor Diagnostic Tool: brand string, frequency, cache (L1/L2/L3), MMX/SSE/AVX/AES/PCLMULQDQ tests, a 120-minute "burn-in" stress; Intel XTU has a built-in stress test (full load, custom length). Source: https://ark.intel.com/content/www/us/en/support/articles/000005567.html , https://www.notebookcheck.net/Intel-Extreme-Tuning-Utility-XTU-Undervolting-Guide.272120.0.html . Proprietary.
- AMD: no current official stress tool; AMD OverDrive (legacy) had a stability tab; AMD points users to third-party tools for Curve Optimizer. Source: search summary of https://github.com/sp00n/corecycler/ and kitguru. **[unverified]** (AMD CO FAQ PDF timed out).
- Intel Raptor Lake "Vmin shift" instability (2024-2025): degradation raises the minimum stable voltage; instability appears under sustained heavy workloads such as shader compilation/benchmarks, and gets worse with temperature. Intel recommends default settings plus microcode 0x12B; Intel had no detection tool. Shows the value of a stock-settings test that **detects wrong results**, not just crashes. Sources: https://www.hwcooling.net/en/intel-raptor-lake-cpus-need-another-fix-to-prevent-damage/ , https://www.tomshardware.com/pc-components/cpus/intel-doesnt-have-a-tool-to-detect-if-a-chip-is-affected-by-crashing-errors-yet-chipmaker-recommends-intel-default-settings-even-after-0x12b-patch-is-applied

---

## 2. Techniques

### 2.1 Verification methods, ranked for us

| Method | Used by | Pros | Cons |
|---|---|---|---|
| **Cross-thread / cross-core golden compare** (compute once, every thread recomputes and compares; or ring hash compare) | OpenDCDiag, FIRESTARTER | simple, no tolerance, finds core-specific faults, works for any deterministic kernel | golden value could be corrupted if computed on the weak core: compute it on 2-3 cores at the start and require agreement; identical faults on all threads go unseen |
| **Exact integer arithmetic with an algebraic invariant** (NTT forward/inverse round-trip, modular identity mod 2^61-1, Freivalds for GEMM) | y-cruncher, 7-Zip (CRC), general | zero false positives, fast verify | needs careful design |
| **FFT round-off + SUMINP/SUMOUT** | Prime95, Mlucas | sensitive to FP unit faults | tolerance thresholds -> rare false positives (see OCCT/Linpack AMD false positive); use as secondary |
| **Residual check** (Linpack) | Linpack family | standard | threshold dependent, false positives on new CPUs |
| **Hash of register state per iteration** | FIRESTARTER | works for power-virus code with no meaningful result | needs >= 2 threads |
| **Round trip (compress/decompress, encrypt/decrypt) + known-answer test** | OpenDCDiag, 7-Zip, Geekbench (SHA1 verify) | covers fixed-function units (AES-NI, SHA-NI, CLMUL), branchy code | |
| **CRC while copying / pattern read-back** | stressapptest, memtest | memory path | |
| **Redundant execution on two cores** (same input -> same output) | SDE literature | detects non-deterministic faults | halves throughput |

Seed/replay: print the RNG seed and the failing kernel+core so a failure is reproducible (OpenDCDiag `-s`). Source: writing_tests.md above.

### 2.2 Load shapes

- **Steady max load** (Prime95, Linpack, AIDA64 FPU, OCCT steady, FIRESTARTER): power/thermal/cooling checks.
- **Variable / transient load**: OCCT "Variable"; stress-ng `--cpu-load` + slice; FIRESTARTER `-l`/`-p`; CoreCycler "SSE + Huge FFT low load" looking for crashes during load changes. Targets VRM response, Vcore droop/overshoot, boost transitions, C-state/P-state changes (a classic PBO/CO failure at idle or light load). Source: https://github.com/sp00n/corecycler/discussions/71 , https://maketecheasier.com/occt-stress-testing-cpus-gpus/
- **Per-core, single-thread** (CoreCycler, OCCT core cycling at 150 ms to minutes).
- **Mixed-ISA phases**: alternate AVX-512/AVX2/SSE kernels and integer kernels so the voltage/frequency controller keeps reacting (y-cruncher's component list does this by rotating tests).

### 2.3 ISA and data-set sizing

- Detect with `is_x86_feature_detected!` (checks CPUID and OS XSAVE support). Rust 1.89 stabilized the AVX-512 target features and more intrinsics, so toolchain 1.90 can compile `#[target_feature(enable = "avx512f")]` kernels without nightly. Source: https://www.phoronix.com/news/Rust-1.89-Released , https://releases.rs/docs/1.89.0/
- Windows API alternative: `IsProcessorFeaturePresent` has `PF_AVX_INSTRUCTIONS_AVAILABLE` (39), `PF_AVX2_...` (40), `PF_AVX512F_...` (41) since Windows 10 2004. Source: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-isprocessorfeaturepresent
- Intel 12th gen onward (Alder/Raptor Lake) ship **without AVX-512** (fused off); Zen 4 has double-pumped 256-bit AVX-512, Zen 5 full 512-bit with no fixed clock offset, and AVX-512 does not add much power on Zen 4/5 in typical software; Intel server/older client parts can have large AVX offsets; Intel's next-gen is reported to bring AVX-512 back. Sources: https://www.pcworld.com/article/545013/intel-alder-lake-to-offer-8-p-core-only-model-and-have-avx512-too.html , https://chipsandcheese.com/p/zen-5s-avx-512-frequency-behavior , https://www.phoronix.com/review/amd-zen5-avx-512-9950x/7
- Data-set sizing relative to caches (derive from `GetLogicalProcessorInformationEx(RelationCache)` per core): *L1-resident* (about 16-24 KB/thread), *L2-resident* (about 50-75 % of L2 per thread, Prime95 29.8 "options based on cache sizes"), *L3-resident* (fraction of per-CCX L3 divided by threads sharing it), *RAM* (>= 4x total L3 and >= 256 MB-1 GB per thread, or a big share of free RAM). PassMark kernels use about 240 KB per core (integer/FP), 1 MB (encryption), 4 MB (compression, primes), 25 MB (sort). Source: https://www.cpubenchmark.net/cpu_test_info.html

### 2.4 Windows scheduling, pinning and topology (no admin needed)

- **Processor groups**: a group is up to 64 logical processors. Before Windows 11, a process is constrained to one group by default (round-robin assignment); to use more, set each thread's group affinity (`SetThreadGroupAffinity`). From Windows 11 / Server 2022, threads span all groups by default but have a "primary group", and non-group-aware affinity APIs act on the primary group. Source: https://learn.microsoft.com/en-us/windows/win32/procthread/processor-groups
- **CPU Sets** (Windows 10+): `GetSystemCpuSetInformation` returns per logical CPU: `Id`, `Group`, `LogicalProcessorIndex`, `CoreIndex` (same for SMT siblings), `LastLevelCacheIndex` (same for CPUs sharing a cache), `NumaNodeIndex`, `EfficiencyClass` (higher = faster and less efficient; use to tell P from E cores), `Parked`. `SetThreadSelectedCpuSets` is a soft affinity respecting power management; a hard affinity mask overrides it. Sources: https://learn.microsoft.com/en-us/windows/win32/procthread/cpu-sets , https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-system_cpu_set_information
- **Hybrid caveats**: on Meteor Lake the E and LP-E cores share an efficiency class, so extra detection is needed there; CPUID leaf 0x1A gives the core type pre-Meteor Lake. Source: https://forums.intel.com/t5/Processors/Detecting-LP-E-Cores-on-Meteor-Lake-in-software/m-p/1585842
- **AMD CCD/CCX**: group logical CPUs by `LastLevelCacheIndex` (one value per L3 domain), then by `NumaNodeIndex`. Per-CCD reporting helps users map errors to the CO/PBO domain. (derived from the struct documentation above)
- **EcoQoS / power throttling**: set `ThreadPowerThrottling` with `ControlMask = EXECUTION_SPEED, StateMask = 0` (HighQoS) so Windows does not move or slow stress threads; otherwise the system may use "more power efficient cores" for threads it classifies as background. Source: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadinformation
- Priority: normal/above-normal is fine without admin; real-time class needs admin. Keep one logical CPU for the UI, optional "leave 1 thread free".
- Windows `GetLogicalProcessorInformationEx(RelationCache)` gives cache sizes; `RelationProcessorCore` gives SMT/efficiency class. (documented in the same MS Learn pages)

### 2.5 WHEA and crash evidence

- Provider name: **Microsoft-Windows-WHEA-Logger**, channel System; query with `EvtQuery` and XPath `*[System/Provider[@Name="Microsoft-Windows-WHEA-Logger"]]` (Microsoft sample). Source: https://learn.microsoft.com/en-us/windows-hardware/drivers/whea/querying-the-system-event-log-for-hardware-error-events
- **Non-admin read access: yes.** The default System-log SDDL is `O:BAG:SYD:(A;;0xf0007;;;SY)(A;;0x7;;;BA)(A;;0x3;;;BO)(A;;0x5;;;SO)(A;;0x1;;;IU)(A;;0x3;;;SU)(A;;0x1;;;S-1-5-3)(A;;0x2;;;S-1-5-33)(A;;0x1;;;S-1-5-32-573)`; the fifth ACE gives Interactive Users read (0x1). So a normal interactive process can read WHEA events (unless a policy changed the SDDL). Source: https://learn.microsoft.com/en-us/troubleshoot/windows-server/group-policy/set-event-log-security-locally-or-via-group-policy . Our .NET service (SYSTEM) could also read them if ever needed.
- Event IDs (WHEA-Logger): **17** corrected hardware error via PCIe AER (warning; GPU/NVMe/slot), **18** fatal hardware error / Machine Check Exception (usually accompanied by bugcheck 0x124 `WHEA_UNCORRECTABLE_ERROR`, so you read it **after the reboot**), **19** corrected machine check (CPU, e.g. "Cache Hierarchy Error", "Bus/Interconnect", "Memory Controller", "TLB"; carries APIC ID of the reporting logical processor, which does not prove that core is the culprit). IDs 1, 20, 46, 47 also appear in the wild (1 is described variously) **[unverified: meanings]**. Sources: https://gaming-st.com/trouble-settings/whea-logger-event-id-17-18-19-guide/ , https://winevt-kb.readthedocs.io/en/stable/sources/eventlog-providers/Provider-Microsoft-Windows-WHEA-Logger.html (provider GUID {c26c4f3c-3f66-4e99-8f8a-39405cfed220}, message DLL whealogr.dll)
- Practical use: snapshot the newest WHEA record time at test start, poll (or `EvtSubscribe`) during the run and once more at the end (records can lag), report count by type and by APIC ID. Also look after a reboot for Kernel-Power 41, BugCheck 1001 and WHEA 18 (these are standard System-log events **[unverified here, standard knowledge]**).
- Prior art: community advice to run HWiNFO's WHEA counter next to y-cruncher because tests can "pass" while WHEA corrected errors pile up. Source: https://forums.anandtech.com/threads/y-cruncher-for-stability-test.2588309/

### 2.6 Surviving crashes (design idea, not prior art)

Overclock failures are usually a BSOD or a hard reset, after which no in-process error log exists. Keep a small **journal** (state: mode, core under test, kernel, elapsed, last heartbeat) written with fsync every second or two. On next start, a missing "clean end" marker plus WHEA 18 / BugCheck / Kernel-Power 41 in the System log => report "system crashed during test X on core N after T s". This is what makes per-core cycling directly actionable (it names the core). OCCT/CoreCycler do not do this. **[original proposal]**

---

## 3. CPU benchmarks: how scores are computed

| Benchmark | Workload | Unit / score | Single vs multi | Run length / warm-up | Notes |
|---|---|---|---|---|---|
| **Geekbench 6** | 8 productivity/dev/ML/image workloads (File compression with LZ4/ZSTD+SHA1 verify, Navigation/Dijkstra, HTML5 browser, PDF render, Photo library MobileNet+SQLite, Clang compile Lua, Text processing in Python, Asset compression ASTC/BC7/DXT5, Object detection, Background blur, Object remover, Horizon detection, Photo filter, HDR, Ray tracer, Structure from Motion) | score calibrated to 2500 = Dell Precision 3460 with Core i7-12700; composite = **weighted arithmetic mean of Integer 65 % + Floating-point 35 %**, each subsection a **geometric mean** of workload scores | single-core and multi-core; multi-core uses a **"shared task" model** (threads split one task) instead of GB5's independent copies per thread; build base ISA SSE2 or AVX2 with runtime-dispatched AES/SHA/AVX-512/AMX paths | **5 s gap between workloads** (2 s in 6.0) to limit thermal carry-over | Sources: https://www.geekbench.com/doc/geekbench6-cpu-workloads.pdf , https://www.geekbench.com/doc/geekbench6-benchmark-internals.pdf (read via `pdftotext`) |
| **Cinebench 2024** | Redshift CPU render of one Cinema 4D scene | points (higher is better), new scale incompatible with R23; "2026" scale again new | single, multi (and SMT in 2026) | minimum-duration setting; R23 defaults to 10 min to reach sustained thermals (CLI `g_CinebenchMinimumTestDuration`); 2024 single-core run is much longer (about 14 min on an i7-13700K reported); runs vary with thermals | Instruction mix: mostly scalar FP and AVX 128/256-bit, almost no AVX-512, code footprint spills to L2, about 20 GB/s DRAM on 16 cores, IPC > 2, deterministic branches. Sources: https://www.maxon.net/en/tech-info-cinebench , https://chipsandcheese.com/p/cinebench-2024-reviewing-the-benchmark , https://www.cgdirector.com/cinebench-2024-scores , https://techspot.com/downloads/7579-cinebench-r23.html |
| **CPU-Z bench** | tiny scalar FP32-heavy SSE loop, data < 32 KB (all in L1D), about 99 % L1 hit rate, little memory or branch pressure | points; MT score divided by ST score gives the "multi-thread ratio"; reference CPU in the older versions | single and multi (same workload per thread) | seconds | Criticised as unrepresentative ("useless to designers and end users"), but cheap and fast. Sources: https://chipsandcheese.com/p/cpu-zs-inadequate-benchmark , https://valid.cpuid.com/bench/1 |
| **PassMark CPU Mark** | 8 tests: integer math (random 32/64-bit add/sub/mul/div), FP math (30 % add, 30 % sub, 30 % mul, 10 % div, mix of f32/f64), prime numbers (Sieve of Atkin to 32 M), sorting (quicksort of strings), encryption (AES, SHA256, ECDSA), compression (Crypto++ Gzip/DEFLATE), physics (Bullet), extended instructions (SSE/FMA/AVX/AVX-512/NEON) | operations/s, KB/s; **CPU Mark = weighted average, weights set so each test has equal influence on a hypothetical average CPU**; separate single-thread score | multi-threaded by default (all logical CPUs) plus single-thread test | per-test seconds; memory per thread from 240 KB to 25 MB | Sources: https://www.cpubenchmark.net/cpu_test_info.html , https://www.passmark.com/support/performancetest_faq/understanding-results.php |
| **7-Zip `b`** | LZMA compression and decompression of synthetic data (text/executable-like), optional `-mm=*` for hash/crypto codecs | **MIPS rating** normalised to an Intel Core 2 (hash/crypto: AMD K8); columns Dict (2^N), Usage %, R/U (rating at 100 % usage) | `-mmt{N}` threads; one thread rating = R/U | N iterations (`7z b 30` is used as a RAM error check), dictionary 2^21 and up | Compression depends on memory latency, decompression on integer speed; it verifies decompressed data so it doubles as an error check. LZMA SDK is public domain, 7-Zip core is LGPL. Source: https://7-zip.opensource.jp/chm/cmdline/commands/bench.htm |

Key design lessons:
1. **Fixed work, not fixed time**, with a baseline normalisation (Geekbench, 7-Zip) gives comparable, unit-bearing numbers; report **per-kernel rates** (ops/s, MB/s, MIPS-like) plus a composite using the **geometric mean** so no single kernel dominates.
2. **Multi-core model choice matters**: independent copies per thread (GB5, CPU-Z, PassMark: scales almost linearly, tests throughput) vs shared single task (GB6: reflects scaling limits). For a hardware monitor, independent copies are simpler and much less noisy; report "scaling = MT / (ST x threads)" as a secondary number.
3. **Warm-up and repeatability**: discard a warm-up pass; use several repetitions and take the median or best; pin threads; disable EcoQoS; record clock, temperature and power during the run so the user can see boost/throttle effects. Provide a **short "burst" score** and an optional **sustained** run (Cinebench's 10-minute rule) to show thermal throttling.
4. **Self-verifying kernels**: Geekbench file-compression verifies with SHA1, 7-Zip checks the decompressed data. A benchmark result computed with wrong answers must be discarded and flagged.
5. **Units**: Geekbench dimensionless score, Cinebench points, 7-Zip MIPS, PassMark ops/s and KB/s. The "Mops/s / Gop/s" style in your brief is used by our own kernels (we can define "Gop/s" as retired application-level operations, not instructions).

---

## 4. Licenses summary (what we may reuse)

| Project | License | Reuse in our GPL-3.0-or-later app |
|---|---|---|
| Prime95 / gwnum | custom EULA (source available, prize clause) | No code. Algorithms and ideas only. |
| y-cruncher | proprietary freeware; only PublicLibs/DigitViewer/Launcher BSD-3 | No code. |
| OCCT | proprietary freeware (Personal = non-commercial) | No. |
| AIDA64, Cinebench, Geekbench, CPU-Z, PassMark | commercial/proprietary | No. Study published methodology only. |
| Intel MKL Linpack, LinX, IntelBurnTest, Linpack Xtreme | proprietary | No. HPL, LAPACK, OpenBLAS are BSD-style if we ever need reference code. |
| CoreCycler | CC BY-NC-SA | Not GPL compatible (NC + SA). Ideas only. |
| stress-ng | GPL-2.0-or-later | Yes: can port methods. |
| FIRESTARTER | GPL-3.0-or-later (source headers) | Yes: can adapt the ring error-detection and instruction-group design. Keep copyright headers. |
| OpenDCDiag | Apache-2.0 | Yes (one-way compatible with GPLv3); keep NOTICE. |
| stressapptest | Apache-2.0 | Yes. |
| Mlucas | GPL-2.0-or-later (most files) | Yes for files marked "or later". |
| memtester | GPL-2.0-only | No (incompatible with GPLv3-only combination); study. |
| Memtest86+ | GPL-2.0 (check headers) | Study only unless "or later". |
| MemTest86 (PassMark), TestMem5, HCI, Karhu | proprietary | No. |
| 7-Zip | LGPL (+unRAR restriction); LZMA SDK public domain | LZMA SDK usable; we would rather use a Rust crate. |
| RustFFT | MIT OR Apache-2.0 (v6.4.1) | Yes (a dependency, or write our own for stress). |
| matrixmultiply | MIT/Apache-2.0 (v0.3.11) | Yes. |
| gemm | MIT (v0.19.0, no AVX-512 flag) | Yes. |
- Sources for the Rust crates: https://crates.io/api/v1/crates/rustfft , https://crates.io/api/v1/crates/matrixmultiply , https://crates.io/api/v1/crates/gemm. The project CLAUDE.md already forbids copying proprietary text/headers; the same discipline applies here.
- A dependency is not even needed for a stress engine: hand-written kernels with `core::arch` intrinsics are small and give exact control over instruction mix and data values.

---

## 5. Answers to the specific questions

- **Should the normal test detect errors?** Yes, with the cheap checks (per-iteration hash, FFT SUMINP/SUMOUT, exact integer checks). Reasons: (a) silent corruption exists on stock hardware and degrades over time (Google, Meta, Intel SDE paper, Raptor Lake Vmin shift); (b) the cost is negligible; (c) users interpret "PASS" as "the CPU is healthy". The normal test must **not** stop at the first error by default but keep running, count errors per core/kernel, and mark the result FAIL (an error is never "acceptable"). What changes for the OC test: more kernels, per-core cycling, transient load, longer duration, WHEA/crash-journal evidence, and a "stop on first error" option.
- **Is the OC test required to detect computation errors?** Yes, it is the entire point; additionally report hangs/crashes (journal) and WHEA corrected events as "marginal".
- **Is per-core pinning workable without admin?** Yes (SetThreadGroupAffinity / CPU sets).
- **Does AVX-512 matter?** On Zen 4/5 and Intel server/HEDT/11th gen it is a separate stability domain; on Intel 12th+ consumer parts it does not exist. Always detect at runtime, offer "AVX-512 (Zen 4/5, Intel with support)" only if present.
- **Tests that catch memory-controller issues?** Large in-place FFT, y-cruncher large-size/N63 at large memory, memtest-style patterns, CRC-while-copy.

---

## 6. Proposed modes for our app

Naming: user-facing "profile" (Normal / Overclocking) chooses a **playlist of kernels** with different durations and strictness. Each kernel is described once. All kernels verify their own output, run on N pinned worker threads, and report `{kernel, core(s), iteration, expected_hash, got_hash}` on mismatch. All use `std::arch` intrinsics with runtime dispatch (AVX-512 -> AVX2+FMA -> SSE2). All keep operands bounded and non-trivial (FIRESTARTER finding).

### 6.1 Kernel catalogue

| # | Name | Workload | ISA | Data set | Targets | Error detection | Normal duration (per kernel) | OC duration | Feasible in Rust, no admin? |
|---|---|---|---|---|---|---|---|---|---|
| K1 | **Power Virus (FMA burn)** | FIRESTARTER-style generated loop: register-only FMA chain with a ratio of L1/L2 loads and stores, bounded operands | AVX-512 / AVX2+FMA / SSE2 | L1/L2 | max power, VRM, cooling, boost-clock sustain | ring hash compare: each thread CRC32s its vector registers per iteration number and compares with neighbour thread (needs >= 2 threads); for 1 thread compare against a golden hash computed at start | 10-30 min (thermal) | 5-10 min as one phase | Yes (hand-written asm-like kernel via intrinsics; FIRESTARTER GPL-3 code can be studied/adapted) |
| K2 | **Small FFT (in-cache)** | our own complex-double FFT (radix-4/split-radix) forward + inverse on L2-sized arrays, in place | AVX2+FMA / AVX-512 (+ SSE2 fallback) | per-thread arrays sized to about 50-75 % of L2, plus a "smallest" variant (L1) | FP units, L1/L2, highest FPU stress | (a) SUMINP==SUMOUT style DC check, (b) round-trip error < threshold, (c) **bitwise-identical result across threads/iterations** (deterministic data, so any bit flip is visible without tolerance) | 10-20 min | 30 min-hours; also the main per-core cycling kernel | Yes |
| K3 | **Large FFT (RAM-bound)** | same FFT with arrays several times bigger than total L3 (e.g. 256 MB-1 GB per thread) strided/in-place | AVX2 / AVX-512 | RAM | memory controller, DRAM, Infinity Fabric / ring, uncore | same as K2 | 10 min | 30-60 min | Yes (large heap via VirtualAlloc; pages 4 KB unless `SeLockMemoryPrivilege`) |
| K4 | **Blend** | rotate K2 sizes from L1 to RAM, non-in-place (Prime95 Blend idea) | as K2 | L1->RAM | everything incl. cache/memory transitions | as K2 | 15 min | 1 h+ | Yes (composition of K2/K3) |
| K5 | **Integer NTT (N63-like)** | 64-bit modular NTT (prime near 2^63), forward/inverse, modular multiplication via `mulx`/`mul` | scalar x86-64 (BMI2/ADX) and optional AVX-512 IFMA variant | L2 / L3 / RAM variants (SNT = in-cache, N63 = large) | scalar integer multiplier/ALU, caches, different voltage-droop signature than FP | **exact**: inverse transform must return the original input bit-for-bit, plus modular checksum (mod 2^61-1) of the product -> **no tolerance, zero false positives** | 10 min | 30-60 min; considered the most sensitive y-cruncher test | Yes |
| K6 | **Vector-integer/FP mix (VT3-like)** | multiplication of big numbers with a mixed integer+FP vector transform | AVX2 / AVX-512 | L2..RAM | AVX mixed domain | modular identity check + round trip | 10 min | 30-60 min | Feasible but largest design effort; start with K2+K5 and add later |
| K7 | **DGEMM / Linpack-like** | blocked GEMM (`matrixmultiply`-style or own micro-kernel), optional LU solve | AVX2+FMA / AVX-512 | N chosen to fit L2/L3 or RAM | highest sustained FP throughput, memory bandwidth at large N | **Freivalds check** (A(Br) == C r) on small-integer matrices (exact in f64) or cross-thread compare of C hash; classic Ax-b residual as an optional secondary | 10 min | 20-30 min | Yes |
| K8 | **Crypto/Hash/Compress mix** | AES-NI + SHA-NI + CRC32C + CLMUL chains, zstd/deflate round trips, xxHash, sorting/branchy code, bignum | AES-NI, SHA-NI, PCLMULQDQ, scalar | L1..L3 | fixed-function units, branch predictor, front-end, scalar integer | known-answer vectors + round trip (compress/decompress equality) + cross-thread hash; uses OpenDCDiag/7-Zip ideas | 10 min | 20-30 min | Yes (RustCrypto crates or own intrinsics) |
| K9 | **Cache/core-to-core ping-pong** | threads pass 64-byte checksummed lines between pinned cores/CCDs (producer/consumer rings) | scalar + atomics | L1/L2/L3 coherency | cache coherency, uncore, CCD interconnect | per-line checksum, sequence numbers | 5 min | 10-20 min | Yes |
| K10 | **RAM pattern (basic)** | multi-threaded moving inversions, modulo-20, random pattern, address-in-address, CRC-while-copy (stressapptest style) over 60-80 % of free RAM, SIMD streaming stores | AVX2 / SSE2 | large heap | DRAM, IMC, memory training margin | read-back compare, CRC | 5-10 min | 30-60 min | Partly: user-mode only; cannot cover all physical RAM; not a replacement for MemTest86/TM5/Karhu (say so in UI) |

### 6.2 Load-shape modifiers (apply to any kernel)

| Modifier | Behaviour | Targets | Feasible |
|---|---|---|---|
| **Steady** | 100 % duty | thermals/power | trivial |
| **Variable / transient** | duty cycle square wave (e.g. 10-500 ms busy, same idle) plus random bursts, and kernel hopping (K1 -> K5 -> idle) with jittered period | VRM droop/overshoot, boost and C-state transitions; light-load/idle crashes typical of Curve Optimizer | Yes; use high-resolution timers; threads sleep with `WaitForSingleObject`/spin hybrid |
| **Light-load single-thread** | one or two threads, SSE2 or scalar, with periodic idle | the max-boost, low-load case (CoreCycler "SSE + huge FFT") | Yes |
| **Core cycling** | pin the kernel to one physical core for X s (20 s-10 min, option 150 ms-style fast cycling), then next; separate passes for P and E cores; one thread per core or both SMT threads; skip parked cores | per-core instability (CO, PBO, undervolt) | Yes: `SetThreadGroupAffinity`, topology via `GetSystemCpuSetInformation`; report per-core pass/fail table; AMD: group by `LastLevelCacheIndex` for CCD |
| **All-core / by cluster** | all logical CPUs, or by CCD/E-core cluster | total power and cooling, shared-resource limits | Yes |

### 6.3 Suggested profiles

**A. "Normal stability / thermal check" (default).**
Goal: verify cooling, power limits, and that the CPU computes correctly at stock settings.
Playlist (all-core, steady, verified): K1 (power virus) 10 min -> K2 small FFT 5 min -> K7 DGEMM 5 min -> K8 mixed 5 min -> optionally K3 3 min. Total about 30 min (user can choose 10 min / 30 min / 1 h / custom; scale each phase).
Verification: always on; errors counted per core and per kernel; verdict FAIL on any error; WHEA events during the run shown as warnings (corrected) or errors (fatal, post-reboot). Also display live: clock, temp, power, throttling flags from the existing sensors, with a "thermal throttle / clock drop" annotation. Resource note: honor the perf budget by running the engine in a separate process (like `oma-overlay.exe`) or a thread pool of the app; the nucleus at rest stays untouched.

**B. "Overclocking / undervolt stability".**
Goal: find the weakest core/kernel/voltage state.
Playlist, repeated in loops: (1) all-core K2 + K5 + K7 + K3 + K10 (10-15 min each) as the base "everything" pass; (2) **per-core cycling** with K2 AVX2 (light, highest boost), K5 (scalar integer), and the light-load variable modifier, 3-10 min per core by default, looping; (3) variable/transient load pass; (4) K9 ping-pong and K4 blend for the uncore/IMC; (5) AVX-512 phases if supported. Default 1-2 hours "quick" and an "overnight" preset (8-12 h; Prime95 advice 6-24 h; CoreCycler 12 h per core for a very strict claim).
Strictness: stop-on-first-error toggle (default off in cycling so all cores get reported, on in the all-core pass), journal + crash recovery ("system crashed during core 5, K5"), WHEA corrected errors treated as "marginal: fix before trusting", per-core and per-kernel error report, and a "weakest core" summary suitable for tuning CO.
Show clocks per core at the time of error.

**C. "Quick check" (2-5 min):** K2 + K5 + K8 verified, no cycling. For the app's "is my CPU OK" button.

**D. Benchmark (separate feature, reuse kernels).**
Single-core and multi-core scores from a fixed-work set: integer (K5 NTT, K8 compress/hash, a sort, a prime sieve, a branchy interpreter), FP (K2 FFT, K7 GEMM, an n-body/ray-march scalar+SIMD kernel). Per-kernel rate in Mops/s or MB/s, composite = geometric mean vs a fixed baseline score (e.g. define 1000 = our reference CPU measured once), single-core pinned to the best core (P-core with highest `EfficiencyClass`) and multi-core as independent copies per logical CPU (scaling factor reported). One warm-up pass discarded, 3 repetitions and the median, 2-5 s gap between kernels (Geekbench uses 5 s), optional sustained mode (5-10 min, show throttling curve from our sensors). Every kernel self-verifies; a failed verification invalidates the run and is flagged. No admin needed.

### 6.4 Feasibility summary (Rust, no admin)

- **Feasible, recommended first release:** K1, K2, K3, K4, K5, K7, K8, steady/variable/light-load modifiers, core cycling, WHEA polling, crash journal, benchmark with K2/K5/K7/K8 and extra scalar kernels.
- **Feasible, second release:** K9 (ping-pong), K10 (basic RAM), K6 (VT3-like), AVX-512 IFMA integer variant, per-CCD / per-E-cluster reporting refinements.
- **Not feasible without admin/driver:** reading MCA banks/MSRs directly (could be added later through our PawnIO path, which is already shipped as an optional "advanced sensors" component; out of scope here), testing all physical RAM, guaranteed large pages, real-time priority, controlling voltage/frequency.
- **Risks to design for:** (1) false positives from floating-point tolerance: prefer bitwise determinism (same input, same instruction sequence, compare results exactly across threads and with a golden hash); (2) golden value computed on a weak core: compute on three cores and require agreement; (3) Windows moving threads: hard affinity + EcoQoS off; (4) AVX-512 downclocking on some Intel parts; (5) the UI process and other apps competing: leave one logical CPU free by default; (6) thermal runaway with no cooling: stop on temperature limit (we already read sensors) and warn before starting; (7) antivirus/anti-cheat heuristics on a tight-loop process: use a separate, signed helper like the overlay, and keep the engine free of self-modifying code (FIRESTARTER JIT-generates machine code; avoid that on Windows, use precompiled intrinsics kernels instead); (8) AMD false positives in tolerance-based checks (OCCT Linpack 17.1.4 example).

---

## 7. Source list (by topic)

Prime95/GIMPS: https://www.mersenne.org/download/readme.txt , https://www.mersenne.org/download/stress.txt , https://www.mersenne.org/various/math.php , https://www.mersenne.org/download/whatsnew_298b3.txt , https://www.mersenne.org/download/whatsnew_293.txt , https://www.mersenne.org/download/whatsnew_303b6.txt , https://www.playtool.com/pages/prime95/prime95.html , https://en.wikipedia.org/wiki/Prime95 , https://rieselprime.de/ziki/Prime95 , https://mlucas.sourceforge.io
y-cruncher: https://numberworld.org/y-cruncher/ , https://numberworld.org/y-cruncher/faq.html , https://www.numberworld.org/y-cruncher/version_history.html , https://www.numberworld.org/y-cruncher/internals/multiplication.html , https://numberworld.org/y-cruncher/internals/arch-optimizations.html , https://github.com/Mysticial/y-cruncher-GUI/blob/master/ParameterSpecs/StressTester.md , https://github.com/Mysticial/y-cruncher , https://forums.anandtech.com/threads/y-cruncher-for-stability-test.2588309/
OCCT: https://www.ocbase.com/download , https://rtech.support/guides/how-to-use-occt/ , https://videocardz.com/newz/occt-8-0-0-now-comes-with-core-cycling-feature-for-advanced-cpu-testing , https://mundobytes.com/en/How-to-perform-stability-tests-with-OCCT-on-CPU--GPU--RAM--and-PSU/ , https://techenclave.com/t/occt-vs-orthos-vs-prime95/135205
CoreCycler: https://github.com/sp00n/corecycler , https://github.com/sp00n/corecycler/discussions/71
Linpack: https://arxiv.org/pdf/0806.4907 , https://forums.anandtech.com/threads/new-linpack-stress-test-released.2553490
AIDA64: https://www.aida64.com/user-manual/tools/stability-test
stress-ng: https://github.com/ColinIanKing/stress-ng , https://raw.githubusercontent.com/ColinIanKing/stress-ng/master/stress-ng.1 , https://raw.githubusercontent.com/ColinIanKing/stress-ng/master/stress-cpu.c
FIRESTARTER: https://github.com/tud-zih-energy/FIRESTARTER , https://arxiv.org/pdf/2108.01470 , https://github.com/tud-zih-energy/FIRESTARTER/blob/master/include/firestarter/ErrorDetectionStruct.hpp
OpenDCDiag / SDC: https://github.com/opendcdiag/opendcdiag , https://github.com/opendcdiag/opendcdiag/blob/main/docs/writing_tests.md , https://cdrdv2-public.intel.com/788204/788204%20Data%20Center%20Silent%20Data%20Errors%20Tech%20Paper%20Rev1-1.pdf , https://www.sigarch.org/silent-data-corruption-at-scale/ , https://research.google/pubs/cores-that-dont-count/
RAM testers: https://memtest86.com/tech_individual-test-descr.html , https://android.googlesource.com/platform/external/stressapptest/+/refs/heads/android12-d1-release/README.md , https://www.memtest.org/
Windows: https://learn.microsoft.com/en-us/windows/win32/procthread/processor-groups , https://learn.microsoft.com/en-us/windows/win32/procthread/cpu-sets , https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-system_cpu_set_information , https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadinformation , https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-isprocessorfeaturepresent , https://learn.microsoft.com/en-us/windows/win32/memory/large-page-support , https://learn.microsoft.com/en-us/windows-hardware/drivers/whea/querying-the-system-event-log-for-hardware-error-events , https://learn.microsoft.com/en-us/troubleshoot/windows-server/group-policy/set-event-log-security-locally-or-via-group-policy , https://winevt-kb.readthedocs.io/en/stable/sources/eventlog-providers/Provider-Microsoft-Windows-WHEA-Logger.html , https://gaming-st.com/trouble-settings/whea-logger-event-id-17-18-19-guide/
Benchmarks: https://www.geekbench.com/doc/geekbench6-cpu-workloads.pdf , https://www.geekbench.com/doc/geekbench6-benchmark-internals.pdf , https://www.maxon.net/en/tech-info-cinebench , https://chipsandcheese.com/p/cinebench-2024-reviewing-the-benchmark , https://chipsandcheese.com/p/cpu-zs-inadequate-benchmark , https://www.cpubenchmark.net/cpu_test_info.html , https://7-zip.opensource.jp/chm/cmdline/commands/bench.htm
Rust/ISA: https://www.phoronix.com/news/Rust-1.89-Released , https://crates.io/api/v1/crates/rustfft , https://crates.io/api/v1/crates/matrixmultiply , https://crates.io/api/v1/crates/gemm , https://chipsandcheese.com/p/zen-5s-avx-512-frequency-behavior

## 8. Gaps / things to verify before writing the spec

- OCCT internals (kernel list, exact error detection, WHEA integration) are undocumented publicly; treat our design as independent.
- Prime95 `stress.txt` FFT size ranges per option were not retrieved; not required.
- WHEA event IDs 1, 20, 46, 47 semantics; confirm by generating events on a test VM or from Microsoft's WHEA event reference.
- AMD's official Curve Optimizer testing advice (the FAQ PDF timed out).
- Memtest86+ exact license variant (GPL-2.0-only vs -or-later), gem license details for any crate we actually depend on.
- Whether Windows 10 22H2 still treats consumer hybrid CPUs well enough that pinned threads avoid scheduler interference (we believe pinning removes the issue; test on the user's hardware).
