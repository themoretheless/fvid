//! HEVC residual syntax primitives, shared by the block coefficient reader.
use super::hevc_cabac::{HevcCabac, Syntax};
use crate::{Result, invalid};
pub trait ResidualBins {
    fn decision(&mut self, syntax: Syntax, increment: usize) -> Result<bool>;
    fn bypass(&mut self) -> Result<bool>;
    fn align_coefficient_bypass(&mut self) -> Result<()> {
        Err(invalid("residual bin reader does not implement HEVC alignment"))
    }
    fn rice_statistic(&self, _class: usize) -> PersistentRiceStatistic { Default::default() }
    fn set_rice_statistic(&mut self, _class: usize, _value: PersistentRiceStatistic) {}
}
impl ResidualBins for HevcCabac<'_> {
    fn align_coefficient_bypass(&mut self) -> Result<()> { HevcCabac::align_coefficient_bypass(self) }
    fn rice_statistic(&self, class: usize) -> PersistentRiceStatistic { self.rice_statistics[class] }
    fn set_rice_statistic(&mut self, class: usize, value: PersistentRiceStatistic) { self.rice_statistics[class] = value; }
    fn decision(&mut self, syntax: Syntax, increment: usize) -> Result<bool> {
        HevcCabac::decision(self, syntax, increment)
    }
    fn bypass(&mut self) -> Result<bool> {
        HevcCabac::bypass(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scan {
    Diagonal,
    Horizontal,
    Vertical,
}
/// Both context-coded prefixes precede either bypass suffix in the bitstream.
/// Coordinates are returned in sample order, after the vertical-scan swap.
pub fn last_position(
    b: &mut impl ResidualBins,
    log2_size: u8,
    chroma: bool,
    scan: Scan,
) -> Result<[usize; 2]> {
    if !(2..=5).contains(&log2_size) {
        return Err(invalid("invalid HEVC transform size"));
    }
    let (offset, shift) = if chroma {
        (15, log2_size - 2)
    } else {
        (
            3 * usize::from(log2_size - 2) + usize::from((log2_size - 1) >> 2),
            (log2_size + 1) >> 2,
        )
    };
    let max = usize::from(2 * log2_size - 1);
    let mut prefix = [0; 2];
    for (axis, syntax) in [Syntax::LastX, Syntax::LastY].into_iter().enumerate() {
        while prefix[axis] < max && b.decision(syntax, offset + (prefix[axis] >> shift))? {
            prefix[axis] += 1;
        }
    }
    let mut position = prefix;
    for axis in 0..2 {
        if prefix[axis] > 3 {
            let bits = (prefix[axis] >> 1) - 1;
            let mut suffix = 0;
            for _ in 0..bits {
                suffix = (suffix << 1) | usize::from(b.bypass()?);
            }
            position[axis] = ((2 + (prefix[axis] & 1)) << bits) + suffix;
        }
    }
    if scan == Scan::Vertical {
        position.swap(0, 1);
    }
    Ok(position)
}
/// coeff_abs_level_remaining with extended precision disabled.
/// Persistent adaptation may supply Rice parameters above the base-profile cap.
pub fn remaining_level(b: &mut impl ResidualBins, rice: u8) -> Result<u32> {
    if rice > 31 {
        return Err(invalid("HEVC Rice parameter exceeds coefficient storage"));
    }
    let mut prefix = 0u32;
    while prefix < 4 && b.bypass()? {
        prefix += 1;
    }
    let (mut value, mut width) = if prefix < 4 {
        (u64::from(prefix) << rice, rice)
    } else {
        let mut value = 4u64 << rice;
        let mut width = rice + 1;
        while b.bypass()? {
            value = value
                .checked_add(1u64 << width)
                .filter(|&v| v <= u64::from(u32::MAX))
                .ok_or_else(|| invalid("HEVC coefficient remainder overflow"))?;
            width += 1;
            if width > 31 {
                return Err(invalid("HEVC coefficient remainder code too long"));
            }
        }
        (value, width)
    };
    let mut suffix = 0u64;
    while width > 0 {
        suffix = (suffix << 1) | u64::from(b.bypass()?);
        width -= 1;
    }
    value += suffix;
    u32::try_from(value).map_err(|_| invalid("HEVC coefficient remainder overflow"))
}
/// Extended-precision remainder, H.265 9.3.3.11 and limited EGk (9.3.3.4).
/// This primitive does not by itself enable extended-precision picture decoding.
pub fn remaining_level_extended(
    b: &mut impl ResidualBins,
    rice: u8,
    bit_depth: u8,
) -> Result<u32> {
    if !(8..=16).contains(&bit_depth) {
        return Err(invalid("invalid extended HEVC coefficient bit depth"));
    }
    let range = (bit_depth + 6).max(15);
    if rice >= range {
        return Err(invalid("extended HEVC Rice parameter exceeds transform range"));
    }
    let mut prefix = 0u8;
    while prefix < 4 && b.bypass()? {
        prefix += 1;
    }
    let mut value = u64::from(prefix) << rice;
    let width = if prefix < 4 {
        rice
    } else {
        let order = rice + 1;
        let maximum = 28 - range;
        let mut extension = 0u8;
        while extension < maximum && b.bypass()? {
            extension += 1;
        }
        value += ((1u64 << extension) - 1) << order;
        if extension == maximum { range } else { extension + order }
    };
    let mut suffix = 0u64;
    for _ in 0..width {
        suffix = (suffix << 1) | u64::from(b.bypass()?);
    }
    u32::try_from(value + suffix).map_err(|_| invalid("HEVC coefficient remainder overflow"))
}

/// Non-persistent Rice state resets at each 4x4 coefficient group. Adaptation
/// uses the previous decoded absolute level, not merely its remainder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RiceState {
    parameter: u8,
    previous_absolute: u32,
}
/// Persistent range-extension statistic for one component/transform class.
/// The caller updates it only for the first coded remainder in a coefficient
/// group and resets all classes when initializing slice entropy contexts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PersistentRiceStatistic(u8);
impl PersistentRiceStatistic {
    pub fn parameter(self) -> u8 {
        self.0 / 4
    }
    pub fn observe_first_remainder(&mut self, remainder: u32) {
        let threshold = 1u64 << self.parameter();
        if u64::from(remainder) >= 3 * threshold {
            // u32 remainders cannot meet the increment threshold at k >= 31.
            self.0 += 1;
        } else if u64::from(remainder) * 2 < threshold && self.0 > 0 {
            self.0 -= 1;
        }
    }
}
impl RiceState {
    pub fn parameter(&self) -> u8 {
        self.parameter
    }
    pub fn decode(&mut self, b: &mut impl ResidualBins, base_level: u32) -> Result<u32> {
        self.decode_with_precision(b, base_level, None)
    }
    fn decode_with_precision(&mut self, b: &mut impl ResidualBins, base_level: u32,
        extended_depth: Option<u8>) -> Result<u32> {
        if !(1..=3).contains(&base_level) {
            return Err(invalid("invalid HEVC coefficient base level"));
        }
        let rice =
            (self.parameter + u8::from(self.previous_absolute > (3u32 << self.parameter))).min(4);
        let remainder = match extended_depth {
            Some(depth) => remaining_level_extended(b, rice, depth)?,
            None => remaining_level(b, rice)?,
        };
        let absolute = base_level
            .checked_add(remainder)
            .ok_or_else(|| invalid("HEVC absolute coefficient overflow"))?;
        self.parameter = rice;
        self.previous_absolute = absolute;
        Ok(absolute)
    }
}

fn scan_positions(side: usize, scan: Scan) -> &'static [[usize; 2]] {
    static SCANS: std::sync::OnceLock<[[Vec<[usize; 2]>; 4]; 3]> = std::sync::OnceLock::new();
    let scans = SCANS.get_or_init(|| {
        std::array::from_fn(|kind| {
            let scan = [Scan::Diagonal, Scan::Horizontal, Scan::Vertical][kind];
            std::array::from_fn(|log| make_scan(1 << log, scan))
        })
    });
    let kind = match scan {
        Scan::Diagonal => 0,
        Scan::Horizontal => 1,
        Scan::Vertical => 2,
    };
    &scans[kind][side.ilog2() as usize]
}
fn make_scan(side: usize, scan: Scan) -> Vec<[usize; 2]> {
    let mut positions = Vec::with_capacity(side * side);
    match scan {
        Scan::Horizontal => {
            for y in 0..side {
                for x in 0..side {
                    positions.push([x, y]);
                }
            }
        }
        Scan::Vertical => {
            for x in 0..side {
                for y in 0..side {
                    positions.push([x, y]);
                }
            }
        }
        Scan::Diagonal => {
            for sum in 0..2 * side - 1 {
                for x in 0..side {
                    if sum >= x && sum - x < side {
                        positions.push([x, sum - x]);
                    }
                }
            }
        }
    }
    positions
}
/// Read one Main/Main10 residual block into raster-order signed coefficients.
/// Transform-skip syntax is read by the caller before invoking this function.
/// `hide_sign` must already exclude transquant bypass and RDPCM cases.
/// Abort the slice on any error; consumed CABAC bins cannot be rolled back here.
pub fn read_block(
    b: &mut impl ResidualBins,
    log2_size: u8,
    chroma: bool,
    scan: Scan,
    hide_sign: bool,
) -> Result<Vec<i32>> {
    read_block_with_skip_context(b, log2_size, chroma, scan, hide_sign, false)
}
/// RExt significance-context override for actual transform-skip/bypass blocks.
pub fn read_block_with_skip_context(
    b: &mut impl ResidualBins,
    log2_size: u8,
    chroma: bool,
    scan: Scan,
    hide_sign: bool,
    skip_context: bool,
) -> Result<Vec<i32>> {
    read_block_with_rice(b, log2_size, chroma, scan, hide_sign, skip_context, None)
}
pub(crate) fn read_block_with_rice(
    b: &mut impl ResidualBins, log2_size: u8, chroma: bool, scan: Scan,
    hide_sign: bool, skip_context: bool, persistent_class: Option<usize>,
) -> Result<Vec<i32>> {
    read_block_with_precision(b, log2_size, chroma, scan, hide_sign, skip_context,
        persistent_class, None, false)
}
pub(crate) fn read_block_with_precision(
    b: &mut impl ResidualBins, log2_size: u8, chroma: bool, scan: Scan,
    hide_sign: bool, skip_context: bool, persistent_class: Option<usize>,
    extended_depth: Option<u8>, alignment: bool,
) -> Result<Vec<i32>> {
    let last = last_position(b, log2_size, chroma, scan)?;
    let side = 1usize << log2_size;
    let group_side = side / 4;
    let groups = scan_positions(group_side, scan);
    let inner = scan_positions(4, scan);
    let last_group = groups
        .iter()
        .position(|&p| p == [last[0] / 4, last[1] / 4])
        .unwrap();
    let last_local = inner
        .iter()
        .position(|&p| p == [last[0] % 4, last[1] % 4])
        .unwrap();
    let mut coded_storage = [false; 64];
    let coded = &mut coded_storage[..group_side * group_side];
    let mut output = vec![0i32; side * side];
    let mut previous_c1 = 1usize;
    for group in (0..=last_group).rev() {
        let [gx, gy] = groups[group];
        let explicit_group = group > 0 && group < last_group;
        let enabled = if explicit_group {
            b.decision(
                Syntax::CodedSubBlock,
                coded_group_context(log2_size, chroma, [gx, gy], &coded)?,
            )?
        } else {
            true
        };
        coded[gy * group_side + gx] = enabled;
        if !enabled {
            continue;
        }
        let mut significant = [false; 16];
        let mut infer_dc = explicit_group;
        let start = if group == last_group {
            significant[last_local] = true;
            last_local
        } else {
            16
        };
        for n in (0..start).rev() {
            let [x, y] = inner[n];
            significant[n] = if n == 0 && infer_dc {
                true
            } else {
                b.decision(
                    Syntax::SignificantCoefficient,
                    significance_context(
                        log2_size,
                        chroma,
                        scan,
                        [gx * 4 + x, gy * 4 + y],
                        &coded,
                        skip_context,
                    )?,
                )?
            };
            if significant[n] {
                infer_dc = false;
            }
        }
        let mut indices_storage = [0; 16];
        let mut count = 0;
        for n in (0..16).rev() {
            if significant[n] {
                indices_storage[count] = n;
                count += 1;
            }
        }
        let indices = &indices_storage[..count];
        if indices.is_empty() {
            continue;
        }
        let ctx_set = if group > 0 && !chroma { 2 } else { 0 } + usize::from(previous_c1 == 0);
        let mut c1 = 1usize;
        let mut levels = [1u32; 16];
        let mut first_greater = None;
        for &n in indices.iter().take(8) {
            let greater = b.decision(
                Syntax::Greater1,
                usize::from(chroma) * 16 + ctx_set * 4 + c1.min(3),
            )?;
            if greater {
                levels[n] = 2;
                if first_greater.is_none() {
                    first_greater = Some(n);
                }
            }
            if c1 > 0 {
                c1 = if greater { 0 } else { c1 + 1 };
            }
        }
        previous_c1 = c1;
        if let Some(n) = first_greater {
            levels[n] +=
                u32::from(b.decision(Syntax::Greater2, usize::from(chroma) * 4 + ctx_set)?);
        }
        // H.265 7.3.8.11: escapeDataPresent is set when a remainder is
        // coded. Alignment precedes the first sign and preserves input position.
        let escape = indices.iter().enumerate().any(|(ordinal, &n)| {
            let threshold = if ordinal >= 8 { 1 } else if Some(n) == first_greater { 3 } else { 2 };
            levels[n] == threshold
        });
        if alignment && escape { b.align_coefficient_bypass()?; }
        let lowest = *indices.last().unwrap();
        let hidden = hide_sign && indices[0] - lowest > 3;
        let mut negative = [false; 16];
        for &n in indices {
            if !hidden || n != lowest {
                negative[n] = b.bypass()?;
            }
        }
        let mut statistic = persistent_class.map(|class| b.rice_statistic(class));
        let mut first_remainder = true;
        let mut rice = RiceState::default();
        if let Some(statistic) = statistic { rice.parameter = statistic.parameter(); }
        let mut sum = 0u64;
        for (ordinal, &n) in indices.iter().enumerate() {
            let threshold = if ordinal >= 8 {
                1
            } else if Some(n) == first_greater {
                3
            } else {
                2
            };
            if levels[n] == threshold {
                if let Some(ref mut statistic) = statistic {
                    let remainder = match extended_depth {
                        Some(depth) => remaining_level_extended(b, rice.parameter, depth)?,
                        None => remaining_level(b, rice.parameter)?,
                    };
                    levels[n] = levels[n].checked_add(remainder)
                        .ok_or_else(|| invalid("HEVC absolute coefficient overflow"))?;
                    if first_remainder { statistic.observe_first_remainder(remainder); first_remainder = false; }
                    if u64::from(levels[n]) > (3u64 << rice.parameter) { rice.parameter += 1; }
                } else { levels[n] = rice.decode_with_precision(b, levels[n], extended_depth)?; }
            }
            sum += u64::from(levels[n]);
            if hidden && n == lowest {
                negative[n] = sum % 2 != 0;
            }
            let level = i32::try_from(levels[n])
                .map_err(|_| invalid("HEVC coefficient exceeds signed storage"))?;
            let [x, y] = inner[n];
            output[(gy * 4 + y) * side + gx * 4 + x] = if negative[n] { -level } else { level };
        }
        if let (Some(class), Some(statistic)) = (persistent_class, statistic) { b.set_rice_statistic(class, statistic); }
    }
    Ok(output)
}

