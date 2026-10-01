// One 3x3 box pass on luma, clamping neighbours at the transformed plane edges.
fn process_byte(value: u32, plane: u32, x: u32, y: u32) -> u32 {
    if plane != 0u { return value; }
    var total = 0u;
    for (var dy = -1; dy <= 1; dy += 1) {
        for (var dx = -1; dx <= 1; dx += 1) {
            total += sample(plane, i32(x) + dx, i32(y) + dy);
        }
    }
    return total / 9u;
}
