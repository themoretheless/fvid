// Invert luma; retain the chroma signal.
fn process_byte(value: u32, plane: u32, x: u32, y: u32) -> u32 {
    if plane == 0u { return 255u - value; }
    return value;
}
