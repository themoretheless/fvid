//! Complex QMF application of the interpolated M0/M1/M2 coefficient planes.
//! Tree construction, decorrelator routing and residual decoding are separate.
use super::{
    Result,
    aac_ld_mps_matrix_time::{Layout, MatrixFrame},
    aac_ld_mps_prepare::qmf_map,
    aac_sbr_qmf::Complex,
    invalid,
};

/// Samples indexed [slot][channel][QMF bin].
pub type Samples = Vec<Vec<Vec<Complex>>>;

/// The coefficient frame uses [band][plane][row][column] ordering. Selecting
/// no imaginary plane implements a real matrix (M0 and ordinary M2).
/// LD-MPS uses native QMF bins, without hybrid-band conjugation signs.
pub fn apply(
    frame: &MatrixFrame,
    layout: Layout,
    parameter_bands: u8,
    qmf_bands: usize,
    real_plane: u8,
    imaginary_plane: Option<u8>,
    input: &[Vec<Vec<Complex>>],
) -> Result<Samples> {
    let map = qmf_map(parameter_bands, qmf_bands)?;
    let rows = usize::from(layout.rows);
    let columns = usize::from(layout.columns);
    if !(1..=7).contains(&layout.planes)
        || !(1..=8).contains(&rows)
        || !(1..=13).contains(&columns)
        || real_plane >= layout.planes
        || imaginary_plane.is_some_and(|p| p >= layout.planes || p == real_plane)
        || input.len() > 32
        || frame.slots.len() != input.len()
    {
        return Err(invalid("invalid LD-MPS QMF matrix geometry"));
    }
    let plane_width = rows * columns;
    let band_width = usize::from(layout.planes) * plane_width;
    let size = usize::from(parameter_bands) * band_width;
    // Validate all slots before calculating any output; unused planes also
    // belong to this frame and must not hide invalid coefficients.
    if frame
        .slots
        .iter()
        .any(|s| s.len() != size || s.iter().any(|v| !v.is_finite()))
        || input.iter().any(|s| {
            s.len() != columns
                || s.iter().any(|c| {
                    c.len() != qmf_bands || c.iter().any(|v| !v.re.is_finite() || !v.im.is_finite())
                })
        })
    {
        return Err(invalid("invalid LD-MPS QMF matrix coefficients or samples"));
    }
    let mut output = Vec::with_capacity(input.len());
    for (slot, coefficients) in input.iter().zip(&frame.slots) {
        let mut result = vec![vec![Complex::default(); qmf_bands]; rows];
        for (row, channel) in result.iter_mut().enumerate() {
            for (bin, value) in channel.iter_mut().enumerate() {
                let base = usize::from(map[bin]) * band_width + row * columns;
                for (column, source) in slot.iter().enumerate() {
                    let re = coefficients[base + usize::from(real_plane) * plane_width + column];
                    let im = imaginary_plane.map_or(0.0, |p| {
                        coefficients[base + usize::from(p) * plane_width + column]
                    });
                    value.re += re * source[bin].re - im * source[bin].im;
                    value.im += re * source[bin].im + im * source[bin].re;
                }
                if !value.re.is_finite() || !value.im.is_finite() {
                    return Err(invalid("overflowing LD-MPS QMF matrix output"));
                }
            }
        }
        output.push(result);
    }
    Ok(output)
}
