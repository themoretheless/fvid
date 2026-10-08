//! Owned legacy SBR patch construction and complex high-frequency generation.
use super::{
    Result,
    aac_sbr_grid::TimeGrid,
    aac_sbr_predictor::{Predictor, predict},
    aac_sbr_qmf::Complex,
    invalid,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Patch {
    pub source: u8,
    pub target: u8,
    pub bands: u8,
}
/// Normative patch decision flow (ISO SBR Figure 9). `sample_rate` is the
/// synthesis/output rate; kx is the high-table start after crossover.
pub fn patches(master: &[u8], kx: u8, sample_rate: u32) -> Result<Vec<Patch>> {
    if sample_rate == 0
        || master.len() < 2
        || master.len() > 65
        || master[0] < 2
        || master[0] > 32
        || master[master.len() - 1] > 64
        || master.windows(2).any(|x| x[0] >= x[1])
        || kx > 32
        || !master.contains(&kx)
        || kx == master[master.len() - 1]
    {
        return Err(invalid("invalid SBR patch geometry"));
    }
    let k0 = i32::from(master[0]);
    let end = i32::from(master[master.len() - 1]);
    let goal = (2_048_000u64 + u64::from(sample_rate) / 2) / u64::from(sample_rate);
    let mut k = if goal < end as u64 {
        master.iter().position(|&x| u64::from(x) >= goal).unwrap()
    } else {
        master.len() - 1
    };
    let mut msb = k0;
    let mut usb = i32::from(kx);
    let mut result = Vec::new();
    let mut visited = std::collections::HashSet::new();
    loop {
        if !visited.insert((msb, usb, k)) {
            return Err(invalid("non-progressing SBR patch geometry"));
        }
        let (sb, odd) = (0..=k)
            .rev()
            .find_map(|j| {
                let sb = i32::from(master[j]);
                let odd = (sb - 2 + k0) % 2;
                (sb <= k0 - 1 + msb - odd).then_some((sb, odd))
            })
            .ok_or_else(|| invalid("no SBR patch source boundary"))?;
        let count = (sb - usb).max(0);
        if count > 0 {
            let source = k0 - odd - count;
            if source < 0 || source + count > k0 {
                return Err(invalid("SBR patch source exceeds low bands"));
            }
            result.push(Patch {
                source: source as u8,
                target: usb as u8,
                bands: count as u8,
            });
            usb = sb;
            msb = sb;
        } else {
            msb = i32::from(kx);
        }
        if sb == i32::from(master[k]) {
            k = master.len() - 1;
        }
        if sb == end {
            break;
        }
    }
    if result.len() > 1 && result.last().unwrap().bands < 3 {
        result.pop();
    }
    if result.is_empty() {
        return Err(invalid("empty SBR patch geometry"));
    }
    Ok(result)
}
fn product(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re * b.re - a.im * b.im,
        im: a.re * b.im + a.im * b.re,
    }
}
/// `low` begins with two predecessor slots; its remaining `2*slots+6` slots
/// begin at tHFAdj. Output uses that same origin and contains only generated
/// high bands. Non-patched bands and slots outside the envelope remain zero,
/// including the optional discarded final patch of one or two bands.
pub fn generate(
    low: &[[Complex; 32]],
    slots: u8,
    grid: &TimeGrid,
    k0: u8,
    patch_list: &[Patch],
    noise: &[u8],
    bandwidth: &[f64],
) -> Result<Vec<[Complex; 64]>> {
    if !matches!(slots, 15 | 16)
        || low.len() != 2 * usize::from(slots) + 8
        || !(2..=32).contains(&k0)
        || noise.len() < 2
        || noise.len() > 6
        || noise[0] < k0
        || noise[noise.len() - 1] > 64
        || noise.windows(2).any(|x| x[0] >= x[1])
        || bandwidth.len() + 1 != noise.len()
        || bandwidth
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        || grid.envelope.len() < 2
        || grid.envelope.windows(2).any(|x| x[0] >= x[1])
        || usize::from(*grid.envelope.last().unwrap()) > usize::from(slots) + 3
    {
        return Err(invalid("invalid SBR HF frame geometry"));
    }
    if low
        .iter()
        .flatten()
        .any(|x| !x.re.is_finite() || !x.im.is_finite())
    {
        return Err(invalid("non-finite SBR HF input"));
    }
    let mut end = noise[0];
    if patch_list.is_empty() || patch_list.len() > 64 {
        return Err(invalid("invalid SBR HF patch count"));
    }
    for patch in patch_list {
        if patch.bands == 0
            || patch.target != end
            || u16::from(patch.source) + u16::from(patch.bands) > u16::from(k0)
            || u16::from(patch.target) + u16::from(patch.bands) > u16::from(*noise.last().unwrap())
            || (i16::from(patch.target) - i16::from(patch.source)) % 2 != 0
        {
            return Err(invalid("invalid SBR HF patch mapping"));
        }
        end = patch.target + patch.bands;
    }
    if noise[noise.len() - 1] - end > 2 {
        return Err(invalid("incomplete SBR HF patch coverage"));
    }
    let mut predictors = [None::<Predictor>; 32];
    let mut output = vec![[Complex::default(); 64]; low.len() - 2];
    let start = 2 * usize::from(grid.envelope[0]);
    let stop = 2 * usize::from(*grid.envelope.last().unwrap());
    for patch in patch_list {
        for band in 0..patch.bands {
            let source = usize::from(patch.source + band);
            let target = usize::from(patch.target + band);
            let g = noise
                .windows(2)
                .position(|b| usize::from(b[0]) <= target && target < usize::from(b[1]))
                .unwrap();
            let bw = bandwidth[g];
            let predictor = if bw == 0.0 {
                Predictor::default()
            } else if let Some(p) = predictors[source] {
                p
            } else {
                let samples: Vec<_> = low.iter().map(|slot| slot[source]).collect();
                let p = predict(&samples, slots)?;
                predictors[source] = Some(p);
                p
            };
            for t in start..stop {
                let current = low[t + 2][source];
                let first = product(predictor.first, low[t + 1][source]);
                let second = product(predictor.second, low[t][source]);
                let value = Complex {
                    re: current.re + bw * first.re + bw * bw * second.re,
                    im: current.im + bw * first.im + bw * bw * second.im,
                };
                if !value.re.is_finite() || !value.im.is_finite() {
                    return Err(invalid("SBR HF arithmetic overflow"));
                }
                output[t][target] = value;
            }
        }
    }
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_geometry_preserves_even_phase_and_bounded_sources() {
        let mut accepted = 0;
        for rate in [
            16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000,
        ] {
            for k0 in 2..=32 {
                for end in k0 + 1..=64 {
                    let master: Vec<_> = (k0..=end).collect();
                    for kx in [k0, (k0 + 2).min(32).min(end - 1)] {
                        if let Ok(list) = patches(&master, kx, rate) {
                            accepted += 1;
                            let mut next = kx;
                            for p in list {
                                assert_eq!(p.target, next);
                                assert!(p.bands > 0);
                                assert!(p.source + p.bands <= k0);
                                assert_eq!((p.target - p.source) % 2, 0);
                                next += p.bands;
                            }
                            assert!(end - next <= 2);
                        }
                    }
                }
            }
        }
        assert!(accepted > 10000);
        assert_eq!(
            patches(&(10..=28).collect::<Vec<_>>(), 10, 48000).unwrap(),
            vec![
                Patch {
                    source: 2,
                    target: 10,
                    bands: 8
                },
                Patch {
                    source: 2,
                    target: 18,
                    bands: 8
                }
            ]
        );
        for master in [vec![], vec![0, 4], vec![5, 5], vec![10, 65], vec![33, 40]] {
            assert!(patches(&master, 10, 48000).is_err());
        }
        assert!(patches(&[10, 20], 10, 0).is_err());
        assert!(patches(&[10, 20], 11, 48000).is_err());
    }
    #[test]
    fn independent_decimal_hf_trace_and_copy_mode() {
        let data =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-sbr-hf-decimal.f64le");
        let mut offset = 0;
        let read = |offset: &mut usize| {
            let x = f64::from_le_bytes(data[*offset..*offset + 8].try_into().unwrap());
            *offset += 8;
            x
        };
        for slots in [15, 16] {
            let low: Vec<[Complex; 32]> = (0..2 * slots + 8)
                .map(|_| {
                    std::array::from_fn(|_| Complex {
                        re: read(&mut offset),
                        im: read(&mut offset),
                    })
                })
                .collect();
            let grid = TimeGrid {
                envelope: vec![1, slots + 2],
                noise: vec![1, slots + 2],
            };
            let list = [
                Patch {
                    source: 2,
                    target: 10,
                    bands: 8,
                },
                Patch {
                    source: 2,
                    target: 18,
                    bands: 8,
                },
            ];
            let result =
                generate(&low, slots, &grid, 10, &list, &[10, 17, 28], &[0.6, 0.9]).unwrap();
            for row in &result {
                for x in row {
                    let re = read(&mut offset);
                    let im = read(&mut offset);
                    assert!((x.re - re).abs() < 2e-11);
                    assert!((x.im - im).abs() < 2e-11);
                }
            }
            let copy = generate(&low, slots, &grid, 10, &list, &[10, 17, 28], &[0.0, 0.0]).unwrap();
            for p in list {
                for band in 0..p.bands {
                    for t in 2..2 * usize::from(slots + 2) {
                        assert_eq!(
                            copy[t][usize::from(p.target + band)],
                            low[t + 2][usize::from(p.source + band)]
                        );
                    }
                }
            }
            assert!(result[0].iter().all(|x| *x == Complex::default()));
            assert!(
                result
                    .iter()
                    .all(|r| r[26..].iter().all(|x| *x == Complex::default()))
            );
            assert!(
                generate(
                    &low,
                    slots,
                    &grid,
                    10,
                    &list,
                    &[10, 17, 28],
                    &[f64::NAN, 0.9]
                )
                .is_err()
            );
            assert!(generate(&low, slots, &grid, 10, &list, &[10, 17, 29], &[0.6, 0.9]).is_err());
            let mut bad = list;
            bad[1].source = 1;
            assert!(generate(&low, slots, &grid, 10, &bad, &[10, 17, 28], &[0.6, 0.9]).is_err());
            assert!(
                generate(
                    &low[..low.len() - 1],
                    slots,
                    &grid,
                    10,
                    &list,
                    &[10, 17, 28],
                    &[0.6, 0.9]
                )
                .is_err()
            );
        }
        assert_eq!(offset, data.len());
    }
}
