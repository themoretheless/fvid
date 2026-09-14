// Three tightly packed output planes. Parameters are validated by the Rust host.
// Every invocation writes exactly one output byte, without packed-word races.
extern "C" __global__ void fvid_transform(
    const unsigned char* input,
    unsigned char* output,
    const unsigned int* params)
{
    const unsigned int index = blockIdx.x * blockDim.x + threadIdx.x;
    if (index >= params[27]) return;

    unsigned int plane = index < params[9] ? 0 : (index < params[17] ? 1 : 2);
    const unsigned int* p = params + plane * 8;
    const unsigned int local = index - p[1];
    unsigned int x = local % p[5];
    unsigned int y = local / p[5];
    if (params[24]) x = p[5] - 1 - x;
    if (params[25]) y = p[6] - 1 - y;
    const unsigned int source = p[0] + (p[4] + y) * p[2] + p[3] + x;
    output[index] = input[source];
}
