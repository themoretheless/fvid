use super::{aac_synthesis::WindowSequence, bits::BitReader};
use std::f64::consts::FRAC_PI_2;

/// Called after tns_data_present. Input cursor commits only on success.
pub fn read(bits: &mut BitReader<'_>, sequence: WindowSequence) -> Result<TnsData> {
    let short = sequence == WindowSequence::EightShort;
    let mut cursor = bits.clone();
    let mut windows = Vec::with_capacity(if short { 8 } else { 1 });
    for _ in 0..if short { 8 } else { 1 } {
        let count = cursor.read(if short { 1 } else { 2 })?;
        let resolution = if count > 0 {
            3 + cursor.read(1)? as u8
        } else {
            3
        };
        let mut filters = Vec::new();
        for _ in 0..count {
            let length = cursor.read(if short { 4 } else { 6 })? as usize;
            let order = cursor.read(if short { 3 } else { 5 })? as usize;
            if order > if short { 7 } else { 12 } {
                return Err(invalid("AAC-LC TNS order exceeds limit"));
            }
            let mut reverse = false;
            let mut lpc = Vec::with_capacity(order);
            if order > 0 {
                reverse = cursor.bit()?;
                let width = resolution - u8::from(cursor.bit()?);
                for _ in 0..order {
                    let raw = cursor.read(width)? as i32;
                    let signed = if raw & (1 << (width - 1)) != 0 {
                        raw - (1 << width)
                    } else {
                        raw
                    };
                    let base = (1u32 << (resolution - 1)) as f64;
                    let denominator = base + if signed < 0 { 0.5 } else { -0.5 };
                    let reflection = (f64::from(signed) * FRAC_PI_2 / denominator).sin();
                    let mut previous = [0.0; 12];
                    let count = lpc.len();
                    previous[..count].copy_from_slice(&lpc);
                    for i in 0..count {
                        lpc[i] = previous[i] + reflection * previous[count - 1 - i];
                    }
                    lpc.push(reflection);
                }
            }
            filters.push(TnsFilter {
                length,
                reverse,
                lpc,
            });
        }
        windows.push(filters);
    }
    *bits = cursor;
    Ok(TnsData { windows })
}
