// Deterministic off-screen scene: many alpha-blended, textured, rotated quads (overdraw).
cbuffer P : register(b0) { uint frame; uint n; uint furSteps; uint pad; };
Texture2D tex : register(t0);
SamplerState smp : register(s0);
struct V { float4 pos : SV_Position; float2 uv : TEXCOORD0; float4 col : COLOR0; };
V vs(uint vid : SV_VertexID, uint iid : SV_InstanceID) {
    float2 corner = float2((vid == 1 || vid == 4 || vid == 5) ? 1.0 : 0.0,
                           (vid == 2 || vid == 3 || vid == 5) ? 1.0 : 0.0);
    uint h = (iid + frame * 7919u) * 2654435761u;
    float2 center = float2((h & 1023u) / 1023.0, ((h >> 10) & 1023u) / 1023.0) * 2.0 - 1.0;
    float size = 0.1 + ((h >> 20) & 255u) / 255.0 * 0.5;
    float s, c;
    sincos(iid * 0.37, s, c);
    float2 p = (corner - 0.5) * size;
    V o;
    o.pos = float4(center + float2(p.x * c - p.y * s, p.x * s + p.y * c), 0.5, 1.0);
    o.uv = corner * 4.0;
    o.col = float4(((h >> 3) & 255u) / 255.0, ((h >> 11) & 255u) / 255.0, ((h >> 19) & 255u) / 255.0, 0.08);
    return o;
}
float4 ps(V i) : SV_Target {
    float4 t = tex.Sample(smp, i.uv);
    float2 uv = i.uv;
    [loop] for (uint k = 0; k < furSteps; k++) {
        uv = frac(uv * 1.7 + t.xy * 0.3);
        t = 0.5 * t + 0.5 * tex.Sample(smp, uv);
    }
    return float4(t.rgb * i.col.rgb, i.col.a);
}
