//! Dependent AAC coupling in per-window spectral order.
use super::{Result, invalid};

/// Mix grouped band gains into a target spectrum. Validation and overflow
/// failures leave the destination unchanged. Offsets include the window end.
pub fn mix_spectrum(
    source: &[f32],
    destination: &mut [f32],
    offsets: &[usize],
    groups: &[u8],
    gains: &[Vec<f32>],
) -> Result<()> {
    let size = offsets.last().copied().unwrap_or(0);
    if !matches!(source.len(), 960 | 1024)
        || destination.len() != source.len()
        || offsets.first() != Some(&0)
        || offsets.windows(2).any(|v| v[0] >= v[1])
        || groups.is_empty()
        || groups.contains(&0)
        || groups
            .iter()
            .map(|&v| usize::from(v))
            .sum::<usize>()
            .checked_mul(size)
            != Some(source.len())
        || gains.len() != groups.len()
        || gains
            .iter()
            .any(|g| g.len() >= offsets.len() || g.iter().any(|v| !v.is_finite()))
        || source
            .iter()
            .chain(destination.iter())
            .any(|v| !v.is_finite())
    {
        return Err(invalid("AAC coupling spectral geometry mismatch"));
    }
    let mut mixed = destination.to_vec();
    let mut first = 0;
    for (&length, bands) in groups.iter().zip(gains) {
        for (band, &gain) in bands.iter().enumerate() {
            for window in first..first + usize::from(length) {
                for bin in window * size + offsets[band]..window * size + offsets[band + 1] {
                    mixed[bin] += source[bin] * gain;
                    if !mixed[bin].is_finite() {
                        return Err(invalid("AAC coupling spectral overflow"));
                    }
                }
            }
        }
        first += usize::from(length);
    }
    destination.copy_from_slice(&mixed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouped_gains_preserve_uncoupled_bands_and_failed_mix_is_atomic() {
        let source = vec![1.0; 1024];
        let mut destination = vec![3.0; 1024];
        mix_spectrum(
            &source,
            &mut destination,
            &[0, 4, 8, 12, 128],
            &[4, 4],
            &[vec![-1.0, 0.0, 2.0], vec![2.0, 0.0, -1.0]],
        )
        .unwrap();
        for window in 0..8 {
            let first = window * 128;
            let expected = if window < 4 {
                [2.0, 3.0, 5.0]
            } else {
                [5.0, 3.0, 2.0]
            };
            for band in 0..3 {
                assert_eq!(
                    &destination[first + band * 4..first + (band + 1) * 4],
                    &[expected[band]; 4]
                );
            }
            assert!(
                destination[first + 12..first + 128]
                    .iter()
                    .all(|&v| v == 3.0)
            );
        }
        let saved = destination.clone();
        assert!(
            mix_spectrum(
                &source,
                &mut destination,
                &[0, 4, 128],
                &[4, 3],
                &[vec![1.0], vec![1.0]]
            )
            .is_err()
        );
        assert_eq!(destination, saved);
        let mut overflow = source;
        overflow[1023] = f32::MAX;
        assert!(mix_spectrum(&overflow, &mut destination, &[0, 1024], &[1], &[vec![2.0]]).is_err());
        assert_eq!(destination, saved);
    }
}
