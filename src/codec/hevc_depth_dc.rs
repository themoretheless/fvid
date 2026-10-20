//! Annex I depth DC syntax primitives. Coding-unit integration is separate.
use super::{hevc_cabac::Syntax, hevc_residual::ResidualBins};
use crate::{Result, invalid};

/// I.7.3.8.5: CU skip suppresses the early skip_intra_flag.
pub fn read_skip_intra_flag(
    b: &mut impl ResidualBins,
    cu_skipped: bool,
    enabled: bool,
) -> Result<bool> {
    Ok(!cu_skipped && enabled && b.decision(Syntax::DepthSkipIntra, 0)?)
}

/// I.7.3.8.5.2 / table I.15: TR(cMax=3,k=0), first bin context coded.
/// Values 0/1 select angular 26/10; 2/3 select single-left/single-top.
pub fn read_skip_intra_mode(b: &mut impl ResidualBins) -> Result<u8> {
    if !b.decision(Syntax::DepthSkipMode, 0)? {
        return Ok(0);
    }
    if !b.bypass()? {
        return Ok(1);
    }
    Ok(if b.bypass()? { 3 } else { 2 })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntraMode {
    Ordinary,
    Wedge(usize),
    Contour,
}

/// I.7.3.8.5.1 / I.7.4.9.5.1 intra extension. Enable flags are for this layer.
pub fn read_intra_mode(
    b: &mut impl ResidualBins,
    log2_size: u8,
    enabled: [bool; 2],
    nal_type: u8,
    table: Option<&super::hevc_wedge::Table>,
) -> Result<IntraMode> {
    if !(2..=6).contains(&log2_size) || nal_type > 31 {
        return Err(invalid("invalid HEVC depth intra geometry or NAL type"));
    }
    if !enabled.iter().any(|&v| v) || log2_size == 6 || b.decision(Syntax::DepthNoDim, 0)? {
        return Ok(IntraMode::Ordinary);
    }
    let contour = if enabled == [true, true] {
        b.decision(Syntax::DepthIntraMode, 0)?
    } else {
        !enabled[0] || enabled[1]
    };
    if contour {
        if matches!(nal_type, 16..=20) {
            return Err(invalid("HEVC IRAP depth block cannot use contour"));
        }
        return Ok(IntraMode::Contour);
    }
    let table = table.ok_or_else(|| invalid("HEVC wedge mode requires indexed table"))?;
    if table.size() != 1usize << log2_size {
        return Err(invalid("HEVC wedge table disagrees with block size"));
    }
    Ok(IntraMode::Wedge(table.read_index(b)?))
}

/// Inputs to the dc_only_flag branch of I.7.3.8.5.2, after prediction syntax.
#[derive(Clone, Copy, Debug)]
pub struct CodingUnit {
    pub intra: bool,
    pub full_partition: bool,
    pub skipped: bool,
    pub skip_intra: bool,
    /// IntraDcOnlyWedgeEnabledFlag, InterDcOnlyEnabledFlag for this layer.
    pub enabled: [bool; 2],
}

/// Infer zero without consuming a bin whenever dc_only_flag is not present.
pub fn read_dc_only(b: &mut impl ResidualBins, cu: CodingUnit) -> Result<bool> {
    if cu.skip_intra || cu.skipped || !cu.full_partition || !cu.enabled[usize::from(!cu.intra)] {
        return Ok(false);
    }
    b.decision(Syntax::DepthDcOnly, 0)
}

/// I.9.3.3.11: three context-coded truncated-unary bins, then bypass EG0.
fn absolute(b: &mut impl ResidualBins) -> Result<u32> {
    let mut value = 0u32;
    while value < 3 {
        if !b.decision(Syntax::DepthDcAbs, 0)? {
            return Ok(value);
        }
        value += 1;
    }
    let mut order = 0;
    while b.bypass()? {
        // Preserve a signed offset domain and bound hostile unary input.
        if order >= 30 {
            return Err(invalid("HEVC depth DC magnitude overflow"));
        }
        value += 1 << order;
        order += 1;
    }
    for shift in (0..order).rev() {
        value += u32::from(b.bypass()?) << shift;
    }
    if value > i32::MAX as u32 - 1 {
        return Err(invalid("HEVC depth DC magnitude overflow"));
    }
    Ok(value)
}

/// Read one eligible prediction block, I.7.3.8.5.3 and equation I-51.
/// `dim` selects two segments; only intra DC-only blocks signal presence.
/// The unused second offset is zero. Callers must select eligible blocks.
pub fn read_offsets(
    b: &mut impl ResidualBins,
    intra: bool,
    dc_only: bool,
    dim: bool,
) -> Result<[i32; 2]> {
    if !dc_only && !dim {
        return Err(invalid("HEVC depth DC block is not eligible"));
    }
    let present = !(intra && dc_only) || b.decision(Syntax::DepthDcPresent, 0)?;
    let segments = if dim { 2 } else { 1 };
    let mut offsets = [0; 2];
    for offset in &mut offsets[..segments] {
        // Absent syntax infers abs=0 and sign=0; for one segment this is +1.
        let magnitude = if present { absolute(b)? } else { 0 } + (2 - segments) as u32;
        let negative = present && magnitude > 0 && b.bypass()?;
        *offset = if negative {
            -(magnitude as i32)
        } else {
            magnitude as i32
        };
    }
    Ok(offsets)
}

/// Coding-unit depth_dcs selection, I.7.3.8.5 and I.7.3.8.5.3.
/// NxN entries are in raster order (top-left, top-right, bottom-left,
/// bottom-right); other partitions use entry zero. Ineligible entries stay zero.
pub fn read_cu_offsets(
    b: &mut impl ResidualBins,
    intra: bool,
    intra_nxn: bool,
    dc_only: bool,
    skip_intra: bool,
    dim: [bool; 4],
) -> Result<[[i32; 2]; 4]> {
    if intra_nxn && (!intra || dc_only) || skip_intra && dc_only {
        return Err(invalid("invalid HEVC depth DC coding-unit selection"));
    }
    let mut offsets = [[0; 2]; 4];
    if !dc_only && (!intra || skip_intra) {
        return Ok(offsets);
    }
    for index in 0..if intra_nxn { 4 } else { 1 } {
        if dim[index] || dc_only {
            offsets[index] = read_offsets(b, intra, dc_only, dim[index])?;
        }
    }
    Ok(offsets)
}
