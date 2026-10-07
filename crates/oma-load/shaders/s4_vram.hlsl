// Adapted from memtest_vulkan (https://github.com/GpuZelenograd/memtest_vulkan),
// commit fd9ff59cde85cf11e25263e3f32d7adae0ba5e3b: address-derived rotated pattern, write once and re-read in
// rotated order, bit-error statistics.
// Original work: Copyright (c) 2022 galkinvv by GpuZelenograd, licensed under the zlib
// License (see THIRD_PARTY_LICENSES.txt).
// Modified for OpenMonitor Advanced: ported to HLSL cs_5_0 and D3D11, sized
// from the DXGI video memory budget, classic passes added.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.

// S4, the VRAM check (plan DG6): one thread per 16-byte element of a window of one chunk.
// `pattern` and `op` are `gpu::vram::Pattern` and `gpu::vram::Op`. The statistics (32 bytes,
// reset to zero) accumulate until the CPU reads them:
//   [0] wrong words, [1] and [2] low and high half of the word index of the first wrong
//   word found, [3] its expected word, [4] its read word, [5] OR of the wrong bits.
cbuffer P : register(b0) {
    uint base_lo;   // word index of the chunk's first word in the whole allocation
    uint base_hi;
    uint start;     // first element of the window in the chunk
    uint count;     // elements of the window, a multiple of ROTATION
    uint key;
    uint aux;
    uint step;      // pattern | op << 8
    uint round;
};
RWByteAddressBuffer mem : register(u0);
RWStructuredBuffer<uint> stats : register(u1);

// memtest_vulkan's TEST_WINDOW_1D_MAX_GROUPS and TEST_WINDOW_READ_ADDR_ROTATION_GRANULARITY.
#define GROUPS_X 16384u
#define ROTATION 0x2000u

// memtest_vulkan's test_value_by_index, on a 64-bit word index: `gpu::vram::vram_word`.
uint vram_word(uint lo, uint hi) {
    uint a = lo + hi * 0x81u + key + 1u;
    uint s = a % 31u;
    return s == 0u ? a : (a << s) | (a >> (32u - s));
}

uint expected(uint lo, uint hi) {
    switch (step & 0xFFu) {
    case 0u: return vram_word(lo, hi);
    case 1u: return 1u << ((lo + key) & 31u);
    case 2u: return key;
    default: return ((hi % 20u) * 16u + lo % 20u) % 20u == aux ? key : ~key; // 2^32 % 20 = 16
    }
}

[numthreads(256, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint t = id.x + id.y * GROUPS_X * 256u;
    if (t >= count) return;
    uint op = step >> 8;
    if (op == 3u) {
        // Flip: thread 0 XORs word `key` of the chunk with `aux`.
        if (t == 0u) mem.Store(key * 4u, mem.Load(key * 4u) ^ aux);
        return;
    }
    uint e;
    if (op == 0u) {
        // Write in reverse order within the window, unlike the re-read (memtest_vulkan
        // mirrors within its write granularity).
        e = count - 1u - t;
    } else {
        // memtest_vulkan's rotated re-read: a permutation within each group of ROTATION
        // elements, since 11 is odd.
        uint addr_mod = t % ROTATION;
        uint new_mod = (11u * t + 999u * round + key + 7u * (t / ROTATION)) % ROTATION;
        e = t - addr_mod + new_mod;
    }
    uint elem = start + e;
    // Chunks are multiples of 128 KiB, so lo is a multiple of 4 and lo + 3 cannot carry.
    uint lo = base_lo + elem * 4u;
    uint hi = base_hi + (lo < base_lo ? 1u : 0u);
    uint4 want = uint4(expected(lo, hi), expected(lo + 1u, hi), expected(lo + 2u, hi),
                       expected(lo + 3u, hi));
    if (op == 0u) {
        mem.Store4(elem * 16u, want);
        return;
    }
    uint4 got = mem.Load4(elem * 16u);
    if (op == 2u) mem.Store4(elem * 16u, ~want);
    uint4 bad = got ^ want;
    uint4 d = (uint4)(bad != 0u);
    uint n = d.x + d.y + d.z + d.w;
    if (n == 0u) return;
    uint lane = d.x ? 0u : (d.y ? 1u : (d.z ? 2u : 3u));
    uint before;
    InterlockedAdd(stats[0], n, before);
    InterlockedOr(stats[5], bad.x | bad.y | bad.z | bad.w);
    // Only the very first wrong word since the reset sees 0: its record stays consistent.
    if (before == 0u) {
        stats[1] = lo + lane;
        stats[2] = hi;
        stats[3] = want[lane];
        stats[4] = got[lane];
    }
}
