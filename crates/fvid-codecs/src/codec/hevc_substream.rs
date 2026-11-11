//! H.265 F.10 extraction/independent rewriting of length-prefixed NAL sequences.
//! The caller supplies a dependency-closed layer list from the active VPS.
//! F.10.2 rewrites a capability-checked independent layer to layer zero.
use crate::codec::{
    config::NalUnits,
    hevc_nal::{NalHeader, NalRbsp},
};
use crate::{Result, invalid};

#[derive(Clone, Copy)]
enum Extraction {
    Ordinary { reduced: bool },
    Additional { rewrite_vps: bool },
    Independent { layer: u8 },
}
fn keep(
    nal: &[u8],
    h: NalHeader,
    layers: u64,
    temporal: u8,
    policy: Extraction,
    budget: usize,
) -> Result<bool> {
    if let Extraction::Independent { layer } = policy {
        if h.temporal_id > temporal || h.unit_type == 32 {
            return Ok(false);
        }
        return Ok(match h.unit_type {
            33 | 34 => h.layer_id == 0 || h.layer_id == layer,
            37 => true,
            _ => h.layer_id == layer,
        });
    }
    if let Extraction::Additional { .. } = policy {
        if h.temporal_id > temporal {
            return Ok(false);
        }
        return Ok(match h.unit_type {
            32..=34 | 36 => h.layer_id == 0 || layers & (1u64 << h.layer_id) != 0,
            37 => true,
            _ => layers & (1u64 << h.layer_id) != 0,
        });
    }
    if h.layer_id == 63 || layers & (1u64 << h.layer_id) == 0 || h.temporal_id > temporal {
        return Ok(false);
    }
    if matches!(policy, Extraction::Ordinary { reduced: true })
        && h.layer_id == 0
        && matches!(h.unit_type, 39 | 40)
    {
        let rbsp = NalRbsp::parse(nal, budget)?;
        let mut at = 0usize;
        let mut timing = false;
        while at < rbsp.bytes.len() {
            if rbsp.bytes[at] == 128 && rbsp.bytes[at + 1..].iter().all(|&v| v == 0) {
                return Ok(!timing);
            }
            let mut code = || -> Result<usize> {
                let mut value = 0usize;
                loop {
                    let byte = *rbsp
                        .bytes
                        .get(at)
                        .ok_or_else(|| invalid("HEVC extraction truncated SEI field"))?;
                    at += 1;
                    value = value
                        .checked_add(byte as usize)
                        .ok_or_else(|| invalid("HEVC extraction SEI field overflow"))?;
                    if byte != 255 {
                        return Ok(value);
                    }
                }
            };
            let kind = code()?;
            let size = code()?;
            at = at
                .checked_add(size)
                .filter(|&end| end <= rbsp.bytes.len())
                .ok_or_else(|| invalid("HEVC extraction SEI payload exceeds RBSP"))?;
            // Scalable-nested messages are opaque: only top-level timing applies.
            timing |= matches!(kind, 0 | 1 | 130);
        }
        return Err(invalid("HEVC extraction SEI missing trailing bits"));
    }
    Ok(true)
}

/// Extract a complete supplied sequence, preserving retained NAL bytes and order.
/// Reduction is determined across the whole input, not independently per AU:
/// base-layer timing SEI may precede the first removed picture. To extract an
/// ongoing stream, first establish its global operation point; do not call this
/// separately per packet and assume packet-local timing metadata is sufficient.
/// Budget bounds additional output and temporary RBSP storage; input is borrowed.
pub fn extract_layered_substream(
    input: &[u8],
    length_size: u8,
    layers: u64,
    temporal: u8,
    budget: usize,
) -> Result<Vec<u8>> {
    if layers == 0 || layers >> 63 != 0 || temporal > 6 {
        return Err(invalid("invalid HEVC extraction target"));
    }
    let mut reduced = false;

    for nal in NalUnits::new(input, length_size)? {
        let nal = nal?;
        let h = NalHeader::parse(nal)?;
        reduced |= layers & (1u64 << h.layer_id) == 0 || h.temporal_id > temporal;
    }
    extract_with_policy(
        input,
        length_size,
        layers,
        temporal,
        Extraction::Ordinary { reduced },
        budget,
    )
}