fn group_neighbours(log2_size: u8, position: [usize; 2], coded: &[bool]) -> Result<[bool; 2]> {
    if !(2..=5).contains(&log2_size) {
        return Err(invalid("invalid HEVC group geometry"));
    }
    let side = 1usize << (log2_size - 2);
    let [x, y] = position;
    if coded.len() != side * side || x >= side || y >= side {
        return Err(invalid("invalid HEVC coefficient-group map"));
    }
    Ok([
        x + 1 < side && coded[y * side + x + 1],
        y + 1 < side && coded[(y + 1) * side + x],
    ])
}
/// Context for coded_sub_block_flag. Coordinates are in 4x4 coefficient groups.
pub fn coded_group_context(
    log2_size: u8,
    chroma: bool,
    group: [usize; 2],
    coded: &[bool],
) -> Result<usize> {
    let neighbours = group_neighbours(log2_size, group, coded)?;
    Ok(usize::from(chroma) * 2 + usize::from(neighbours[0] || neighbours[1]))
}
/// Context for sig_coeff_flag, in the local SignificantCoefficient bank.
/// `skip_context` means the range-extension skip/bypass context override is active.
pub fn significance_context(
    log2_size: u8,
    chroma: bool,
    scan: Scan,
    position: [usize; 2],
    coded: &[bool],
    skip_context: bool,
) -> Result<usize> {
    let [x, y] = position;
    let neighbours = group_neighbours(log2_size, [x / 4, y / 4], coded)?;
    if skip_context {
        return Ok(if chroma { 43 } else { 42 });
    }
    let base = usize::from(chroma) * 27;
    if log2_size == 2 {
        const MAP: [usize; 15] = [0, 1, 4, 5, 2, 3, 4, 5, 6, 6, 8, 8, 7, 7, 8];
        return MAP
            .get(y * 4 + x)
            .map(|&v| base + v)
            .ok_or_else(|| invalid("bottom-right 4x4 significance is inferred"));
    }
    if x + y == 0 {
        return Ok(base);
    }
    let (xp, yp) = (x % 4, y % 4);
    let pattern = usize::from(neighbours[0]) + 2 * usize::from(neighbours[1]);
    let local = match pattern {
        0 => {
            if xp + yp == 0 {
                2
            } else if xp + yp < 3 {
                1
            } else {
                0
            }
        }
        1 => {
            if yp == 0 {
                2
            } else if yp == 1 {
                1
            } else {
                0
            }
        }
        2 => {
            if xp == 0 {
                2
            } else if xp == 1 {
                1
            } else {
                0
            }
        }
        _ => 2,
    };
    if chroma {
        Ok(base + local + if log2_size == 3 { 9 } else { 12 })
    } else {
        Ok(local
            + if x / 4 + y / 4 > 0 { 3 } else { 0 }
            + if log2_size == 3 {
                if scan == Scan::Diagonal { 9 } else { 15 }
            } else {
                21
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    #[derive(Clone)]
    enum Bin {
        Context(Syntax, usize, bool),
        Bypass(bool),
        Align,
    }
    struct Script(VecDeque<Bin>);
    impl ResidualBins for Script {
        fn align_coefficient_bypass(&mut self) -> Result<()> {
            assert!(matches!(self.0.pop_front(), Some(Bin::Align)), "wrong alignment order");
            Ok(())
        }
        fn decision(&mut self, s: Syntax, c: usize) -> Result<bool> {
            let Some(Bin::Context(expected, index, value)) = self.0.pop_front() else {
                panic!("wrong bin order")
            };
            assert_eq!(
                std::mem::discriminant(&s),
                std::mem::discriminant(&expected)
            );
            assert_eq!(c, index);
            Ok(value)
        }
        fn bypass(&mut self) -> Result<bool> {
            let Some(Bin::Bypass(value)) = self.0.pop_front() else {
                panic!("wrong bypass order")
            };
            Ok(value)
        }
    }
    fn encoded_remainder(value: u32, rice: u8) -> Script {
        let mut output = VecDeque::new();
        let value = u64::from(value);
        let limit = 4u64 << rice;
        let prefix = value.min(limit) >> rice;
        for _ in 0..prefix {
            output.push_back(Bin::Bypass(true));
        }
        let (suffix, width) = if value < limit {
            output.push_back(Bin::Bypass(false));
            (value & ((1 << rice) - 1), rice)
        } else {
            let mut remainder = value - limit;
            let mut order = rice + 1;
            while remainder >= (1u64 << order) {
                output.push_back(Bin::Bypass(true));
                remainder -= 1u64 << order;
                order += 1;
            }
            output.push_back(Bin::Bypass(false));
            (remainder, order)
        };
        for bit in (0..width).rev() {
            output.push_back(Bin::Bypass(suffix & (1 << bit) != 0));
        }
        Script(output)
    }
    #[test]
    fn alignment_precedes_sign_only_when_escape_data_is_present() {
        for level in 1..=3 {
            let mut bins = Script(VecDeque::from([
                Bin::Context(Syntax::LastX, 0, false),
                Bin::Context(Syntax::LastY, 0, false),
                Bin::Context(Syntax::Greater1, 1, level > 1),
            ]));
            if level > 1 { bins.0.push_back(Bin::Context(Syntax::Greater2, 0, level > 2)); }
            if level == 3 { bins.0.push_back(Bin::Align); }
            bins.0.push_back(Bin::Bypass(true));
            if level == 3 { bins.0.push_back(Bin::Bypass(false)); }
            let block = read_block_with_precision(&mut bins, 2, false, Scan::Diagonal,
                true, false, None, Some(12), true).unwrap();
            assert_eq!(block[0], -level);
            assert!(block[1..].iter().all(|&v| v == 0));
            assert!(bins.0.is_empty());
        }
    }
    #[test]
    fn complete_dc_block_reads_level_sign_and_remainder() {
        let mut bins = Script(VecDeque::from([
            Bin::Context(Syntax::LastX, 0, false),
            Bin::Context(Syntax::LastY, 0, false),
            Bin::Context(Syntax::Greater1, 1, true),
            Bin::Context(Syntax::Greater2, 0, true),
            Bin::Bypass(true), // negative
            Bin::Bypass(true),
            Bin::Bypass(true),
            Bin::Bypass(false), // Rice remainder 2
        ]));
        let block = read_block(&mut bins, 2, false, Scan::Diagonal, true).unwrap();
        assert_eq!(block[0], -5);
        assert!(block[1..].iter().all(|&v| v == 0));
        assert!(bins.0.is_empty());
    }
    #[test]
    fn sign_hiding_infers_lowest_coefficient_from_group_parity() {
        let mut bins = Script(VecDeque::from([
            Bin::Context(Syntax::LastX, 0, true),
            Bin::Context(Syntax::LastX, 1, true),
            Bin::Context(Syntax::LastX, 2, true),
            Bin::Context(Syntax::LastY, 0, false),
        ]));
        // Last scan position 9=(3,0); only position0 is additionally significant.
        for context in [4, 6, 7, 4, 3, 6, 1, 2] {
            bins.0
                .push_back(Bin::Context(Syntax::SignificantCoefficient, context, false));
        }
        bins.0
            .push_back(Bin::Context(Syntax::SignificantCoefficient, 0, true));
        bins.0.extend([
            Bin::Context(Syntax::Greater1, 1, false),
            Bin::Context(Syntax::Greater1, 2, true),
            Bin::Context(Syntax::Greater2, 0, false),
            Bin::Bypass(false),
        ]);
        let block = read_block(&mut bins, 2, false, Scan::Diagonal, true).unwrap();
        assert_eq!(block[0], -2);
        assert_eq!(block[3], 1);
        assert_eq!(block.iter().filter(|&&v| v != 0).count(), 2);
        assert!(bins.0.is_empty());
    }
    #[test]
    fn groups_skip_infer_dc_carry_level_context_and_reset_rice() {
        let mut bins = Script(VecDeque::new());
        // 8x8 last position (4,4), the DC of diagonal group 3.
        for axis in [Syntax::LastX, Syntax::LastY] {
            for context in [3, 3, 4, 4] {
                bins.0.push_back(Bin::Context(axis, context, true));
            }
            bins.0.push_back(Bin::Context(axis, 5, false));
        }
        bins.0.extend([
            Bin::Bypass(false),
            Bin::Bypass(false),
            Bin::Context(Syntax::Greater1, 9, true),
            Bin::Context(Syntax::Greater2, 2, true),
            Bin::Bypass(false),
            Bin::Bypass(true),
            Bin::Bypass(false), // level 4, which would increase Rice if retained
            Bin::Context(Syntax::CodedSubBlock, 1, false), // group 2 absent
            Bin::Context(Syntax::CodedSubBlock, 1, true), // group 1 present
        ]);
        // No explicit significance: group 1 DC is inferred, consuming no bin.
        for context in [12, 12, 12, 13, 12, 12, 14, 13, 12, 12, 14, 13, 12, 14, 13] {
            bins.0
                .push_back(Bin::Context(Syntax::SignificantCoefficient, context, false));
        }
        bins.0.extend([
            Bin::Context(Syntax::Greater1, 13, true),
            Bin::Context(Syntax::Greater2, 3, true),
            Bin::Bypass(true),
            Bin::Bypass(false), // fresh Rice 0: level 3
        ]);
        // Group 0 is implicit, but its DC significance is explicit.
        for context in [9, 9, 9, 9, 9, 10, 9, 9, 10, 11, 9, 10, 11, 10, 11] {
            bins.0
                .push_back(Bin::Context(Syntax::SignificantCoefficient, context, false));
        }
        bins.0.extend([
            Bin::Context(Syntax::SignificantCoefficient, 0, true),
            Bin::Context(Syntax::Greater1, 5, true),
            Bin::Context(Syntax::Greater2, 1, false),
            Bin::Bypass(false),
        ]);
        for skip_context in [false, true] {
            let mut script = Script(bins.0.clone());
            if skip_context {
                for bin in &mut script.0 {
                    if let Bin::Context(Syntax::SignificantCoefficient, context, _) = bin {
                        *context = 42;
                    }
                }
            }
            let block = read_block_with_skip_context(
                &mut script, 3, false, Scan::Diagonal, true, skip_context,
            ).unwrap();
            assert_eq!(block[4 * 8 + 4], 4);
            assert_eq!(block[4 * 8], -3);
            assert_eq!(block[0], 2);
            assert_eq!(block.iter().filter(|&&v| v != 0).count(), 3);
            assert!(script.0.is_empty());
        }
    }
    #[test]
    fn rice_remainders_cross_prefix_escape_and_integer_boundaries() {
        for rice in 0..=31 {
            for value in (0..4096).chain([65535, 1 << 20, u32::MAX - 3, u32::MAX]) {
                let mut bins = encoded_remainder(value, rice);
                assert_eq!(remaining_level(&mut bins, rice).unwrap(), value);
                assert!(bins.0.is_empty());
            }
        }
        let mut too_long = Script(
            std::iter::repeat_with(|| Bin::Bypass(true))
                .take(64)
                .collect(),
        );
        assert!(remaining_level(&mut too_long, 0).is_err());
        assert!(remaining_level(&mut Script(VecDeque::new()), 32).is_err());
    }
    #[test]
    fn extended_remainder_normative_vectors_and_truncations() {
        struct Bounded(VecDeque<bool>);
        impl ResidualBins for Bounded {
            fn decision(&mut self, _: Syntax, _: usize) -> Result<bool> {
                Err(invalid("unexpected context bin"))
            }
            fn bypass(&mut self) -> Result<bool> {
                self.0.pop_front().ok_or_else(|| invalid("truncated bypass bins"))
            }
        }
        // TR prefixes followed by limited EGk extension and suffix.
        for (bits, rice, depth, expected) in [
            ("0", 0, 12, 0),
            ("1110", 0, 12, 3),
            ("11010", 2, 12, 10),
            ("111100", 0, 12, 4),
            ("111101", 0, 12, 5),
            ("11111000", 0, 12, 6),
            ("11111011", 0, 12, 9),
            ("1111110000", 0, 12, 10),
        ] {
            let bins: VecDeque<_> = bits.bytes().map(|b| b == b'1').collect();
            let mut full = Bounded(bins.clone());
            assert_eq!(remaining_level_extended(&mut full, rice, depth).unwrap(), expected);
            assert!(full.0.is_empty());
            for cut in 0..bins.len() {
                let mut short = Bounded(bins.iter().take(cut).cloned().collect());
                assert!(remaining_level_extended(&mut short, rice, depth).is_err());
            }
        }
        // At the maximum extension no terminating zero is consumed.
        for depth in [8u8, 10, 12, 16] {
            let range = (depth + 6).max(15);
            let maximum = 28 - range;
            let mut bins = Bounded(std::iter::repeat(true)
                .take(usize::from(4 + maximum + range)).collect());
            let expected = 4 + (((1u32 << maximum) - 1) << 1) + (1u32 << range) - 1;
            assert_eq!(remaining_level_extended(&mut bins, 0, depth).unwrap(), expected);
            assert!(bins.0.is_empty());
        }
        for (rice, depth) in [(0, 7), (0, 17), (18, 12)] {
            assert!(remaining_level_extended(&mut Bounded(VecDeque::new()), rice, depth).is_err());
        }
    }

    #[test]
    fn rice_adaptation_uses_previous_absolute_level_and_caps_at_four() {
        let mut state = RiceState::default();
        assert_eq!(state.decode(&mut encoded_remainder(1, 0), 3).unwrap(), 4);
        assert_eq!(state.decode(&mut encoded_remainder(4, 1), 3).unwrap(), 7);
        assert_eq!(state.parameter(), 1);
        assert_eq!(
            state.decode(&mut encoded_remainder(100, 2), 1).unwrap(),
            101
        );
        assert_eq!(
            state.decode(&mut encoded_remainder(100, 3), 1).unwrap(),
            101
        );
        assert_eq!(
            state.decode(&mut encoded_remainder(100, 4), 1).unwrap(),
            101
        );
        assert_eq!(state.decode(&mut encoded_remainder(0, 4), 1).unwrap(), 1);
        assert_eq!(state.parameter(), 4);
        let saved = state;
        assert!(
            state
                .decode(&mut encoded_remainder(u32::MAX, 4), 1)
                .is_err()
        );
        assert_eq!(state, saved);
    }
    #[test]
    fn persistent_statistic_uses_remainders_and_four_observations_per_parameter() {
        let mut statistic = PersistentRiceStatistic::default();
        for _ in 0..3 {
            statistic.observe_first_remainder(3);
            assert_eq!(statistic.parameter(), 0);
        }
        statistic.observe_first_remainder(3);
        assert_eq!(statistic.parameter(), 1);
        statistic.observe_first_remainder(1);
        assert_eq!(statistic.parameter(), 1);
        statistic.observe_first_remainder(0);
        assert_eq!(statistic.parameter(), 0);
        for _ in 0..1000 {
            statistic.observe_first_remainder(u32::MAX);
        }
        assert_eq!(statistic.parameter(), 31);
        for _ in 0..1000 {
            statistic.observe_first_remainder(0);
        }
        assert_eq!(statistic, PersistentRiceStatistic::default());
    }
    #[test]
    fn significance_contexts_obey_group_edges_and_component_banks() {
        let grid = [false; 4];
        assert_eq!(coded_group_context(3, false, [0, 0], &grid).unwrap(), 0);
        assert_eq!(
            coded_group_context(3, true, [0, 0], &[false, true, true, false]).unwrap(),
            3
        );
        // Values outside the right/bottom picture edge are never consulted.
        assert_eq!(coded_group_context(3, true, [1, 1], &[true; 4]).unwrap(), 2);
        assert_eq!(
            significance_context(3, false, Scan::Diagonal, [0, 0], &grid, false).unwrap(),
            0
        );
        assert_eq!(
            significance_context(3, true, Scan::Diagonal, [0, 0], &grid, false).unwrap(),
            27
        );
        for (flags, expected) in [
            ([false, false, false, false], 9),
            ([false, true, false, false], 9),
            ([false, false, true, false], 11),
            ([false, true, true, false], 11),
        ] {
            assert_eq!(
                significance_context(3, false, Scan::Diagonal, [0, 3], &flags, false).unwrap(),
                expected
            );
        }
        assert_eq!(
            significance_context(3, false, Scan::Horizontal, [4, 0], &grid, false).unwrap(),
            20
        );
        assert_eq!(
            significance_context(4, true, Scan::Diagonal, [4, 0], &[false; 16], false).unwrap(),
            41
        );
        assert_eq!(
            significance_context(2, false, Scan::Diagonal, [2, 2], &[false], false).unwrap(),
            8
        );
        assert_eq!(
            significance_context(2, true, Scan::Diagonal, [2, 2], &[false], true).unwrap(),
            43
        );
        assert!(significance_context(2, false, Scan::Diagonal, [3, 3], &[false], false).is_err());
        assert!(coded_group_context(3, false, [2, 0], &grid).is_err());
        assert!(significance_context(3, false, Scan::Diagonal, [8, 0], &grid, false).is_err());
    }
    #[test]
    fn every_position_size_component_and_scan_matches_normative_bins() {
        const START: [usize; 10] = [0, 1, 2, 3, 4, 6, 8, 12, 16, 24];
        const SUFFIX: [usize; 10] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3];
        let luma: [&[usize]; 4] = [
            &[0, 1, 2],
            &[3, 3, 4, 4, 5],
            &[6, 6, 7, 7, 8, 8, 9],
            &[10, 10, 11, 11, 12, 12, 13, 13, 14],
        ];
        let chroma: [&[usize]; 4] = [
            &[15, 16, 17],
            &[15, 15, 16, 16, 17],
            &[15, 15, 15, 15, 16, 16, 16],
            &[15, 15, 15, 15, 15, 15, 15, 15, 16],
        ];
        for log in 2..=5 {
            for component in [false, true] {
                let contexts = if component {
                    chroma[log - 2]
                } else {
                    luma[log - 2]
                };
                for x in 0..1 << log {
                    for y in 0..1 << log {
                        for scan in [Scan::Diagonal, Scan::Horizontal, Scan::Vertical] {
                            let raw = if scan == Scan::Vertical {
                                [y, x]
                            } else {
                                [x, y]
                            };
                            let prefix =
                                raw.map(|v| START.iter().rposition(|&start| start <= v).unwrap());
                            let mut script = Script(VecDeque::new());
                            for (axis, syntax) in
                                [Syntax::LastX, Syntax::LastY].into_iter().enumerate()
                            {
                                for (bin, &ctx) in contexts.iter().enumerate() {
                                    if bin > prefix[axis] {
                                        break;
                                    }
                                    script.0.push_back(Bin::Context(
                                        syntax,
                                        ctx,
                                        bin < prefix[axis],
                                    ));
                                }
                            }
                            for axis in 0..2 {
                                let suffix = raw[axis] - START[prefix[axis]];
                                for shift in (0..SUFFIX[prefix[axis]]).rev() {
                                    script.0.push_back(Bin::Bypass(suffix & (1 << shift) != 0));
                                }
                            }
                            assert_eq!(
                                last_position(&mut script, log as u8, component, scan).unwrap(),
                                [x, y]
                            );
                            assert!(script.0.is_empty());
                        }
                    }
                }
            }
        }
    }
}
