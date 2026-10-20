//! Checked rational output clock for links with different source sample rates.
use fvid_control::error::{Result, invalid};
#[derive(Clone, Copy)]
pub(crate) struct Clock {
    pub whole: i64,
    pub phase: (u128, u128),
    rate: u32,
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
fn overflow() -> fvid_control::error::Error {
    invalid("Ogg audio rational clock overflow")
}
impl Clock {
    pub fn new(rate: u32) -> Self {
        Self {
            whole: 0,
            phase: (0, 1),
            rate,
        }
    }
    pub fn ceil(self) -> Result<i64> {
        self.whole
            .checked_add(i64::from(self.phase.0 != 0))
            .ok_or_else(overflow)
    }
    pub fn point(self, local: i64, input_rate: u32) -> Result<i64> {
        let denominator = i128::try_from(self.phase.1).map_err(|_| overflow())?;
        let phase = i128::try_from(self.phase.0).map_err(|_| overflow())?;
        let numerator = i128::from(local)
            .checked_mul(i128::from(self.rate))
            .and_then(|n| n.checked_mul(denominator))
            .and_then(|n| {
                phase
                    .checked_mul(i128::from(input_rate))
                    .and_then(|p| n.checked_add(p))
            })
            .ok_or_else(overflow)?;
        let denominator = denominator
            .checked_mul(i128::from(input_rate))
            .ok_or_else(overflow)?;
        let offset = i64::try_from(numerator.div_euclid(denominator)).map_err(|_| overflow())?;
        self.whole.checked_add(offset).ok_or_else(overflow)
    }
    pub fn advance(&mut self, samples: i64, input_rate: u32) -> Result<()> {
        let product = u128::try_from(samples)
            .map_err(|_| overflow())?
            .checked_mul(u128::from(self.rate))
            .ok_or_else(overflow)?;
        let input = u128::from(input_rate);
        let quotient = i64::try_from(product / input).map_err(|_| overflow())?;
        self.whole = self.whole.checked_add(quotient).ok_or_else(overflow)?;
        let remainder = product % input;
        let divisor = gcd(self.phase.1, input);
        let denominator = self
            .phase
            .1
            .checked_mul(input / divisor)
            .ok_or_else(overflow)?;
        let numerator = self
            .phase
            .0
            .checked_mul(input / divisor)
            .and_then(|n| {
                remainder
                    .checked_mul(self.phase.1 / divisor)
                    .and_then(|r| n.checked_add(r))
            })
            .ok_or_else(overflow)?;
        self.whole = self
            .whole
            .checked_add(i64::try_from(numerator / denominator).map_err(|_| overflow())?)
            .ok_or_else(overflow)?;
        let remainder = numerator % denominator;
        let divisor = gcd(remainder, denominator);
        self.phase = (remainder / divisor, denominator / divisor);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linked_fractional_durations_round_only_at_presentation_boundaries() {
        let mut clock = Clock::new(12000);
        clock.advance(411, 12000).unwrap();
        assert_eq!(clock.ceil().unwrap(), 411);
        clock.advance(411, 8000).unwrap();
        assert_eq!(clock.phase, (1, 2));
        assert_eq!(clock.ceil().unwrap(), 1028);
        assert_eq!(clock.point(0, 8000).unwrap(), 1027);
        assert_eq!(clock.point(-1, 8000).unwrap(), 1026);
        clock.advance(411, 8000).unwrap();
        assert_eq!(clock.ceil().unwrap(), 1644);
    }
}