fn extract_with_policy(
    input: &[u8],
    length_size: u8,
    layers: u64,
    temporal: u8,
    policy: Extraction,
    budget: usize,
) -> Result<Vec<u8>> {
    let reduced = matches!(policy, Extraction::Ordinary { reduced: true });
    let mut scratch = 0usize;
    for nal in NalUnits::new(input, length_size)? {
        let nal = nal?;
        let h = NalHeader::parse(nal)?;
        if h.layer_id == 0
            && layers & 1 != 0
            && h.temporal_id <= temporal
            && matches!(h.unit_type, 39 | 40)
        {
            scratch = scratch.max(nal.len());
        }
    }
    let mut size = 0usize;
    for nal in NalUnits::new(input, length_size)? {
        let nal = nal?;
        if keep(
            nal,
            NalHeader::parse(nal)?,
            layers,
            temporal,
            policy,
            budget,
        )? {
            if matches!(policy, Extraction::Additional { rewrite_vps: true })
                && NalHeader::parse(nal)?.unit_type == 32
                && (nal.len() < 4 || nal[2] & 8 == 0 && (nal[2] & 3) == 0 && nal[3] >> 4 == 0)
            {
                return Err(invalid("HEVC additional extraction VPS header is invalid"));
            }
            size = size
                .checked_add(length_size as usize)
                .and_then(|s| s.checked_add(nal.len()))
                .ok_or_else(|| invalid("HEVC extraction output size overflow"))?;
        }
    }
    let overhead = std::mem::size_of::<Vec<u8>>() + std::mem::size_of::<NalRbsp>();
    let required = size
        .checked_add(if reduced { scratch } else { 0 })
        .and_then(|s| s.checked_add(overhead))
        .ok_or_else(|| invalid("HEVC extraction storage overflow"))?;
    if required > budget {
        return Err(invalid("HEVC extraction exceeds memory budget"));
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(size)
        .map_err(|_| invalid("cannot allocate HEVC extracted substream"))?;
    for nal in NalUnits::new(input, length_size)? {
        let nal = nal?;
        if keep(
            nal,
            NalHeader::parse(nal)?,
            layers,
            temporal,
            policy,
            budget - size - overhead,
        )? {
            output.extend_from_slice(&(nal.len() as u32).to_be_bytes()[4 - length_size as usize..]);
            let start = output.len();
            output.extend_from_slice(nal);
            if matches!(policy, Extraction::Additional { rewrite_vps: true })
                && NalHeader::parse(nal)?.unit_type == 32
            {
                // F.10.3 changes only vps_base_layer_available_flag. Internal
                // base makes this byte nonzero; external base requires a nonzero
                // max_layers_minus1 spanning this byte and the next. Thus the
                // first two RBSP bytes cannot both become zero, and EBSP
                // escaping/NAL length cannot change when bit 2 is cleared.
                output[start + 2] &= !4;
            } else if matches!(policy, Extraction::Independent { .. }) {
                // Only nuh_layer_id changes. NAL type, temporal ID and the
                // complete EBSP payload/length stay byte-identical.
                output[start] &= 0xfe;
                output[start + 1] &= 7;
            }
        }
    }
    Ok(output)
}

/// Header inventory of the complete source CVSG, including non-VCL NAL units.
/// Observe every packet before finishing a finite source; an incomplete scan
/// cannot determine whether earlier timing SEI applies to later removed layers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubstreamScan {
    layers: u64,
    highest_temporal: Option<u8>,
}
impl SubstreamScan {
    /// Combine contiguous inventories belonging to the same activated VPS.
    pub fn merged(self, other: Self) -> Self {
        Self {
            layers: self.layers | other.layers,
            highest_temporal: self.highest_temporal.max(other.highest_temporal),
        }
    }

