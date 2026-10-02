
/// Reconstruct one ordinary spectral band using its accumulated scalefactor.
/// Output is in AAC spectral units, not normalized PCM. Transform/PCM scaling
/// must be applied separately by the packet decoder. Failure preserves output.
pub fn inverse_quantize(quantized: &[i16], scalefactor: i16, output: &mut [f32]) -> Result<()> {
    if quantized.len() != output.len() || !(0..=255).contains(&scalefactor) {
        return Err(invalid("invalid AAC spectral band size or scalefactor"));
    }
    // Escape magnitudes reach 8191; four pulse corrections can add 60.
    if quantized.iter().any(|&q| i32::from(q).abs() > 8251) {
        return Err(invalid("AAC quantized coefficient exceeds escape range"));
    }
    let scale = 2.0f64.powf((f64::from(scalefactor) - 100.0) / 4.0);
    for (&q, value) in quantized.iter().zip(output) {
        let magnitude = f64::from(q).abs();
        *value = (f64::from(q).signum() * magnitude * magnitude.cbrt() * scale) as f32;
    }
    Ok(())
}
