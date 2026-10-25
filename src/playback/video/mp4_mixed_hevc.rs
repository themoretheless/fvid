//! One HEVC window in a mixed-codec track, sharing its original indexed demuxer.
use super::layered::{LayeredPipeline, LayeredVideoFrame};
use super::*;
pub(super) struct MixedHevc {
    pipeline: LayeredPipeline,
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
        let mut pipeline = LayeredPipeline::open(demux, track, start..end, 0, 6, budget, true)?;
        if index > start {
            let pts = demux.tracks()[track].samples.get(index).unwrap().pts;
            pipeline.seek_to_sync(demux, pts);
        }
        let mut metadata = DecoderMetadata::default();
        let config = crate::codec::config::HevcConfig::parse(
            &demux.tracks()[track]
                .description_for_sample(start)
                .unwrap()
                .configuration,
        )?;
        let layer = pipeline.opening_layer();
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
            lookahead: None,
            metadata,
        })
    }
    pub(super) fn reset(&mut self) {
        self.pipeline.rewind();
        self.lookahead = None;
        self.metadata = DecoderMetadata::default();
    }
    pub(super) fn contains(&self, index: usize) -> bool {
        index >= self.pipeline.window_start && index < self.pipeline.window_end
    }
    pub(super) fn next_sample(&self) -> usize {
        self.pipeline.next_sample()
    }
    pub(super) fn pending_pts<R: Read + Seek>(&self, demux: &Mp4Reader<R>) -> Option<i64> {
        self.pipeline
            .pending_pts(demux)
            .into_iter()
            .chain(self.lookahead.as_ref().map(|f| f.presentation_time.ticks))
            .min()
    }
    pub(super) fn read<R: Read + Seek>(
        &mut self,
        demux: &mut Mp4Reader<R>,
    ) -> Result<Option<(VideoFrame, DecoderMetadata, u8)>> {
        let first = if self.lookahead.is_some() {
            self.lookahead.take()
        } else {
            self.pipeline.read_frame(demux)?
        };
        let Some(mut chosen) = first else {
            return Ok(None);
        };
        while let Some(next) = self.pipeline.read_frame(demux)? {
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
        if held > self.pipeline.display_available()? {
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
