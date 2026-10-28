//! EP1 transport-facing ESC association; no FEC or invented byte framing.
//! ISO/IEC 14496-3 1.6.3.6 and 4.5.2.4.2 carry ESCs in dependency order.
use super::{
    Result,
    aac_ep::{RecoveredClass, RecoveredFrame},
    invalid,
};

/// One complete EP1 AU with one or more consecutive ESC instances.
/// `bit_len` excludes transport padding; ignored low octet bits are never decoded.
#[derive(Clone, Copy, Debug)]
pub struct AccessUnit<'a> {
    pub stream_id: u16,
    pub depends_on: Option<u16>,
    pub data: &'a [u8],
    pub bit_len: usize,
    pub pts: i64,
    pub duration: u64,
    pub timescale: u32,
}
/// Exact EP1 AU bit regions in dependency order, with the root stream's clock.
#[derive(Clone, Debug)]
pub struct Frame {
    pub payload: RecoveredFrame,
    pub pts: i64,
    pub duration: u64,
    pub timescale: u32,
}
/// Associate an entire source frame before any decoder state can change.
/// Input order is arbitrary; missing dependencies, forks, cycles, duplicate
/// streams and mismatched source times are errors, not concealment requests.
pub fn associate(units: &[AccessUnit<'_>]) -> Result<Frame> {
    if units.is_empty() {
        return Err(invalid("EP1 frame has no ESC access units"));
    }
    if units.len() > usize::from(u16::MAX) + 1 {
        return Err(invalid("EP1 frame has too many elementary streams"));
    }
    let mut sorted: Vec<&AccessUnit<'_>> = Vec::new();
    sorted
        .try_reserve_exact(units.len())
        .map_err(|_| invalid("EP1 stream table allocation failed"))?;
    sorted.extend(units.iter());
    sorted.sort_unstable_by_key(|unit| unit.stream_id);
    if sorted
        .windows(2)
        .any(|pair| pair[0].stream_id == pair[1].stream_id)
    {
        return Err(invalid("duplicate EP1 elementary stream"));
    }
    let mut next = vec![None; units.len()];
    let mut root = None;
    let first = sorted[0];
    for (index, unit) in sorted.iter().enumerate() {
        if unit.timescale == 0 {
            return Err(invalid("zero EP1 source timescale"));
        }
        if unit.data.len() != unit.bit_len.div_ceil(8) {
            return Err(invalid(
                "EP1 access unit bit boundary disagrees with storage",
            ));
        }
        if i128::from(unit.pts) * i128::from(first.timescale)
            != i128::from(first.pts) * i128::from(unit.timescale)
            || u128::from(unit.duration) * u128::from(first.timescale)
                != u128::from(first.duration) * u128::from(unit.timescale)
        {
            return Err(invalid(
                "EP1 access units do not describe the same source time",
            ));
        }
        if let Some(parent) = unit.depends_on {
            let parent = sorted
                .binary_search_by_key(&parent, |unit| unit.stream_id)
                .map_err(|_| invalid("missing EP1 elementary stream dependency"))?;
            if next[parent].replace(index).is_some() {
                return Err(invalid("forked EP1 elementary stream dependency"));
            }
        } else if root.replace(index).is_some() {
            return Err(invalid("multiple EP1 root streams"));
        }
    }
    let root = root.ok_or_else(|| invalid("cyclic EP1 elementary stream dependency"))?;
    let mut order = Vec::new();
    order
        .try_reserve_exact(units.len())
        .map_err(|_| invalid("EP1 dependency order allocation failed"))?;
    let mut at = Some(root);
    while let Some(index) = at {
        if order.len() == units.len() {
            return Err(invalid("cyclic EP1 elementary stream dependency"));
        }
        order.push(index);
        at = next[index];
    }
    if order.len() != units.len() {
        return Err(invalid(
            "disconnected or cyclic EP1 elementary stream dependency",
        ));
    }
    let mut classes = Vec::new();
    classes
        .try_reserve_exact(order.len())
        .map_err(|_| invalid("EP1 ESC table allocation failed"))?;
    for (class_index, index) in order.into_iter().enumerate() {
        let unit = sorted[index];
        let mut bits = Vec::new();
        bits.try_reserve_exact(unit.bit_len)
            .map_err(|_| invalid("EP1 ESC information allocation failed"))?;
        bits.extend((0..unit.bit_len).map(|i| (unit.data[i / 8] >> (7 - i % 8)) & 1));
        classes.push(RecoveredClass { class_index, bits });
    }
    let root = sorted[root];
    Ok(Frame {
        payload: RecoveredFrame {
            set_index: 0,
            classes,
        },
        pts: root.pts,
        duration: root.duration,
        timescale: root.timescale,
    })
}
