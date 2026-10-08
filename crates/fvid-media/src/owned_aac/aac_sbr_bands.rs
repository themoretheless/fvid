//! Owned SBR master/envelope/noise frequency geometry.
//!
//! ISO/IEC 14496-3:2001/Amd.1:2003, 4.6.18.3.2. The caller supplies the
//! rate-dependent k0/k2 bounds; this module does not claim SBR PCM decoding.
use super::{Result, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrequencyTables {
    pub master: Vec<u8>,
    pub high: Vec<u8>,
    pub low: Vec<u8>,
    pub noise: Vec<u8>,
}

// Construct independently rounded geometric borders, then order their widths.
// Sorting widths, rather than borders, is part of the normative band grouping.
fn geometric_widths(start: u8, stop: u8, count: usize) -> Result<Vec<u8>> {
    if count == 0 || count > 64 {
        return Err(invalid("invalid SBR geometric band count"));
    }
    let ratio = f64::from(stop) / f64::from(start);
    let mut previous = start;
    let mut widths = Vec::with_capacity(count);
    for i in 1..=count {
        let border = if i == count {
            stop
        } else {
            (f64::from(start) * ratio.powf(i as f64 / count as f64)).round() as u8
        };
        let width = border
            .checked_sub(previous)
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid("zero or descending SBR geometric band"))?;
        widths.push(width);
        previous = border;
    }
    widths.sort_unstable();
    Ok(widths)
}

