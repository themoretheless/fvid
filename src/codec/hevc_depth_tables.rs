//! Owned H.265 Annex I depth lookup tables. Rendering integration is separate.
use super::bits::BitReader;
use crate::{Result, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub id: u8,
    /// None means identity; Some(empty) is a signalled empty table.
    pub values: Option<Vec<u16>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tables {
    pub bit_depth: u8,
    pub layers: Vec<Layer>,
}
struct Reader<'a> {
    bits: BitReader<'a>,
    end: usize,
}
impl Reader<'_> {
    fn take(&mut self, n: u8) -> Result<u32> {
        if self
            .bits
            .position()
            .checked_add(usize::from(n))
            .is_none_or(|p| p > self.end)
        {
            return Err(invalid("truncated HEVC depth lookup table"));
        }
        self.bits.read(n)
    }
    fn flag(&mut self) -> Result<bool> {
        Ok(self.take(1)? != 0)
    }
}
fn bytes(count: usize) -> Result<usize> {
    count
        .checked_mul(2)
        .ok_or_else(|| invalid("HEVC DLT allocation overflow"))
}
fn values(count: usize, retained: usize, budget: usize) -> Result<Vec<u16>> {
    let fits = |size| retained.checked_add(size).is_some_and(|n| n <= budget);
    if !fits(bytes(count)?) {
        return Err(invalid("HEVC DLT allocation exceeds budget"));
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| invalid("HEVC DLT allocation failed"))?;
    if !fits(bytes(output.capacity())?) {
        return Err(invalid("HEVC DLT allocation exceeds budget"));
    }
    Ok(output)
}
fn width(value: u32) -> u8 {
    (32 - value.leading_zeros()) as u8
}
fn delta(r: &mut Reader<'_>, depth: u8, retained: usize, budget: usize) -> Result<Vec<u16>> {
    let count = r.take(depth)? as usize;
    let mut output = values(count, retained, budget)?;
    if count == 0 {
        return Ok(output);
    }
    let maximum = if count > 1 { r.take(depth)? } else { 0 };
    let minimum = if count > 2 && maximum > 0 {
        let encoded = r.take(width(maximum))?;
        if encoded >= maximum {
            return Err(invalid("HEVC DLT minimum difference exceeds maximum"));
        }
        encoded + 1
    } else {
        maximum
    };
    let mut value = r.take(depth)?;
    output.push(value as u16);
    let limit = (1u32 << depth) - 1;
    for _ in 1..count {
        let difference = if maximum > minimum {
            let extra = r.take(width(maximum - minimum))?;
            if extra > maximum - minimum {
                return Err(invalid("HEVC DLT difference exceeds maximum"));
            }
            minimum + extra
        } else {
            minimum
        };
        value = value
            .checked_add(difference)
            .filter(|v| *v <= limit)
            .ok_or_else(|| invalid("HEVC DLT value exceeds depth range"))?;
        output.push(value as u16);
    }
    // The normative set-membership derivation removes repeated delta values.
    output.dedup();
    Ok(output)
}
fn symmetric_difference(a: &[u16], b: &[u16], mut emit: impl FnMut(u16)) {
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if j == b.len() || (i < a.len() && a[i] < b[j]) {
            emit(a[i]);
            i += 1;
        } else if i == a.len() || b[j] < a[i] {
            emit(b[j]);
            j += 1;
        } else {
            i += 1;
            j += 1;
        }
    }
}
impl Tables {
    /// Vector ownership only; callers account for this struct and Arc headers.
    pub fn heap_bytes(&self) -> usize {
        self.layers
            .capacity()
            .saturating_mul(std::mem::size_of::<Layer>())
            .saturating_add(
                self.layers
                    .iter()
                    .filter_map(|l| l.values.as_ref())
                    .fold(0usize, |n, v| {
                        n.saturating_add(v.capacity().saturating_mul(2))
                    }),
            )
    }
    /// Parse PPS 3D extension with SPS bit depth and VPS depth-layer ID order.
    /// `end_bit` excludes rbsp_trailing_bits (but may include later extensions).
    /// `budget` bounds this parser's retained vectors and simultaneous scratch.
    /// Any failure leaves the caller's bit position unchanged.
    pub fn read(
        bits: &mut BitReader<'_>,
        end_bit: usize,
        depth: u8,
        layer_ids: &[u8],
        budget: usize,
    ) -> Result<Option<Self>> {
        if end_bit < bits.position() || end_bit > bits.position().saturating_add(bits.remaining()) {
            return Err(invalid("invalid HEVC DLT syntax boundary"));
        }
        let mut r = Reader {
            bits: bits.clone(),
            end: end_bit,
        };
        if !r.flag()? {
            *bits = r.bits;
            return Ok(None);
        }
        if !(8..=16).contains(&depth) {
            return Err(invalid("invalid HEVC DLT sample depth"));
        }
        let count = r.take(6)? as usize + 1;
        let encoded_depth = r.take(4)? as u8 + 8;
        if encoded_depth != depth {
            return Err(invalid("HEVC DLT depth disagrees with SPS"));
        }
        if count != layer_ids.len() {
            return Err(invalid("HEVC DLT layer count disagrees with VPS"));
        }
        let mut seen = 0u64;
        for &id in layer_ids {
            if id > 62 || seen & (1u64 << id) != 0 {
                return Err(invalid("invalid HEVC DLT depth-layer IDs"));
            }
            seen |= 1u64 << id;
        }
        let mut layers = Vec::new();
        let allocation = count
            .checked_mul(std::mem::size_of::<Layer>())
            .filter(|n| *n <= budget)
            .ok_or_else(|| invalid("HEVC DLT allocation exceeds budget"))?;
        layers
            .try_reserve_exact(count)
            .map_err(|_| invalid("HEVC DLT allocation failed"))?;
        let mut retained = layers
            .capacity()
            .checked_mul(std::mem::size_of::<Layer>())
            .filter(|n| *n <= budget)
            .ok_or_else(|| invalid("HEVC DLT allocation exceeds budget"))?;
        debug_assert!(retained >= allocation);
        for (index, &id) in layer_ids.iter().enumerate() {
            let table = if !r.flag()? {
                None
            } else {
                let predicted = r.flag()?;
                if predicted
                    && (index == 0 || layers.first().is_none_or(|l: &Layer| l.values.is_none()))
                {
                    return Err(invalid("HEVC DLT prediction lacks a first-layer table"));
                }
                let flags = if predicted { false } else { r.flag()? };
                let table = if flags {
                    let count = 1usize << depth;
                    let start = r.bits.clone();
                    let mut selected = 0;
                    for _ in 0..count {
                        selected += usize::from(r.flag()?);
                    }
                    r.bits = start;
                    let mut output = values(selected, retained, budget)?;
                    for value in 0..count {
                        if r.flag()? {
                            output.push(value as u16);
                        }
                    }
                    output
                } else {
                    let update = delta(&mut r, depth, retained, budget)?;
                    if predicted {
                        let reference = layers[0].values.as_ref().unwrap();
                        let mut count = 0;
                        symmetric_difference(reference, &update, |_| count += 1);
                        let scratch = retained
                            .checked_add(bytes(update.capacity())?)
                            .ok_or_else(|| invalid("HEVC DLT allocation overflow"))?;
                        let mut output = values(count, scratch, budget)?;
                        symmetric_difference(reference, &update, |v| output.push(v));
                        output
                    } else {
                        update
                    }
                };
                retained = retained
                    .checked_add(bytes(table.capacity())?)
                    .filter(|n| *n <= budget)
                    .ok_or_else(|| invalid("HEVC DLT allocation exceeds budget"))?;
                Some(table)
            };
            layers.push(Layer { id, values: table });
        }
        *bits = r.bits;
        Ok(Some(Self {
            bit_depth: depth,
            layers,
        }))
    }
    fn sample_limit(&self) -> Result<usize> {
        if !(8..=16).contains(&self.bit_depth) {
            return Err(invalid("invalid HEVC DLT sample depth"));
        }
        Ok((1usize << self.bit_depth) - 1)
    }
    fn layer(&self, id: u8) -> Result<&Layer> {
        self.layers
            .iter()
            .find(|l| l.id == id)
            .ok_or_else(|| invalid("unknown HEVC DLT depth layer"))
    }
    /// I-56 nearest depth index. Exact midpoint ties select the upper entry.
    pub fn prediction_index(&self, layer_id: u8, sample: u16) -> Result<usize> {
        let limit = self.sample_limit()?;
        if usize::from(sample) > limit {
            return Err(invalid("HEVC depth predictor exceeds sample range"));
        }
        let Some(values) = &self.layer(layer_id)?.values else {
            return Ok(usize::from(sample));
        };
        if values.is_empty() {
            return Err(invalid(
                "HEVC empty depth lookup table cannot reconstruct samples",
            ));
        }
        let upper = values.partition_point(|v| *v <= sample);
        if upper == 0 {
            return Ok(0);
        }
        if upper == values.len() {
            return Ok(upper - 1);
        }
        Ok(if sample - values[upper - 1] < values[upper] - sample {
            upper - 1
        } else {
            upper
        })
    }
    /// I-55 full sample-domain lookup: unused table indices map to zero.
    /// `value` remains strict introspection of explicitly signalled entries.
    pub fn reconstruction_value(&self, layer_id: u8, index: usize) -> Result<u16> {
        if index > self.sample_limit()? {
            return Err(invalid("HEVC depth index exceeds sample range"));
        }
        Ok(match &self.layer(layer_id)?.values {
            Some(values) => values.get(index).copied().unwrap_or(0),
            None => index as u16,
        })
    }
    /// I-93/94 DC prediction. Without DLT, clipping belongs to reconstruction.
    /// With DLT, Clip1Y clips the index domain, not the signalled table length.
    pub fn dc_prediction(&self, layer_id: u8, predictor: u16, offset: i64) -> Result<i128> {
        let limit = self.sample_limit()?;
        if usize::from(predictor) > limit {
            return Err(invalid("HEVC depth predictor exceeds sample range"));
        }
        if self.layer(layer_id)?.values.is_none() {
            return Ok(i128::from(predictor) + i128::from(offset));
        }
        let index = self.prediction_index(layer_id, predictor)?;
        let index = (index as i128 + i128::from(offset)).clamp(0, limit as i128) as usize;
        Ok(i128::from(self.reconstruction_value(layer_id, index)?))
    }
    /// I-95..97 DC-offset assignment on a strided transform block before deblocking.
    /// Geometry and sample values are validated before modifying any sample.
    pub fn assign_dc_offset(
        &self,
        layer_id: u8,
        samples: &mut [u16],
        stride: usize,
        origin: [usize; 2],
        size: usize,
        offset: i64,
    ) -> Result<()> {
        let limit = self.sample_limit()?;
        let layer = self.layer(layer_id)?;
        if !(4..=64).contains(&size) || !size.is_power_of_two() {
            return Err(invalid("invalid HEVC depth DC block size"));
        }
        let right = origin[0]
            .checked_add(size)
            .filter(|v| *v <= stride)
            .ok_or_else(|| invalid("HEVC depth DC block exceeds row"))?;
        let last_row = origin[1]
            .checked_add(size - 1)
            .ok_or_else(|| invalid("HEVC depth DC block geometry overflow"))?;
        last_row
            .checked_mul(stride)
            .and_then(|v| v.checked_add(right))
            .filter(|v| *v <= samples.len())
            .ok_or_else(|| invalid("HEVC depth DC block exceeds buffer"))?;
        for y in origin[1]..=last_row {
            if samples[y * stride + origin[0]..y * stride + right]
                .iter()
                .any(|v| usize::from(*v) > limit)
            {
                return Err(invalid("HEVC depth DC input exceeds sample range"));
            }
        }
        let adjustment = if layer.values.is_some() {
            let left = origin[0];
            let right = right - 1;
            let corners = [
                origin[1] * stride + left,
                origin[1] * stride + right,
                last_row * stride + left,
                last_row * stride + right,
            ];
            let predictor =
                ((corners.iter().map(|&i| u32::from(samples[i])).sum::<u32>() + 2) >> 2) as u16;
            i128::from(self.dc_prediction(layer_id, predictor, offset)?) - i128::from(predictor)
        } else {
            i128::from(offset)
        };
        for y in origin[1]..=last_row {
            for value in &mut samples[y * stride + origin[0]..y * stride + right] {
                *value = (i128::from(*value) + adjustment).clamp(0, limit as i128) as u16;
            }
        }
        Ok(())
    }
    pub fn value(&self, layer_id: u8, index: usize) -> Result<u16> {
        if !(8..=16).contains(&self.bit_depth) {
            return Err(invalid("invalid HEVC DLT sample depth"));
        }
        let layer = self
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .ok_or_else(|| invalid("unknown HEVC DLT depth layer"))?;
        if let Some(values) = &layer.values {
            values
                .get(index)
                .copied()
                .ok_or_else(|| invalid("HEVC depth index exceeds lookup table"))
        } else if index < (1usize << self.bit_depth) {
            Ok(index as u16)
        } else {
            Err(invalid("HEVC depth index exceeds sample range"))
        }
    }
}
