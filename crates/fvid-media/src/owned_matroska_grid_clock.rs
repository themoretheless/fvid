//! Exact dyadic track-grid comparisons, without floating point clock arithmetic.
use std::cmp::Ordering;

/// Original block coordinates plus the integer-ns displacement of a lace frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketGrid {
    pub cluster: u64,
    pub relative: i32,
    pub offset_ns: i128,
}
#[derive(Clone, Copy, Default)]
struct Wide([u64; 4]);
impl Wide {
    fn from_u128(v: u128) -> Self {
        Self([v as u64, (v >> 64) as u64, 0, 0])
    }
    fn zero(self) -> bool {
        self.0.iter().all(|&v| v == 0)
    }
    fn cmp(self, other: Self) -> Ordering {
        for i in (0..4).rev() {
            let c = self.0[i].cmp(&other.0[i]);
            if c != Ordering::Equal {
                return c;
            }
        }
        Ordering::Equal
    }
    fn bits(self) -> i32 {
        for i in (0..4).rev() {
            if self.0[i] != 0 {
                return (i * 64 + 64 - self.0[i].leading_zeros() as usize) as i32;
            }
        }
        0
    }
    fn mul(self, v: u64) -> Self {
        let mut out = [0; 4];
        let mut carry = 0u128;
        for (i, o) in out.iter_mut().enumerate() {
            let n = u128::from(self.0[i]) * u128::from(v) + carry;
            *o = n as u64;
            carry = n >> 64;
        }
        debug_assert_eq!(carry, 0);
        Self(out)
    }
    fn add(self, other: Self) -> Self {
        let mut out = [0; 4];
        let mut carry = 0u128;
        for (i, o) in out.iter_mut().enumerate() {
            let n = u128::from(self.0[i]) + u128::from(other.0[i]) + carry;
            *o = n as u64;
            carry = n >> 64;
        }
        debug_assert_eq!(carry, 0);
        Self(out)
    }
    fn sub(self, other: Self) -> Self {
        debug_assert!(self.cmp(other) != Ordering::Less);
        let mut out = [0; 4];
        let mut borrow = false;
        for (i, o) in out.iter_mut().enumerate() {
            let (n, a) = self.0[i].overflowing_sub(other.0[i]);
            let (n, b) = n.overflowing_sub(u64::from(borrow));
            *o = n;
            borrow = a || b;
        }
        debug_assert!(!borrow);
        Self(out)
    }
    fn shl(self, bits: u32) -> Self {
        debug_assert!(bits < 256);
        let mut out = [0; 4];
        let words = bits as usize / 64;
        let shift = bits % 64;
        for i in 0..4 {
            let at = i + words;
            if at < 4 {
                out[at] |= self.0[i] << shift;
                if shift != 0 && at + 1 < 4 {
                    out[at + 1] |= self.0[i] >> (64 - shift);
                }
            }
        }
        Self(out)
    }
}
#[derive(Clone, Copy)]
struct Signed {
    negative: bool,
    magnitude: Wide,
}
impl Signed {
    fn from_i128(n: i128) -> Self {
        Self {
            negative: n < 0,
            magnitude: Wide::from_u128(n.unsigned_abs()),
        }
    }
    fn add(self, negative: bool, magnitude: Wide) -> Self {
        let (negative, magnitude) = if self.negative == negative {
            (negative, self.magnitude.add(magnitude))
        } else if self.magnitude.cmp(magnitude) != Ordering::Less {
            (self.negative, self.magnitude.sub(magnitude))
        } else {
            (negative, magnitude.sub(self.magnitude))
        };
        Self {
            negative: negative && !magnitude.zero(),
            magnitude,
        }
    }
    fn cmp_dyadic(self, negative: bool, magnitude: Wide, shift: i32) -> Ordering {
        let negative = negative && !magnitude.zero();
        if self.negative != negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let result = if magnitude.zero() {
            self.magnitude.cmp(magnitude)
        } else if self.magnitude.zero() {
            Ordering::Less
        } else {
            match self.magnitude.bits().cmp(&(magnitude.bits() + shift)) {
                Ordering::Equal => {
                    if shift >= 0 {
                        self.magnitude.cmp(magnitude.shl(shift as u32))
                    } else {
                        self.magnitude.shl((-shift) as u32).cmp(magnitude)
                    }
                }
                other => other,
            }
        };
        if self.negative {
            result.reverse()
        } else {
            result
        }
    }
}

