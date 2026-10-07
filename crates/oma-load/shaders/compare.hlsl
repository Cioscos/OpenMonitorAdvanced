// Compares the output of S1 or S2 with the golden copy, one 16-byte element per thread
// (plan DG5). The counters (32 bytes, reset to COUNTERS_RESET) accumulate until the CPU
// reads them:
//   [0] words that differ, [1] lowest differing word index,
//   [2] word index of the first mismatch found, [3] its golden word, [4] its output word,
//   [5] the submission it was found in; [6] and [7] are unused.
cbuffer P : register(b0) { uint elements; uint submission; uint pad0; uint pad1; };
StructuredBuffer<uint4> outb : register(t0);
StructuredBuffer<uint4> golden : register(t1);
RWStructuredBuffer<uint> counters : register(u0);
[numthreads(256, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= elements) return;
    uint4 a = outb[id.x];
    uint4 g = golden[id.x];
    uint4 d = (uint4)(a != g);
    uint n = d.x + d.y + d.z + d.w;
    if (n == 0) return;
    uint lane = d.x ? 0 : (d.y ? 1 : (d.z ? 2 : 3));
    uint word = id.x * 4 + lane;
    uint before;
    InterlockedAdd(counters[0], n, before);
    InterlockedMin(counters[1], word);
    // Only the very first mismatch since the reset sees 0: its record stays consistent.
    if (before == 0) {
        counters[2] = word;
        counters[3] = g[lane];
        counters[4] = a[lane];
        counters[5] = submission;
    }
}
