// The graphics loads of the GPU benchmark (plan DH2): every instance is one quad as large
// as the 1920x1080 render target, so a draw of n instances covers exactly n * 1920 * 1080
// pixels on every GPU. `textured` = 0 for `fill` and `overdraw` (a constant color; the
// blend state tells them apart), 1 for `texture` (8 independent bilinear reads per pixel).
cbuffer P : register(b0) { uint textured; uint pad0; uint pad1; uint pad2; };
Texture2D tex : register(t0);
SamplerState smp : register(s0);
struct V { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
V vs(uint vid : SV_VertexID, uint iid : SV_InstanceID) {
    float2 corner = float2((vid == 1 || vid == 4 || vid == 5) ? 1.0 : 0.0,
                           (vid == 2 || vid == 3 || vid == 5) ? 1.0 : 0.0);
    V o;
    o.pos = float4(corner * 2.0 - 1.0, 0.5, 1.0);
    // About one texel per pixel of the 1024x1024 texture, shifted per instance.
    o.uv = corner * float2(1.875, 1.0547) + (iid & 63u) * 0.0137;
    return o;
}
float4 ps(V i) : SV_Target {
    if (textured == 0) {
        return float4(0.9, 0.2, 0.6, 0.5);
    }
    // Fixed offsets, none depending on another read: the 8 fetches are independent.
    float4 t = 0;
    [unroll] for (uint k = 0; k < 8; k++) {
        t += tex.Sample(smp, i.uv + float2(k * 0.1171, k * 0.0723));
    }
    return float4(t.rgb * 0.125, 1.0);
}
