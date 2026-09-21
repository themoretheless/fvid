//! VP9 compressed-header probability updates (specification section 6.3).
use super::{vp9::Header, vp9_bool::BoolDecoder, vp9_tables::*};
use crate::{Result, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probabilities {
    pub partition: [[u8; 3]; 16],
    pub y_mode: [[u8; 9]; 4],
    pub uv_mode: [[u8; 9]; 10],
    pub skip: [u8; 3],
    pub is_inter: [u8; 4],
    pub comp_mode: [u8; 5],
    pub comp_ref: [u8; 5],
    pub single_ref: [[u8; 2]; 5],
    pub mv_sign: [u8; 2],
    pub mv_bits: [[u8; 10]; 2],
    pub mv_class0_bit: [u8; 2],
    pub tx: [[[u8; 3]; 2]; 4],
    pub inter_mode: [[u8; 3]; 7],
    pub interp_filter: [[u8; 2]; 4],
    pub mv_joint: [u8; 3],
    pub mv_class: [[u8; 10]; 2],
    pub mv_class0_fr: [[[u8; 3]; 2]; 2],
    pub mv_class0_hp: [u8; 2],
    pub mv_fr: [[u8; 3]; 2],
    pub mv_hp: [u8; 2],
    pub coef: [[[[[[u8; 3]; 6]; 6]; 2]; 2]; 4],
}
impl Default for Probabilities {
    fn default() -> Self {
        Self {
            partition: DEFAULT_PARTITION_PROBS,
            y_mode: DEFAULT_Y_MODE_PROBS,
            uv_mode: DEFAULT_UV_MODE_PROBS,
            skip: DEFAULT_SKIP_PROB,
            is_inter: DEFAULT_IS_INTER_PROB,
            comp_mode: DEFAULT_COMP_MODE_PROB,
            comp_ref: DEFAULT_COMP_REF_PROB,
            single_ref: DEFAULT_SINGLE_REF_PROB,
            mv_sign: DEFAULT_MV_SIGN_PROB,
            mv_bits: DEFAULT_MV_BITS_PROB,
            mv_class0_bit: DEFAULT_MV_CLASS0_BIT_PROB,
            tx: DEFAULT_TX_PROBS,
            inter_mode: DEFAULT_INTER_MODE_PROBS,
            interp_filter: DEFAULT_INTERP_FILTER_PROBS,
            mv_joint: DEFAULT_MV_JOINT_PROBS,
            mv_class: DEFAULT_MV_CLASS_PROBS,
            mv_class0_fr: DEFAULT_MV_CLASS0_FR_PROBS,
            mv_class0_hp: DEFAULT_MV_CLASS0_HP_PROB,
            mv_fr: DEFAULT_MV_FR_PROBS,
            mv_hp: DEFAULT_MV_HP_PROB,
            coef: DEFAULT_COEF_PROBS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceMode {
    Single,
    Compound,
    Select,
}
#[derive(Clone, Debug)]
pub struct CompressedHeader {
    /// 0=4x4, 1=up to 8x8, 2=up to 16x16, 3=up to 32x32, 4=per-block selection.
    pub tx_mode: u8,
    pub reference_mode: ReferenceMode,
    pub probabilities: Probabilities,
}
impl CompressedHeader {
    /// Apply updates to a copy; the caller owns context reset, refresh and adaptation.
    pub fn parse(frame: &[u8], header: &Header, previous: &Probabilities) -> Result<Self> {
        if header.show_existing.is_some() {
            return Err(invalid("VP9 show-existing has no compressed header"));
        }
        let data = frame
            .get(header.compressed_header.clone())
            .ok_or_else(|| invalid("VP9 compressed header exceeds frame"))?;
        let mut b = BoolDecoder::new(data)?;
        let mut p = previous.clone();
        let mut tx_mode = 0;
        if !header.lossless() {
            tx_mode = b.literal(2)? as u8;
            if tx_mode == 3 {
                tx_mode += b.literal(1)? as u8;
            }
        }
        if tx_mode == 4 {
            for size in 1..4 {
                for context in &mut p.tx[size] {
                    update(&mut b, &mut context[..size])?;
                }
            }
        }
        for size in 0..=usize::from(tx_mode.min(3)) {
            if b.read(128)? {
                for plane in &mut p.coef[size] {
                    for reference in plane {
                        for (band, contexts) in reference.iter_mut().enumerate() {
                            for nodes in &mut contexts[..if band == 0 { 3 } else { 6 }] {
                                update(&mut b, nodes)?;
                            }
                        }
                    }
                }
            }
        }
        update(&mut b, &mut p.skip)?;
        let mut reference_mode = ReferenceMode::Single;
        if !header.is_intra() {
            for context in &mut p.inter_mode {
                update(&mut b, context)?;
            }
            if header.interpolation_filter.is_none() {
                for context in &mut p.interp_filter {
                    update(&mut b, context)?;
                }
            }
            update(&mut b, &mut p.is_inter)?;
            if header.sign_bias[1..]
                .iter()
                .any(|&s| s != header.sign_bias[0])
                && b.read(128)?
            {
                reference_mode = if b.read(128)? {
                    ReferenceMode::Select
                } else {
                    ReferenceMode::Compound
                };
            }
            if reference_mode == ReferenceMode::Select {
                update(&mut b, &mut p.comp_mode)?;
            }
            if reference_mode != ReferenceMode::Compound {
                for context in &mut p.single_ref {
                    update(&mut b, context)?;
                }
            }
            if reference_mode != ReferenceMode::Single {
                update(&mut b, &mut p.comp_ref)?;
            }
            for context in &mut p.y_mode {
                update(&mut b, context)?;
            }
            for context in &mut p.partition {
                update(&mut b, context)?;
            }
            update_mv(&mut b, &mut p.mv_joint)?;
            for i in 0..2 {
                update_mv(&mut b, std::slice::from_mut(&mut p.mv_sign[i]))?;
                update_mv(&mut b, &mut p.mv_class[i])?;
                update_mv(&mut b, std::slice::from_mut(&mut p.mv_class0_bit[i]))?;
                update_mv(&mut b, &mut p.mv_bits[i])?;
            }
            for i in 0..2 {
                for context in &mut p.mv_class0_fr[i] {
                    update_mv(&mut b, context)?;
                }
                update_mv(&mut b, &mut p.mv_fr[i])?;
            }
            if header.high_precision_mv {
                for i in 0..2 {
                    update_mv(&mut b, std::slice::from_mut(&mut p.mv_class0_hp[i]))?;
                    update_mv(&mut b, std::slice::from_mut(&mut p.mv_hp[i]))?;
                }
            }
        }
        b.finish()?;
        Ok(Self {
            tx_mode,
            reference_mode,
            probabilities: p,
        })
    }
}
fn update(b: &mut BoolDecoder<'_>, values: &mut [u8]) -> Result<()> {
    for p in values {
        if *p == 0 {
            return Err(invalid("zero VP9 update probability"));
        }
        if b.read(252)? {
            let delta = if !b.read(128)? {
                b.literal(4)?
            } else if !b.read(128)? {
                b.literal(4)? + 16
            } else if !b.read(128)? {
                b.literal(5)? + 32
            } else {
                let v = b.literal(7)?;
                if v < 65 {
                    v + 64
                } else {
                    2 * v - 1 + u32::from(b.read(128)?)
                }
            };
            *p = remap(delta as usize, *p)?;
        }
    }
    Ok(())
}
fn remap(delta: usize, probability: u8) -> Result<u8> {
    let v = i32::from(
        *INV_MAP_TABLE
            .get(delta)
            .ok_or_else(|| invalid("invalid VP9 probability delta"))?,
    );
    if probability == 0 {
        return Err(invalid("zero VP9 update probability"));
    }
    let m = i32::from(probability) - 1;
    let recenter = |v: i32, m: i32| {
        if v > 2 * m {
            v
        } else if v & 1 != 0 {
            m - ((v + 1) >> 1)
        } else {
            m + (v >> 1)
        }
    };
    Ok(if 2 * m <= 255 {
        1 + recenter(v, m)
    } else {
        255 - recenter(v, 254 - m)
    } as u8)
}
fn update_mv(b: &mut BoolDecoder<'_>, values: &mut [u8]) -> Result<()> {
    for p in values {
        if b.read(252)? {
            *p = ((b.literal(7)? << 1) | 1) as u8;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remapped_probabilities_are_nonzero_for_the_entire_domain() {
        for p in 1..=255 {
            for delta in 0..255 {
                assert_ne!(remap(delta, p).unwrap(), 0);
            }
        }
        assert!(remap(255, 128).is_err());
        assert!(remap(0, 0).is_err());
        assert_eq!(remap(0, 128).unwrap(), 124);
        assert_eq!(remap(19, 128).unwrap(), 255);
    }
    #[test]
    fn real_keyframe_compressed_header_and_partition_bounds() {
        let ivf = include_bytes!("../../tests/fixtures/vp9/header.ivf");
        let size = u32::from_le_bytes(ivf[32..36].try_into().unwrap()) as usize;
        let frame = &ivf[44..44 + size];
        let mut state = super::super::vp9::HeaderState::default();
        let h = state.parse(frame).unwrap();
        let defaults = Probabilities::default();
        let ch = CompressedHeader::parse(frame, &h, &defaults).unwrap();
        assert!(ch.tx_mode <= 4);
        assert_eq!(ch.reference_mode, ReferenceMode::Single);
        for end in h.compressed_header.start..h.compressed_header.end {
            let mut short = h.clone();
            short.compressed_header.end = end;
            assert!(CompressedHeader::parse(frame, &short, &defaults).is_err());
        }
        assert_eq!(defaults, Probabilities::default());
    }
}