impl FrequencyTables {
    /// Build all three derived tables from a complete master table. Bounds must
    /// already satisfy the output-rate-specific bandwidth constraints. Header
    /// frequency_scale is 0..=3, crossover is five bits, noise_bands is 0..=3.
    pub fn from_qmf_bounds(
        k0: u8,
        k2: u8,
        frequency_scale: u8,
        alter_scale: bool,
        crossover: u8,
        noise_bands: u8,
    ) -> Result<Self> {
        if k0 == 0
            || k0 >= k2
            || k2 > 64
            || frequency_scale > 3
            || crossover > 31
            || noise_bands > 3
        {
            return Err(invalid("invalid SBR frequency table parameters"));
        }
        let mut widths;
        if frequency_scale == 0 {
            let span = usize::from(k2 - k0);
            let step = if alter_scale { 2usize } else { 1usize };
            let count = if alter_scale {
                2 * ((span + 2) / 4)
            } else {
                2 * (span / 2)
            };
            if count == 0 {
                return Err(invalid("empty SBR master table"));
            }
            widths = vec![step as u8; count];
            let difference = span as isize - (count * step) as isize;
            if difference > 0 {
                widths[count - 1] += difference as u8;
            } else {
                for width in widths.iter_mut().take((-difference) as usize) {
                    *width -= 1;
                }
            }
        } else {
            let bands = f64::from(14 - 2 * frequency_scale);
            let split = u32::from(k2) * 10_000 > u32::from(k0) * 22_449;
            let middle = if split { k0 * 2 } else { k2 };
            let count =
                (bands * (f64::from(middle) / f64::from(k0)).log2() / 2.0).round() as usize * 2;
            widths = geometric_widths(k0, middle, count)?;
            if split {
                let warp = if alter_scale { 1.3 } else { 1.0 };
                let count = (bands * (f64::from(k2) / f64::from(middle)).log2() / (2.0 * warp))
                    .round() as usize
                    * 2;
                let mut upper = geometric_widths(middle, k2, count)?;
                let lower_max = *widths.last().unwrap();
                let upper_min = upper[0];
                if upper_min < lower_max {
                    let last = upper.len() - 1;
                    let change = (lower_max - upper_min).min((upper[last] - upper_min) / 2);
                    upper[0] += change;
                    upper[last] -= change;
                    upper.sort_unstable();
                }
                widths.extend(upper);
            }
        }
        if widths.len() > 64 || usize::from(crossover) >= widths.len() {
            return Err(invalid("SBR crossover exceeds master table"));
        }
        let mut master = Vec::with_capacity(widths.len() + 1);
        master.push(k0);
        for width in widths {
            let border = master
                .last()
                .unwrap()
                .checked_add(width)
                .filter(|n| *n <= k2)
                .ok_or_else(|| invalid("SBR master border exceeds bounds"))?;
            master.push(border);
        }
        if master.last() != Some(&k2) {
            return Err(invalid("SBR master table does not cover stop band"));
        }
        let high = master[usize::from(crossover)..].to_vec();
        let kx = high[0];
        if kx > 32 {
            return Err(invalid("SBR crossover is above core QMF range"));
        }
        let high_count = high.len() - 1;
        let low_count = high_count.div_ceil(2);
        let mut low = Vec::with_capacity(low_count + 1);
        low.push(kx);
        for i in 1..=low_count {
            low.push(high[2 * i - high_count % 2]);
        }
        let count = (f64::from(noise_bands) * (f64::from(k2) / f64::from(kx)).log2())
            .round()
            .max(1.0) as usize;
        if count > 5 || count > low_count {
            return Err(invalid("SBR noise band count exceeds limits"));
        }
        let mut noise = Vec::with_capacity(count + 1);
        noise.push(kx);
        let mut index = 0;
        for i in 1..=count {
            index += (low_count - index) / (count + 1 - i);
            noise.push(low[index]);
        }
        Ok(Self {
            master,
            high,
            low,
            noise,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logarithmic_and_derived_tables_match_high_precision_decimal_oracles() {
        let manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-frequency-oracles.json"
        ))
        .unwrap();
        for case in manifest["vectors"].as_array().unwrap() {
            let p = &case["parameters"];
            let result = FrequencyTables::from_qmf_bounds(
                p[0].as_u64().unwrap() as u8,
                p[1].as_u64().unwrap() as u8,
                p[2].as_u64().unwrap() as u8,
                p[3].as_bool().unwrap(),
                p[4].as_u64().unwrap() as u8,
                p[5].as_u64().unwrap() as u8,
            );
            let expected = &case["expected"];
            if expected.is_null() {
                assert!(result.is_err(), "{p}");
            } else {
                let t = result.unwrap_or_else(|error| panic!("{p}: {error}"));
                assert_eq!(
                    serde_json::json!([t.master, t.high, t.low, t.noise]),
                    *expected,
                    "{p}"
                );
            }
        }
    }
    #[test]
    fn exact_linear_adjustments_and_odd_high_projection() {
        let t = FrequencyTables::from_qmf_bounds(17, 24, 0, false, 1, 0).unwrap();
        assert_eq!(t.master, [17, 18, 19, 20, 21, 22, 24]);
        assert_eq!(t.high, [18, 19, 20, 21, 22, 24]);
        assert_eq!(t.low, [18, 19, 21, 24]);
        assert_eq!(t.noise, [18, 24]);
        for (end, expected) in [
            (23, vec![17, 18, 19, 21, 23]),
            (24, vec![17, 18, 20, 22, 24]),
            (25, vec![17, 19, 21, 23, 25]),
            (26, vec![17, 19, 21, 23, 26]),
        ] {
            assert_eq!(
                FrequencyTables::from_qmf_bounds(17, end, 0, true, 0, 0)
                    .unwrap()
                    .master,
                expected
            );
        }
    }
    #[test]
    fn all_qmf_bounds_and_header_fields_are_bounded() {
        let mut accepted = 0;
        for k0 in 0..=65 {
            for k2 in 0..=65 {
                for scale in 0..=4 {
                    for alter in [false, true] {
                        for crossover in 0..=32 {
                            for count in 0..=4 {
                                if let Ok(t) = FrequencyTables::from_qmf_bounds(
                                    k0, k2, scale, alter, crossover, count,
                                ) {
                                    accepted += 1;
                                    assert!(
                                        k0 > 0
                                            && k0 < k2
                                            && k2 <= 64
                                            && scale <= 3
                                            && crossover <= 31
                                            && count <= 3
                                    );
                                    assert_eq!(t.master.first(), Some(&k0));
                                    assert_eq!(t.high, t.master[usize::from(crossover)..]);
                                    for table in [&t.master, &t.high, &t.low, &t.noise] {
                                        assert_eq!(table.last(), Some(&k2));
                                        assert!(
                                            table.len() <= 65
                                                && table.windows(2).all(|p| p[0] < p[1])
                                        );
                                    }
                                    assert!(t.high[0] <= 32 && t.noise.len() <= 6);
                                    assert!(t.low.iter().all(|b| t.high.contains(b)));
                                    assert!(t.noise.iter().all(|b| t.low.contains(b)));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(accepted > 10_000, "{accepted}");
    }
}