/// Test the floor/ceil block tick bracket around the cumulative coded sample.
/// The f64 factor is treated as its exact IEEE mantissa and exponent. Cluster
/// ticks are never multiplied by the track factor. No rounded quantum is used.
/// Valid input products fit in 192 bits; arbitrary shifts are compared by bit
/// length before shifting, including subnormal and maximal finite factors.
pub fn grid_sample_matches(
    current: PacketGrid,
    epoch: PacketGrid,
    samples: u64,
    rate: u32,
    scale: u64,
    factor: f64,
) -> bool {
    if rate == 0 || scale == 0 || !factor.is_finite() || factor <= 0. {
        return false;
    }
    let Some(offset) = current
        .offset_ns
        .checked_sub(epoch.offset_ns)
        .and_then(|n| n.checked_mul(i128::from(rate)))
    else {
        return false;
    };
    let Some(target) = (i128::from(samples) * 1_000_000_000).checked_sub(offset) else {
        return false;
    };
    let cluster = i128::from(current.cluster) - i128::from(epoch.cluster);
    let base = Wide::from_u128(cluster.unsigned_abs() * u128::from(scale)).mul(u64::from(rate));
    let target = Signed::from_i128(target).add(cluster >= 0, base);
    let bits = factor.to_bits();
    let exp = ((bits >> 52) & 2047) as i32;
    let mut mantissa = bits & ((1u64 << 52) - 1);
    let shift = if exp == 0 {
        -1074
    } else {
        mantissa |= 1u64 << 52;
        exp - 1023 - 52
    };
    let compare = |step: i64| {
        let relative = i64::from(current.relative) + step - i64::from(epoch.relative);
        let magnitude = Wide::from_u128(u128::from(relative.unsigned_abs()) * u128::from(scale))
            .mul(mantissa)
            .mul(u64::from(rate));
        target.cmp_dyadic(relative < 0, magnitude, shift)
    };
    compare(0) == Ordering::Equal
        || (compare(-1) == Ordering::Greater && compare(1) == Ordering::Less)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn g(cluster: u64, relative: i32) -> PacketGrid {
        PacketGrid {
            cluster,
            relative,
            offset_ns: 0,
        }
    }
    #[test]
    fn nonunit_and_cluster_shifted_ticks_use_their_exact_grid() {
        assert!(grid_sample_matches(
            g(0, 10),
            g(0, 0),
            1024,
            48000,
            1_000_000,
            2.
        ));
        assert!(grid_sample_matches(
            g(0, 11),
            g(0, 0),
            1024,
            48000,
            1_000_000,
            2.
        ));
        assert!(!grid_sample_matches(
            g(0, 9),
            g(0, 0),
            1024,
            48000,
            1_000_000,
            2.
        ));
        assert!(grid_sample_matches(
            g(1, 10),
            g(0, 0),
            1024,
            48000,
            1_000_000,
            2.
        ));
        assert!(grid_sample_matches(
            g(0, 42),
            g(0, 0),
            1024,
            48000,
            1_000_000,
            0.5
        ));
        assert!(grid_sample_matches(
            g(0, 43),
            g(0, 0),
            1024,
            48000,
            1_000_000,
            0.5
        ));
    }
    #[test]
    fn near_unit_and_third_factors_cannot_be_rounded_into_a_different_quantum() {
        assert!(grid_sample_matches(
            g(0, 63),
            g(0, 0),
            3072,
            48000,
            1_000_000,
            1.00000000001
        ));
        assert!(grid_sample_matches(
            g(0, 64),
            g(0, 0),
            3072,
            48000,
            1_000_000,
            1.00000000001
        ));
        assert!(!grid_sample_matches(
            g(0, 63),
            g(0, 0),
            3072,
            48000,
            1_000_000,
            1.
        ));
        assert!(!grid_sample_matches(
            g(0, 65),
            g(0, 0),
            3072,
            48000,
            1_000_000,
            1.
        ));
        assert!(grid_sample_matches(
            g(0, 128),
            g(0, 0),
            2048,
            48000,
            1_000_000,
            1. / 3.
        ));
    }
    #[test]
    fn lace_offsets_and_signed_coordinates_preserve_the_same_sample_point() {
        let mut current = g(0, 0);
        current.offset_ns = 21_333_334;
        assert!(grid_sample_matches(
            current,
            g(0, 0),
            1024,
            48000,
            1_000_000,
            2.
        ));
        current.offset_ns = 23_333_334;
        assert!(!grid_sample_matches(
            current,
            g(0, 0),
            1024,
            48000,
            1_000_000,
            2.
        ));
        assert!(grid_sample_matches(
            g(4, 45),
            g(7, -3),
            1024,
            48000,
            1_000_000,
            0.5
        ));
    }
    #[test]
    fn wide_cancellation_and_extreme_ieee_factors_do_not_overflow() {
        assert!(grid_sample_matches(
            g(u64::MAX, -1),
            g(u64::MAX - 1, 0),
            0,
            u32::MAX,
            u64::MAX,
            1.
        ));
        assert!(grid_sample_matches(
            g(0, 0),
            g(0, 0),
            u64::MAX,
            u32::MAX,
            u64::MAX,
            f64::MAX
        ));
        assert!(!grid_sample_matches(
            g(0, 0),
            g(0, 0),
            1,
            48000,
            1,
            f64::from_bits(1)
        ));
        assert!(!grid_sample_matches(g(0, 0), g(0, 0), 1, 0, 1, 1.));
        assert!(!grid_sample_matches(
            g(0, 0),
            g(0, 0),
            1,
            48000,
            1,
            f64::NAN
        ));
        let mut bad = g(0, 0);
        bad.offset_ns = i128::MAX;
        assert!(!grid_sample_matches(bad, g(0, 0), 1, u32::MAX, 1, 1.));
    }
}

