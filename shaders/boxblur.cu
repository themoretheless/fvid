// 3x3 luma blur, matching boxblur.wgsl; chroma remains unchanged.
__device__ unsigned int process_byte(unsigned int value, unsigned int plane,
                                    unsigned int x, unsigned int y,
                                    FvidSampler sampler) {
    if (plane != 0u) return value;
    unsigned int total = 0u;
    for (int dy = -1; dy <= 1; ++dy)
        for (int dx = -1; dx <= 1; ++dx)
            total += sample(sampler, (long long)x + dx, (long long)y + dy);
    return total / 9u;
}
