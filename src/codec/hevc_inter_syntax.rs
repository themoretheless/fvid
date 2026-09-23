//! HEVC prediction-unit CABAC syntax (7.3.8.6, 9.3.3).
use super::{
    hevc_cabac::{HevcCabac, SliceType, Syntax},
    hevc_slice::SliceHeader,
};
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Partition {
    Full,
    Horizontal,
    Vertical,
    Quarters,
    Top,
    Bottom,
    Left,
    Right,
}
impl Partition {
    pub fn rectangles(self, size: u32) -> Vec<[u32; 4]> {
        let n = size;
        let h = n / 2;
        let q = n / 4;
        match self {
            Self::Full => vec![[0, 0, n, n]],
            Self::Horizontal => vec![[0, 0, n, h], [0, h, n, h]],
            Self::Vertical => vec![[0, 0, h, n], [h, 0, h, n]],
            Self::Quarters => vec![[0, 0, h, h], [h, 0, h, h], [0, h, h, h], [h, h, h, h]],
            Self::Top => vec![[0, 0, n, q], [0, q, n, n - q]],
            Self::Bottom => vec![[0, 0, n, n - q], [0, n - q, n, q]],
            Self::Left => vec![[0, 0, q, n], [q, 0, n - q, n]],
            Self::Right => vec![[0, 0, n - q, n], [n - q, 0, q, n]],
        }
    }
}
pub fn partition(b: &mut HevcCabac<'_>, log: u8, min_log: u8, amp: bool) -> Result<Partition> {
    if b.decision(Syntax::PartMode, 0)? {
        return Ok(Partition::Full);
    }
    let horizontal = b.decision(Syntax::PartMode, 1)?;
    if log == min_log {
        if horizontal {
            return Ok(Partition::Horizontal);
        }
        return Ok(if log == 3 || b.decision(Syntax::PartMode, 2)? {
            Partition::Vertical
        } else {
            Partition::Quarters
        });
    }
    if !amp || b.decision(Syntax::PartMode, 3)? {
        return Ok(if horizontal {
            Partition::Horizontal
        } else {
            Partition::Vertical
        });
    }
    Ok(match (horizontal, b.bypass()?) {
        (true, false) => Partition::Top,
        (true, true) => Partition::Bottom,
        (false, false) => Partition::Left,
        (false, true) => Partition::Right,
    })
}
#[derive(Clone, Copy, Debug)]
pub enum Prediction {
    Merge(u8),
    Explicit {
        references: [Option<u8>; 2],
        differences: [[i16; 2]; 2],
        predictors: [usize; 2],
    },
}
fn difference(b: &mut HevcCabac<'_>) -> Result<[i16; 2]> {
    let nonzero = [b.decision(Syntax::Mvd0, 0)?, b.decision(Syntax::Mvd0, 0)?];
    let mut large = [false; 2];
    for i in 0..2 {
        if nonzero[i] {
            large[i] = b.decision(Syntax::Mvd1, 0)?;
        }
    }
    let mut values = [0; 2];
    for i in 0..2 {
        if nonzero[i] {
            let mut value = 1u32;
            if large[i] {
                value = 2;
                let mut order = 1;
                while b.bypass()? {
                    value += 1 << order;
                    order += 1;
                    if order > 15 {
                        return Err(invalid("HEVC MVD exceeds range"));
                    }
                }
                for k in (0..order).rev() {
                    value += u32::from(b.bypass()?) << k;
                }
            }
            let negative = b.bypass()?;
            if value > if negative { 32768 } else { 32767 } {
                return Err(invalid("HEVC MVD exceeds signed range"));
            }
            values[i] = (if negative {
                -(value as i32)
            } else {
                value as i32
            }) as i16;
        }
    }
    Ok(values)
}
pub fn prediction(
    b: &mut HevcCabac<'_>,
    h: &SliceHeader,
    skip: bool,
    depth: u8,
    dimensions: [u32; 2],
) -> Result<Prediction> {
    if skip || b.decision(Syntax::MergeFlag, 0)? {
        let mut index = 0;
        while index + 1 < h.max_merge_candidates {
            let more = if index == 0 {
                b.decision(Syntax::MergeIdx, 0)?
            } else {
                b.bypass()?
            };
            if !more {
                break;
            }
            index += 1;
        }
        return Ok(Prediction::Merge(index));
    }
    let mask = if h.slice_type == SliceType::P {
        1
    } else if dimensions[0] + dimensions[1] != 12
        && b.decision(Syntax::InterPred, depth as usize)?
    {
        3
    } else if b.decision(Syntax::InterPred, 4)? {
        2
    } else {
        1
    };
    let mut references = [None; 2];
    let mut differences = [[0; 2]; 2];
    let mut predictors = [0; 2];
    for list in 0..2 {
        if mask & (1 << list) == 0 {
            continue;
        }
        let mut reference = 0;
        while reference + 1 < h.references[list] {
            let more = if reference < 2 {
                b.decision(Syntax::RefIdx, reference as usize)?
            } else {
                b.bypass()?
            };
            if !more {
                break;
            }
            reference += 1;
        }
        references[list] = Some(reference);
        if !(list == 1 && mask == 3 && h.mvd_l1_zero) {
            differences[list] = difference(b)?;
        }
        predictors[list] = usize::from(b.decision(Syntax::Mvp, 0)?);
    }
    Ok(Prediction::Explicit {
        references,
        differences,
        predictors,
    })
}
