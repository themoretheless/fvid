// Input sampling in transformed output-plane coordinates. Rust validates all
// plane offsets, strides and extents before launching this kernel.
struct FvidSampler {
    const unsigned char* input;
    const unsigned int* params;
    unsigned int plane;
};
__device__ unsigned int sample(FvidSampler sampler, long long x, long long y) {
    const unsigned int* p = sampler.params + sampler.plane * 8;
    unsigned int sx = x < 0 ? 0u : (x >= (long long)p[5] ? p[5] - 1u : (unsigned int)x);
    unsigned int sy = y < 0 ? 0u : (y >= (long long)p[6] ? p[6] - 1u : (unsigned int)y);
    if (sampler.params[24]) sx = p[5] - 1u - sx;
    if (sampler.params[25]) sy = p[6] - 1u - sy;
    return sampler.input[p[0] + (p[4] + sy) * p[2] + p[3] + sx];
}
