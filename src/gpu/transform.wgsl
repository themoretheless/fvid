// Byte-exact YUV gather. Integer textures avoid normalized color conversions.
struct Parameters { values: array<vec4<u32>, 8> }
@group(0) @binding(0) var input_bytes: texture_2d<u32>;
@group(0) @binding(1) var<uniform> params: Parameters;
fn parameter(index: u32) -> u32 { return params.values[index / 4u][index % 4u]; }

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(positions[index], 0.0, 1.0);
}
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
    let texel = input_index / 4u;
    let tex_width = parameter(28u);
    return textureLoad(input_bytes, vec2<i32>(i32(texel % tex_width), i32(texel / tex_width)), 0)[input_index % 4u];
}
@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<u32> {
    let index = (u32(position.y) * parameter(29u) + u32(position.x)) * 4u;
    return vec4<u32>(gather(index), gather(index + 1u), gather(index + 2u), gather(index + 3u));
}
