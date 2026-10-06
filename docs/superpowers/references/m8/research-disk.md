# Disk benchmark and disk stress test: research report

Target: OpenMonitor Advanced (Rust/Tauri app + .NET 10 service, GPL-3.0-or-later, Windows 10/11, no admin).
Date of research: 2026-10-06. "Verified from source" means I read the actual source file or docs page; "web search" means a search snippet only; "unverified" is flagged explicitly.

Contents
1. Executive summary
2. How the existing tools work (CrystalDiskMark, DiskSpd, AS SSD, ATTO, Anvil, HD Tune, PassMark, fio, OCCT)
3. Windows I/O mechanics that matter for us (unbuffered I/O, alignment, write-through, preallocation, IOCP)
4. Sustained, thermal and mixed-load stress tests
5. Data-integrity verification (h2testw, f3, ValiDrive, fio verify) and a cheap design
6. Risks and safety (wear, free space, cleanup, HDD standby, network/removable/BitLocker, antivirus)
7. Scoring and gauge scales
8. Licenses
9. Proposed modes for our app (catalog)
10. Source list

---

## 1. Executive summary

- CrystalDiskMark (CDM) is a thin UI over `diskspd.exe` (MIT). Source read directly: every step is one `diskspd` run with `-b<size>K -o<Q> -t<T> -W0 -S -w0|-w100|-wN [-r] -Z[<size>K] -d<measure s> -L` on one test file, 1 discarded warm-up run plus N measured runs, best result kept. Default profile is SEQ1M Q8T1, SEQ1M Q1T1, RND4K Q32T1, RND4K Q1T1; NVMe profile is SEQ1M Q8T1, SEQ128K Q32T1, RND4K Q32T16, RND4K Q1T1.
- CDM and DiskSpd use `FILE_FLAG_NO_BUFFERING` (`-S` = `-Su`). Write-through (`-Sh` = `-Suw`) is NOT used by CDM. This is a deliberate choice: write-through (FUA) bypasses the SSD's own DRAM/volatile cache and lowers numbers.
- Time-boxed benchmarks write a lot more than the file size: CDM's write steps run for a fixed 5 s, so a Gen4 NVMe writes ~15-70 GB per step; forum reports of 225 GiB per full run are plausible. Our app should cap by bytes as well as by time.
- The test file must be written once in full before read tests. `SetEndOfFile` alone only moves the end-of-file; reads beyond the valid data length are returned as zeros by NTFS without disk access, which would inflate read results. `SetFileValidData` needs `SE_MANAGE_VOLUME_NAME` (admin), so a real write fill is required. CDM does exactly that (1 MiB buffer written `size/1MiB` times).
- Unbuffered I/O requirements (Microsoft docs): offsets, sizes and buffer addresses must be multiples of the volume sector size; align to the physical sector (use `IOCTL_STORAGE_QUERY_PROPERTY`, `BytesPerPhysicalSector`). `VirtualAlloc` (page aligned, 4096) is enough.
- Data pattern matters: random data is the honest default; 0x00 fill is a separate "peak/compressible" option because some controllers compress or dedupe (SandForce era; "ATTO 256 MB uses extremely compressible data" per reviewers).
- Integrity checking cheaply: no tool stores the data; all regenerate it. h2testw/f3 write a per-512-byte-sector block whose first 8 bytes are its own offset and the rest a seeded LCG stream (f3: `x = x * 4294967311 + 17`). On read they regenerate and classify blocks as OK / "slightly changed" (up to 7 bit errors) / "overwritten" (header offset differs: addressing error) / "corrupted". Fio does the same with a per-block header (magic, offset, seed, checksum). We can do the same with a 64-byte header per 4 KiB block plus an xxHash/CRC.
- Drive-side effects to design around: SLC cache (sustained 1 MiB sequential writes show a cliff after tens to hundreds of GB), thermal throttling (saw-tooth speed plus high temperature), dynamic cache shrinking on fuller drives.
- Risks: write wear (modest if capped: 100 GiB is ~0.017% of a 600 TBW drive), free-space exhaustion, orphan test files after crashes (`FILE_FLAG_DELETE_ON_CLOSE` handles process crash but not power loss), HDD spin-up, Defender/Controlled Folder Access, network and cloud-synced folders.
- Scoring: AS SSD total = weighted sum of MB/s with heavy weight on 4K and 4K-64Thrd (formula in section 7); PassMark Disk Mark = average of sequential read, sequential write, random seek r/w. CDM has no combined score. For two gauges (Read, Write in MB/s) we propose an auto-scaling "nice ladder" scale driven by the detected bus class.
- Licenses: DiskSpd MIT, CDM MIT (source header), fio GPL-2.0-only (study only, cannot be merged into a GPL-3.0-or-later codebase), f3 GPL-3.0, KDiskMark GPL-3.0. h2testw, ValiDrive, AS SSD, ATTO, Anvil, HD Tune, PassMark, OCCT are closed/freeware.

---

## 2. How the existing tools work

### 2.1 CrystalDiskMark (verified from source, hiyohiyo/CrystalDiskMark master, `DiskBench.cpp`, `DiskMarkDlg.cpp`)

