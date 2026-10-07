cbuffer P : register(b0) { uint iters; uint seed; uint k; uint pad; };
RWStructuredBuffer<uint4> outb : register(u0);
uint4 step4(uint4 h, uint4 add) {
    h = h * k;
    h ^= h >> 15;
    h = (h << 13) | (h >> 19);
    return h + add;
}
[numthreads(256, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint4 h0 = uint4(id.x, id.x ^ seed, id.x * 3 + 1, id.x + seed);
    uint4 h1 = h0 ^ 0x9E3779B9u; uint4 h2 = h0 + 0x85EBCA6Bu; uint4 h3 = h0 * 0xC2B2AE35u;
    [loop] for (uint i = 0; i < iters; i++) {
        h0 = step4(h0, uint4(1, 2, 3, 4));
        h1 = step4(h1, uint4(5, 6, 7, 8));
        h2 = step4(h2, uint4(9, 10, 11, 12));
        h3 = step4(h3, uint4(13, 14, 15, 16));
    }
    outb[id.x] = h0 ^ h1 ^ h2 ^ h3;
}
