//! Annex F.8.1.3 layer initialization and associated-IRAP RASL suppression.
//! Preview remains private until the complete AU reconstructs successfully.
use super::{LayeredAccessUnit, LayeredEndMarkers};
use crate::{Result, invalid};
#[derive(Clone, Copy, Default)]
pub(super) struct RandomAccess {
    seen: u64,
    initialized: u64,
    no_rasl: u64,
    pending_eos: u64,
}
pub(super) struct Preview {
    pub next: RandomAccess,
    pub suppressed: u64,
    pub ignored: u64,
    pub starts_clvs: u64,
    pub release: [u64; 63],
    pub output_prior: [(u64, bool); 63],
    pub complete_target_idr: bool,
}
impl RandomAccess {
    pub fn preview(self, plan: &LayeredAccessUnit) -> Result<Preview> {
        let mut next = self;
        let mut suppressed = 0u64;
        let mut ignored = 0u64;
        let mut starts_clvs = 0u64;
        let mut release = [0u64; 63];
        let mut output_prior = [(0u64, false); 63];
        let mut complete_target_idr = false;
        for p in plan.pictures() {
            let h = &p.slices[0];
            let id = h.nal.layer_id;
            let bit = 1u64 << id;
            let (smallest, direct, predicted) = if let Some(e) = &p.binding.vps.extension {
                let ols = e
                    .output
                    .sets
                    .get(p.binding.sps.output_layer_set)
                    .ok_or_else(|| invalid("HEVC random access OLS is absent"))?;
                if ols.necessary == 0 {
                    return Err(invalid("HEVC random access OLS has no necessary layer"));
                }
                let smallest = ols.necessary.trailing_zeros() as u8;
                let index = e
                    .graph
                    .layer_ids
                    .binary_search(&id)
                    .map_err(|_| invalid("HEVC random access layer is absent"))?;
                let direct = *e
                    .graph
                    .direct
                    .get(index)
                    .ok_or_else(|| invalid("HEVC random access graph is incomplete"))?;
                let mut predicted = 0u64;
                for (&layer, &references) in e.graph.layer_ids.iter().zip(&e.graph.transitive) {
                    if references & bit != 0 {
                        predicted |= 1u64 << layer;
                    }
                }
                (smallest, direct, predicted)
            } else if p.binding.vps.inferred_base_layer()
                && id == 0
                && p.binding.sps.output_layer_set == 0
            {
                (0, 0, 0)
            } else {
                return Err(invalid("HEVC random access has no VPS extension"));
            };
            let syntax = h
                .layered
                .as_ref()
                .ok_or_else(|| invalid("HEVC random access has no layered slice syntax"))?;
            if h.nal.is_irap() {
                // HandleCraAsBlaFlag defaults to zero. External override is not
                // supplied by this entry; BLA and coded cross-layer BLA are owned.
                let bla = matches!(h.nal.unit_type, 16..=18);
                let no_clras = id == smallest
                    && (next.seen == 0
                        || next.pending_eos & ((1u64 << smallest) | 1) != 0
                        || bla
                        || h.nal.is_idr() && syntax.cross_layer_bla);
                if no_clras {
                    next.initialized = 0;
                    release[id as usize] = (1u64 << 63) - 1;
                }
                let layer_reset = id > smallest
                    && (next.pending_eos & bit != 0
                        || bla
                        || h.nal.is_idr() && syntax.cross_layer_bla);
                if layer_reset {
                    next.initialized &= !predicted;
                    release[id as usize] |= bit | predicted;
                }
                let ready = direct & !next.initialized == 0;
                let no_rasl = h.nal.is_idr()
                    || bla
                    || next.seen & bit == 0
                    || next.pending_eos & bit != 0
                    || next.initialized & bit == 0 && ready;
                if no_rasl {
                    starts_clvs |= bit;
                    next.no_rasl |= bit;
                    if id == 0 || next.initialized & bit == 0 && ready {
                        next.initialized |= bit;
                    }
                } else {
                    next.no_rasl &= !bit;
                }
                // F.13.5.2.2: CRA always discards prior pictures when
                // NoRaslOutput is set; IDR/BLA use the coded prior-output flag.
                if no_rasl && (id == smallest || id > smallest && direct == 0) {
                    let scope = if no_clras {
                        (1u64 << 63) - 1
                    } else if layer_reset {
                        bit | predicted
                    } else {
                        bit
                    };
                    output_prior[id as usize] = (
                        scope,
                        h.nal.unit_type == 21 || h.no_output_of_prior_pictures,
                    );
                }
                next.pending_eos &= !bit;
            }
            // F.8.1.7 CL-RAS and 8.1.3 leading RASL may be ignored: neither can
            // affect an output picture. Keep raw flags for conformance checks.
            if next.initialized & bit == 0
                || matches!(h.nal.unit_type, 8 | 9) && next.no_rasl & bit != 0
            {
                suppressed |= bit;
                // Base-layer reconstruction still validates missing temporal
                // references; CL-RAS omission is a non-base-layer process.
                if id != 0 || matches!(h.nal.unit_type, 8 | 9) && next.no_rasl & bit != 0 {
                    ignored |= bit;
                }
            }
            next.seen |= bit;
        }
        // Every target layer starts with IDR, so no reference from the prior
        // CVSG survives. Apply this before translating into a new VPS graph.
        if let Some(first) = plan.pictures().first() {
            let idr_layers = plan
                .pictures()
                .iter()
                .fold(0, |mask, p| mask | (1u64 << p.slices[0].nal.layer_id));
            if plan.pictures().iter().all(|p| p.slices[0].nal.is_idr())
                && first
                    .binding
                    .vps
                    .covers_idr_target(first.binding.sps.output_layer_set, idr_layers)
            {
                release[first.slices[0].nal.layer_id as usize] = (1u64 << 63) - 1;
                complete_target_idr = true;
            }
        }
        Ok(Preview {
            next,
            suppressed,
            ignored,
            starts_clvs,
            release,
            output_prior,
            complete_target_idr,
        })
    }
    pub fn end(&mut self, end: LayeredEndMarkers, affected: u64) {
        if end.bitstream {
            *self = Self::default();
        } else {
            self.initialized &= !affected;
            self.no_rasl &= !affected;
            self.pending_eos |= end.layers;
        }
    }
}
