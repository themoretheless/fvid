//! One HEVC window in a mixed-codec track, sharing its original indexed demuxer.
use super::layered::{LayeredPipeline, LayeredVideoFrame};
use super::*;
pub(super) struct MixedHevc {
    pipeline: Option<LayeredPipeline>,
    prefix: Option<Box<HevcDecoder>>,
    window_start: usize,
    window_end: usize,
    prefix_end: usize,
    prefix_index: usize,
    prefix_description: u32,
    initial_description: u32,
    track: usize,
    packet: Vec<u8>,
    normalized: Vec<u8>,
    prefix_budget: usize,
    packet_budget: usize,
    lookahead: Option<LayeredVideoFrame>,
    pub(super) metadata: DecoderMetadata,
}
impl MixedHevc {
    pub(super) fn open<R: Read + Seek>(
        demux: &mut Mp4Reader<R>,
        track: usize,
        index: usize,
        budget: usize,
    ) -> Result<Self> {
        let samples = demux.tracks()[track].samples.len();
        let hevc = |i| {
            demux.tracks()[track]
                .description_for_sample(i)
                .is_some_and(|s| matches!(&s.codec, b"hvc1" | b"hev1"))
        };
        let start = (0..index).rev().find(|&i| !hevc(i)).map_or(0, |i| i + 1);
        let end = (index..samples).find(|&i| !hevc(i)).unwrap_or(samples);
        let budget = budget
            .checked_sub(std::mem::size_of::<Self>())
            .ok_or_else(|| invalid("mixed HEVC state exceeds memory budget"))?;
        let setup = demux.tracks()[track].description_for_sample(start).unwrap();
        let initial_description = demux.tracks()[track]
            .samples
            .get(start)
            .unwrap()
            .description_index;
        if setup.configuration.len() > budget / 16 {
            return Err(invalid("HEVC window configuration copy exceeds budget"));
        }
        let mut config_bytes = setup.configuration.clone();
        let config = crate::codec::config::HevcConfig::parse(&config_bytes)?;
        let mut prefix_end = start;
        let mut received_configuration = None;
        if !config
            .arrays
            .iter()
            .any(|a| a.nal_type == 32 && !a.units.is_empty())
        {
            let mut bank = crate::codec::hevc_decoder::LayeredParameters::from_configuration(
                &config_bytes,
                budget / 8,
            )?;
            let mut first_vps = None;
            let mut prefix_coded = false;
            let mut prefix_ended = false;
            let mut previous = initial_description;
            let mut raw = Vec::new();
            let mut packet = Vec::new();
            prefix_end = end;
            'inventory: for i in start..end {
                previous = layered_packet::read_normalized(
                    demux,
                    track,
                    i,
                    previous,
                    &mut raw,
                    &mut packet,
                    budget / 16,
                )?;
                for n in crate::codec::config::NalUnits::new(&packet, 4)? {
                    let n = n?;
                    let h = crate::codec::hevc_nal::NalHeader::parse(n)?;
                    if h.unit_type == 37 || h.unit_type == 36 && h.layer_id == 0 {
                        prefix_ended = true;
                    }
                    if matches!(h.unit_type, 32..=34) {
                        bank.receive(n)?;
                    }
                    if h.unit_type == 32 {
                        first_vps.get_or_insert(i);
                    }
                    if h.is_vcl() {
                        if bank.referenced_vps_if_available(n)?.is_some() {
                            if prefix_coded && !prefix_ended && !matches!(h.unit_type, 16..=20) {
                                return Err(invalid("HEVC active VPS changed before sequence end"));
                            }
                            prefix_end = first_vps.unwrap_or(start);
                            received_configuration =
                                Some(bank.received_configuration(&config_bytes, budget / 16)?);
                            break 'inventory;
                        }
                        h.require_base_layer()?;
                        prefix_coded = true;
                        prefix_ended = false;
                    }
                }
            }
        }
        config_bytes[21] |= 3;
        let has_prefix = prefix_end > start;
        let prefix_budget = if has_prefix { budget / 3 } else { 0 };
        let mut prefix = if has_prefix {
            Some(Box::new(HevcDecoder::from_inband_configuration(
                &config_bytes,
                prefix_budget,
            )?))
        } else {
            None
        };
        let mut pipeline = if prefix_end < end {
            Some(LayeredPipeline::open(
                demux,
                track,
                prefix_end..end,
                0,
                6,
                budget - prefix_budget - if has_prefix { budget / 8 } else { 0 },
                true,
                received_configuration.as_deref(),
            )?)
        } else {
            None
        };
        let mut prefix_index = start;
        if index > start {
            if index < prefix_end {
                let mut previous = initial_description;
                let mut raw = Vec::new();
                let mut packet = Vec::new();
                for i in start..index {
                    previous = layered_packet::read_normalized(
                        demux,
                        track,
                        i,
                        previous,
                        &mut raw,
                        &mut packet,
                        budget / 16,
                    )?;
                    prefix.as_mut().unwrap().prime_parameters(&packet)?;
                }
                prefix_index = index;
            } else {
                prefix_index = prefix_end;
                if let Some(pipeline) = &mut pipeline {
                    let pts = demux.tracks()[track].samples.get(index).unwrap().pts;
                    pipeline.seek_to_sync(demux, pts);
                }
            }
        }
        let mut metadata = DecoderMetadata::default();
        let config = crate::codec::config::HevcConfig::parse(
            &demux.tracks()[track]
                .description_for_sample(start)
                .unwrap()
                .configuration,
        )?;
        let layer = if has_prefix {
            0
        } else {
            pipeline.as_ref().unwrap().opening_layer()
        };
        for array in &config.arrays {
            for nal in &array.units {
                let header = crate::codec::hevc_nal::NalHeader::parse(nal)?;
                if array.nal_type == 33 && header.layer_id == 0 {
                    let s = crate::codec::hevc_sps::Sps::parse(nal, budget)?;
                    metadata.colour = Some(crate::playback_native::AvcColour::from_hevc_vui(
                        s.vui.as_ref(),
                    )?);
                    metadata.signal = s
                        .vui
                        .as_ref()
                        .and_then(|v| v.signal.as_ref())
                        .map(|v| {
                            let [primaries, transfer, matrix] = v.colour.unwrap_or([0; 3]);
                            ColourDescription {
                                primaries,
                                transfer,
                                matrix,
                                full_range: v.full_range,
                            }
                        })
                        .unwrap_or_default();
                }
                if matches!(array.nal_type, 39 | 40) && header.layer_id == layer {
                    let rbsp = crate::codec::hevc_nal::NalRbsp::parse(nal, budget)?;
                    crate::codec::hevc_sei::update_hdr_from_rbsp(&mut metadata.hdr, &rbsp.bytes)?;
                }
            }
        }
        Ok(Self {
            pipeline,
            prefix,
            window_start: start,
            window_end: end,
            prefix_end,
            prefix_index,
            prefix_description: initial_description,
            initial_description,
            track,
            packet: Vec::new(),
            normalized: Vec::new(),
            prefix_budget,
            packet_budget: budget / 16,
            lookahead: None,
            metadata,
        })
    }
    pub(super) fn reset(&mut self) {
        if let Some(p) = &mut self.pipeline {
            p.rewind();
        }
        if let Some(p) = &mut self.prefix {
            p.reset();
        }
        self.prefix_index = self.window_start;
        self.prefix_description = self.initial_description;
        self.lookahead = None;
        self.metadata = DecoderMetadata::default();
    }
    pub(super) fn contains(&self, index: usize) -> bool {
        index >= self.window_start && index < self.window_end
    }
    pub(super) fn next_sample(&self) -> usize {
        if self.prefix_index < self.prefix_end {
            self.prefix_index
        } else {
            self.pipeline
                .as_ref()
                .map_or(self.prefix_end, |p| p.next_sample())
        }
    }
    pub(super) fn pending_pts<R: Read + Seek>(&self, demux: &Mp4Reader<R>) -> Option<i64> {
        self.pipeline
            .as_ref()
            .and_then(|p| p.pending_pts(demux))
            .into_iter()
            .chain(self.lookahead.as_ref().map(|f| f.presentation_time.ticks))
            .min()
    }
    pub(super) fn read<R: Read + Seek>(
        &mut self,
        demux: &mut Mp4Reader<R>,
    ) -> Result<Option<(VideoFrame, DecoderMetadata, u8)>> {
        while self.prefix_index < self.prefix_end {
            let index = self.prefix_index;
            self.prefix_description = layered_packet::read_normalized(
                demux,
                self.track,
                index,
                self.prefix_description,
                &mut self.packet,
                &mut self.normalized,
                self.packet_budget,
            )?;
            let decoder = self.prefix.as_mut().unwrap();
            let decoded = decoder.decode_packet(&self.normalized)?;
            self.prefix_index += 1;
            let Some(decoded) = decoded.filter(|f| f.output) else {
                continue;
            };
            let s = &decoder.parameters().0;
            let packed = Arc::new(crate::playback_native::hevc_picture_with_vui(
                &decoded.picture,
                s.chroma_format,
                s.vui.as_ref(),
            )?);
            if packed
                .frame
                .data
                .len()
                .saturating_add(self.packet.capacity())
                .saturating_add(self.normalized.capacity())
                > self.prefix_budget
            {
                return Err(invalid("VPS-less HEVC prefix display exceeds budget"));
            }
            self.metadata = DecoderMetadata {
                colour: Some(crate::playback_native::AvcColour::from_hevc_vui(
                    s.vui.as_ref(),
                )?),
                signal: dispatch::signal(s.vui.as_ref()),
                hdr: decoder.hdr(),
            };
            let sample = demux.tracks()[self.track].samples.get(index).unwrap();
            let p = &decoded.picture;
            let frame = VideoFrame {
                packed: Some(packed),
                picture: Arc::new(IntraPicture {
                    coded_width: p.dimensions[0] as usize,
                    coded_height: p.dimensions[1] as usize,
                    crop: p.crop.map(|v| v as usize),
                    bit_depth: p.depth[0],
                    chroma_bit_depth: p.depth[1],
                    y: vec![],
                    cb: vec![],
                    cr: vec![],
                }),
                planes8: None,
                #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
                surface: None,
                presentation_time: MediaTime {
                    ticks: sample.pts,
                    timescale: demux.tracks()[self.track].timescale,
                },
                duration: MediaTime {
                    ticks: i64::from(sample.duration),
                    timescale: demux.tracks()[self.track].timescale,
                },
                sample_index: index,
            };
            return Ok(Some((frame, self.metadata, 0)));
        }
        let Some(pipeline) = &mut self.pipeline else {
            return Ok(None);
        };
        let first = if self.lookahead.is_some() {
            self.lookahead.take()
        } else {
            pipeline.read_frame(demux)?
        };
        let Some(mut chosen) = first else {
            return Ok(None);
        };
        while let Some(next) = pipeline.read_frame(demux)? {
            if next.sample_index != chosen.sample_index {
                self.lookahead = Some(next);
                break;
            }
            if next.decoded.layer_id > chosen.decoded.layer_id {
                chosen = next;
            }
        }
        let held = chosen
            .decoded
            .retained_bytes()?
            .saturating_mul(2)
            .saturating_add(
                self.lookahead
                    .as_ref()
                    .map(|p| p.decoded.retained_bytes())
                    .transpose()?
                    .unwrap_or(0),
            );
        if held > pipeline.display_available()? {
            return Err(invalid(
                "mixed HEVC display selection exceeds memory budget",
            ));
        }
        let s = &chosen.decoded.binding.sps.sps;
        let colour = crate::playback_native::AvcColour::from_hevc_vui(s.vui.as_ref())?;
        let signal = s
            .vui
            .as_ref()
            .and_then(|v| v.signal.as_ref())
            .map(|s| {
                let [primaries, transfer, matrix] = s.colour.unwrap_or([0; 3]);
                ColourDescription {
                    primaries,
                    transfer,
                    matrix,
                    full_range: s.full_range,
                }
            })
            .unwrap_or_default();
        self.metadata = DecoderMetadata {
            colour: Some(colour),
            signal,
            hdr: chosen.decoded.hdr,
        };
        let layer = chosen.decoded.layer_id;
        Ok(Some((chosen.into_video_frame()?, self.metadata, layer)))
    }
}
