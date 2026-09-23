//! VP9 compressed-header updates and backward probability adaptation (sections 6.3 and 8.4).
use super::{vp9::Header, vp9_bool::BoolDecoder, vp9_tables::*};
use crate::{Result, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probabilities<T = u8> {
    pub partition: [[T; 3]; 16],
    pub y_mode: [[T; 9]; 4],
    pub uv_mode: [[T; 9]; 10],
    pub skip: [T; 3],
    pub is_inter: [T; 4],
    pub comp_mode: [T; 5],
    pub comp_ref: [T; 5],
    pub single_ref: [[T; 2]; 5],
    pub mv_sign: [T; 2],
    pub mv_bits: [[T; 10]; 2],
    pub mv_class0_bit: [T; 2],
    pub tx: [[[T; 3]; 2]; 4],
    pub inter_mode: [[T; 3]; 7],
    pub interp_filter: [[T; 2]; 4],
    pub mv_joint: [T; 3],
    pub mv_class: [[T; 10]; 2],
    pub mv_class0_fr: [[[T; 3]; 2]; 2],
    pub mv_class0_hp: [T; 2],
    pub mv_fr: [[T; 3]; 2],
    pub mv_hp: [T; 2],
    pub coef: [[[[[[T; 3]; 6]; 6]; 2]; 2]; 4],
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

impl<T: Copy> Probabilities<T> {
    pub(crate) fn filled(value: T) -> Self {
        Self {
            partition: [[value; 3]; 16],
            y_mode: [[value; 9]; 4],
            uv_mode: [[value; 9]; 10],
            skip: [value; 3],
            is_inter: [value; 4],
            comp_mode: [value; 5],
            comp_ref: [value; 5],
            single_ref: [[value; 2]; 5],
            mv_sign: [value; 2],
            mv_bits: [[value; 10]; 2],
            mv_class0_bit: [value; 2],
            tx: [[[value; 3]; 2]; 4],
            inter_mode: [[value; 3]; 7],
            interp_filter: [[value; 2]; 4],
            mv_joint: [value; 3],
            mv_class: [[value; 10]; 2],
            mv_class0_fr: [[[value; 3]; 2]; 2],
            mv_class0_hp: [value; 2],
            mv_fr: [[value; 3]; 2],
            mv_hp: [value; 2],
            coef: [[[[[[value; 3]; 6]; 6]; 2]; 2]; 4],
        }
    }
}
pub(crate) type Counts = Probabilities<[u32; 2]>;

pub(crate) fn counted(
    b: &mut BoolDecoder<'_>,
    probability: u8,
    counts: &mut [u32; 2],
) -> Result<bool> {
    let bit = b.read(probability)?;
    counts[usize::from(bit)] += 1;
    Ok(bit)
}

/// Counts are branch totals, equivalent to summing symbol frequencies below each tree node.
pub(crate) fn count_symbol(tree: &[i8], counts: &mut [[u32; 2]], symbol: u8) {
    fn visit(tree: &[i8], counts: &mut [[u32; 2]], at: usize, symbol: u8) -> bool {
        for bit in 0..2 {
            let child = tree[at + bit];
            let found = if child <= 0 {
                (-child) as u8 == symbol
            } else {
                visit(tree, counts, child as usize, symbol)
            };
            if found {
                counts[at / 2][bit] += 1;
                return true;
            }
        }
        false
    }
    let found = visit(tree, counts, 0, symbol);
    debug_assert!(found);
}

fn merge(pre: u8, c: [u32; 2], saturation: u64, factor: u64) -> u8 {
    let zero = u64::from(c[0]);
    let total = zero + u64::from(c[1]);
    if total == 0 {
        return pre;
    }
    let probability = ((zero * 256 + total / 2) / total).clamp(1, 255);
    let weight = factor * total.min(saturation) / saturation;
    ((u64::from(pre) * (256 - weight) + probability * weight + 128) >> 8) as u8
}

// Apply the same branch update to fixed probability arrays of any rank.
trait MergeCounts: Sized {
    type Counts;
    fn merged(&self, counts: &Self::Counts, saturation: u64, factor: u64) -> Self;
}
impl MergeCounts for u8 {
    type Counts = [u32; 2];
    fn merged(&self, counts: &Self::Counts, saturation: u64, factor: u64) -> Self {
        merge(*self, *counts, saturation, factor)
    }
}
impl<T: MergeCounts, const N: usize> MergeCounts for [T; N] {
    type Counts = [T::Counts; N];
    fn merged(&self, counts: &Self::Counts, saturation: u64, factor: u64) -> Self {
        std::array::from_fn(|i| self[i].merged(&counts[i], saturation, factor))
    }
}
impl Probabilities {
    pub(crate) fn adapt(
        &mut self,
        base: &Self,
        counts: &Counts,
        intra: bool,
        previous_key: bool,
        allow_hp: bool,
    ) {
        // Backward adaptation starts from the saved context, not this frame's
        // compressed-header updates. Intra frames retain updated skip/tx tables.
        let factor = if !intra && previous_key { 128 } else { 112 };
        self.coef = base.coef.merged(&counts.coef, 24, factor);
        if !intra {
            self.partition = base.partition.merged(&counts.partition, 20, 128);
            self.y_mode = base.y_mode.merged(&counts.y_mode, 20, 128);
            self.uv_mode = base.uv_mode.merged(&counts.uv_mode, 20, 128);
            self.skip = base.skip.merged(&counts.skip, 20, 128);
            self.is_inter = base.is_inter.merged(&counts.is_inter, 20, 128);
            self.comp_mode = base.comp_mode.merged(&counts.comp_mode, 20, 128);
            self.comp_ref = base.comp_ref.merged(&counts.comp_ref, 20, 128);
            self.single_ref = base.single_ref.merged(&counts.single_ref, 20, 128);
            self.mv_sign = base.mv_sign.merged(&counts.mv_sign, 20, 128);
            self.mv_bits = base.mv_bits.merged(&counts.mv_bits, 20, 128);
            self.mv_class0_bit = base.mv_class0_bit.merged(&counts.mv_class0_bit, 20, 128);
            self.tx = base.tx.merged(&counts.tx, 20, 128);
            self.inter_mode = base.inter_mode.merged(&counts.inter_mode, 20, 128);
            self.interp_filter = base.interp_filter.merged(&counts.interp_filter, 20, 128);
            self.mv_joint = base.mv_joint.merged(&counts.mv_joint, 20, 128);
            self.mv_class = base.mv_class.merged(&counts.mv_class, 20, 128);
            self.mv_class0_fr = base.mv_class0_fr.merged(&counts.mv_class0_fr, 20, 128);
            self.mv_fr = base.mv_fr.merged(&counts.mv_fr, 20, 128);
            if allow_hp {
                self.mv_class0_hp = base.mv_class0_hp.merged(&counts.mv_class0_hp, 20, 128);
                self.mv_hp = base.mv_hp.merged(&counts.mv_hp, 20, 128);
            }
        }
    }
}

#[cfg(test)]
mod adaptation_tests {
    use super::*;

    #[test]
    fn merge_rounding_saturation_and_empty_counts() {
        for p in 1..=255 {
            assert_eq!(merge(p, [0, 0], 20, 128), p);
            for c0 in 0u32..30 {
                for c1 in 0u32..30 {
                    if c0 + c1 == 0 {
                        continue;
                    }
                    let total = c0 + c1;
                    let empirical = ((256 * c0 + total / 2) / total).clamp(1, 255);
                    let weight = 128 * total.min(20) / 20;
                    let expected = (u32::from(p) * (256 - weight) + empirical * weight + 128) / 256;
                    assert_eq!(merge(p, [c0, c1], 20, 128), expected as u8);
                }
            }
        }
        assert_eq!(merge(128, [24, 0], 24, 112), 184);
        assert_eq!(merge(128, [24, 0], 24, 128), 192);
    }

    #[test]
    fn forced_partition_symbols_include_unread_tree_branches() {
        let mut counts = [[0; 2]; 3];
        count_symbol(&[0, 2, -1, 4, -2, -3], &mut counts, 3);
        count_symbol(&[0, 2, -1, 4, -2, -3], &mut counts, 1);
        assert_eq!(counts, [[0, 2], [1, 1], [0, 1]]);
    }

    #[test]
    fn adaptation_uses_saved_context_and_preserves_intra_header_updates() {
        let base = Probabilities::filled(128);
        let mut updated = Probabilities::filled(200);
        let mut counts = Counts::filled([0; 2]);
        counts.coef[0][0][0][0][0][0] = [24, 0];
        counts.skip[0] = [20, 0];
        updated.adapt(&base, &counts, true, true, false);
        assert_eq!(updated.coef[0][0][0][0][0], [184, 128, 128]);
        assert_eq!(updated.skip[0], 200);
        updated.adapt(&base, &counts, false, true, false);
        assert_eq!(updated.coef[0][0][0][0][0][0], 192);
        assert_eq!(updated.skip[0], 192);
    }
}