    /// Atomic on malformed framing/header: the previous inventory is unchanged.
    pub fn observe(&mut self, packet: &[u8], length_size: u8) -> Result<()> {
        let mut next = *self;
        for nal in NalUnits::new(packet, length_size)? {
            let h = NalHeader::parse(nal?)?;
            next.layers |= 1u64 << h.layer_id;
            next.highest_temporal = Some(next.highest_temporal.unwrap_or(0).max(h.temporal_id));
        }
        *self = next;
        Ok(())
    }
    /// F-57 resolves necessary decoding layers, not merely output layers.
    /// The immutable VPS snapshot pins this selection for the source CVSG.
    pub fn select(
        self,
        vps: std::sync::Arc<crate::codec::hevc_vps::LayeredVps>,
        ols: usize,
        temporal: u8,
    ) -> Result<LayeredSubstream> {
        if temporal > 6 {
            return Err(invalid("invalid HEVC extraction target"));
        }
        let (layers, additional) = if let Some(extension) = &vps.extension {
            let set = extension
                .output
                .sets
                .get(ols)
                .ok_or_else(|| invalid("HEVC extraction output layer set is absent"))?;
            let additional = if set.layer_set_index >= vps.vps.layers.sets.len() {
                let list = extension
                    .layer_sets
                    .layer_sets
                    .get(set.layer_set_index)
                    .ok_or_else(|| invalid("HEVC extraction additional layer set is absent"))?;
                let mut mask = 0u64;
                for &id in list {
                    if id > 62 || mask & (1u64 << id) != 0 {
                        return Err(invalid("invalid HEVC extraction additional layer list"));
                    }
                    mask |= 1u64 << id;
                }
                if mask == 0 || mask & 1 != 0 || set.necessary & !mask != 0 {
                    return Err(invalid("invalid HEVC extraction additional layer list"));
                }
                Some(mask)
            } else {
                None
            };
            let layers = set.necessary;
            let mut known = 0u64;
            for (i, &layer) in extension.graph.layer_ids.iter().enumerate() {
                if layer > 63 {
                    return Err(invalid("invalid HEVC extraction dependency graph"));
                }
                known |= 1u64 << layer;
                let references = *extension
                    .graph
                    .transitive
                    .get(i)
                    .ok_or_else(|| invalid("invalid HEVC extraction dependency graph"))?;
                if layers & (1u64 << layer) != 0 && references & !layers != 0 {
                    return Err(invalid("HEVC extraction target omits a reference layer"));
                }
                if additional
                    .is_some_and(|mask| mask & (1u64 << layer) != 0 && references & !mask != 0)
                {
                    return Err(invalid(
                        "HEVC additional extraction omits a reference layer",
                    ));
                }
            }
            if layers & !known != 0 || additional.is_some_and(|mask| mask & !known != 0) {
                return Err(invalid("HEVC extraction names an absent layer"));
            }
            (layers, additional)
        } else if ols == 0 {
            (1, None)
        } else {
            return Err(invalid("HEVC extraction output layer set is absent"));
        };
        if !vps.vps.layers.base_internal && additional.is_none() {
            return Err(invalid(
                "HEVC extraction external base requires an additional layer set",
            ));
        }
        if !vps.vps.layers.base_available && additional.is_none() {
            return Err(invalid(
                "HEVC extraction unavailable base requires an additional layer set",
            ));
        }
        if layers == 0 || layers >> 63 != 0 {
            return Err(invalid("invalid HEVC extraction target"));
        }
        Ok(LayeredSubstream {
            vps,
            ols,
            layers,
            additional,
            temporal,
            source: self,
            reduced: self.layers & !layers != 0
                || self.highest_temporal.is_some_and(|t| t > temporal),
        })
    }
}