#[cfg(test)]
mod rational_oracle_tests {
    use super::*;
    #[test]
    fn bounded_dyadic_grids_match_an_independent_i128_rational_oracle() {
        let mut cases = 0;
        let mut accepted = 0;
        for numerator in [1i128, 3, 257] {
            for denominator in [2i128, 8, 1024] {
                for scale in [1u64, 1_000_000] {
                    for cluster in [0u64, 3, 7] {
                        for anchor in [0u64, 5] {
                            for origin_relative in [-3, 0] {
                                for relative in [-30, -1, 0, 1, 21, 42, 64, 100] {
                                    for samples in [0u64, 1, 1024, 3072] {
                                        for rate in [8000u32, 44100, 48000] {
                                            for offset in [0i128, 21_333_334] {
                                                let current = PacketGrid {
                                                    cluster,
                                                    relative,
                                                    offset_ns: offset,
                                                };
                                                let epoch = PacketGrid {
                                                    cluster: anchor,
                                                    relative: origin_relative,
                                                    offset_ns: 17,
                                                };
                                                let target = (i128::from(samples) * 1_000_000_000
                                                    - (offset - 17) * i128::from(rate)
                                                    - (i128::from(cluster) - i128::from(anchor))
                                                        * i128::from(scale)
                                                        * i128::from(rate))
                                                    * denominator;
                                                let point = (i128::from(relative)
                                                    - i128::from(origin_relative))
                                                    * i128::from(scale)
                                                    * i128::from(rate)
                                                    * numerator;
                                                let quantum = i128::from(scale)
                                                    * i128::from(rate)
                                                    * numerator;
                                                let expected = target == point
                                                    || (target > point - quantum
                                                        && target < point + quantum);
                                                let actual = grid_sample_matches(
                                                    current,
                                                    epoch,
                                                    samples,
                                                    rate,
                                                    scale,
                                                    numerator as f64 / denominator as f64,
                                                );
                                                assert_eq!(
                                                    actual, expected,
                                                    "{current:?} / {epoch:?}, {samples}, {rate}, {scale}, {numerator}/{denominator}"
                                                );
                                                cases += 1;
                                                accepted += usize::from(actual);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 41472);
        assert!(accepted > 100);
    }
}
