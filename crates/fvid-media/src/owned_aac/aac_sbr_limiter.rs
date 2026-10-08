//! Owned legacy SBR limiter frequency borders (protocol Figure 10).
use super::{Result, aac_sbr_hf::Patch, invalid};
/// Merge low-resolution borders and internal patch borders, then remove close
/// non-patch borders. Patch borders win when only one of a close pair is a
/// patch boundary; two distinct patch boundaries are both retained.
/// Internal patch ends are candidates; all patch ends are protected. The SBR
/// range endpoints are also protected: Cor.1 gain equations define k(m) for
/// every 0<=m<M, requiring a covering partition even after a short final HF
/// patch was discarded. This resolves Figure 4.40's endpoint-removal ambiguity
/// without inventing a new patch, limiter interval, or limiter-mode fallback.
pub fn borders(low: &[u8], patches: &[Patch], mode: u8) -> Result<Vec<u8>> {
    if mode > 3
        || low.len() < 2
        || low.len() > 65
        || low[0] < 2
        || low[low.len() - 1] > 64
        || low.windows(2).any(|x| x[0] >= x[1])
        || patches.is_empty()
        || patches.len() > 64
    {
        return Err(invalid("invalid SBR limiter geometry"));
    }
    let mut patch_borders = vec![low[0]];
    let mut next = low[0];
    for patch in patches {
        let end = u16::from(patch.target) + u16::from(patch.bands);
        if patch.bands == 0
            || patch.target != next
            || end > u16::from(*low.last().unwrap())
            || u16::from(patch.source) + u16::from(patch.bands) > u16::from(low[0].min(32))
            || (i16::from(patch.target) - i16::from(patch.source)) % 2 != 0
        {
            return Err(invalid("invalid SBR limiter patch mapping"));
        }
        next = end as u8;
        patch_borders.push(next);
    }
    if low[low.len() - 1] - next > 2 {
        return Err(invalid("incomplete SBR limiter patch coverage"));
    }
    if mode == 0 {
        return Ok(vec![low[0], low[low.len() - 1]]);
    }
    let density = [1.2, 2.0, 3.0][usize::from(mode - 1)];
    let mut candidates = low.to_vec();
    candidates.extend_from_slice(&patch_borders[1..patch_borders.len() - 1]);
    candidates.sort_unstable();
    let mut i = 1;
    while i < candidates.len() {
        let previous = candidates[i - 1];
        let current = candidates[i];
        let close = (f64::from(current) / f64::from(previous)).log2() * density < 0.49;
        if !close {
            i += 1;
            continue;
        }
        let protected = |border| {
            border == low[0] || border == *low.last().unwrap() || patch_borders.contains(&border)
        };
        if current == previous || !protected(current) {
            candidates.remove(i);
        } else if !protected(previous) {
            candidates.remove(i - 1);
        } else {
            i += 1;
        }
    }
    if candidates.len() < 2 {
        return Err(invalid("SBR limiter geometry has no remaining band"));
    }
    Ok(candidates)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_high_precision_geometry_oracles_cover_all_modes_and_patch_priority() {
        let data = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-limiter-decimal.json"
        );
        let cases: serde_json::Value = serde_json::from_slice(data).unwrap();
        let rows = cases["cases"].as_array().unwrap();
        assert!(rows.len() > 1000);
        for row in rows {
            let low: Vec<_> = row["low"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as u8)
                .collect();
            let patches: Vec<_> = row["patches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| Patch {
                    source: p[0].as_u64().unwrap() as u8,
                    target: p[1].as_u64().unwrap() as u8,
                    bands: p[2].as_u64().unwrap() as u8,
                })
                .collect();
            let mode = row["mode"].as_u64().unwrap() as u8;
            if row["expected"].is_null() {
                assert!(borders(&low, &patches, mode).is_err());
                continue;
            }
            let expected: Vec<_> = row["expected"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as u8)
                .collect();
            let actual = borders(&low, &patches, mode).unwrap();
            assert_eq!(
                actual, expected,
                "low {low:?} patches {patches:?} mode {mode}"
            );
            assert_eq!(actual[0], low[0]);
            assert_eq!(actual.last(), low.last());
            assert!(actual.windows(2).all(|x| x[0] < x[1]));
            if mode > 0 {
                for p in &patches[1..] {
                    assert!(actual.contains(&p.target));
                }
            }
        }
    }
    #[test]
    fn discarded_short_tail_still_has_limiter_interval_for_every_gain_band() {
        use super::super::aac_sbr_gain::{self, Band};
        let patches = [
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
        for tail in 0..=2 {
            let end = 26 + tail;
            let low: Vec<_> = (10..end).step_by(2).chain(std::iter::once(end)).collect();
            for mode in 0..=3 {
                let borders = borders(&low, &patches, mode).unwrap();
                assert_eq!((borders[0], *borders.last().unwrap()), (10, end));
                let bands = vec![
                    Band {
                        target: 64.0,
                        current: 0.0,
                        noise_ratio: 0.25,
                        harmonic_band: false,
                        harmonic_line: false
                    };
                    usize::from(end - 10)
                ];
                let levels = aac_sbr_gain::calculate(&bands, false).unwrap();
                let output = aac_sbr_gain::limit(&bands, &levels, 10, &borders, 2, false).unwrap();
                assert_eq!(output.len(), bands.len());
                assert!(output.iter().all(|x| x.gain.is_finite() && x.noise > 0.0));
                // The old missing terminal boundary reproduces the actual
                // preparation -> gain interface error, rather than patch refusal.
                if tail > 0 {
                    assert!(aac_sbr_gain::limit(&bands, &levels, 10, &[10, 26], 2, false).is_err());
                }
            }
        }
    }
    #[test]
    fn mode_zero_and_close_distinct_patch_boundaries_are_preserved() {
        let patches = [
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
        assert_eq!(
            borders(&[10, 12, 14, 16, 18, 20, 22, 24, 26, 28], &patches, 0).unwrap(),
            vec![10, 28]
        );
        let close = [
            Patch {
                source: 0,
                target: 20,
                bands: 4,
            },
            Patch {
                source: 0,
                target: 24,
                bands: 4,
            },
            Patch {
                source: 0,
                target: 28,
                bands: 4,
            },
        ];
        assert_eq!(
            borders(&[20, 21, 24, 25, 28, 29, 32], &close, 1).unwrap(),
            vec![20, 24, 28, 32]
        );
        for low in [vec![], vec![10], vec![10, 10], vec![0, 12], vec![10, 65]] {
            assert!(borders(&low, &patches, 1).is_err());
        }
        assert!(borders(&[10, 28], &patches, 4).is_err());
        assert!(borders(&[10, 28], &[], 1).is_err());
        for patch in [
            Patch {
                source: 2,
                target: 11,
                bands: 8,
            },
            Patch {
                source: 31,
                target: 10,
                bands: 8,
            },
            Patch {
                source: 2,
                target: 10,
                bands: 0,
            },
            Patch {
                source: 2,
                target: 10,
                bands: 30,
            },
        ] {
            assert!(borders(&[10, 28], &[patch], 1).is_err());
        }
        assert!(borders(&[10, 29], &patches, 1).is_err());
    }
}
