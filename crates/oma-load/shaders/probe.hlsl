// Writes a known constant (PROBE_VALUE in gpu::shaders): the smallest round trip through a
// submission, for the tests.
RWStructuredBuffer<uint> outb : register(u0);
[numthreads(1, 1, 1)]
void main() { outb[0] = 0x0A11C0DEu; }
