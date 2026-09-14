// Pitched NV12 crop / hflip / vflip. One thread per output luma pixel.
extern "C" __global__ void fvid_nv12_transform(
    const unsigned char* src_y,
    const unsigned char* src_uv,
    unsigned char* dst_y,
    unsigned char* dst_uv,
    const unsigned int* params)
{
    // params: src_pitch_y, src_pitch_uv, dst_pitch_y, dst_pitch_uv,
    //         crop_x, crop_y, out_w, out_h, hflip, vflip
    const unsigned int out_w = params[6];
    const unsigned int out_h = params[7];
    const unsigned int x = blockIdx.x * blockDim.x + threadIdx.x;
    const unsigned int y = blockIdx.y * blockDim.y + threadIdx.y;
    if (x >= out_w || y >= out_h) return;

    const unsigned int src_pitch_y = params[0];
    const unsigned int src_pitch_uv = params[1];
    const unsigned int dst_pitch_y = params[2];
    const unsigned int dst_pitch_uv = params[3];

    unsigned int sx = x;
    unsigned int sy = y;
    if (params[8]) sx = out_w - 1u - sx;
    if (params[9]) sy = out_h - 1u - sy;
    sx += params[4];
    sy += params[5];

    dst_y[y * dst_pitch_y + x] = src_y[sy * src_pitch_y + sx];

    if (((x | y) & 1u) == 0u) {
        const unsigned int dst_i = (y >> 1) * dst_pitch_uv + x;
        const unsigned int src_i = (sy >> 1) * src_pitch_uv + ((sx >> 1) << 1);
        dst_uv[dst_i] = src_uv[src_i];
        dst_uv[dst_i + 1u] = src_uv[src_i + 1u];
    }
}