/// Packet extractor using the reduction decision from a completed source scan.
#[derive(Clone, Debug)]
pub struct LayeredSubstream {
    pub(crate) vps: std::sync::Arc<crate::codec::hevc_vps::LayeredVps>,
    pub(crate) ols: usize,
    layers: u64,
    additional: Option<u64>,
    temporal: u8,
    source: SubstreamScan,
    reduced: bool,
}
impl LayeredSubstream {
    /// Retained selection and its immutable parsed VPS, for source budgeting.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.vps.storage_bytes())
            .saturating_add(2 * std::mem::size_of::<usize>())
    }
    pub fn highest_temporal_id(&self) -> u8 {
        self.temporal
    }
    pub fn layers(&self) -> u64 {
        self.layers
    }
    pub fn output_layer_set(&self) -> usize {
        self.ols
    }
    /// F.10.3 exports the complete additional layer set, which may include
    /// layers unnecessary for the selected OLS's native reconstruction.
    pub fn extracted_layers(&self) -> u64 {
        self.additional.unwrap_or(self.layers)
    }
    /// F.10.2 independent non-base layer rewriting. The selected OLS must
    /// consist of one independent layer beginning an additional layer set,
    /// and its effective PTL must signal Annex A INBLD at the target temporal
    /// layer. Output deliberately contains no VPS; retained NALs use layer 0.
    pub fn rewrite_independent_packet(
        &self,
        packet: &[u8],
        length_size: u8,
        budget: usize,
    ) -> Result<Vec<u8>> {
        let layer = self.layers.trailing_zeros() as u8;
        let ext = self
            .vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC independent-layer rewrite requires a layered VPS"))?;
        let set = ext
            .output
            .sets
            .get(self.ols)
            .ok_or_else(|| invalid("HEVC independent-layer rewrite OLS is absent"))?;
        let index = ext.graph.layer_ids.iter().position(|&id| id == layer);
        if self.layers.count_ones() != 1
            || layer == 0
            || layer > 62
            || set.output != self.layers
            || set.necessary != self.layers
            || index
                .and_then(|i| ext.graph.direct.get(i))
                .is_none_or(|&refs| refs != 0)
            || !ext
                .layer_sets
                .layer_sets
                .iter()
                .skip(self.vps.vps.layers.sets.len())
                .any(|layers| layers.first() == Some(&layer))
        {
            return Err(invalid(
                "HEVC independent-layer rewrite requires one independent additional-set root",
            ));
        }
        let maximum = self
            .vps
            .vps
            .ordering
            .len()
            .checked_sub(1)
            .filter(|&n| n <= 6)
            .ok_or_else(|| invalid("invalid HEVC independent-layer temporal metadata"))?;
        let temporal = (self.temporal as usize).min(maximum);
        let profile = {
            let scratch = self
                .vps
                .vps
                .profile
                .sub_layers
                .len()
                .checked_mul(std::mem::size_of::<crate::codec::hevc_profile::SubLayer>())
                .and_then(|n| {
                    n.checked_add(std::mem::size_of::<
                        crate::codec::hevc_profile::ProfileTierLevel,
                    >())
                })
                .ok_or_else(|| invalid("HEVC independent-layer profile storage overflow"))?;
            if scratch > budget {
                return Err(invalid(
                    "HEVC independent-layer profile check exceeds memory budget",
                ));
            }
            let ptl = self.vps.profile_for_layer(self.ols, layer)?;
            if temporal < maximum {
                ptl.sub_layers
                    .get(temporal)
                    .and_then(|p| p.profile)
                    .or(ptl.profile)
            } else {
                ptl.profile
            }
        };
        if !profile.is_some_and(|p| p.inbld()) {
            return Err(invalid(
                "HEVC independent-layer rewrite requires signalled INBLD capability",
            ));
        }
        let mut seen = SubstreamScan::default();
        seen.observe(packet, length_size)?;
        if seen.layers & !self.source.layers != 0
            || seen.highest_temporal > self.source.highest_temporal
        {
            return Err(invalid("HEVC packet exceeds scanned extraction source"));
        }
        extract_with_policy(
            packet,
            length_size,
            self.layers,
            self.temporal,
            Extraction::Independent { layer },
            budget,
        )
    }
    pub fn extract_packet(&self, packet: &[u8], length_size: u8, budget: usize) -> Result<Vec<u8>> {
        self.extract_checked(packet, length_size, budget, false)
    }
    /// The owned decoder keeps the original VPS namespace and immutable bindings.
    /// Filter coded layers to the necessary OLS without rewriting that namespace;
    /// exported sub-bitstreams separately apply the required F.10.3 VPS change.
    pub(super) fn extract_for_decode(
        &self,
        packet: &[u8],
        length_size: u8,
        budget: usize,
    ) -> Result<Vec<u8>> {
        self.extract_checked(packet, length_size, budget, true)
    }
    fn extract_checked(
        &self,
        packet: &[u8],
        length_size: u8,
        budget: usize,
        decoding: bool,
    ) -> Result<Vec<u8>> {
        let mut seen = SubstreamScan::default();
        seen.observe(packet, length_size)?;
        if seen.layers & !self.source.layers != 0
            || seen.highest_temporal > self.source.highest_temporal
        {
            return Err(invalid("HEVC packet exceeds scanned extraction source"));
        }
        extract_with_policy(
            packet,
            length_size,
            if decoding {
                self.layers
            } else {
                self.extracted_layers()
            },
            self.temporal,
            if self.additional.is_some() {
                Extraction::Additional {
                    rewrite_vps: !decoding,
                }
            } else {
                Extraction::Ordinary {
                    reduced: self.reduced,
                }
            },
            budget,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pack(nals: &[&[u8]], length: u8) -> Vec<u8> {
        nals.iter()
            .flat_map(|n| {
                (n.len() as u32).to_be_bytes()[4 - length as usize..]
                    .iter()
                    .copied()
                    .chain(n.iter().copied())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    #[test]
    fn framing_opaque_discard_and_sei_message_walk() {
        let hdr = [78, 1, 144, 4, 0, 1, 0, 2, 128];
        // Full-NAL removal, including otherwise retained metadata in the same NAL.
        let mixed = [78, 1, 144, 4, 0, 1, 0, 2, 1, 0, 128];
        let future = [82, 9, 0, 0, 1]; // excluded layer 1: never de-escape invalid payload
        let valid = [2, 1, 128];
        for length in [1, 2, 4] {
            let input = pack(&[&hdr, &mixed, &future, &valid], length);
            assert_eq!(
                extract_layered_substream(&input, length, 1, 0, 4096).unwrap(),
                pack(&[&hdr, &valid], length)
            );
            let bad = pack(&[&[78, 1, 1, 255], &future], length);
            assert_eq!(
                extract_layered_substream(&bad, length, 1, 0, 4096)
                    .err()
                    .unwrap()
                    .to_string(),
                "HEVC extraction truncated SEI field"
            );
            let bad = pack(&[&[78, 1, 1, 5, 128], &future], length);
            assert_eq!(
                extract_layered_substream(&bad, length, 1, 0, 4096)
                    .err()
                    .unwrap()
                    .to_string(),
                "HEVC extraction SEI payload exceeds RBSP"
            );
        }
    }
}
