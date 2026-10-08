//! Current HF envelope energy for SBR adjustment (published 6.18.7.3).
use super::{
    Result, aac_sbr_bands::FrequencyTables, aac_sbr_grid::TimeGrid, aac_sbr_qmf::Complex, invalid,
};
fn valid_borders(b: &[u8]) -> bool {
    b.len() >= 2
        && b.len() <= 65
        && b[0] > 0
        && *b.last().unwrap() <= 64
        && b.windows(2).all(|x| x[0] < x[1])
}
/// Output is envelope-major, each row indexed relative to high[0]. Input
/// rows use tHFAdj as origin, as returned by aac_sbr_hf::generate.
/// Frequency interpolation measures each QMF band separately; otherwise each
/// envelope frequency band's joint time/frequency energy is replicated over
/// that band's QMF columns. Both dimensions use exclusive upper boundaries.
pub fn estimate(
    high: &[[Complex; 64]],
    slots: u8,
    grid: &TimeGrid,
    resolution: &[bool],
    tables: &FrequencyTables,
    interpolate: bool,
) -> Result<Vec<Vec<f64>>> {
    if !matches!(slots, 15 | 16)
        || high.len() != 2 * usize::from(slots) + 6
        || grid.envelope.len() < 2
        || grid.envelope.len() > 6
        || grid.envelope.windows(2).any(|x| x[0] >= x[1])
        || usize::from(*grid.envelope.last().unwrap()) > usize::from(slots) + 3
        || resolution.len() + 1 != grid.envelope.len()
        || !valid_borders(&tables.high)
        || !valid_borders(&tables.low)
        || tables.high.first() != tables.low.first()
        || tables.high.last() != tables.low.last()
        || tables.low.iter().any(|x| !tables.high.contains(x))
    {
        return Err(invalid("invalid SBR energy geometry"));
    }
    if high
        .iter()
        .flatten()
        .any(|x| !x.re.is_finite() || !x.im.is_finite())
    {
        return Err(invalid("non-finite SBR energy input"));
    }
    let kx = usize::from(tables.high[0]);
    let stop = usize::from(*tables.high.last().unwrap());
    let mut output = Vec::with_capacity(resolution.len());
    for (envelope, &fine) in resolution.iter().enumerate() {
        let time_start = 2 * usize::from(grid.envelope[envelope]);
        let time_stop = 2 * usize::from(grid.envelope[envelope + 1]);
        let mut row = vec![0.0; stop - kx];
        if interpolate {
            for band in kx..stop {
                row[band - kx] = mean_energy(&high[time_start..time_stop], band, band + 1)?;
            }
        } else {
            let borders = if fine { &tables.high } else { &tables.low };
            for b in borders.windows(2) {
                let start = usize::from(b[0]);
                let end = usize::from(b[1]);
                let energy = mean_energy(&high[time_start..time_stop], start, end)?;
                row[start - kx..end - kx].fill(energy);
            }
        }
        output.push(row);
    }
    Ok(output)
}
fn mean_energy(time: &[[Complex; 64]], start: usize, end: usize) -> Result<f64> {
    let mut scale = 0.0f64;
    for row in time {
        for x in &row[start..end] {
            scale = scale.max(x.re.abs()).max(x.im.abs());
        }
    }
    if scale == 0.0 {
        return Ok(0.0);
    }
    let mut sum = 0.0;
    for row in time {
        for x in &row[start..end] {
            let re = x.re / scale;
            let im = x.im / scale;
            sum += re * re + im * im;
        }
    }
    // Normalize before restoring amplitude: raw squares may overflow even when
    // the final mean is representable (for example a sparse strong impulse).
    let rms = (sum / ((time.len() * (end - start)) as f64)).sqrt() * scale;
    let energy = rms * rms;
    if !energy.is_finite() {
        return Err(invalid("SBR envelope energy overflow"));
    }
    Ok(energy)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tables() -> FrequencyTables {
        FrequencyTables {
            master: vec![],
            high: vec![10, 11, 14, 18, 21, 28],
            low: vec![10, 11, 18, 28],
            noise: vec![],
        }
    }
    #[test]
    fn independent_decimal_energy_traces_cover_modes_resolution_and_hf_composition() {
        let data = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-energy-decimal.f64le"
        );
        let mut offset = 0;
        let read = |offset: &mut usize| {
            let v = f64::from_le_bytes(data[*offset..*offset + 8].try_into().unwrap());
            *offset += 8;
            v
        };
        for slots in [15, 16] {
            let grid = TimeGrid {
                envelope: vec![1, 4, 9, slots + 2],
                noise: vec![1, 9, slots + 2],
            };
            for case in 0..5 {
                let high: Vec<[Complex; 64]> = (0..2 * slots + 6)
                    .map(|_| {
                        std::array::from_fn(|_| Complex {
                            re: read(&mut offset),
                            im: read(&mut offset),
                        })
                    })
                    .collect();
                let mut input = high.clone();
                if case == 4 {
                    let low: Vec<[Complex; 32]> = (0..2 * slots + 8)
                        .map(|t| {
                            std::array::from_fn(|p| Complex {
                                re: ((i32::from(t) * 17 + p as i32 * 7) % 29 - 14) as f64 / 16.0,
                                im: ((i32::from(t) * 13 + p as i32 * 11) % 31 - 15) as f64 / 32.0,
                            })
                        })
                        .collect();
                    let hf_grid = TimeGrid {
                        envelope: vec![1, slots + 2],
                        noise: vec![1, slots + 2],
                    };
                    let patches = super::super::aac_sbr_hf::patches(
                        &(10..=28).collect::<Vec<_>>(),
                        10,
                        48000,
                    )
                    .unwrap();
                    input = super::super::aac_sbr_hf::generate(
                        &low,
                        slots,
                        &hf_grid,
                        10,
                        &patches,
                        &[10, 17, 28],
                        &[0.6, 0.9],
                    )
                    .unwrap();
                }
                for interpolate in [false, true] {
                    let actual = estimate(
                        &input,
                        slots,
                        &grid,
                        &[false, true, false],
                        &tables(),
                        interpolate,
                    )
                    .unwrap();
                    assert_eq!(actual.len(), 3);
                    for row in actual {
                        assert_eq!(row.len(), 18);
                        for value in row {
                            let expected = read(&mut offset);
                            assert!(
                                (value - expected).abs() < 2e-12,
                                "slots {slots} case {case} interpolate {interpolate}: {value} != {expected}"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(offset, data.len());
    }
    #[test]
    fn exact_one_band_normalization_boundaries_and_extremes() {
        let mut high = vec![[Complex::default(); 64]; 38];
        let grid = TimeGrid {
            envelope: vec![0, 16],
            noise: vec![0, 16],
        };
        high[0][10] = Complex { re: 3.0, im: 4.0 };
        high[32][10].re = 1e100; // outside the exclusive envelope end
        let result = estimate(&high, 16, &grid, &[false], &tables(), false).unwrap();
        assert!((result[0][0] - 25.0 / 32.0).abs() < 2e-16);
        assert!(result[0][1..].iter().all(|x| *x == 0.0));
        high[0][10] = Complex { re: 2e154, im: 0.0 };
        high[32][10].re = 0.0;
        let result = estimate(&high, 16, &grid, &[true], &tables(), true).unwrap();
        assert!((result[0][0] / 1.25e307 - 1.0).abs() < 5e-16);
        high[0][10].re = f64::MAX;
        assert!(estimate(&high, 16, &grid, &[true], &tables(), true).is_err());
        high[0][10].re = 1e-160;
        let tiny = estimate(&high, 16, &grid, &[true], &tables(), true).unwrap()[0][0];
        assert!(tiny > 0.0 && tiny < 1e-320);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            high[0][10].re = bad;
            assert!(estimate(&high, 16, &grid, &[true], &tables(), true).is_err());
        }
    }
    #[test]
    fn malformed_public_geometry_is_rejected_without_panics() {
        let high = vec![[Complex::default(); 64]; 38];
        let grid = TimeGrid {
            envelope: vec![0, 16],
            noise: vec![0, 16],
        };
        for envelope in [
            vec![],
            vec![0],
            vec![0, 0],
            vec![0, 20],
            vec![0, 1, 2, 3, 4, 5, 6],
        ] {
            assert!(
                estimate(
                    &high,
                    16,
                    &TimeGrid {
                        envelope,
                        noise: vec![]
                    },
                    &[true],
                    &tables(),
                    true
                )
                .is_err()
            );
        }
        for borders in [vec![], vec![10], vec![10, 10], vec![0, 20], vec![10, 65]] {
            let mut bad = tables();
            bad.high = borders;
            assert!(estimate(&high, 16, &grid, &[true], &bad, true).is_err());
        }
        let mut bad = tables();
        bad.low = vec![10, 12, 28];
        assert!(estimate(&high, 16, &grid, &[true], &bad, false).is_err());
        assert!(estimate(&high[..37], 16, &grid, &[true], &tables(), true).is_err());
        assert!(estimate(&high, 14, &grid, &[true], &tables(), true).is_err());
        assert!(estimate(&high, 16, &grid, &[], &tables(), true).is_err());
    }
}
