//! Annex F.7.3.2.1.6 bitstream partition HRD metadata.
use super::super::{
    bits::BitReader,
    hevc_hrd::{Cpb, Hrd},
    hevc_vps::LayerSets,
};
use super::{LayeredExtensionPrefix, VpsVuiPrefix, bounded};
use crate::{Result, invalid};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BspScheduleEntry {
    pub hrd_index: u16,
    pub cpb_index: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BspScheme {
    pub partitions: Vec<u64>,
    /// Temporal sublayer -> delivery schedule -> partition entries.
    pub schedules: Vec<Vec<Vec<BspScheduleEntry>>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BspHrdParameters {
    pub additional_hrd: Vec<Hrd>,
    /// OLS zero has no coded schedule syntax here.
    pub sets: Vec<Option<Vec<BspScheme>>>,
    pub base_parameter_compatibility: Vec<Option<bool>>,
    pub end_bit: usize,
}
fn debit(left: &mut usize, bytes: usize) -> Result<()> {
    *left = left
        .checked_sub(bytes)
        .ok_or_else(|| invalid("HEVC BSP HRD metadata exceeds memory budget"))?;
    Ok(())
}
fn alloc<T>(count: usize, left: &mut usize) -> Result<Vec<T>> {
    debit(
        left,
        count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| invalid("HEVC BSP allocation size overflow"))?,
    )?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| invalid("cannot allocate HEVC BSP metadata"))?;
    Ok(result)
}
fn ordered(previous: &Cpb, current: &Cpb) -> bool {
    current.bit_rate_value > previous.bit_rate_value
        && current.size_value <= previous.size_value
        && match (previous.decoding_unit, current.decoding_unit) {
            (Some((ps, pb)), Some((cs, cb))) => cb > pb && cs <= ps,
            _ => true,
        }
}
impl VpsVuiPrefix {
    /// Existing VPS HRDs are borrowed in coded order. This budget covers newly
    /// retained BSP metadata, not the caller's graph/VUI or frame buffers.
    pub fn bsp_hrd(
        &self,
        bytes: &[u8],
        prefix: &LayeredExtensionPrefix,
        layers: &LayerSets,
        existing: &[Hrd],
        max_sublayer: u8,
        timing_present: bool,
        budget: usize,
    ) -> Result<Option<BspHrdParameters>> {
        if !self.bsp_hrd_present {
            return Ok(None);
        }
        if !timing_present {
            return Err(invalid("HEVC BSP HRD requires VPS timing"));
        }
        let n = prefix.graph.layer_ids.len();
        if n == 0
            || n > 63
            || prefix.graph.direct.len() != n
            || prefix.graph.transitive.len() != n
            || prefix.graph.layer_ids.iter().any(|&id| id > 63)
            || existing.len() > 1024
            || max_sublayer > 6
            || prefix.output.sets.is_empty()
            || prefix.output.sets.len() > 3070
            || prefix.layer_sets.layer_sets.len() != prefix.layer_sets.set_sublayers_minus1.len()
        {
            return Err(invalid("invalid HEVC BSP metadata geometry"));
        }
        let mut left = budget;
        debit(&mut left, std::mem::size_of::<BspHrdParameters>())?;
        let mut b = BitReader::new(bytes);
        b.skip(self.end_bit)?;
        let count = bounded(&mut b, (1024 - existing.len()) as u32)? as usize;
        let mut added = alloc::<Hrd>(count, &mut left)?;
        for i in 0..count {
            let index = existing.len() + i;
            let present = index == 0 || b.bit()?;
            let temporal = bounded(&mut b, u32::from(max_sublayer))? as u8;
            let previous = added.last().or_else(|| existing.last());
            let parameters = Hrd::read(
                &mut b,
                temporal,
                if present {
                    None
                } else {
                    previous.map(|h| &h.common)
                },
            )?;
            let dynamic = parameters.layers.capacity()
                * std::mem::size_of::<super::super::hevc_hrd::SubLayer>()
                + parameters
                    .layers
                    .iter()
                    .map(|s| (s.nal.capacity() + s.vcl.capacity()) * std::mem::size_of::<Cpb>())
                    .sum::<usize>();
            debit(&mut left, dynamic)?;
            added.push(parameters);
        }
        let total = existing.len() + added.len();
        let get = |index: usize| -> &Hrd {
            if index < existing.len() {
                &existing[index]
            } else {
                &added[index - existing.len()]
            }
        };
        let mut sets = alloc::<Option<Vec<BspScheme>>>(prefix.output.sets.len(), &mut left)?;
        sets.push(None);
        for output in prefix.output.sets.iter().skip(1) {
            if total == 0 {
                sets.push(None);
                continue;
            }
            let list = prefix
                .layer_sets
                .layer_sets
                .get(output.layer_set_index)
                .ok_or_else(|| invalid("HEVC BSP layer set index out of range"))?;
            let temporal = prefix.layer_sets.set_sublayers_minus1[output.layer_set_index];
            if list.is_empty()
                || list.len() > 63
                || temporal > max_sublayer
                || list
                    .iter()
                    .any(|id| prefix.graph.layer_ids.binary_search(id).is_err())
            {
                return Err(invalid("invalid HEVC BSP layer list"));
            }
            let signalled = bounded(&mut b, 16)? as usize;
            let mut schemes = alloc::<BspScheme>(signalled + 1, &mut left)?;
            let mut inferred = alloc::<u64>(list.len(), &mut left)?;
            inferred.extend(list.iter().map(|&id| 1u64 << id));
            schemes.push(BspScheme {
                partitions: inferred,
                schedules: Vec::new(),
            });
            for _ in 0..signalled {
                let count = bounded(&mut b, list.len() as u32 - 1)? as usize + 1;
                let mut partitions = alloc::<u64>(count, &mut left)?;
                let mut assigned = 0u64;
                for _ in 0..count {
                    let mut mask = 0u64;
                    for &id in list {
                        if b.bit()? {
                            mask |= 1u64 << id;
                        }
                    }
                    if mask & assigned != 0 {
                        return Err(invalid("HEVC BSP layer occurs in multiple partitions"));
                    }
                    if !layers.base_internal && mask & 1 != 0 && mask.count_ones() == 1 {
                        return Err(invalid(
                            "external HEVC base cannot be its own signalled BSP partition",
                        ));
                    }
                    let mut earlier_references = 0u64;
                    for &earlier in &partitions {
                        for &id in list {
                            if earlier & (1u64 << id) != 0 {
                                earlier_references |= prefix.graph.transitive
                                    [prefix.graph.layer_ids.binary_search(&id).unwrap()];
                            }
                        }
                    }
                    if mask & earlier_references != 0 {
                        return Err(invalid(
                            "HEVC BSP reference partition follows its dependent",
                        ));
                    }
                    assigned |= mask;
                    partitions.push(mask);
                }
                if assigned != list.iter().fold(0u64, |m, &id| m | (1u64 << id)) {
                    return Err(invalid("HEVC BSP partitioning omits a layer"));
                }
                schemes.push(BspScheme {
                    partitions,
                    schedules: Vec::new(),
                });
            }
            for scheme in &mut schemes {
                scheme.schedules =
                    alloc::<Vec<Vec<BspScheduleEntry>>>(usize::from(temporal) + 1, &mut left)?;
                for tid in 0..=temporal {
                    let count = bounded(&mut b, 31)? as usize + 1;
                    let mut schedules = alloc::<Vec<BspScheduleEntry>>(count, &mut left)?;
                    for j in 0..count {
                        let mut schedule =
                            alloc::<BspScheduleEntry>(scheme.partitions.len(), &mut left)?;
                        for k in 0..scheme.partitions.len() {
                            let width = (usize::BITS - (total - 1).leading_zeros()) as u8;
                            let index = if total > 1 {
                                b.read(width)? as usize
                            } else {
                                0
                            };
                            if index >= total {
                                return Err(invalid("HEVC BSP HRD index out of range"));
                            }
                            let h = get(index);
                            let sub = h.layers.get(usize::from(tid)).ok_or_else(|| {
                                invalid("HEVC BSP HRD lacks scheduled temporal sublayer")
                            })?;
                            let cpb = b.unsigned_golomb()?;
                            if cpb >= u32::from(sub.cpb_count) {
                                return Err(invalid("HEVC BSP CPB schedule index out of range"));
                            }
                            let entry = BspScheduleEntry {
                                hrd_index: index as u16,
                                cpb_index: cpb as u8,
                            };
                            if j > 0 {
                                let p = &schedules[j - 1][k];
                                let previous =
                                    &get(usize::from(p.hrd_index)).layers[usize::from(tid)];
                                for (old, new) in
                                    [(&previous.nal, &sub.nal), (&previous.vcl, &sub.vcl)]
                                {
                                    if let (Some(old), Some(new)) =
                                        (old.get(usize::from(p.cpb_index)), new.get(cpb as usize))
                                    {
                                        if !ordered(old, new) {
                                            return Err(invalid(
                                                "HEVC BSP schedules violate bitrate or buffer ordering",
                                            ));
                                        }
                                    }
                                }
                            }
                            schedule.push(entry);
                        }
                        schedules.push(schedule);
                    }
                    scheme.schedules.push(schedules);
                }
            }
            sets.push(Some(schemes));
        }
        let mut compatibility = alloc::<Option<bool>>(n, &mut left)?;
        compatibility.resize(n, None);
        for (i, flag) in compatibility.iter_mut().enumerate().skip(1) {
            if prefix.graph.direct[i] == 0 {
                *flag = Some(b.bit()?);
            }
        }
        Ok(Some(BspHrdParameters {
            additional_hrd: added,
            sets,
            base_parameter_compatibility: compatibility,
            end_bit: b.position(),
        }))
    }
}
