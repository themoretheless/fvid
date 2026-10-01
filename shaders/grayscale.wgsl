// Player RGB shader: preserve perceived brightness after colour management.
fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let luma = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
    return vec3(luma);
}
