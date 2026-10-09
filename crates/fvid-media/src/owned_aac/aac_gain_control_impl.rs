#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adjustment {
    pub level: u8,
    pub location: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GainControl {
    /// Higher bands only; band zero has no transmitted adjustments.
    pub bands: Vec<Vec<Vec<Adjustment>>>,
}
impl GainControl {
    /// Parse atomically. The syntax bounds storage to 3 bands * 8 windows * 7 adjustments.
    pub fn read(bits: &mut BitReader<'_>, sequence: WindowSequence) -> Result<Self> {
        let mut reader = bits.clone();
        let (windows, transition, width) = match sequence {
            WindowSequence::OnlyLong => (1, false, 5),
            WindowSequence::LongStart => (2, true, 2),
            WindowSequence::EightShort => (8, false, 2),
            WindowSequence::LongStop => (2, true, 5),
        };
        let count = reader.read(2)? as usize;
        let mut bands = Vec::with_capacity(count);
        for _ in 0..count {
            let mut rows = Vec::with_capacity(windows);
            for window in 0..windows {
                let n = reader.read(3)? as usize;
                let location_bits = if window == 0 && transition { 4 } else { width };
                let mut adjustments = Vec::with_capacity(n);
                for _ in 0..n {
                    adjustments.push(Adjustment {
                        level: reader.read(4)? as u8,
                        location: reader.read(location_bits)? as u8,
                    });
                }
                rows.push(adjustments);
            }
            bands.push(rows);
        }
        *bits = reader;
        Ok(Self { bands })
    }
    pub fn is_empty(&self) -> bool {
        self.bands.iter().flatten().all(Vec::is_empty)
    }
}
