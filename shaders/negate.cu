// CUDA C byte shader for fvid_cuda::CudaPipeline::with_shaders.
__device__ unsigned int process_byte(unsigned int value, unsigned int plane,
                                    unsigned int x, unsigned int y) {
    return 255u - value;
}
