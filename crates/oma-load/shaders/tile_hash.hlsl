// S6, the artifact scan (plan DG11): an FNV-1a 32 hash of each 16x16 tile of the render
// target, over its RGBA8 bytes row by row (pixels past the image edge are skipped), the
// same as `gpu::graphics::fnv1a32` on those bytes. Modes:
//   0: writes the hashes to `refs` (the reference frame);
//   1: compares them with `refs`; in the 4 words of `slot`: [0] tiles that differ,
//      [1] lowest differing tile (reset to 0xFFFFFFFF);
//   2: thread 0 records in [2] and [3] of `slot` the reference and the frame's hash of
//      that lowest tile (run after mode 1, on the same frame);
//   3: thread 0 flips bit 0 of `refs[0]` (fault injection, DA18).
cbuffer P : register(b0) { uint mode; uint slot; uint width; uint height; };
Texture2D<float4> rt : register(t0);
RWStructuredBuffer<uint> refs : register(u0);
RWStructuredBuffer<uint> results : register(u1);
static const uint TILE = 16;

uint tile_hash(uint t, uint cols) {
    uint x0 = (t % cols) * TILE;
    uint y0 = (t / cols) * TILE;
    uint h = 2166136261u;
    [loop] for (uint y = y0; y < min(y0 + TILE, height); y++) {
        [loop] for (uint x = x0; x < min(x0 + TILE, width); x++) {
            // UNORM8 reads back as n / 255 exactly, so this gives the stored byte.
            uint4 b = (uint4)round(saturate(rt.Load(int3(x, y, 0))) * 255.0);
            h = (h ^ b.r) * 16777619u;
            h = (h ^ b.g) * 16777619u;
            h = (h ^ b.b) * 16777619u;
            h = (h ^ b.a) * 16777619u;
        }
    }
    return h;
}

[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint cols = (width + TILE - 1) / TILE;
    uint tiles = cols * ((height + TILE - 1) / TILE);
    uint t = id.x;
    if (mode == 3) {
        if (t == 0) refs[0] = refs[0] ^ 1u;
        return;
    }
    if (mode == 2) {
        uint first = results[slot * 4 + 1];
        if (t == 0 && first < tiles) {
            results[slot * 4 + 2] = refs[first];
            results[slot * 4 + 3] = tile_hash(first, cols);
        }
        return;
    }
    if (t >= tiles) return;
    uint h = tile_hash(t, cols);
    if (mode == 0) {
        refs[t] = h;
    } else if (h != refs[t]) {
        InterlockedAdd(results[slot * 4], 1u);
        InterlockedMin(results[slot * 4 + 1], t);
    }
}
