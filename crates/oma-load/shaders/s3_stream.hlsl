// S3, the memory stream (plan DH5): one thread per 16-byte element of a piece. With
// `fill` 0 it copies `src` into `dst`, like STREAM's "copy" (16 bytes read, 16 written);
// with `fill` 1 it writes the seeded pattern into `dst` (the sources, once).
cbuffer P : register(b0) {
    uint count;     // elements of the piece
    uint fill;
    uint seed;
    uint pad;
};
RWByteAddressBuffer src : register(u0);
RWByteAddressBuffer dst : register(u1);

// As in s4_vram.hlsl: pieces have more than 65535 groups of 256 threads.
#define GROUPS_X 16384u

[numthreads(256, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint t = id.x + id.y * GROUPS_X * 256u;
    if (t >= count) return;
    if (fill != 0u) {
        uint w = t * 4u + seed;
        dst.Store4(t * 16u, uint4(w, w + 1u, w + 2u, w + 3u) * 0x9E3779B1u);
        return;
    }
    dst.Store4(t * 16u, src.Load4(t * 16u));
}
