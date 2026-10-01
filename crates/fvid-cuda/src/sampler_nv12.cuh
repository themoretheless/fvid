struct FvidSampler {
    const unsigned char* y;
    const unsigned char* uv;
    const unsigned int* params;
    unsigned int plane;
};
__device__ unsigned int sample(FvidSampler sampler, long long x, long long y) {
    const unsigned int* p = sampler.params;
    const unsigned int shift = sampler.plane == 0u ? 0u : 1u;
    const unsigned int width = p[6] >> shift, height = p[7] >> shift;
    unsigned int sx = x < 0 ? 0u : (x >= (long long)width ? width - 1u : (unsigned int)x);
    unsigned int sy = y < 0 ? 0u : (y >= (long long)height ? height - 1u : (unsigned int)y);
    if (p[8]) sx = width - 1u - sx;
    if (p[9]) sy = height - 1u - sy;
    sx += p[4] >> shift;
    sy += p[5] >> shift;
    return sampler.plane == 0u ? sampler.y[sy * p[0] + sx]
        : sampler.uv[sy * p[1] + 2u * sx + (sampler.plane - 1u)];
}
