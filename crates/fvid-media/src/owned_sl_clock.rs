//! Exact modular counter coordinates. Epoch and clock rate belong to the
//! surrounding transport; this helper never invents wall-clock synchronization.
use super::{Result, invalid};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counter {
    width: u8,
    value: Option<i128>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterCheckpoint {
    width: u8,
    value: Option<i128>,
}
impl Counter {
    pub fn new(width: u8) -> Result<Self> {
        if !(1..=64).contains(&width) {
            return Err(invalid("SL counter width must be in 1..=64"));
        }
        Ok(Self { width, value: None })
    }
    /// Set a coordinate supplied by a trusted transport clock or seek index.
    /// The next observation is resolved to its nearest unambiguous epoch.
    pub fn anchor(&mut self, value: i128) {
        self.value = Some(value);
    }
    pub fn value(&self) -> Option<i128> {
        self.value
    }
    /// Discard the epoch as well as observed history. Re-anchor after a seek
    /// when the caller needs an absolute coordinate rather than a fresh epoch.
    pub fn reset(&mut self) {
        self.value = None;
    }
    pub fn checkpoint(&self) -> CounterCheckpoint {
        CounterCheckpoint {
            width: self.width,
            value: self.value,
        }
    }
    pub fn restore(&mut self, checkpoint: &CounterCheckpoint) -> Result<()> {
        if self.width != checkpoint.width {
            return Err(invalid("SL counter checkpoint width mismatch"));
        }
        self.value = checkpoint.value;
        Ok(())
    }
    /// Resolve the shortest modular displacement, allowing composition-time
    /// reordering in either direction. A half-period displacement is ambiguous
    /// and requires an external anchor; it never silently picks an epoch.
    /// Errors preserve the previous coordinate.
    pub fn observe(&mut self, raw: u64) -> Result<i128> {
        let modulus = 1i128 << self.width;
        if i128::from(raw) >= modulus {
            return Err(invalid("SL counter value exceeds field width"));
        }
        let value = if let Some(previous) = self.value {
            let displacement = (i128::from(raw) - previous.rem_euclid(modulus)).rem_euclid(modulus);
            let half = modulus / 2;
            if displacement == half {
                return Err(invalid("ambiguous SL counter half-period displacement"));
            }
            let delta = if displacement > half {
                displacement - modulus
            } else {
                displacement
            };
            previous
                .checked_add(delta)
                .ok_or_else(|| invalid("SL counter coordinate overflow"))?
        } else {
            i128::from(raw)
        };
        self.value = Some(value);
        Ok(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_width_wraps_in_both_directions_and_restores_exact_epochs() {
        for width in 2..=64 {
            let modulus = 1i128 << width;
            let mask = (modulus - 1) as u64;
            let mut c = Counter::new(width).unwrap();
            c.anchor(-2);
            for expected in [-2, -1, 0, 1, 0, -1, -2] {
                let raw = (expected as u64) & mask;
                let checkpoint = c.checkpoint();
                assert_eq!(c.observe(raw).unwrap(), expected);
                c.restore(&checkpoint).unwrap();
                assert_eq!(c.observe(raw).unwrap(), expected);
            }
            c.anchor(modulus * 3 - 2);
            for offset in -2..=2 {
                assert_eq!(
                    c.observe((offset as u64) & mask).unwrap(),
                    modulus * 3 + offset
                );
            }
            c.reset();
            assert_eq!(c.value(), None);
            assert_eq!(c.observe(mask).unwrap(), modulus - 1);
        }
    }
    #[test]
    fn ambiguity_invalid_fields_and_overflow_never_advance_the_counter() {
        assert!(Counter::new(0).is_err());
        assert!(Counter::new(65).is_err());
        for width in 1..=64 {
            let mut c = Counter::new(width).unwrap();
            c.anchor(0);
            let half = 1u64 << (width - 1);
            assert!(
                c.observe(half)
                    .unwrap_err()
                    .to_string()
                    .contains("half-period")
            );
            assert_eq!(c.value(), Some(0));
            if width < 64 {
                assert!(c.observe(1u64 << width).is_err());
                assert_eq!(c.value(), Some(0));
            }
            assert_eq!(c.observe(0).unwrap(), 0);
        }
        let mut c = Counter::new(8).unwrap();
        c.anchor(i128::MAX);
        assert!(c.observe(0).is_err());
        assert_eq!(c.value(), Some(i128::MAX));
        assert_eq!(c.observe(254).unwrap(), i128::MAX - 1);
        c.anchor(i128::MIN);
        assert!(c.observe(255).is_err());
        assert_eq!(c.value(), Some(i128::MIN));
        let different = Counter::new(7).unwrap().checkpoint();
        assert!(c.restore(&different).is_err());
        assert_eq!(c.value(), Some(i128::MIN));
    }
}
