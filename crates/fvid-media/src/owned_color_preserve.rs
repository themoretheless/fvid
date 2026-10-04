//! Shared colour preservation metrics used by owned RGB filters.
pub(crate) fn parse_mode(value: &str) -> Result<u8, String> {
    let modes = ["none", "lum", "max", "avg", "sum", "nrm", "pwr"];
    let mode = if let Some(mode) = modes.iter().position(|n| *n == value.trim()) {
        mode as f64
    } else {
        crate::owned_expression::constant(value.trim())?
    };
    if !mode.is_finite() || mode.fract() != 0.0 || !(0.0..=6.0).contains(&mode) {
        return Err("invalid color preservation mode".into());
    }
    Ok(mode as u8)
}
pub(crate) fn measure(mode: u8, rgb: [f32; 3], maximum: f32) -> f32 {
    let [r, g, b] = rgb;
    match mode {
        1 => r.max(g).max(b) + r.min(g).min(b),
        2 => r.max(g).max(b),
        3 => (r + g + b + 1.0) / 3.0,
        4 => r + g + b,
        5 => {
            let [r, g, b] = rgb.map(|v| v / maximum);
            (r * r + g * g + b * b).sqrt()
        }
        6 => {
            let [r, g, b] = rgb.map(|v| v / maximum);
            (r * r * r + g * g * g + b * b * b).cbrt()
        }
        _ => 0.0,
    }
}
