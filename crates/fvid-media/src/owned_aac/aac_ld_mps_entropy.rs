//! Owned LD-MPS CLD/ICC/CPC parameter-pair entropy and index reconstruction.
//! This component does not parse spatial frame scheduling or render PCM.
use super::{Result, aac_ld_mps_huffman_facts as facts, bits::BitReader, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Cld,
    Icc,
    Cpc,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub kind: Kind,
    pub coarse: bool,
    pub paired: bool,
    pub independent: bool,
    pub set_index: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    pub first: Vec<i16>,
    pub second: Option<Vec<i16>>,
}
type Book = &'static [(u8, u32, i16)];
fn book(kind: Kind, context: Option<bool>, lav: Option<usize>) -> Book {
    use facts::*;
    match context {
        None => match kind {
            Kind::Cld => CLD_P0,
            Kind::Icc => ICC_P0,
            Kind::Cpc => CPC_P0,
        },
        Some(time) => match lav {
            None => match (kind, time) {
                (Kind::Cld, false) => CLD_F1,
                (Kind::Cld, true) => CLD_T1,
                (Kind::Icc, false) => ICC_F1,
                (Kind::Icc, true) => ICC_T1,
                (Kind::Cpc, false) => CPC_F1,
                (Kind::Cpc, true) => CPC_T1,
            },
            Some(i) => match (kind, time) {
                (Kind::Cld, false) => [CLD_F2_0, CLD_F2_1, CLD_F2_2, CLD_F2_3][i],
                (Kind::Cld, true) => [CLD_T2_0, CLD_T2_1, CLD_T2_2, CLD_T2_3][i],
                (Kind::Icc, false) => [ICC_F2_0, ICC_F2_1, ICC_F2_2, ICC_F2_3][i],
                (Kind::Icc, true) => [ICC_T2_0, ICC_T2_1, ICC_T2_2, ICC_T2_3][i],
                (Kind::Cpc, false) => [CPC_F2_0, CPC_F2_1, CPC_F2_2, CPC_F2_3][i],
                (Kind::Cpc, true) => [CPC_T2_0, CPC_T2_1, CPC_T2_2, CPC_T2_3][i],
            },
        },
    }
}
pub(super) fn symbol(bits: &mut BitReader<'_>, table: Book) -> Result<i32> {
    let max = table.iter().map(|r| r.0).max().unwrap_or(0);
    let mut prefix = 0u32;
    for width in 1..=max {
        prefix = (prefix << 1) | bits.read(1)?;
        if let Some(row) = table.iter().find(|r| r.0 == width && r.1 == prefix) {
            return Ok(i32::from(row.2));
        }
    }
    Err(invalid("invalid LD-MPS parameter prefix code"))
}
fn quant(kind: Kind, coarse: bool) -> (u32, i32, bool) {
    match (kind, coarse) {
        (Kind::Cld, false) => (31, 15, false),
        (Kind::Cld, true) => (15, 7, false),
        (Kind::Icc, false) => (8, 0, false),
        (Kind::Icc, true) => (4, 0, false),
        (Kind::Cpc, false) => (51, 20, true),
        (Kind::Cpc, true) => (26, 10, false),
    }
}
fn grouped(bits: &mut BitReader<'_>, levels: u32, count: usize) -> Result<Vec<i32>> {
    let group = match levels {
        3 => 5,
        7 => 6,
        11 => 2,
        13 | 19 | 51 => 4,
        25 => 3,
        4 | 8 | 15 | 26 | 31 => 1,
        _ => return Err(invalid("invalid LD-MPS PCM alphabet")),
    };
    let mut result = Vec::with_capacity(count);
    while result.len() < count {
        let n = group.min(count - result.len());
        let bound = levels.pow(n as u32);
        let width = (u32::BITS - (bound - 1).leading_zeros()) as u8;
        let mut value = bits.read(width)?;
        if value >= bound {
            return Err(invalid("unused LD-MPS PCM codeword"));
        }
        let start = result.len();
        result.resize(start + n, 0);
        for i in (start..start + n).rev() {
            result[i] = (value % levels) as i32;
            value /= levels;
        }
    }
    Ok(result)
}
fn signed(bits: &mut BitReader<'_>, kind: Kind, time: bool) -> Result<i32> {
    let magnitude = symbol(bits, book(kind, Some(time), None))?;
    if magnitude < 0 {
        return Err(invalid("invalid LD-MPS signed parameter leaf"));
    }
    Ok(if magnitude != 0 && bits.bit()? {
        -magnitude
    } else {
        magnitude
    })
}
fn unfold(bits: &mut BitReader<'_>, raw: i32, lav: i32) -> Result<[i32; 2]> {
    let a = raw >> 4;
    let b = raw & 15;
    let sum = a + b;
    let difference = a - b;
    let mut pair = if sum > lav {
        [2 * lav + 1 - sum, -difference]
    } else {
        [sum, difference]
    };
    if pair[0] + pair[1] != 0 && bits.bit()? {
        pair[0] = -pair[0];
        pair[1] = -pair[1];
    }
    if pair[0] - pair[1] != 0 && bits.bit()? {
        pair.swap(0, 1);
    }
    Ok(pair)
}
fn differences(
    bits: &mut BitReader<'_>,
    kind: Kind,
    time: bool,
    two_dimensional: bool,
    count: usize,
) -> Result<Vec<i32>> {
    let mut out = Vec::with_capacity(count);
    if !two_dimensional {
        if !time {
            out.push(symbol(bits, book(kind, None, None))?);
        }
        while out.len() < count {
            out.push(signed(bits, kind, time)?);
        }
        return Ok(out);
    }
    // Even a vector containing only its first absolute value carries LAV.
    let index = symbol(bits, facts::LAV)? as usize;
    let lav = match kind {
        Kind::Cld => [3, 5, 7, 9],
        Kind::Icc => [1, 3, 5, 7],
        Kind::Cpc => [3, 6, 9, 12],
    }[index];
    if !time {
        out.push(symbol(bits, book(kind, None, None))?);
    }
    let pairs = (count - out.len()) / 2;
    let mut escapes = Vec::new();
    for _ in 0..pairs {
        let raw = symbol(bits, book(kind, Some(time), Some(index)))?;
        let at = out.len();
        if raw < 0 {
            escapes.push(at);
            out.extend([0, 0]);
        } else {
            out.extend(unfold(bits, raw, lav)?);
        }
    }
    if !escapes.is_empty() {
        let values = grouped(bits, (2 * lav + 1) as u32, escapes.len() * 2)?;
        for (at, pair) in escapes.into_iter().zip(values.chunks_exact(2)) {
            out[at] = pair[0] - lav;
            out[at + 1] = pair[1] - lav;
        }
    }
    if out.len() < count {
        out.push(signed(bits, kind, time)?);
    }
    Ok(out)
}
/// The caller supplies history in the selected quantization domain. Frequency
/// vectors and the first independent set do not consult it. The cursor and
/// history are unchanged on any failure, including a later paired vector/LSB.
pub fn decode(bits: &mut BitReader<'_>, history: &[i16], options: Options) -> Result<Pair> {
    let count = history.len();
    if count > 23 || options.set_index > 1 || options.paired && options.set_index != 0 {
        return Err(invalid("invalid LD-MPS parameter-pair geometry"));
    }
    let mut b = bits.clone();
    let (levels, offset, lsb) = quant(options.kind, options.coarse);
    let rows = 1 + usize::from(options.paired);
    let mut data = vec![vec![0i32; count]; rows];
    if b.bit()? {
        let values = grouped(&mut b, levels, count * rows)?;
        for (i, value) in values.into_iter().enumerate() {
            data[i % rows][i / rows] = value - offset;
        }
    } else {
        if count == 0 {
            return Err(invalid("empty LD-MPS Huffman parameter vector"));
        }
        let allow_history = !options.independent || options.set_index != 0;
        let mut time = [false; 2];
        if options.paired || allow_history {
            time[0] = b.bit()?;
        }
        if options.paired && (!time[0] || allow_history) {
            time[1] = b.bit()?;
        }
        let two_dimensional = b.bit()?;
        let mut msb = Vec::<Vec<i32>>::new();
        for row in 0..rows {
            let delta = differences(&mut b, options.kind, time[row], two_dimensional, count)?;
            let cumulative = if row == 0 {
                !time[0] || !allow_history
            } else {
                !time[1]
            };
            let values = if cumulative {
                let mut sum = 0;
                delta
                    .into_iter()
                    .map(|v| {
                        sum += v;
                        sum
                    })
                    .collect::<Vec<_>>()
            } else {
                let previous = if row == 0 {
                    if history
                        .iter()
                        .any(|v| i32::from(*v) < -offset || i32::from(*v) >= levels as i32 - offset)
                    {
                        return Err(invalid("invalid LD-MPS parameter history"));
                    }
                    history
                        .iter()
                        .map(|v| (i32::from(*v) + offset) >> u32::from(lsb))
                        .collect::<Vec<_>>()
                } else {
                    msb[0].clone()
                };
                previous
                    .into_iter()
                    .zip(delta)
                    .map(|(a, d)| a + d)
                    .collect()
            };
            msb.push(values);
        }
        for row in 0..rows {
            for k in 0..count {
                data[row][k] = (msb[row][k] << u32::from(lsb))
                    + (if lsb { b.read(1)? as i32 } else { 0 })
                    - offset;
            }
        }
    }
    if data
        .iter()
        .flatten()
        .any(|v| *v < -offset || *v >= levels as i32 - offset)
    {
        return Err(invalid("LD-MPS parameter index exceeds quantization range"));
    }
    let first = data[0].iter().map(|v| *v as i16).collect();
    let second = options
        .paired
        .then(|| data[1].iter().map(|v| *v as i16).collect());
    *bits = b;
    Ok(Pair { first, second })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_numeric_leaf_decodes_at_all_offsets_without_consuming_the_next_symbol() {
        let mut books = vec![facts::LAV];
        for kind in [Kind::Cld, Kind::Icc, Kind::Cpc] {
            books.push(book(kind, None, None));
            for context in [false, true] {
                books.push(book(kind, Some(context), None));
                for lav in 0..4 {
                    books.push(book(kind, Some(context), Some(lav)));
                }
            }
        }
        assert_eq!(books.len(), 34);
        let mut count = 0;
        for table in books {
            for &(length, word, value) in table {
                for offset in 0..8 {
                    let mut bytes = vec![0u8; (offset + usize::from(length) + 8).div_ceil(8)];
                    for i in 0..usize::from(length) {
                        bytes[(offset + i) / 8] |= (((word >> (usize::from(length) - i - 1)) & 1)
                            as u8)
                            << (7 - (offset + i) % 8);
                    }
                    for i in 0..8 {
                        let at = offset + usize::from(length) + i;
                        bytes[at / 8] |= ((0xa5u8 >> (7 - i)) & 1) << (7 - at % 8);
                    }
                    let mut bits = BitReader::new(&bytes);
                    bits.skip(offset).unwrap();
                    assert_eq!(symbol(&mut bits, table).unwrap(), i32::from(value));
                    assert_eq!(bits.position(), offset + usize::from(length));
                    assert_eq!(bits.read(8).unwrap(), 0xa5);
                }
                count += 1;
            }
        }
        assert_eq!(count, 1504);
    }
}
