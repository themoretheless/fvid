//! Owned Layer III MS and intensity stereo on reordered spectral coefficients.
use crate::{
    Result, invalid,
    mp3_frame::{Granule, Version},
    mp3_hybrid::Window,
    mp3_scalefactors::ScaleFactors,
    mp3_spectral::Bands,
};

fn ratios(version: Version, position: u8, width: u8, scale: bool) -> Result<Option<(f64, f64)>> {
    if version == Version::Mpeg1 {
        if position > 15 {
            return Err(invalid("invalid MPEG-1 intensity position"));
        }
        if position >= 7 {
            return Ok(None);
        }
        if position == 6 {
            return Ok(Some((1.0, 0.0)));
        }
        let t = (f64::from(position) * std::f64::consts::PI / 12.0).tan();
        Ok(Some((t / (1.0 + t), 1.0 / (1.0 + t))))
    } else {
        if width > 5
            || (width == 0 && position != 0)
            || (width > 0 && u16::from(position) > ((1u16 << width) - 1))
        {
            return Err(invalid("invalid LSF intensity position or width"));
        }
        if width > 0 && u16::from(position) == (1u16 << width) - 1 {
            return Ok(None);
        }
        let exponent = f64::from((u16::from(position) + 1) / 2) * if scale { -0.5 } else { -0.25 };
        let attenuation = 2.0f64.powf(exponent);
        Ok(Some(if position % 2 == 0 {
            (1.0, attenuation)
        } else {
            (attenuation, 1.0)
        }))
    }
}

/// Independent stereo leaves channels untouched. Joint stereo applies intensity
/// only above each right-channel nonzero bound; remaining lines use MS when set.
/// Fresh outputs make validation/overflow failures transactional for callers.
pub fn process(
    left: &[f64; 576],
    right: &[f64; 576],
    version: Version,
    rate: u32,
    left_granule: &Granule,
    right_granule: &Granule,
    sf: &ScaleFactors,
    mode: u8,
    extension: u8,
) -> Result<[[f64; 576]; 2]> {
    if mode > 2 || extension > 3 || left.iter().chain(right).any(|v| !v.is_finite()) {
        return Err(invalid("invalid Layer III stereo input"));
    }
    let bands = Bands::for_rate(rate)?;
    let ms = mode == 1 && extension & 2 != 0;
    let intensity = mode == 1 && extension & 1 != 0;
    if ms
        && (left_granule.window != right_granule.window
            || left_granule.mixed != right_granule.mixed)
    {
        return Err(invalid("Layer III MS windows must switch synchronously"));
    }
    if intensity && sf.window != right_granule.window {
        return Err(invalid("Layer III intensity scalefactor window mismatch"));
    }
    let mut positions = [None; 576];
    if intensity {
        let long_count = if right_granule.window != Window::Short {
            22
        } else if right_granule.mixed {
            bands.mixed_long_bands
        } else {
            0
        };
        let boundary = bands.long[long_count];
        if right_granule.window == Window::Short {
            for w in 0..3 {
                for band in (if right_granule.mixed { 3 } else { 0 }..13).rev() {
                    let indices = (bands.short[band]..bands.short[band + 1]).map(|f| 3 * f + w);
                    if indices.clone().any(|i| right[i] != 0.0) {
                        break;
                    }
                    let coded = band.min(11);
                    let ratio = ratios(
                        version,
                        sf.short[coded][w],
                        sf.short_bits[coded][w],
                        sf.intensity_scale,
                    )?;
                    for i in indices {
                        positions[i] = ratio;
                    }
                }
            }
        }
        // A nonzero short coefficient prevents intensity in the entire mixed
        // long prefix, irrespective of zero holes within that prefix.
        if right_granule.window != Window::Short || right[boundary..].iter().all(|&v| v == 0.0) {
            for band in (0..long_count).rev() {
                let range = bands.long[band]..bands.long[band + 1];
                if right[range.clone()].iter().any(|&v| v != 0.0) {
                    break;
                }
                let coded = band.min(20);
                let ratio = ratios(
                    version,
                    sf.long[coded],
                    sf.long_bits[coded],
                    sf.intensity_scale,
                )?;
                positions[range].fill(ratio);
            }
        }
    }
    let mut output = [*left, *right];
    for i in 0..576 {
        if let Some((l, r)) = positions[i] {
            output[0][i] = left[i] * l;
            output[1][i] = left[i] * r;
        } else if ms {
            output[0][i] = (left[i] + right[i]) * std::f64::consts::FRAC_1_SQRT_2;
            output[1][i] = (left[i] - right[i]) * std::f64::consts::FRAC_1_SQRT_2;
        }
    }
    if output.iter().flatten().any(|v| !v.is_finite()) {
        return Err(invalid("Layer III stereo output overflow"));
    }
    Ok(output)
}
