//! Transactional Annex F.8.3.1 POC transitions across a complete access unit.
//! Reconstruction publishes `into_state` only after every picture succeeds.
use super::{Reset, ResetPeriods};
use crate::codec::{
    hevc_slice::{LayeredSliceContext, SliceHeader},
    hevc_vps::LayeredVps,
};
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Layer {
    previous: Option<i32>,
    decoded: bool,
    periods: ResetPeriods,
    bits: Option<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Graph {
    id: u8,
    internal: bool,
    aligned: bool,
    registered: u64,
    predicted: [u64; 63],
}
impl Graph {
    fn read(vps: &LayeredVps) -> Result<Self> {
        if vps.inferred_base_layer() {
            return Ok(Self {
                id: vps.vps.id,
                internal: true,
                aligned: true,
                registered: 1,
                predicted: [0; 63],
            });
        }
        let e = vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC layered POC requires resolved VPS extension"))?;
        let ids = &e.graph.layer_ids;
        if ids.is_empty()
            || ids.len() > 63
            || ids[0] != 0
            || ids.iter().any(|&id| id > 62)
            || ids.windows(2).any(|p| p[0] >= p[1])
            || e.graph.direct.len() != ids.len()
            || e.graph.transitive.len() != ids.len()
        {
            return Err(invalid("invalid HEVC layered POC graph"));
        }
        let mut graph = Self {
            id: vps.vps.id,
            internal: vps.vps.layers.base_internal,
            aligned: e.representations.poc_lsb_aligned,
            registered: 0,
            predicted: [0; 63],
        };
        let mut closure = [0u64; 63];
        for (i, &id) in ids.iter().enumerate() {
            let direct = e.graph.direct[i];
            if direct & !graph.registered != 0 {
                return Err(invalid("invalid HEVC layered POC dependencies"));
            }
            closure[i] = direct;
            for (j, &reference) in ids[..i].iter().enumerate() {
                if direct & (1u64 << reference) != 0 {
                    closure[i] |= closure[j];
                }
            }
            if closure[i] != e.graph.transitive[i] {
                return Err(invalid("inconsistent HEVC layered POC dependency closure"));
            }
            for &reference in &ids[..i] {
                if closure[i] & (1u64 << reference) != 0 {
                    graph.predicted[usize::from(reference)] |= 1u64 << id;
                }
            }
            graph.registered |= 1u64 << id;
        }
        Ok(graph)
    }
}
/// One already parsed first slice and its exact active parameter binding.
pub struct PocPicture<'a> {
    pub header: &'a SliceHeader,
    pub context: &'a LayeredSliceContext<'a>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DerivedPoc {
    pub layer_id: u8,
    pub poc: i32,
    pub resetting: bool,
}
/// Fixed-size state; no decoded sample buffers or hidden DPB ownership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredPocState {
    layers: [Layer; 63],
    graph: Option<Graph>,
    period: Option<u8>,
    period_full: Option<bool>,
    previous_initial: bool,
    decremented: u64,
    ended: u64,
    restart_all: bool,
}
impl Default for LayeredPocState {
    fn default() -> Self {
        Self {
            layers: [Layer::default(); 63],
            graph: None,
            period: None,
            period_full: None,
            previous_initial: false,
            decremented: 0,
            ended: 0,
            restart_all: false,
        }
    }
}
/// Candidate POC/DPB metadata changes. Original state is never mutated on error.
#[derive(Clone, Debug)]
pub struct PocTransition {
    next: LayeredPocState,
    pictures: [Option<DerivedPoc>; 63],
    decrements: [i64; 63],
}
fn idc(reset: Option<Reset>) -> u8 {
    match reset {
        None => 0,
        Some(Reset::Msb) => 1,
        Some(Reset::Full) => 2,
        Some(Reset::Signalled { .. }) => 3,
    }
}
impl LayeredPocState {
    /// End the signalled layers and every layer predicted from them. No pixel
    /// ownership lives here; the DPB owner drops the returned reference layers.
    pub fn end_sequences(&mut self, sources: u64) -> Result<u64> {
        if sources >> 63 != 0 || self.graph.is_some_and(|g| sources & !g.registered != 0) {
            return Err(invalid("HEVC EOS layer is not registered"));
        }
        let mut affected = sources;
        if let Some(graph) = self.graph {
            for id in 0..63 {
                if sources & (1u64 << id) != 0 {
                    affected |= graph.predicted[id];
                }
            }
        }
        for id in 0..63 {
            if affected & (1u64 << id) != 0 {
                self.layers[id] = Layer::default();
            }
        }
        self.ended |= sources;
        self.decremented &= !affected;
        if self.graph.is_some_and(|g| g.registered & !affected == 0) {
            self.graph = None;
            self.period = None;
            self.period_full = None;
            self.previous_initial = false;
            self.decremented = 0;
        }
        Ok(affected)
    }
    pub fn end_bitstream(&mut self) {
        *self = Self::default();
        self.restart_all = true;
    }
    pub fn previous(&self, layer_id: u8) -> Option<i32> {
        self.layers
            .get(usize::from(layer_id))
            .and_then(|l| l.previous)
    }
    pub fn preview(&self, vps: &LayeredVps, pictures: &[PocPicture<'_>]) -> Result<PocTransition> {
        if pictures.is_empty() || pictures.len() > 63 {
            return Err(invalid("invalid HEVC layered POC access unit size"));
        }
        let graph = Graph::read(vps)?;
        if self.graph.is_some_and(|old| old != graph) {
            // Base BLA or complete target IDR can begin a new CVSG.
            // History from removed layers and
            // reset periods belongs to the old graph, including after seek.
            let first = pictures[0].header.nal;
            let complete_idr = pictures.iter().all(|p| p.header.nal.is_idr())
                && vps.covers_idr_target(pictures[0].context.sps.output_layer_set,
                    pictures.iter().fold(0, |mask, p| mask | (1u64 << p.header.nal.layer_id)));
            let cross_layer_bla = first.is_idr()
                && pictures[0].header.layered.as_ref().is_some_and(|s| s.cross_layer_bla);
            if first.layer_id == 0 && (matches!(first.unit_type, 16..=18) || complete_idr || cross_layer_bla) {
                return Self::default().preview(vps, pictures);
            }
            return Err(invalid("HEVC layered POC configuration requires reset"));
        }
        let mut next = self.clone();
        if next.restart_all {
            if !pictures[0].header.nal.is_irap() {
                return Err(invalid("HEVC access unit after EOB must begin with IRAP"));
            }
            next.restart_all = false;
        }
        for picture in pictures {
            let h = picture.header;
            for source in 0..63 {
                if next.ended & (1u64 << source) == 0 {
                    continue;
                }
                let affected = (1u64 << source) | graph.predicted[source];
                if affected & (1u64 << h.nal.layer_id) != 0 {
                    if h.nal.layer_id as usize != source || !h.nal.is_irap() {
                        return Err(invalid(
                            "HEVC picture after EOS must restart its source layer with IRAP",
                        ));
                    }
                    next.ended &= !(1u64 << source);
                }
            }
        }
        next.graph = Some(graph);
        let mut derived = [None; 63];
        let mut decrements = [0i64; 63];
        let first = pictures[0].header;
        let syntax = first
            .layered
            .as_ref()
            .ok_or_else(|| invalid("HEVC layered POC has no parsed layered syntax"))?
            .poc_reset;
        let reset_idc = idc(syntax.reset);
        let mut seen = 0u64;
        let mut previous_id = None;
        for picture in pictures {
            let h = picture.header;
            let c = picture.context;
            c.validate_poc_header(h)?;
            if c.vps != vps {
                return Err(invalid("HEVC layered POC active VPS differs"));
            }
            if !h.first || h.dependent || previous_id.is_some_and(|id| id >= h.nal.layer_id) {
                return Err(invalid(
                    "HEVC layered POC pictures must be first slices in increasing layer order",
                ));
            }
            previous_id = Some(h.nal.layer_id);
            seen |= 1u64 << h.nal.layer_id;
            let r = h.layered.as_ref().unwrap().poc_reset;
            if h.nal.temporal_id != first.nal.temporal_id
                || idc(r.reset) != reset_idc
                || r.period != syntax.period
            {
                return Err(invalid(
                    "HEVC layered access unit disagrees on temporal ID or POC reset period/IDC",
                ));
            }
            if let Some(Reset::Signalled { full, .. }) = r.reset {
                if self.period == r.period
                    && self.period_full.is_some_and(|expected| expected != full)
                {
                    return Err(invalid(
                        "HEVC layered signalled reset disagrees with initial access unit",
                    ));
                }
            }
            let state = &next.layers[usize::from(h.nal.layer_id)];
            if state.bits.is_some_and(|bits| bits != c.sps.sps.poc_bits) {
                return Err(invalid("HEVC layered POC width change requires reset"));
            }
        }
        if seen & !graph.registered != 0 {
            return Err(invalid("HEVC layered POC has unregistered picture layer"));
        }
        if graph.internal {
            if let Some(base) = pictures.iter().find(|p| p.header.nal.layer_id == 0) {
                let nal = base.header.nal;
                if nal.is_idr() && pictures.iter().any(|p| !p.header.nal.is_idr()) && reset_idc != 2
                {
                    return Err(invalid(
                        "HEVC mixed base IDR access unit requires full POC reset",
                    ));
                }
                if !nal.is_idr() && reset_idc == 2 {
                    return Err(invalid(
                        "HEVC non-IDR base access unit cannot fully reset POC",
                    ));
                }
                if nal.is_irap()
                    && pictures.iter().any(|p| !p.header.nal.is_irap())
                    && !matches!(reset_idc, 1 | 2)
                {
                    return Err(invalid(
                        "HEVC mixed base IRAP access unit requires POC reset",
                    ));
                }
            }
        }
        if pictures.iter().any(|p| {
            p.header.nal.layer_id > 0
                && p.header.nal.is_idr()
                && pictures
                    .iter()
                    .any(|other| other.header.nal.unit_type != p.header.nal.unit_type)
        }) && !matches!(reset_idc, 1 | 2)
        {
            return Err(invalid(
                "HEVC mixed enhancement IDR types require POC reset",
            ));
        }
        let initial = matches!(reset_idc, 1 | 2);
        if initial && self.previous_initial && self.period == syntax.period {
            return Err(invalid(
                "consecutive HEVC layered initial resets reuse a period ID",
            ));
        }
        if initial || syntax.period.is_some() && syntax.period != self.period {
            next.period = syntax.period;
            next.period_full = if initial { Some(reset_idc == 2) } else { None };
            next.decremented = 0;
            for layer in &mut next.layers {
                layer.periods = ResetPeriods::default();
            }
        }
        next.previous_initial = initial;
        // F.8.1.3 resets FirstPicInLayerDecodedFlag before F.8.3.1 derives
        // POC. NoClras applies to all layers, including independent layers that
        // are not in the smallest layer's predicted closure. Retain POC header
        // history for ordinary CL-RAS omission; that alone is not a reset.
        let smallest = if let Some(extension) = &vps.extension {
            let ols = extension
                .output
                .sets
                .get(pictures[0].context.sps.output_layer_set)
                .ok_or_else(|| invalid("HEVC POC reset OLS is absent"))?;
            if ols.necessary == 0 {
                return Err(invalid("HEVC POC reset OLS has no necessary layer"));
            }
            ols.necessary.trailing_zeros() as u8
        } else if vps.inferred_base_layer() && pictures[0].context.sps.output_layer_set == 0 {
            0
        } else {
            return Err(invalid("HEVC POC reset OLS is absent"));
        };
        let mut reset_decoded = 0u64;
        for picture in pictures {
            let h = picture.header;
            if !h.nal.is_irap() {
                continue;
            }
            let id = h.nal.layer_id;
            let bit = 1u64 << id;
            let coded_bla = matches!(h.nal.unit_type, 16..=18)
                || h.nal.is_idr() && h.layered.as_ref().unwrap().cross_layer_bla;
            if id == smallest && (coded_bla || self.ended & ((1u64 << smallest) | 1) != 0) {
                reset_decoded |= graph.registered;
            } else if id > smallest && (coded_bla || self.ended & bit != 0) {
                reset_decoded |= graph.predicted[id as usize];
            }
        }
        for id in 0..63 {
            if reset_decoded & (1u64 << id) != 0 {
                next.layers[id].decoded = false;
            }
        }
        let mut access_poc = None;
        for picture in pictures {
            let h = picture.header;
            let c = picture.context;
            let id = usize::from(h.nal.layer_id);
            let syntax = h.layered.as_ref().unwrap();
            let reset = syntax.poc_reset;
            let state = next.layers[id];
            let bits = c.sps.sps.poc_bits;
            let (periods, period_reset) =
                state
                    .periods
                    .advance_layered(reset, h.nal, h.poc_lsb, syntax.discardable)?;
            let resetting =
                period_reset && (!graph.aligned || next.decremented & (1u64 << id) == 0);
            let affected = (1u64 << id)
                | if graph.aligned {
                    graph.predicted[id]
                } else {
                    0
                };
            let (poc, delta) = if resetting {
                super::derive_reset(
                    bits,
                    h.poc_lsb,
                    if state.decoded { state.previous } else { None },
                    reset
                        .reset
                        .ok_or_else(|| invalid("missing HEVC layered reset operation"))?,
                    reset.msb_cycle,
                )?
            } else {
                let modulus = 1i32 << bits;
                let msb = if let Some(cycle) = reset.msb_cycle {
                    i64::from(cycle) * i64::from(modulus)
                } else if !state.decoded || h.nal.is_idr() {
                    0
                } else {
                    let previous = state
                        .previous
                        .ok_or_else(|| invalid("missing HEVC layered previous POC"))?;
                    let old = previous.rem_euclid(modulus);
                    super::current_msb_wide(
                        h.poc_lsb.into(),
                        old.into(),
                        i64::from(previous) - i64::from(old),
                        modulus.into(),
                    )
                };
                (
                    i32::try_from(msb + i64::from(h.poc_lsb))
                        .map_err(|_| invalid("HEVC layered POC overflow"))?,
                    0,
                )
            };
            if access_poc.is_some_and(|value| value != poc) {
                return Err(invalid("HEVC access unit derived POCs disagree"));
            }
            access_poc = Some(poc);
            if resetting && state.decoded {
                for layer in 0..63 {
                    let mask = 1u64 << layer;
                    if affected & mask != 0 && next.decremented & mask == 0 {
                        decrements[layer] = delta;
                    }
                }
                next.decremented |= affected;
            }
            if super::updates_previous(h.nal) && !syntax.discardable {
                for layer in 0..63 {
                    if affected & (1u64 << layer) != 0 {
                        next.layers[layer].previous = Some(poc);
                    }
                }
            } else if matches!(reset.reset, Some(Reset::Signalled { .. }))
                && (!state.decoded || period_reset)
            {
                let Some(Reset::Signalled { full, lsb }) = reset.reset else {
                    unreachable!()
                };
                for layer in 0..63 {
                    if affected & (1u64 << layer) != 0 {
                        next.layers[layer].previous = Some(if full { 0 } else { lsb as i32 });
                    }
                }
            }
            next.layers[id].decoded = true;
            next.layers[id].bits = Some(bits);
            next.layers[id].periods = periods;
            derived[id] = Some(DerivedPoc {
                layer_id: h.nal.layer_id,
                poc,
                resetting,
            });
        }
        Ok(PocTransition {
            next,
            pictures: derived,
            decrements,
        })
    }
}
impl PocTransition {
    pub fn pictures(&self) -> impl Iterator<Item = DerivedPoc> + '_ {
        self.pictures.iter().flatten().copied()
    }
    pub fn decrement(&self, layer_id: u8) -> Result<i64> {
        self.decrements
            .get(usize::from(layer_id))
            .copied()
            .ok_or_else(|| invalid("HEVC DPB layer out of range"))
    }
    /// Translate DPB identities and collocated references using each saved
    /// reference's layer. Budget covers retained sources and metadata copies.
    pub fn translate_references(
        &self,
        references: &[(u8, crate::codec::hevc_motion::Reference)],
        budget: usize,
    ) -> Result<Vec<(u8, crate::codec::hevc_motion::Reference)>> {
        use crate::codec::hevc_motion::Reference;
        use std::sync::Arc;
        let graph = self
            .next
            .graph
            .ok_or_else(|| invalid("missing HEVC layered POC graph"))?;
        let mut retained = 0usize;
        for (layer, reference) in references {
            if *layer > 62 || reference.layer_id != *layer || graph.registered & (1u64 << layer) == 0 {
                return Err(invalid("HEVC translated DPB has unregistered layer"));
            }
            let source = reference
                .picture
                .as_ref()
                .ok_or_else(|| invalid("HEVC translated DPB has no picture"))?;
            retained = retained
                .checked_add(
                    source
                        .storage_bytes()
                        .ok_or_else(|| invalid("HEVC layered DPB storage overflow"))?,
                )
                .ok_or_else(|| invalid("HEVC layered DPB storage overflow"))?;
            if source.poc_translation_required(&self.decrements)? {
                retained = retained
                    .checked_add(
                        source
                            .poc_translation_bytes()
                            .ok_or_else(|| invalid("HEVC layered DPB metadata overflow"))?,
                    )
                    .ok_or_else(|| invalid("HEVC layered DPB metadata overflow"))?;
            }
        }
        let mut out = Vec::new();
        let requested = references
            .len()
            .checked_mul(std::mem::size_of::<(u8, Reference)>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Vec<(u8, Reference)>>()))
            .and_then(|n| n.checked_add(retained))
            .ok_or_else(|| invalid("HEVC layered DPB allocation overflow"))?;
        if requested > budget {
            return Err(invalid("HEVC layered DPB translation exceeds budget"));
        }
        out.try_reserve_exact(references.len())
            .map_err(|_| invalid("cannot allocate HEVC layered DPB translation"))?;
        if out
            .capacity()
            .checked_mul(std::mem::size_of::<(u8, Reference)>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Vec<(u8, Reference)>>()))
            .and_then(|n| n.checked_add(retained))
            .is_none_or(|n| n > budget)
        {
            return Err(invalid("HEVC layered DPB translation exceeds budget"));
        }
        for (layer, reference) in references {
            let delta = self.decrements[usize::from(*layer)];
            let poc = i64::from(reference.poc)
                .checked_sub(delta)
                .and_then(|n| i32::try_from(n).ok())
                .ok_or_else(|| invalid("HEVC layered translated POC overflow"))?;
            let source = reference.picture.as_ref().unwrap();
            let picture = if !source.poc_translation_required(&self.decrements)? {
                source.clone()
            } else {
                Arc::new(source.translated_layer_pocs(&self.decrements, budget)?)
            };
            out.push((
                *layer,
                Reference { layer_id: *layer,
                    poc,
                    long_term: reference.long_term,
                    picture: Some(picture),
                },
            ));
        }
        Ok(out)
    }
    pub fn translate_temporal_references(&self, references: &[(u8,crate::codec::hevc_motion::Reference)], budget: usize) -> Result<Vec<(u8,crate::codec::hevc_motion::Reference)>> {
        self.translate_references(references,budget)
    }
    pub fn into_state(self) -> LayeredPocState {
        self.next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{hevc_decoder::HevcDecoder, hevc_motion::Reference};
    use std::sync::Arc;
    #[test]
    fn layered_dpb_translation_preserves_published_pixels_and_shifts_nested_identities() {
        let data = include_bytes!(
            "../../tests/fixtures/playback-errors/hevc-poc-vps-aligned-cycle-only-cycle-only-synthetic.mp4"
        );
        let mut movie =
            crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default())
                .unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20).unwrap();
        let mut packet = Vec::new();
        let mut decoded = None;
        for index in 0..2 {
            movie.read_packet(0, index, &mut packet).unwrap();
            decoded = decoder.decode_packet(&packet).unwrap();
        }
        let decoded = decoded.unwrap();
        assert_eq!(decoded.poc, 33);
        let source = Arc::new(decoded.picture.translated_pocs(-324, 32 << 20).unwrap());
        assert!(source.reference_pocs.iter().flatten().any(|&p| p == 356));
        let graph = Graph {
            id: 0,
            internal: true,
            aligned: true,
            registered: (1 << 0) | (1 << 2) | (1 << 5) | (1 << 9),
            predicted: [0; 63],
        };
        let mut next = LayeredPocState::default();
        next.graph = Some(graph);
        let mut plan = PocTransition {
            next,
            pictures: [None; 63],
            decrements: [0; 63],
        };
        for id in [0, 2, 5, 9] {
            plan.decrements[id] = 256;
        }
        let references: Vec<_> = [0, 2, 5, 9]
            .into_iter()
            .map(|layer| {
                (
                    layer,
                    Reference { layer_id: layer,
                        poc: 357,
                        long_term: layer == 2,
                        picture: Some(source.clone()),
                    },
                )
            })
            .collect();
        assert!(
            plan.translate_temporal_references(&references, 1)
                .err()
                .unwrap()
                .to_string()
                .contains("exceeds budget")
        );
        let translated = plan
            .translate_temporal_references(&references, 32 << 20)
            .unwrap();
        for ((layer, before), (after_layer, after)) in references.iter().zip(&translated) {
            assert_eq!(layer, after_layer);
            assert_eq!(after.poc, 101);
            assert_eq!(after.long_term, before.long_term);
            let picture = after.picture.as_ref().unwrap();
            assert!(!Arc::ptr_eq(picture, &source));
            for c in 0..3 {
                assert_eq!(picture.planes[c].samples(), source.planes[c].samples());
                assert_eq!(
                    picture.planes[c].samples().as_ptr(),
                    source.planes[c].samples().as_ptr()
                );
            }
            for list in 0..2 {
                assert_eq!(
                    picture.reference_pocs[list],
                    source.reference_pocs[list]
                        .iter()
                        .map(|p| p - 256)
                        .collect::<Vec<_>>()
                );
            }
            assert_eq!(before.poc, 357);
        }
        assert!(source.reference_pocs.iter().flatten().any(|&p| p == 356));
        let mut overflow = references.clone();
        overflow[1].1.poc = i32::MIN;
        assert!(
            plan.translate_temporal_references(&overflow, 32 << 20)
                .err()
                .unwrap()
                .to_string()
                .contains("translated POC overflow")
        );
        assert_eq!(references[0].1.poc, 357);
        // An unmoved owner still needs a metadata copy when its referenced
        // layer moves. Pixels remain shared and the owner POC stays unchanged.
        plan.decrements = [0; 63];
        plan.decrements[0] = 256;
        let selective = plan.translate_references(&references,32<<20).unwrap();
        let unmoved = &selective[1].1;
        assert_eq!(unmoved.layer_id,2);
        assert_eq!(unmoved.poc,357);
        assert_eq!(unmoved.picture.as_ref().unwrap().reference_pocs[0],
            source.reference_pocs[0].iter().map(|p| p-256).collect::<Vec<_>>());
        assert!(!Arc::ptr_eq(unmoved.picture.as_ref().unwrap(),&source));
        let mut mismatched = references.clone();
        mismatched[1].1.layer_id = 0;
        assert!(plan.translate_references(&mismatched,32<<20).is_err());
        plan.decrements = [0; 63];
        let same = plan
            .translate_temporal_references(&references, 32 << 20)
            .unwrap();
        assert!(
            same.iter()
                .all(|(_, r)| Arc::ptr_eq(r.picture.as_ref().unwrap(), &source))
        );
    }
}