License: MIT (header of each source file says `License : MIT License`; GitHub's automatic license detection shows none because there is no LICENSE file). CDM ships `diskspd32.exe`/`diskspd64.exe` (and ARM builds, plus "L" legacy builds) in `CdmResource\diskspd\` and launches one process per step.

Command line built per step (verbatim from `DiskBench.cpp`):

- Sequential/random read: `-b%dK -o%d -t%d -W0 -S -w0 [-r]`
- Write: `-b%dK -o%d -t%d -W0 -S -w100 [-r]` plus a write buffer option
- Mix: `-w%d` where `MixRatio = (9 - index) * 10`; default index 6, i.e. R70/W30
- Appended: `-d<measure seconds> -A<pid> -L "<test file>"`
- Write buffer option: `-Z` (zeros) when the user selects "0x00 fill", otherwise `-Z<blocksize>K` (random buffer of block-size), described in the code as "Compatible with DiskSpd".

Presets (arrays of 9 slots; type 0 = sequential, 1 = random):

| Profile | Test 0 | Test 1 | Test 2 | Test 3 | Measure / Interval |
|---|---|---|---|---|---|
| Default | SEQ 1M Q8 T1 | SEQ 1M Q1 T1 | RND 4K Q32 T1 | RND 4K Q1 T1 | 5 s / 5 s |
| NVMe SSD (v8 "Peak Performance") | SEQ 1M Q8 T1 | SEQ 128K Q32 T1 | RND 4K Q32 T16 | RND 4K Q1 T1 | 5 s / 5 s |
| Flash memory (Demo) | same as Default | | | | 1 s / 30 s (cool down between steps) |

- Peak Performance profile displays SEQ1M Q8T1 and RND4K Q32T1 results in MB/s, IOPS and latency (µs) simultaneously (`SCORE_MBS`, `SCORE_IOPS`, `SCORE_US`).
- Selectable measure time: 1-10, 20, 30, 60 s. Interval time: 0, 1, 3, 5, 10, 30, 60, 180, 300, 600 s. Default 5 s each.
- Test count (`TestCount`) default index 2 -> 3 runs. Code runs `for j = 0..count`: run 0 is "Preparing" (warm-up, never counted), runs 1..N measured. The reported value is the maximum over runs (`if (j > 0 && score > *maxScore)`), and latency is the minimum. So CDM reports best-of-N, not an average.
- Test size choices: 16, 32, 64, 128, 256, 512 MiB, 1 GiB (default), 2, 4, 8, 16, 32, 64 GiB.
- File handling: creates `CrystalDiskMark<hex>\CrystalDiskMark<hex>.tmp` in the root of the target (or chosen folder); checks that `DiskTestSize <= free bytes` (only check, no reserve); creates with `CREATE_ALWAYS` and `FILE_FLAG_NO_BUFFERING|FILE_FLAG_SEQUENTIAL_SCAN`; `SetFilePointerEx` + `SetEndOfFile` "to prevent fragmentation"; `FSCTL_SET_COMPRESSION` with `COMPRESSION_FORMAT_NONE` (explicitly disables NTFS compression on the file); fills the file by writing a 1 MiB `VirtualAlloc` buffer repeatedly (0x00, or `rand()%256` bytes, so the file is the same 1 MiB pattern repeated); deletes file and directory at exit. No `DELETE_ON_CLOSE`: a killed CDM leaves the folder.
- The unit for read-out can be MB/s (decimal), GB/s, IOPS or µs per test.
- CDM manual note (crystalmark.info): results depend on test file size, file position, fragmentation, controller and CPU, and some SSDs vary "depending on test data (random, 0fill)". CDM itself warns that it "may shorten SSD/USB memory life".

Why random vs 0x00: SandForce-style controllers compress data before writing; 0x00 data reaches 3-4x the speed. Vendors' spec sheets typically use compressible data. Most modern controllers do not compress, but some firmware dedupes/zero-detects; random data is the realistic worst case and what CDM selects by default. Sources: thessdreview Corsair Force GT "Data Types" article and Puget Systems "SSDs Advertised vs Actual Performance".

### 2.2 DiskSpd (Microsoft, MIT, verified)

Releases: 2.0.20a (2018) ... 2.2 (2024-06-13), 2.3 (2026-09-03/04: IoRing path `-u`, BypassIO `-Sy/-SY`, topology-aware affinity, `-bs` buffer separation). Binary release amd64 + arm64; minimum Windows 10 1803; IoRing and BypassIO need Windows 11. GitHub API: license MIT.

Parameters (from the official wiki page "Command line and parameters"):

| Option | Meaning |
|---|---|
| `-b<size>` | I/O size (default 64K) |
| `-o<n>` | outstanding I/Os per target per thread (default 2; 1 = synchronous) |
| `-t<n>` | threads per target |
| `-r[align]` | random I/O, aligned to `align` (default = block size) |
| `-s[i][align]` | sequential stride; `-si` interlocked across threads |
| `-w<pct>` | write percentage (default 0). "a write test will destroy existing data without a warning" |
| `-rs<pct>` | percentage of random (mixed random/sequential) |
| `-S[bhmruwyY]` | caching: `-S`=`-Su` (FILE_FLAG_NO_BUFFERING); `-Sw` (FILE_FLAG_WRITE_THROUGH); `-Sh`=`-Suw` (both); `-Sm` memory mapped; `-Sy/-SY` BypassIO |
| `-Z`, `-Zr`, `-Z<size>[,<file>]` | write buffers: zeros; per-I/O random (extra CPU overhead, not comparable); a random-filled source buffer of `<size>` (default pattern is repeating 0..255) |
| `-W/-d/-C` | warm-up (default 5 s), duration (10 s), cool-down |
| `-D<ms>` | IOPS time series interval (default 1000 ms) |
| `-L` | latency statistics (histogram percentiles in output) |
| `-g<n>[i]` | per-thread throughput throttle in bytes/ms, or IOPS with `i` |
| `-B<base>[:len]` | bounds; `-c<size>` create file; `-F` total threads; `-x` completion routines instead of IOCP |
| `-I<prio>` | I/O priority hint (1 very low, 2 low, 3 normal) |

Mechanics:
- Default I/O completion model is overlapped I/O with an I/O completion port (IOCP); `-x` switches to completion routines. Queue depth is the number of overlapped requests kept in flight per thread; DiskSpd re-issues as soon as one completes.
- File creation with `-c`: DiskSpd tries the fast `SetFileValidData` path when the user has `SeManageVolumePrivilege`, otherwise writes the file. The wiki warns the fast path "may expose previously written but logically deleted content".
- DiskSpd does not verify data (no read-back comparison). It is a load generator.
- Targets: file, `X:` partition, or `#N` physical disk. Our no-admin constraint means file targets only.
- Wiki: "File targets exercise the entire filesystem and storage stack, while a device target focuses on device behavior".

### 2.3 AS SSD Benchmark (closed freeware by Alex Intelligent; web search + oscooshop write-up)

Tests: Seq (1 MiB, Q? single thread, 1 GiB file by default), 4K (Q1), 4K-64Thrd (4K with 64 threads/queue), Acc.Time (random single-sector access time, in ms), plus a copy benchmark and a compression test. Three scores (read, write, total).

Classic formula (from several sources):

- Read score = Seq read x 0.1 + 4K read + 4K-64Thrd read
- Write score = Seq write x 0.1 + 4K write + 4K-64Thrd write
- Total = Seq write x 0.15 + Seq read x 0.1 + 4K read x 2 + 4K write + 4K-64Thrd write + 4K-64Thrd read x 1.5

Access time is displayed but not part of the score. Effective contribution on a typical NVMe: 4K-64Thrd read ~46% and write ~28% of the total, i.e. the score is dominated by deep-queue random.
Caution: a third-party page (ssd-tester.com, could not be fetched directly, 403) claims a new formula "since summer 2026" with Q32T16 and Q1T1 weights. I could not confirm it from the vendor or a second source, so treat it as unverified; use the classic formula only as a reference for how weighting works.

### 2.4 ATTO Disk Benchmark (closed freeware; web search)

Sweeps transfer sizes (512 B up to 64 MB selectable) at a fixed total length (default 256 MB, up to 32 GB) and queue depth (commonly 4 or 10), with options Direct I/O, Overlapped I/O, Neither, and a "write cache bypass" style option; plots read/write MB/s vs transfer size (bar chart) and optionally IOPS. Reviewers note ATTO's 256 MB test uses "extremely compressible data", so results are best-case for compressing controllers. The v5 release adds I/O comparison with test patterns. Useful idea for us: a block-size sweep chart.

### 2.5 Anvil's Storage Utilities (closed freeware; web search)

Customizable runs: sequential 4 MiB, 4K at QD1/4/16/32, 4K-QD16 IOPS; compressibility 100% (incompressible) or 0-fill/46% "application" mix; threads option; total score from a mix of MB/s and IOPS (weights not published in the pages I found). Reviewers typically run 5 times and average the middle three.

### 2.6 HD Tune Pro (shareware; web search)

Benchmark = sequential transfer rate across the whole disk surface (min/max/avg MB/s graph) plus random access time (yellow dots) and burst rate. For HDD the graph slopes down from outer to inner tracks. For us: a "speed over time/position" graph for sustained tests is the equivalent; with a file only we see the zone effect only if the file is large relative to the disk.

### 2.7 PassMark DiskMark (commercial; harddrivebenchmark.net test info)

Three sub-tests: Sequential Read, Sequential Write, Random Seek+RW. Test file 200 MB (500 MB write), block 16 KB, uncached asynchronous I/O with queue length 20, each test at least 20 s. The Disk Mark is an average of the sub-test values (no units).

### 2.8 fio (Jens Axboe, GPL-2.0; fio docs)

The reference for verification workloads. Relevant options: `ioengine=windowsaio`, `direct=1`, `iodepth`, `rw=randrw rwmixread=70`, `time_based runtime ramp_time`, `verify=crc32c|xxhash|md5|sha...` (a header with magic, offset, seed, checksum is written per verify block), `verify_pattern`, `do_verify`, `verify_fatal`, `verify_state_save` (resume verification of a previous run), `randrepeat`, `norandommap`, `buffer_compress_percentage`, `refill_buffers`, `scramble_buffers` (defeat dedup/compression), `unified_rw_reporting`. We do not link or copy fio (GPL-2.0-only), but its design is a good map.

### 2.9 OCCT storage test (closed; overclock3d news)

OCCT added a dedicated storage test described as delivering CrystalDiskMark-like results (sequential and random read/write) with online result upload. I found no technical description of data verification in OCCT's storage test; do not assume it verifies. It offers the "benchmark, with comparison database" model.

### 2.10 KDiskMark (GPL-3.0; Linux Qt front-end for fio)

Clone of CDM on top of fio with the same defaults: 1 MiB Q8T1/Q1T1, 4 KiB Q32T1/Q1T1, "1 GiB (x5)", 5 s measure, 5 s interval. Confirms that the CDM structure is the de facto standard.

---

## 3. Windows I/O mechanics (no admin)

### 3.1 Unbuffered I/O alignment (Microsoft "File Buffering")

- `FILE_FLAG_NO_BUFFERING`: access sizes AND offsets (including the `OVERLAPPED` offset) must be integer multiples of the volume sector size; buffer addresses should be aligned to the physical sector size ("depending on the disk this may not be enforced").
- Logical sector (512 or 4096) from `GetDiskFreeSpace` or `IOCTL_DISK_GET_DRIVE_GEOMETRY`; physical sector from `IOCTL_STORAGE_QUERY_PROPERTY` -> `STORAGE_ACCESS_ALIGNMENT_DESCRIPTOR.BytesPerPhysicalSector`. Microsoft recommends aligning to the physical sector.
- `VirtualAlloc` returns page-aligned (4096) memory, which satisfies every real sector size. `_aligned_malloc` is the alternative. In Rust: `VirtualAlloc` or `std::alloc` with `Layout::from_size_align(len, 4096)`.
- 4K random with `-r` default alignment = block size, so every I/O is aligned to 4 KiB; 512 B random on a 4Kn drive would fail.
- Only the file data cache is bypassed. Metadata stays cached ("file system metadata is always cached"); `FlushFileBuffers` flushes metadata (and sends a flush-cache command to the drive).

### 3.2 Write-through and what it does (Microsoft "File Caching", Raymond Chen)

- `FILE_FLAG_WRITE_THROUGH`: without NO_BUFFERING, data goes to the system cache but the cache manager writes it at once instead of lazily; with NTFS it makes NTFS set the FUA bit on the I/O. DiskSpd `-Sh` = NO_BUFFERING + WRITE_THROUGH.
- Raymond Chen (The Old New Thing, 2017-05-10): EIDE/SATA drivers did not honor FUA, so drive-internal buffers may still hold the data; `FlushFileBuffers` (FLUSH_CACHE) is the reliable way to push data to physical media. On NVMe, FUA writes bypass the SSD DRAM cache and reduce bandwidth (bapco/Solidigm notes).
- Consequence for us: use NO_BUFFERING only for throughput numbers (CDM-comparable); add an explicit "sync write" latency test using WRITE_THROUGH (shows the cost of durable writes and PLP-less drives); use NO_BUFFERING + `FlushFileBuffers` between the write and verify phases of a stability test.

### 3.3 Preallocation and valid data length

- `SetEndOfFile` extends the file but the valid data length (VDL) stays 0; reading beyond VDL returns zeros without touching the disk (MS SetFileValidData page explains VDL; CDM writes the whole file for this reason). So fill by writing.
- `SetFileValidData` skips zero-fill but requires `SE_MANAGE_VOLUME_NAME` (administrator) and can expose old disk contents. Do not use.
- NTFS compression would defeat benchmarks: clear it (`FSCTL_SET_COMPRESSION` + `COMPRESSION_FORMAT_NONE`, as CDM does) and refuse folders with EFS or sparse attributes where it matters; check the volume flag `FS_VOL_IS_COMPRESSED` as CDM does.
- Fragmentation: allocate the whole file in one go (SetEndOfFile then sequential fill) to keep it contiguous.

### 3.4 Queue depth and threads

- Q depth N on one thread = N overlapped requests in flight with `FILE_FLAG_OVERLAPPED` on the handle, completed through an IOCP (`GetQueuedCompletionStatusEx`). T threads = T such loops, each with its own request ring and buffers. DiskSpd default engine.
- Windows 11 IoRing (DiskSpd 2.3 `-u`) exists but is not needed; IOCP works on Windows 10 and 11.
- Latency per I/O: `QueryPerformanceCounter` at submit and at completion, into a log-bucket histogram (like HdrHistogram) -> P50/P95/P99/P99.9; DiskSpd `-L` and CDM's latency column work this way.
- Thread priority/I/O priority: `SetFileInformationByHandle` with `FileIoPriorityHintInfo` (`IoPriorityHintLow`) mirrors DiskSpd `-I`; sensible default for stress runs so the PC stays usable. (Not verified from a source in this session; API exists in Win32 docs.)

### 3.5 Other Windows effects

- Controlled Folder Access (ransomware protection) can silently block writes in Documents/Pictures/Desktop; Defender real-time scanning can slow large file creation 5-10x per one Microsoft-adjacent source (search result; not rigorously verified); OneDrive-redirected "known folders" are cloud placeholders. Use a plain folder on the target volume (the user picks the folder but the app should warn on OneDrive/Dropbox paths and on `FILE_ATTRIBUTE_RECALL_*` ancestors).
- Delete a big test file on an SSD -> TRIM; a following run starts with different cache state. Expect variance between repeated runs.

---

## 4. Sustained, thermal and mixed-load stress

### 4.1 SLC cache exhaustion

- Reviewers (TechPowerUp "Write Intensive Usage", AnandTech "Whole-Drive Fill") write a single-threaded stream of 1 MiB sequential blocks, sampling speed twice per second, on an erased drive, until the drive is full; speed plateaus at the SLC-cache rate (2.5-5+ GB/s on current NVMe), then drops to the direct-TLC/QLC rate (200 MB/s to ~2 GB/s depending on the drive), sometimes steps down again when the drive gets full. Cache sizes range from ~28 GB to 150 GB+ on 1-2 TB drives.
- Others note that using higher queue depths gives bigger numbers that are less realistic. Q1 (or Q2) with 1 MiB is the standard.
- File-level limitation: on a non-empty or non-erased drive the dynamic SLC cache is smaller (it depends on free space and on whether previous data has been folded). We can report "cache-like burst of X GiB at Y MB/s, then Z MB/s" for the free space available, with a clear caveat. We cannot erase/TRIM the whole drive.
- Detection algorithm (proposal): rolling median over 1-2 s windows; baseline = median of the first 2-3 s after warm-up; mark a "cliff" when speed stays below 60% (configurable) of the baseline median for >= 3 s; cache size = bytes written at the cliff start; steady rate = median of the last 20% of the run. If the temperature crossed a threshold within the previous ~10 s the cliff is flagged "thermal suspect" instead (see 4.2).

### 4.2 Thermal throttling detection

- Reviewers log temperature and transfer speed during sustained copies; thermal throttling shows as a repeated saw-tooth speed pattern coinciding with a high controller temperature; cache exhaustion shows as a one-time step that does not recover until idle. Pairing the speed graph with a temperature log is what separates the two (TechPowerUp A2000/SM951 articles; netdata guide).
- NVMe exposes thresholds in Identify Controller (WCTEMP = warning composite temperature, ~70-80 C; CCTEMP = critical, ~80-85 C, vendor-defined) and counters in the SMART log (Warning Composite Temperature Time, Critical Composite Temperature Time, Thermal Management Temperature 1/2 Transition Count: number of times the drive entered a thermal management state). The delta of these counters before/after a test is direct evidence of throttling for drives that support them. Our service already reads SMART/temperatures, so this is natural.
- During the test the disk is busy, so reading its temperature does not wake an HDD or reset its idle timer in a harmful way (see `storage_gate.rs`: `ACTIVITY_WINDOW` already treats recent I/O as authorization to read temperature). Start the test only after the existing `DiskPower` check says the disk is not in `Standby` (or after explicit consent).
- A read-only sustained test is the cheap thermal soak (no wear after the initial fill): 128 KiB-1 MiB reads, Q8-Q32, over a file larger than the drive's DRAM/host-memory cache, running 5-30 min while plotting temperature.

### 4.3 Mixed and long random workloads

- CDM's Mix default is R70/W30 (index 6 in code). DiskSpd `-w30`; fio `rwmixread=70`. Random 4K mixed at Q32 shows controller/GC behaviour; random small writes have higher write amplification than sequential (Digital Citizen / Flash Memory Summit; a 600 TBW 1 TB drive with WAF 1.5 gives ~400 TB host-write endurance).
- SNIA SSS-PTS steady state (reference only): precondition by writing sequential 128 KiB for 2x capacity, then 4 KiB random write until "steady state" (a window of 5 rounds with data excursion < 20% and slope excursion < 10% of the average). A file-level tool cannot reach true steady state (it cannot fill the whole drive, and the user's data is on it), but the "5 consecutive intervals within +-10-20% of their average" idea is a good stopping rule for an optional "until stable" stress duration.
- Block-size mixes: realistic mixes (e.g. 4K 70% / 8K 10% / 16K 10% / 64K 10%, 70/30) are an option for "real-world" stress; keep the default simple (4K + 128K).

### 4.4 Other tools' stress behaviour

OCCT: benchmark-style (see 2.9). h2testw/f3: full-capacity fill + verify (see 5). Burn-in tools (PassMark BurnInTest forum thread) combine sequential verify and random verify cycles. We found no mainstream Windows tool that combines load + continuous inline verification with a per-block generation counter; that is our differentiator.

---

## 5. Data integrity verification

### 5.1 h2testw (Harald Bogeholz, c't; closed freeware; readme via web search)

- Writes files `1.h2w`, `2.h2w`, ... of 1 GB (multiples of 1 MB) to fill the free space, then reads them back and verifies.
- The test data is "made up so as to be able to discern certain typical errors": addressing errors (a sector is written at the wrong address and overwrites another sector), data altered only slightly (< 8 differing bits in a sector), and completely corrupted sectors. Statistics output: OK / overwritten / slightly changed / corrupted.
- Per-sector header carries the sector's own address, which is how overwritten (aliasing) sectors are recognised: this is the signature of fake-capacity flash that wraps around.

### 5.2 f3 (Fight Flash Fraud, AltraMayor; GPL-3.0; verified from source `libutils.c`, `f3write.c`)

- `f3write` fills the filesystem with 1 GiB files `N.h2w` (the same format name as h2testw); `f3read` reads and verifies.
- Block = 512-byte sector (`SECTOR_ORDER`). Generation (`fill_buffer_with_block(buf, block_order, offset, salt)`):
  - `int64[0] = offset` (the byte offset of this block in the whole test stream, NOT salted: "drives know the offset, so applying salt to it leaks the salt");
  - `int64[i] = rnd = next_random_number(rnd)` for i >= 1 with `rnd` seeded from `offset ^ salt`;
  - `next_random_number(x) = x * 4294967311ULL + 17` (a 64-bit LCG).
- Verification (`validate_buffer_with_block`): read `found_offset = int64[0]`, regenerate the stream from `found_offset ^ salt`, compare words and count differing bits (`popcount`), stop counting above 7. Classification:
  - `found_offset == expected && bit_errors == 0` -> good
  - `found_offset == expected && 1..7 bit errors` -> "changed" (slightly changed)
  - `found_offset != expected && <= 7 bit errors` -> "overwritten" (valid block from elsewhere: aliasing / wrong address)
  - otherwise "bad" (corrupted)
  Summary printed: Data OK, Data LOST (Corrupted, Slightly changed, Overwritten).
- `f3probe` (Linux, raw device, root) tests capacity quickly by writing only what is necessary. Needs the block device, so not applicable without admin on Windows (and f3probe is Linux-only).
- Salt: f3 introduced a salt so a malicious drive cannot precompute the stream; relevant only for counterfeit detection; for a stability test a random per-run seed is enough.

### 5.3 ValiDrive (GRC, Steve Gibson; freeware, 113 KB, USB mass storage)

Spot check at 576 evenly spread locations visited in a random non-repeating order: read the region's current contents, fill it with random data noise, read back to verify, then restore the original data. Detects fake capacity quickly, reports access time per location and a map. It first detects RAM caching in the drive and the transfer size needed to bypass it. It uses raw device access (not available to us without admin). Output categories: Validated, Read Error, Write Error, No Storage.

Idea we can borrow in file mode: visit random offsets within the test file in random order and record per-location latency (heat map).

### 5.4 fio verify (verified from docs, summary)

- A per-verify-block header holds a magic number, the block's offset, a seed and a checksum; the data body is generated from the seed; `verify=crc32c`/`xxhash`/`md5`/`sha*` selects the checksum; verification can be inline (`verify_backlog`) or in a read-back pass (`do_verify`); `verify_state_save` allows resuming verification after a restart; mismatch reports include the offset and the type of mismatch.
- This is the same structure we propose below, with a header that makes every block self-describing.

### 5.5 Cheap verification design (proposal)

Goal: detect (a) bit flips, (b) misdirected writes (data of block A found at B), (c) stale data (an older generation returned after an overwrite), (d) zeros / lost writes, (e) short reads and read errors; with no storage of the data and with data generation fast enough not to bottleneck a Gen5 drive.

Block layout (4 KiB, matches our I/O unit; for 128 KiB-1 MiB I/Os it is a sequence of 4 KiB blocks):

| Bytes | Field |
|---|---|
| 0..8 | magic (e.g. `OMADTEST`) |
| 8..16 | run id (random u64 per test run; also detects leftovers from a previous run) |
| 16..24 | block index within the test file (u64) |
| 24..28 | generation counter (u32, increments each time this block is rewritten; sequential fill = 1) |
| 28..32 | flags / pattern version |
| 32..40 | xxHash3-64 (or CRC32C+length) over header[0..32] + payload |
| 40..64 | reserved (could hold a second 64-bit checksum or the previous generation) |
| 64..4096 | payload from PRNG seeded by `(run_id, block_index, generation)`: e.g. xoshiro256++ or SplitMix64 counter mode (fast, multi-GB/s per core); alternative: AES-CTR with AES-NI (10+ GB/s) |

- Verify path: recompute only the checksum (cheap, ~10-30 GB/s with xxh3/CRC32C hardware) and compare with the stored one. Only on mismatch regenerate the payload and compare word by word to classify: bit-flip count (like f3: <= 7 -> "slightly changed"), wrong block index in header -> "misdirected", older generation (header gen < expected) -> "stale", all zero -> "zeros/lost write", otherwise "corrupted". Record offset of first error, error count, and the first N sample diffs (expected vs found word).
- Generation tracking for random-overwrite stress: one u8/u16 per block in RAM (file of 16 GiB = 4M blocks = 4-8 MB of RAM, trivial); use a pre-chosen deterministic sequence so verification on the next pass knows the expected generation; or write the generation and verify against the table.
- Reads must bypass the OS cache (NO_BUFFERING), otherwise we are verifying RAM. Because drives have their own DRAM/SLC buffers, verify in a different order than written (e.g. reverse or random), with the file larger than the drive cache (>= 2x the DRAM/HMB cache, ideally several GiB), and `FlushFileBuffers` before the verify pass.
- Retry on mismatch: re-read the same block once or twice. A mismatch that disappears on re-read indicates a transient read error, cable/controller or host-RAM instability (flag "transient"), not stored corruption. A repeatable mismatch is "persistent".
- Also catches host-side errors (RAM, CPU, PCIe, cable): the message must say that a data error can come from the drive, cable, controller driver, memory or CPU; it cannot isolate the cause.
- The compressibility of the pattern is high-entropy, so no compression/dedupe false results; and every block unique so dedupe cannot hide a lost write.
- Do not use CRC on the payload alone: a drive returning an older generation of the same block would pass a checksum that depends only on block index.

Quick-capacity mode for removable media (h2testw-like): fill free space with 1 GiB chunk files (works around FAT32's 4 GiB limit), verify all, report OK/lost/overwritten bytes. This works without admin because the file system is the unit; fake-capacity drives that wrap around show "overwritten" blocks.

---

## 6. Risks and safety

### 6.1 SSD wear

- NVMe SMART "Data Units Written" counts the 512-byte units the host wrote, reported in thousands, rounded up (1 unit = 512,000 bytes). We can read it before/after a test and show "Host writes during this test: X GiB (from SMART)" if available; SATA uses vendor-specific attributes (e.g. total LBAs written), so show only when mapped.
- Typical consumer ratings: ~600 TBW for 1 TB (e.g. mainstream TLC), 150-300 TBW for 1 TB QLC/DRAM-less. 100 GiB written = 0.0179% of 600 TBW (= 0.04% for a 250 TBW QLC drive). CDM's fixed-time write steps are far larger than people expect: forum analyses (Hardwareluxx "CrystalDiskMark SSD-Killer") reported tens of GB per step; a full default CDM run on a Gen4 drive can write 100-250 GB. "Hardly matters" for one run, but not for a nightly stress loop. Write amplification makes random small writes cost more wear per host byte than sequential.
- Tool warnings: CDM site "may shorten SSD/USB memory life"; DiskSpd "a write test will destroy existing data" (for raw targets). Neither displays an estimate.
- Our policy: every mode shows "estimated host writes: up to X GiB" before starting, and the per-run write budget is capped by bytes in addition to time. The stability modes show the cumulative estimate (file size x passes). Warn if the estimate exceeds 1% of the drive's total TBW (when SMART endurance data is available) or exceeds a fixed amount (e.g. 500 GiB) for non-endurance-rated drives.
- Read-only phases cost no wear, except read disturb which is negligible; prefer read soak for thermal tests.
- USB flash and SD cards have tiny endurance: restrict write size on removable drives (default 1-2 GiB, full-capacity only on explicit "capacity check").

### 6.2 Free space

- CDM only checks `size <= free`. We should leave a reserve: `file_size <= free - max(1 GiB, 5% of the volume)` (a full NTFS volume breaks the user's apps, the pagefile and hibernation if on the same volume). Hard-cap defaults per mode (section 9) and refuse the run when the system volume has < 10% free unless the user confirms.
- Watch for free-space changes during long tests (downloads etc.): handle `ERROR_DISK_FULL` gracefully (stop, delete, report), never retry-loop.
- SSDs slow down when nearly full (SLC cache shrinks); mention in the report that the result reflects current free space and fill level.

### 6.3 No test files left behind

- `FILE_FLAG_DELETE_ON_CLOSE`: the file is deleted when its last handle is closed. If the process dies, Windows closes the handles, so the file is deleted (verified behaviour as documented by Microsoft; the flag requires FILE_SHARE_DELETE on any other open handle). Open the data file(s) with this flag from the start; a crash or kill of the app then cleans up automatically. Drawback: the file cannot be reopened by a second process (that is what we want), and a power loss or BSOD still leaves the file.
- Defence in depth: (1) a clearly named test directory `OMA-DiskTest-<uuid>` or file prefix; (2) a tiny sidecar `.lock` with PID, process start time and run id; (3) on app start (and on opening the disk test page) sweep the user's recent test folders for orphans whose PID/start time no longer match and offer deletion; (4) final cleanup in a `Drop` guard plus Tauri exit hook; (5) never write outside the chosen folder. CDM relies on `Exit()` only.
- Write-pointer discipline: the file name must not collide with user files: use `CREATE_NEW`, never `CREATE_ALWAYS`/overwrite (CDM uses `CREATE_ALWAYS` on its own random name).

### 6.4 HDD in standby and idle policy

- An HDD benchmark spins the disk up and keeps it spinning; the existing app deliberately avoids that for temperature reads (`DiskPower::Standby` from `GetDevicePowerState`; `DiskClass::RotationalOrUnknown`). So: before a test on a rotational or unknown disk, if its power state is Standby, show "this disk is spun down; the test will spin it up" and require an explicit click. After the test, say that the disk's idle timer restarts.
- HDD specifics: Q32 random is not meaningful (an HDD does ~80-120 4K IOPS at Q1); the file-based test only sees the part of the platters where the file lies (outer zones when the disk is fresh); keep HDD defaults shorter and lower Q. Sequential speed varies by zone (about 2x outer vs inner), so a sustained read over a large file or the whole free range reveals a downward slope.
- Do not run stress tests on a drive that is the only drive holding the system without a warning that the PC may become unresponsive; set low I/O priority.

### 6.4a Disk selection

The user chooses a folder; we map it to a volume and physical disk (`GetVolumePathNameW`, then `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` and `IOCTL_STORAGE_QUERY_PROPERTY`; the app already does storage IOCTLs without admin) to show the drive, bus type, class, SMART state and temperature in the pre-flight dialog. A folder on a Storage Spaces / RAID / BitLocker-encrypted / dynamic-disk volume works but the result describes the aggregate.

### 6.5 Network, removable, encrypted volumes

- Network shares: `GetDriveTypeW == DRIVE_REMOTE` (also UNC paths). SMB with NO_BUFFERING has different semantics (DiskSpd has `-Sr` for remote), results include network and server cache; sustained writes can saturate a network. Default: refuse; allow only with an explicit "I understand" and use a limited preset.
- Removable/USB: allowed with the "flash" preset (short, small, capacity-check mode available); USB mass-storage drives have a write cache policy ("quick removal" vs "better performance"); `FlushFileBuffers` before verify is essential. Warn about wear. Sustained write on USB sticks causes severe throttling/heat.
- BitLocker/software encryption: works transparently (NO_BUFFERING goes through the volume filter), uses CPU for crypto and can lower throughput on very fast drives; mention in the result's context string (volume encrypted: yes/no from `Win32_EncryptableVolume` is admin-only, so skip detection or just note possibility). Hardware-encrypted SSDs: no effect.
- Virtual disks (VHD(X), file-backed virtual, RAM disks): detect bus type (`BusTypeFileBackedVirtual`, `BusTypeVirtual`) and label results "virtual disk: includes host file cache".

### 6.6 Antivirus and other interference

- Real-time scanning scans large files on close and may scan on write. Use unbuffered handles, keep the file open for the whole run (scanner triggers at open/close), and name the file `.tmp`/`.oma-disktest` (no executable content). Mention Defender exclusion as an optional tip, never auto-configure (needs admin and is bad practice).
- Controlled Folder Access can block writes in protected folders: when `CreateFileW` fails with `ERROR_ACCESS_DENIED` suggest another folder.
- Indexer/OneDrive/Backup: ask the user to pick a plain folder; detect common sync roots by path name (`OneDrive`, `Dropbox`, `Google Drive`) and warn.
- Background I/O from other apps varies results; show "other I/O on this disk during test" using the existing PDH disk counters (bytes/s outside our test) and flag runs where external I/O > ~5% of the test rate.
- Power plan / laptop on battery / PCIe ASPM / USB selective suspend alter results: show an "on battery" warning.

---

## 7. Scoring and gauge scales

### 7.1 How tools score

| Tool | Result form | Combination |
|---|---|---|
| CDM | per-test MB/s (or IOPS, µs) | no overall score; best-of-N |
| AS SSD | MB/s per test + Acc.Time ms | read/write/total score with weights (section 2.3); random 4K-64Thrd dominates |
| PassMark Disk Mark | unitless rating | average of seq read, seq write, random seek RW sub-scores |
| Anvil | MB/s, IOPS | total score (weights unpublished) |
| ATTO | MB/s per transfer size | none (chart) |
| HD Tune | MB/s min/avg/max, access ms, burst | none |

Units: MB/s (decimal in CDM: 10^6 bytes, DiskSpd reports MiB/s binary) for sequential; IOPS for small random; latency µs/ms for Q1. DiskSpd 2.1+ prints sizes in KiB/MiB/GiB; to avoid mismatch with spec sheets (decimal), show MB/s decimal like CDM and write "MB/s = 10^6 B/s" in the details.

### 7.2 Proposal for our app

- Primary UI: two gauges, Read and Write, in MB/s, showing live throughput for the current step and the final value (the result of the main sequential step: SEQ1M Q8T1; the second ring/marker could be the Q1 or random value). Below them a small table of the other steps (MB/s, IOPS, P99 latency).
- Optional composite score, if wanted: do not use AS SSD's weights (random-heavy and unpublished methodology; unverified current formula). A transparent alternative: score = geometric mean of the four normalized numbers {seq read, seq write, 4K Q1 read IOPS, 4K Q1 write IOPS} (or Q32), each divided by a class-specific reference (HDD, SATA SSD, NVMe Gen3/4/5 reference values) x 100. Show it only as "relative score for this drive class", never compare across classes. Keep it as v2; ship raw numbers first (YAGNI).
- Gauge scale selection, no hard-coded max: use a "nice" ladder of maxima in MB/s: 100, 250, 500, 1000, 2000, 3500, 5000, 7500, 10000, 14000, 20000. Initial maximum from the detected class: HDD 300 (ladder 250), USB flash/external HDD 500, SATA SSD 600 (ladder 750 or 1000), NVMe by PCIe link: Gen3 x4 -> 4000, Gen4 x4 -> 8000, Gen5 x4 -> 14000 (PCIe link speed from the existing device data if available; else start from 1000 and auto-grow). Rule: scale only grows during a run (snap up to the next ladder step when the value exceeds 90% of the maximum, with ~300 ms animation), is reset at the start of the next step, and the final value is drawn against the scale that includes the run's peak so the needle never pins. Show interface ceiling markers if known (SATA 600 MB/s, USB 3.0 ~450, USB 3.2 Gen2 ~1000).
- Gauges for IOPS/latency steps: show MB/s still (consistent), with IOPS and latency in text.

---

## 8. Licenses

| Item | License | Notes |
|---|---|---|
| DiskSpd | MIT (GitHub API `spdx_id: MIT`; LICENSE file) | Can be studied and parts reused with attribution. C++ source (IORequestGenerator). |
| CrystalDiskMark | MIT (header "License : MIT License" in each source file) | C++/MFC; bundles DiskSpd. No LICENSE file, so GitHub shows none. |
| fio | GPL-2.0 (GitHub API `GPL-2.0`; the project states GPL v2) | GPL-2.0-only cannot be merged into GPL-3.0-or-later code; study design, do not copy code. |
| f3 (Fight Flash Fraud) | GPL-3.0 | Compatible with GPL-3.0-or-later; check headers for "or later" before copying. The algorithm (offset + LCG) is simple enough to re-implement independently; recommended. Linux/POSIX only for f3probe. |
| KDiskMark | GPL-3.0 | Linux/Qt front end for fio; irrelevant code-wise. |
| h2testw | closed freeware (c't / heise) | Format-compatible `.h2w` naming is not needed. |
| ValiDrive | closed freeware (GRC) | |
| AS SSD, ATTO, Anvil | closed freeware | |
| HD Tune | shareware (free + Pro) | |
| PassMark PerformanceTest | commercial | |
| OCCT | closed (free personal / paid) | |
| Rust `windows` crate | MIT/Apache-2.0 | for CreateFileW/IOCP; project already uses hand-written bindings per CLAUDE.md. |
| xxhash-rust (XXH3), crc32fast, rand_xoshiro | MIT/Apache-2.0 / BSD-like | Check each in `cargo about` before adding; none is strictly needed. The project convention is minimal dependencies. |

No NVML/ADL-style proprietary headers are involved; none of the tools above needs vendor SDKs. If we reuse DiskSpd logic or CDM presets, add attribution to `THIRD_PARTY_NOTICES.md`; CDM presets themselves are just parameter lists and not copyrightable expression, but name them honestly ("CDM-compatible"). Do not call our presets "CrystalDiskMark" (trademark/branding).

---

## 9. Proposed modes for our app

Design rules shared by all modes:

- Test file(s): in the user-chosen folder on the target volume, `CREATE_NEW`, unique name `OMA-DiskTest-<uuid>.tmp`, `FILE_FLAG_OVERLAPPED | FILE_FLAG_NO_BUFFERING` (+ `FILE_FLAG_DELETE_ON_CLOSE`; + `FILE_FLAG_WRITE_THROUGH` only in the sync-write test), `FSCTL_SET_COMPRESSION` none, `SetEndOfFile` then a full sequential write fill (no `SetFileValidData`).
- Sector alignment from the volume (default 4096 for I/O > 4 KiB; block-size = alignment for random).
- Data pattern: random per-block pattern from the seed scheme of section 5.5 (default); option "0x00 fill" for the compressible/peak variant and "compare random vs 0x00" for flash.
- Pre-flight: bus/class, `DiskPower` (HDD standby consent), free space and reserve, drive type (fixed/removable/remote), path warnings (OneDrive etc.), estimated host writes, estimated duration, battery, system-volume warning.
- Telemetry during the run: throughput (2 Hz), IOPS, latency histogram, temperature (existing provider), external I/O check, host writes via SMART delta when available.
- Safety: byte cap per mode, `ERROR_DISK_FULL` abort, cancel button deletes files immediately, orphan sweep at app start.
- Defaults assume a 1 TB NVMe at ~3-7 GB/s; wear numbers use a reference 600 TBW/TB drive (100 GiB = 0.0179%).

### 9.1 Benchmarks (read-mostly measurement, user data untouched; own file only)

| ID | Name | Workload parameters | Targets | Integrity verification | Default size/duration | Wear estimate |
|---|---|---|---|---|---|---|
| B1 | Quick benchmark (CDM Default-like) | SEQ 1 MiB Q8T1; SEQ 1 MiB Q1T1; RND 4 KiB Q32T1; RND 4 KiB Q1T1; each Read then Write; unbuffered, no write-through; per step 1 warm-up run (discarded) + 3 measured runs of 5 s; report best (CDM-compatible) and median in the details; 5 s pause between steps | Everyday sequential and random throughput, QD1 responsiveness | File fill uses the verifiable pattern; read steps verify the checksum cheaply on a sampled basis (e.g. 1 in 64 blocks) to flag a failing drive; reported only as a warning | 1 GiB file (256 MiB-16 GiB selectable), about 2-3 min total; write steps capped at 16 GiB each (seq) and 4 GiB (4K) | Worst case Gen4 NVMe ~40 GiB (0.007%); SATA SSD ~4-8 GiB; HDD ~1.5 GiB. Fill = file size once. |
| B2 | Peak / NVMe profile | SEQ 1 MiB Q8T1; SEQ 128 KiB Q32T1; RND 4 KiB Q32T16 (16 threads x 32 = 512 outstanding); RND 4 KiB Q1T1 | Controller headroom, NVMe parallelism | as B1 | 1 GiB, 3 min | ~50 GiB worst case |
| B3 | Mixed 70/30 | RND 4 KiB Q1 and Q32 at R70/W30; SEQ 1 MiB R70/W30 Q8 | Controller under mixed traffic | as B1 | 1 GiB, 2 min | ~15-25 GiB worst case |
| B4 | Latency profile | RND 4 KiB Q1T1 read; RND 4 KiB Q1T1 write; sync-write 4 KiB Q1 (WRITE_THROUGH + FlushFileBuffers every N); histogram P50/P95/P99/P99.9; for HDD the average becomes the "access time" (ms) | Latency-sensitive use, DRAM-less/PLP behavior, HDD access time | none | 1 GiB, 30 s per step | < 2 GiB (sync write 4K at Q1 is slow) |
| B5 | Block-size sweep (ATTO-like) | read and write throughput for 512 B/4K/8K/.../1 MiB/4 MiB/16 MiB at Q4 (T1), 2 s each | Shape of the performance curve, optimal I/O size | none | 512 MiB file, ~1 min | ~5-10 GiB |
| B6 | Queue-depth scaling | RND 4 KiB read and write at Q1/2/4/8/16/32/64 (T1), 3 s each, IOPS vs latency curve | Where the drive saturates; latency knee | none | 1 GiB, ~1 min | ~5 GiB |
| B7 | Flash/USB preset | SEQ 1 MiB Q1 R/W and RND 4K Q1 R/W; 1 s measure, 30 s interval (CDM "Flash memory" idea) to avoid heat/wear | USB sticks, SD cards | as B1 | 256 MiB, ~3 min | < 1 GiB |

### 9.2 Normal stress tests ("does it hold up under load"; no integrity guarantee beyond sampled verification)

| ID | Name | Workload parameters | Targets | Integrity verification | Default size/duration | Wear estimate |
|---|---|---|---|---|---|---|
| N1 | Mixed load | R70/W30; 4 KiB random 60% Q16 plus 128 KiB sequential 40% Q4; T2; low I/O priority; rate cap optional (`-g` style) | Sustained controller + NAND under mixed load, GC, temperature | Inline verify of every read block (all reads are of previously written, checksummed blocks) | File 8 GiB (cap 25% of free), 10 min (1/10/30/60 min, "until stopped" with a byte cap) | write rate x 30% x time: e.g. 1.5 GB/s x 0.3 x 600 s = 270 GB. Default byte cap 200 GiB (0.036%) stops the run early; show the estimate before start. Optional rate limit makes wear predictable (e.g. 100 MB/s writes x 600 s = 56 GiB) |
| N2 | Sustained write (SLC cache / sustained) | SEQ 1 MiB Q1 (Q2 optional) write, continuous; speed sampled 2 Hz; speed vs bytes written graph; cliff detection (4.1) and thermal-suspect flag from temperature | SLC cache size and post-cache rate, thermal throttling, drive behavior when full | Written blocks carry the pattern; the last X GiB is verified at the end (sampled) | Up to min(100 GiB, 50% of free space minus reserve); stop at 3x the detected cliff or on cap; ~1-5 min on NVMe, 10+ min on HDD/USB | Up to the cap: 100 GiB = 0.018% (cap raisable in advanced options to 500 GiB with a red warning) |
| N3 | Read soak / thermal soak | SEQ 1 MiB (or 128 KiB) Q8 read, plus 20% RND 4K Q32 read in a second thread; reads over a file larger than drive cache; temperature plot; thermal counters (NVMe WCTEMP/CCTEMP, transition counts) before/after | Thermal throttling without wear; link stability | Every read verified against checksum (cheap) | File 8-16 GiB, 5-30 min (default 10) | Fill once (file size, e.g. 16 GiB); the read soak itself ~0 |
| N4 | Random IOPS stress | RND 4 KiB R/W (R70/W30 default; 100% read or 100% write optional) at Q32 T4, time based, per-second IOPS and P99 | Random small-block endurance, firmware GC, HDD seek stress (Q1-Q4 for HDD) | Inline verify of reads | 8 GiB, 10 min | 4K write 100% at ~300-600 MB/s x 600 s = 180-360 GB; default 70/30 ~ 55-110 GB; byte cap 100 GiB |

### 9.3 Stability tests ("did the data come back right")

| ID | Name | Workload parameters | Targets | Integrity verification | Default size/duration | Wear estimate |
|---|---|---|---|---|---|---|
| V1 | Fill and verify | Phase 1: sequential 1 MiB Q8 write of the whole test region with the 5.5 block scheme; `FlushFileBuffers`; phase 2: read back in reverse order (or random 1 MiB chunks) at Q8 and verify every block; optional phase 3: overwrite 25% randomly at 4 KiB Q32 with generation+1, then verify again. Cycles: 1-N | Silent corruption, controller bugs, bad cables/ports, unstable PCIe/SATA links, RAM errors | Full checksum on every block; mismatch classification (bit flips, misdirected, stale, zeros); retry-read to separate transient from persistent; first N errors with offsets | 10% of free space capped at 8 GiB per cycle by default (options 1 GiB .. 90% of free); 1 cycle ~1-2 min on NVMe | size x cycles x (1 + overwrite share): 8 GiB x 3 cycles x 1.25 = 30 GiB (0.005%) |
| V2 | Random overwrite stability | Time-based random 4 KiB (and 16/64 KiB mix) writes at Q16 T2 into the file with generation tracking, and reads verifying previous generations (R50/W50), with periodic full sweeps (verify all blocks every M minutes) | Wear leveling/GC integrity, stale-data returns, lost writes under churn | Header with block index, generation and checksum; in-RAM generation table (u8/u16 per block); full-sweep verification | 8-16 GiB file, 30 min (10 min ... 24 h) | Write-limited: e.g. 200 MB/s x 50% x 1800 s = 180 GB (0.03%); default rate cap 100 MB/s to keep ≤ ~90 GiB per 30 min; show estimate; byte cap 250 GiB |
| V3 | Capacity / counterfeit check (removable and external only) | Fill all free space (minus 5% or 256 MiB reserve) with 1 GiB chunk files (h2testw/f3 style, SEQ 1 MiB Q1-Q4) then verify all in a different order; reports OK / lost / overwritten (aliasing) / corrupted bytes, write and read speed over position, minimum/maximum speed | Fake capacity flash, failing SD cards, USB sticks | Per-block header with its global offset (like f3: self-addressing) + checksum; classification as f3 | Entire free space; time = capacity / speed (e.g. 64 GB at 30 MB/s = ~35 min each pass); show estimate and cancel | Capacity x 1 write (one full device write, small for flash lifespan but not negligible: 1 P/E cycle). Refuse on internal NVMe/SSD without strong warning; offer only for removable class by default |
| V4 | Sync-write integrity | 4 KiB writes with WRITE_THROUGH + `FlushFileBuffers` at Q1 for N minutes, verifying after; reveals drives that lie about flush | Cache-flush honesty (cannot fully prove: needs power cut) | Checksum read-back (note: without power loss it cannot prove durability) | 1 GiB, 2 min | ~1 GiB |

Notes on the catalog:
- "Normal stress" corresponds to N1-N4 (load, with cheap inline checks); "Stability" corresponds to V1-V3 (verification is the point). Both use the same generator/verifier and I/O engine; only the plan differs.
- Presets for the UI: Quick (B1), Full benchmark (B1+B3+B4), Peak/NVMe (B2), Sustained write (N2), Thermal soak (N3), Stress 10 min (N1), Stability 10 min (V1), Stability long (V2), Capacity check (V3).
- Cancel at any time; always delete test files; always show the results screen with: drive, bus, firmware/model, volume, free space before/after, file size, block sizes, Q/T, data pattern, the SMART host-write delta, temperature min/max, any external I/O flags, error counts.
- Defaults should leave the system responsive: low I/O priority and a CPU thread cap (1-2 workers for generation; Q/T tests use the minimum threads necessary; the data generator must not exceed the budget of the project: "frugal resources", keep CPU use low when idle, only high while a test runs).
- Verification at 7+ GB/s costs CPU: xxHash3 or CRC32C at tens of GB/s is fine; the payload generator (xoshiro256++ ~ 5-10 GB/s/core, AES-CTR 10+ GB/s) is the bottleneck during writes; use a pre-generated pool of random 4 KiB payloads mutated by `block index ^ generation` (cheap xor/add of a 64-bit key every 8 bytes) when the generator cannot keep up, then regenerate only on mismatch classification. Verification remains fully deterministic.

### 9.4 Suggested implementation order (smallest useful steps)

1. I/O engine (CreateFileW flags, aligned buffers, IOCP queue, QPC latency histogram) + B1 with the two gauges.
2. Pre-flight (free space, drive type, HDD standby consent, orphan cleanup, DELETE_ON_CLOSE).
3. Block generator/verifier + V1 (fill and verify).
4. N2 (sustained write with cliff and temperature overlay) and N3 (read soak).
5. N1/N4/V2 (time-based mixes), then V3 and B2-B7 as presets.

---

## 10. Sources

CrystalDiskMark / DiskSpd
- CDM source (DiskBench.cpp, DiskMarkDlg.cpp): https://github.com/hiyohiyo/CrystalDiskMark
- CDM site: https://crystalmark.info/en/software/crystaldiskmark/
- CDM Wikipedia (license and data patterns): https://en.wikipedia.org/wiki/CrystalDiskMark
- DiskSpd repository: https://github.com/microsoft/diskspd
- DiskSpd command line and parameters (wiki): https://github.com/microsoft/diskspd/wiki/Command-line-and-parameters
- DiskSpd customizing tests (file creation, SetFileValidData, entropy): https://github.com/microsoft/diskspd/wiki/Customizing-tests
- DiskSpd releases: https://github.com/microsoft/diskspd/releases
- Digital Citizen (CDM metrics): https://www.digitalcitizen.life/crystaldiskmark-metrics-meaning/
- Hardwareluxx thread on CDM write volume: https://www.hardwareluxx.de/community/threads/crystaldiskmark-ssd-killer.1379838/
- KDiskMark: https://github.com/JonMagon/KDiskMark

Other benchmark tools
- AS SSD score formula: https://www.oscooshop.com/blogs/blogs/how-is-the-as-ssd-benchmark-score-calculated , https://ssd.userbenchmark.com/Faq/How-is-the-AS-SSD-total-score-calculated/37 , (unverified newer formula) https://ssd-tester.com/text_score.php
- PassMark disk test info: https://harddrivebenchmark.net/hdd_test_info.html
- ATTO reviews and settings (search): https://www.guru3d.com/download/download-atto-disk-benchmark/ , https://www.igorslab.de/en/kioxia-exceria-pro-g2-4tb-test-fast-cool-expensive/4/
- Anvil usage in reviews: https://www.legitreviews.com/sabrent-rocket-1tb-ssd-review-tlc-nand-flash_217101/3
- HD Tune: https://www.thessdreview.com/our-reviews/owc-mercury-extreme-pro-6g-240gb-ssd-review-%e2%80%93-hdtune-pro/
- OCCT storage test: https://overclock3d.net/news/software/occt-has-been-upgraded-with-a-dedicated-storage-test/
- fio documentation: https://fio.readthedocs.io/en/latest/fio_doc.html ; fio repo (license): https://github.com/axboe/fio
- Thomas-Krenn on fio under Windows: https://www.thomas-krenn.com/de/wiki/Fio_unter_Windows_nutzen
- Compressibility effect on SSD tests: https://www.thessdreview.com/our-reviews/corsair-force-series-gt-120gb-sata-3-ssd-review-data-types-atto-and-crystal-diskmark-testing/ , https://www.pugetsystems.com/labs/articles/ssds-advertised-vs-actual-performance-179/

Integrity tools
- f3 source: https://github.com/AltraMayor/f3 (src/libutils.c, src/f3write.c, src/f3read.c) and docs https://sources.debian.org/src/f3/9.0-1/doc/usage.rst
- h2testw description: https://www.heise.de/download/h2testw.html , https://www.ghacks.net/?p=15807 , https://www.addictivetips.com/windows-tips/h2testw-checks-damaged-usb-sd-card-for-read-and-write-errors
- ValiDrive: https://www.grc.com/validrive.htm , https://www.grc.com/validrive/ui-details.htm

Windows APIs
- File buffering (alignment): https://learn.microsoft.com/en-us/windows/win32/fileio/file-buffering
- File caching (write-through, metadata): https://learn.microsoft.com/en-us/windows/win32/fileio/file-caching
- CreateFileW (flags): https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
- SetFileValidData (privilege, VDL): https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfilevaliddata
- Raymond Chen on write-through/FUA: https://devblogs.microsoft.com/oldnewthing/20170510-00/?p=95505
- FUA and NVMe (PostgreSQL thread, OSR): https://www.postgresql.org/message-id/CA%2BhUKG%2Ba-7r4GpADsasCnuDBiqC1c31DAQQco2FayVtB9V3sQw%40mail.gmail.com , https://community.osr.com/discussion/264166
- FILE_FLAG_DELETE_ON_CLOSE semantics: https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew (flags table) and https://devblogs.microsoft.com/oldnewthing/ posts surfaced in search

SSD behavior
- TechPowerUp write-intensive / SLC cache method (example): https://www.techpowerup.com/review/wd-black-sn7100-2-tb/6.html
- AnandTech whole-drive fill: https://at-web1.www.anandtech.com/show/13633/the-samsung-860-qvo-ssd-review/2
- Thermal throttling: https://www.techpowerup.com/review/kingston-a2000-1-tb-m-2-nvme-ssd/7.html , https://at-web1.www.anandtech.com/show/9396/samsung-sm951-nvme-256gb-pcie-ssd-review/2 , https://www.netdata.cloud/guides/smartctl-disk-monitoring/smartctl-nvme-thermal-throttling/
- NVMe SMART fields (Data Units Written, WCTEMP/CCTEMP, thermal transitions): https://www.mankier.com/2/nvme_smart_log , https://www.netdata.cloud/guides/nvme/nvme-thermal-management-transitions/
- SNIA SSS PTS (preconditioning, steady state): https://www.snia.org/forums/sssi/pts/iops , https://silvertonconsulting.com/2010/07/13/snias-new-ssd-performance-test-specification/
- Write amplification and TBW: https://www.digitalcitizen.life/write-amplification-in-ssds-why-your-drive-wears-faster-than-you-think/ , https://www.netdata.cloud/guides/smartctl-disk-monitoring/smartctl-data-units-written-tbw/
- HDD random IOPS/seek: https://www.tomshardware.com/reviews/3tb-hdd-hard-drive,2982-8.html

Local code referenced (read-only): `crates/oma-win/src/storage_gate.rs` (DiskPower, DiskClass, ACTIVITY_WINDOW).

Caveats about confidence: the AS SSD "2026" formula, the 5-10x Defender slowdown figure, Anvil's score weights, and the I/O priority hint API usage were not confirmed from primary sources in this session; the CDM, DiskSpd and f3 behaviours above were read directly from their source or official wiki.
