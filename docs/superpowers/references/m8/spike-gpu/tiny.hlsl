RWStructuredBuffer<uint> outb : register(u0);
[numthreads(1, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) { outb[0] = outb[0] + 1; }
