// Byte-exact YUV gather and compute write on storage buffers (UMA / Metal / Vulkan / D3D12).
struct Parameters { values: array<vec4<u32>, 8> }
@group(0) @binding(0) var<storage, read> input_words: array<u32>;
@group(0) @binding(1) var<storage, read_write> output_words: array<u32>;
@group(0) @binding(2) var<uniform> params: Parameters;
fn parameter(index: u32) -> u32 { return params.values[index / 4u][index % 4u]; }

fn gather(output_index: u32) -> u32 {
    if output_index >= parameter(27u) { return 0u; }
    var plane = 0u;
    if output_index >= parameter(9u) { plane = 1u; }
    if output_index >= parameter(17u) { plane = 2u; }
    let base = plane * 8u;
    let width = parameter(base + 5u);
    let height = parameter(base + 6u);
    let offset = output_index - parameter(base + 1u);
    var x = offset % width;
    var y = offset / width;
    if parameter(24u) != 0u { x = width - 1u - x; }
    if parameter(25u) != 0u { y = height - 1u - y; }
    let input_index = parameter(base) + (parameter(base + 4u) + y) * parameter(base + 2u) + parameter(base + 3u) + x;
    let word = input_words[input_index / 4u];
    return (word >> ((input_index % 4u) * 8u)) & 0xffu;
}

@compute @workgroup_size(64)
fn compute_main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let word_index = id.y * (groups.x * 64u) + id.x;
    let index = word_index * 4u;
    if index >= parameter(27u) { return; }
    let b0 = gather(index);
    let b1 = gather(index + 1u);
    let b2 = gather(index + 2u);
    let b3 = gather(index + 3u);
    output_words[word_index] = b0 | (b1 << 8u) | (b2 << 16u) | (b3 << 24u);
}
