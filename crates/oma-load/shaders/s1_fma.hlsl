// S1, FP32 FMA chains (plan DG5), as in the M8b spike. With m = -1 each step maps
// x -> c - x, so every value stays a small integer and the result is exact: the CPU
// reference is `gpu::reference::fma_thread`.
cbuffer P : register(b0) { uint steps; uint seed; float m; uint pad; };
RWStructuredBuffer<float4> outb : register(u0);
[numthreads(256, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    float b = (float)((id.x ^ seed) & 255);
    float4 x0 = float4(b, b + 1, b + 2, b + 3);
    float4 x1 = x0 + 4; float4 x2 = x0 + 8; float4 x3 = x0 + 12;
    float4 c0 = float4(1000, 1001, 1002, 1003);
    float4 c1 = c0 + 4; float4 c2 = c0 + 8; float4 c3 = c0 + 12;
    [loop] for (uint i = 0; i < steps; i++) {
        x0 = mad(x0, m, c0); x1 = mad(x1, m, c1); x2 = mad(x2, m, c2); x3 = mad(x3, m, c3);
        x0 = mad(x0, m, c0); x1 = mad(x1, m, c1); x2 = mad(x2, m, c2); x3 = mad(x3, m, c3);
        x0 = mad(x0, m, c0); x1 = mad(x1, m, c1); x2 = mad(x2, m, c2); x3 = mad(x3, m, c3);
        x0 = mad(x0, m, c0); x1 = mad(x1, m, c1); x2 = mad(x2, m, c2); x3 = mad(x3, m, c3);
    }
    outb[id.x] = x0 + x1 * 2 + x2 * 4 + x3 * 8;
}
