//! Map each HEVC sample entry's framing/parameter seed into one native stream.
use crate::codec::{
    config::{HevcConfig, NalUnits},
    hevc_nal::{NalHeader, NalRbsp},
    hevc_sei,
};
use crate::container::mp4::{Mp4Reader, Track};
use crate::{Result, invalid};
use std::io::{Read, Seek};
fn append(out: &mut Vec<u8>, nal: &[u8], budget: usize) -> Result<()> {
    let length = u32::try_from(nal.len())
        .map_err(|_| invalid("HEVC normalized NAL exceeds length range"))?;
    let required = out
        .len()
        .checked_add(4)
        .and_then(|n| n.checked_add(nal.len()))
        .ok_or_else(|| invalid("HEVC normalized packet size overflow"))?;
    if required > budget {
        return Err(invalid(
            "HEVC normalized packet exceeds source memory budget",
        ));
    }
    out.try_reserve_exact(required - out.len())
        .map_err(|_| invalid("cannot allocate HEVC normalized packet"))?;
    if out.capacity() > budget {
        return Err(invalid(
            "HEVC normalized packet capacity exceeds source memory budget",
        ));
    }
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(nal);
    Ok(())
}
fn configuration(out: &mut Vec<u8>, config: &HevcConfig<'_>, budget: usize) -> Result<()> {
    for a in &config.arrays {
        if matches!(a.nal_type, 32..=34) {
            for n in &a.units {
                append(out, n, budget)?;
            }
        }
    }
    // Like LayeredDecoder::from_configuration, carry static HDR from hvcC.
    // Preserve payload integers exactly; do not promote suffix-only non-HDR
    // messages (e.g. picture hashes) into prefix syntax or emit them on a frame.
    for a in &config.arrays {
        if !matches!(a.nal_type, 39 | 40) {
            continue;
        }
        for n in &a.units {
            let h = NalHeader::parse(n)?;
            if h.layer_id == 63 {
                continue;
            }
            let rbsp = NalRbsp::parse(n, budget.saturating_sub(out.capacity()))?;
            let mut mastering = None;
            let mut light = None;
            hevc_sei::visit_messages(&rbsp.bytes, |m| {
                if m.payload_type == 137
                    && crate::color::hdr::HdrMetadata::from_mdcv(m.payload)
                        .and_then(|h| h.mastering)
                        .is_some()
                {
                    mastering = Some(&m.payload[..24]);
                } else if m.payload_type == 144
                    && crate::color::hdr::HdrMetadata::from_clli(m.payload).is_some()
                {
                    light = Some(&m.payload[..4]);
                }
            })?;
            let mut payload = Vec::new();
            for (kind, value) in [(137, mastering), (144, light)] {
                if let Some(value) = value {
                    payload.push(kind);
                    payload.push(value.len() as u8);
                    payload.extend_from_slice(value);
                }
            }
            drop(rbsp);
            if payload.is_empty() {
                continue;
            }
            payload.push(128);
            let mut nal = vec![(39 << 1) | (n[0] & 1), n[1]];
            let mut zeros = 0;
            for byte in payload {
                if zeros == 2 && byte <= 3 {
                    nal.push(3);
                    zeros = 0;
                }
                nal.push(byte);
                zeros = if byte == 0 { zeros + 1 } else { 0 };
            }
            append(out, &nal, budget)?;
        }
    }
    Ok(())
}
pub(super) fn normalize(
    track: &Track,
    index: usize,
    previous: u32,
    raw: &[u8],
    out: &mut Vec<u8>,
    budget: usize,
) -> Result<u32> {
    let sample = track
        .samples
        .get(index)
        .ok_or_else(|| invalid("HEVC description sample is absent"))?;
    let setup = track
        .description_for_sample(index)
        .ok_or_else(|| invalid("HEVC sample description is absent"))?;
    if !matches!(&setup.codec, b"hvc1" | b"hev1") {
        return Err(invalid(
            "layered MP4 source contains a non-HEVC description",
        ));
    }
    let config = HevcConfig::parse(&setup.configuration)?;
    let inject = track
        .sample_descriptions
        .get(previous as usize - 1)
        .is_none_or(|old| old.configuration != setup.configuration);
    out.clear();
    let mut first = true;
    // AUD remains first, including when a new hvcC parameter bank arrives here.
    for n in NalUnits::new(raw, config.length_size)? {
        let n = n?;
        if first {
            if NalHeader::parse(n)?.layer_id == 63 {
                append(out, n, budget)?;
                continue;
            }
            first = false;
            if NalHeader::parse(n)?.unit_type == 35 {
                append(out, n, budget)?;
                if inject {
                    configuration(out, &config, budget)?;
                }
                continue;
            }
            if inject {
                configuration(out, &config, budget)?;
            }
        }
        append(out, n, budget)?;
    }
    if first && inject {
        configuration(out, &config, budget)?;
    }
    Ok(sample.description_index)
}
pub(super) fn all_hevc(track: &Track) -> bool {
    !track.samples.is_empty()
        && (0..track.samples.len()).all(|i| {
            track
                .description_for_sample(i)
                .is_some_and(|s| matches!(&s.codec, b"hvc1" | b"hev1"))
        })
}

pub(super) fn read_normalized<R: Read + Seek>(
    demux: &mut Mp4Reader<R>,
    track: usize,
    index: usize,
    previous: u32,
    raw: &mut Vec<u8>,
    out: &mut Vec<u8>,
    budget: usize,
) -> Result<u32> {
    let size = demux.tracks()[track]
        .samples
        .get(index)
        .ok_or_else(|| invalid("missing HEVC description sample"))?
        .size as usize;
    if size > budget {
        return Err(invalid("HEVC packet exceeds source memory budget"));
    }
    if raw.capacity() < size {
        *raw = Vec::new();
        raw.try_reserve_exact(size)
            .map_err(|_| invalid("cannot allocate HEVC source packet"))?;
    }
    if raw.capacity() > budget {
        return Err(invalid("HEVC packet capacity exceeds source memory budget"));
    }
    demux.read_packet(track, index, raw)?;
    let selected = demux.tracks()[track]
        .samples
        .get(index)
        .unwrap()
        .description_index;
    let descriptions = &demux.tracks()[track].sample_descriptions;
    let setup = &descriptions[selected as usize - 1];
    if setup.configuration == descriptions[previous as usize - 1].configuration
        && setup
            .configuration
            .get(21)
            .is_some_and(|byte| byte & 3 == 3)
    {
        // Already canonical: retain the original packet buffer, avoiding a
        // second copy of every coded frame in the steady-state four-byte path.
        std::mem::swap(raw, out);
        return Ok(selected);
    }
    normalize(&demux.tracks()[track], index, previous, raw, out, budget)
}
