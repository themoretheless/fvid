//! Explicit QMF time geometry: legacy SBR RATE=2, low-delay SBR RATE=1.
use super::{Result, invalid};
#[derive(Clone, Copy)]
pub(crate) enum Timing {
    Legacy,
    LowDelay,
}
impl Timing {
    pub(crate) fn rate(self) -> usize {
        match self {
            Self::Legacy => 2,
            Self::LowDelay => 1,
        }
    }
    pub(crate) fn tail(self) -> usize {
        match self {
            Self::Legacy => 6,
            Self::LowDelay => 0,
        }
    }
    pub(crate) fn columns(self, slots: u8) -> Result<usize> {
        if !matches!(slots, 15 | 16) {
            return Err(invalid("invalid SBR time-slot count"));
        }
        Ok(self.rate() * usize::from(slots) + self.tail())
    }
    pub(crate) fn maximum_border(self, slots: u8) -> usize {
        usize::from(slots) + self.tail() / self.rate()
    }
}
