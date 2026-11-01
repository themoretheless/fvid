//! Decode in packet order, expose immutable frame bindings in presentation order.
use super::{Decoded, Decoder, Result};
use crate::{owned_matroska::HdrMetadata, owned_webm::WebmReader};
use fvid_control::CopyOptions;
use std::io::{Read, Seek};
pub(crate) struct Frame {
    pub decoded: Decoded,
    pub pts: i64,
    pub duration: Option<u64>,
    pub mono: bool,
    pub colour: Option<(bool, u8, u8, u8)>,
    pub hdr: Option<HdrMetadata>,
    index: usize,
}
pub(crate) struct Presentation {
    index: usize,
    pub consumed: u64,
    future: Vec<i64>,
    queue: Vec<Frame>,
    pending_field: Option<usize>,
}
impl Presentation {
    pub fn new<R: Read + Seek>(
        reader: &WebmReader<R>,
        track: u64,
        options: Option<&CopyOptions>,
    ) -> Result<Self> {
        if let Some(limit) = options
            .and_then(|o| o.max_controlled_bytes)
            .filter(|&limit| limit > 0)
            && reader.packets.len().saturating_mul(std::mem::size_of::<i64>()) > limit
        {
            return Err(
                "controlled memory budget exceeded: Matroska presentation timestamp table".into(),
            );
        }
        let mut future = Vec::new();
        future.try_reserve_exact(reader.packets.len()).map_err(|e|e.to_string())?;
        let mut minimum = i64::MAX;
        for p in reader.packets.iter().rev() {
            if p.track == track && !p.invisible { minimum = minimum.min(p.pts_ns); }
            future.push(minimum);
        }
        future.reverse();
        Ok(Self { index: 0, consumed: 0, future, queue: Vec::new(), pending_field: None })
    }
    pub fn next<R: Read + Seek>(
        &mut self, reader: &mut WebmReader<R>, decoder: &mut Decoder,
        track: u64, default_duration: u64, inferred: Option<&[Option<u64>]>,
        options: Option<&CopyOptions>, pre_read_stop: Option<i128>,
    ) -> Result<Option<Frame>> {
        loop {
            if options.and_then(|o|o.cancel.as_ref()).is_some_and(fvid_control::CancelFlag::is_cancelled) {
                return Err("media operation cancelled".into());
            }
            if let Some(o) = options {
                crate::owned_budget::check_rss_budget(o)?;
            }
            if let Some((index, frame)) = self.queue.iter().enumerate().min_by_key(|(_,f)|(f.pts,f.index)) {
                if self.index == reader.packets.len() || frame.pts <= self.future[self.index] {
                    return Ok(Some(self.queue.swap_remove(index)));
                }
            }
            if self.index == reader.packets.len() {
                if self.pending_field.is_some() { return Err("unpaired AVC field at end of Matroska".into()); }
                return Ok(None);
            }
            let index = self.index;
            let p = reader.packets[index].clone();
            self.index += 1;
            if p.track != track { continue; }
            // A later stored packet can still have an earlier presentation
            // timestamp. Stop before decoding only when the remaining visible
            // timestamps also lie beyond the requested interval.
            if !p.invisible && pre_read_stop.is_some_and(|stop| {
                i128::from(p.pts_ns) >= stop
                    && self.future.get(self.index).is_none_or(|&pts| i128::from(pts) >= stop)
            }) {
                return Ok(None);
            }
            if options.and_then(|o|o.max_packets).is_some_and(|limit|self.consumed>=limit) {
                return Err("FFV1 input packet count exceeds limit".into());
            }
            self.consumed = self.consumed.checked_add(1).ok_or("FFV1 input packet count overflow")?;
            if let Some(options) = options {
                if reader
                    .packet_payload_size(index)
                    .map_err(|e| e.to_string())?
                    > options.max_packet_bytes
                {
                    return Err("WebM packet exceeds budget or has empty lace frame".into());
                }
            }
            let packet = reader.read_packet(index).map_err(|e|e.to_string())?;
            let decoded = decoder.decode(&packet)?;
            if decoder.has_pending_field() { self.pending_field.get_or_insert(index); }
            let Some(decoded) = decoded else { continue; };
            let (index, pts, duration) = if decoder.output_is_field_pair() {
                let first_index = self.pending_field.take().ok_or("missing AVC first-field timing")?;
                let first = &reader.packets[first_index];
                if first.invisible != p.invisible { return Err("AVC field pair visibility differs".into()); }
                let first_duration = first.duration_ns.filter(|&v|v!=0).or((default_duration!=0).then_some(default_duration));
                let second_duration = p.duration_ns.filter(|&v|v!=0).or((default_duration!=0).then_some(default_duration));
                let combined = first_duration.zip(second_duration).map(|(a,b)|a.checked_add(b).ok_or("AVC field-pair duration overflow")).transpose()?;
                (first_index, first.pts_ns, combined.or_else(||inferred.and_then(|v|v[first_index])))
            } else {
                (index, p.pts_ns, p.duration_ns.filter(|&v|v!=0)
                    .or((default_duration!=0).then_some(default_duration))
                    .or_else(||inferred.and_then(|v|v[index])))
            };
            if p.invisible { continue; }
            if let Some(limit) = options
                .and_then(|o| o.max_controlled_bytes)
                .filter(|&limit| limit > 0)
            {
                let frame_bytes = decoded
                    .frame
                    .data
                    .len()
                    .saturating_add(decoded.alpha.as_ref().map_or(0, Vec::len));
                let queued_bytes = self.queue.iter().fold(frame_bytes, |acc, f| {
                    acc.saturating_add(f.decoded.frame.data.len())
                        .saturating_add(f.decoded.alpha.as_ref().map_or(0, Vec::len))
                });
                if queued_bytes > limit {
                    return Err("video frame exceeds controlled memory budget".into());
                }
            }
            self.queue.try_reserve(1).map_err(|e|e.to_string())?;
            self.queue.push(Frame {
                decoded, pts, duration,
                mono: decoder.monochrome().unwrap_or(false),
                colour: decoder.colour(), hdr: decoder.hdr(), index,
            });
        }
    }
}
