// Exact FP32: with m = -1 each step maps x -> c - x, so every value stays a small integer.
cbuffer P : register(b0) { uint iters; uint seed; float m; float pad; };
RWStructuredBuffer<float4> outb : register(u0);
[numthreads(256, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    float b = (float)((id.x ^ seed) & 255);
    float4 x0 = float4(b, b + 1, b + 2, b + 3);
    float4 x1 = x0 + 4; float4 x2 = x0 + 8; float4 x3 = x0 + 12;
    float4 c0 = float4(1000, 1001, 1002, 1003);
    float4 c1 = c0 + 4; float4 c2 = c0 + 8; float4 c3 = c0 + 12;
    [loop] for (uint i = 0; i < iters; i++) {
        x0 = mad(x0, m, c0); x1 = mad(x1, m, c1); x2 = mad(x2, m, c2); x3 = mad(x3, m, c3);
        x0 = mad(x0, m, c0); x1 = mad(x1, m, c1); x2 = mad(x2, m, c2); x3 = mad(x3, m, c3);
        x0 = mad(x0, m, c0); x1 = mad(x1, m, c1); x2 = mad(x2, m, c2); x3 = mad(x3, m, c3);
        x0 = mad(x0, m, c0); x1 = mad(x1, m, c1); x2 = mad(x2, m, c2); x3 = mad(x3, m, c3);
    }
    outb[id.x] = x0 + x1 * 2 + x2 * 4 + x3 * 8;
}
