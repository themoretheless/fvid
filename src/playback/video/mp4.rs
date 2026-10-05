//! Indexed MP4 frame source for AVC, HEVC, VP9 and AV1.
//! Frame timestamps remain in the track media timeline; `track().edits` describes
//! presentation edits separately. This source does not silently discard edits.
use crate::codec::{
    av1_decoder as av1, av1_metadata, avc_decoder::AvcDecoder, avc_picture::IntraPicture,
    hevc_decoder::HevcDecoder, vp9_decoder as vp9,
};
use crate::color::hdr::{ColourDescription, HdrMetadata};
use crate::container::mp4::{Limits, Mp4Reader, Track};
use crate::{Result, invalid};
use std::io::{Read, Seek};
use std::sync::Arc;

/// Exact track-clock timestamp, including negative composition timestamps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaTime {
    pub ticks: i64,
    pub timescale: u32,
}
impl MediaTime {
    /// Truncates toward zero to the nearest nanosecond without floating point.
    pub fn nanoseconds(self) -> Result<i128> {
        if self.timescale == 0 {
            return Err(invalid("zero media timescale"));
        }
        Ok(i128::from(self.ticks) * 1_000_000_000 / i128::from(self.timescale))
    }
}

pub struct VideoFrame {
    /// Decoded samples. For hardware-decoded frames the planes are empty and
    /// `planes8` carries the picture; the geometry fields are still valid.
    pub picture: Arc<IntraPicture>,
    /// Hardware decoder output as packed 8-bit planes, when in use.
    pub planes8: Option<Arc<crate::playback_native::Planar8>>,
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    pub surface: Option<fvid_vt::Surface>,
    pub presentation_time: MediaTime,
    pub duration: MediaTime,
    pub sample_index: usize,
}

/// Indexed MP4/MOV source for the currently supported AVC I/P/B tools,
/// progressive or fragmented (moof/traf/trun). Demux metadata/packet limits are separate.
/// Demux metadata/packet limits are separate. The supplied decoder budget is
/// split equally between decode working storage and display reordering.
/// Decoded frames retained by the caller are outside these budgets.
/// Audio tracks are exposed by the demuxer but are not decoded by this source.
/// Hardware decoding through VideoToolbox, with the stream's SPS kept for
/// colour information the hardware path does not report.
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
struct Hardware {
    session: fvid_vt::Session,
    colour: crate::playback_native::AvcColour,
    shared: bool,
}

enum Decoder {
    Avc(Box<AvcDecoder>),
    Hevc(Box<HevcDecoder>),
    Vp9(Box<vp9::Decoder>),
    Av1(Box<av1::Decoder>),
}
impl Decoder {
    fn reset(&mut self) {
        match self {
            Self::Avc(d) => d.reset(),
            Self::Hevc(d) => d.reset(),
            Self::Vp9(d) => d.reset(),
            Self::Av1(d) => d.reset(),
        }
    }
}
/// Indexed MP4 source with AVC and HEVC codec dispatch.
pub struct Mp4VideoReader<R> {
    demuxer: Mp4Reader<R>,
    decoder: Decoder,
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    hardware: Option<Hardware>,
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    decoded_surface: Option<fvid_vt::Surface>,
    track_index: usize,
    sample_index: usize,
    packet: Vec<u8>,
    /// What an AV1 track states about its own pictures in bytes this reader can
    /// see the moment the file is open: the signal of its sequence header and
    /// the light of its metadata OBUs, read from the `av1C` configuration and
    /// the first sample. A caller that grades the first picture asks then, not
    /// after a frame has been decoded, and an encoder that writes its light
    /// only in-band writes it there. `None` for every other codec, which the
    /// coding answers from its parameter sets instead.
    open_signal: Option<(ColourDescription, HdrMetadata)>,
    failed: bool,
    future_pts: Vec<i64>,
    pending: Vec<VideoFrame>,
    pending_bytes: usize,
    queue_budget: usize,
}
/// Backward-compatible name for the indexed MP4 video reader.
pub type Mp4AvcReader<R> = Mp4VideoReader<R>;
impl<R: Read + Seek> Mp4VideoReader<R> {
    /// Selects the first AVC, HEVC, VP9 or AV1 video track. Use `from_demuxer` to choose explicitly.
    pub fn open(reader: R, limits: Limits, decoder_budget: usize) -> Result<Self> {
        Self::open_with_hardware(reader, limits, decoder_budget, true)
    }
    /// Decode only with FVid codecs, regardless of compiled platform features.
    pub fn open_software(reader: R, limits: Limits, decoder_budget: usize) -> Result<Self> {
        Self::open_with_hardware(reader, limits, decoder_budget, false)
    }
    fn open_with_hardware(
        reader: R,
        limits: Limits,
        decoder_budget: usize,
        allow_hardware: bool,
    ) -> Result<Self> {
        let demuxer = Mp4Reader::open(reader, limits)?;
        let index = demuxer
            .tracks()
            .iter()
            .position(|t| {
                t.handler == *b"vide"
                    && matches!(
                        &t.codec,
                        b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"vp09" | b"av01"
                    )
            })
            .ok_or_else(|| invalid("MP4 has no supported AVC, HEVC, VP9 or AV1 video track"))?;
        Self::from_demuxer_with_hardware(demuxer, index, decoder_budget, allow_hardware)
    }
    pub fn from_demuxer(
        demuxer: Mp4Reader<R>,
        index: usize,
        decoder_budget: usize,
    ) -> Result<Self> {
        Self::from_demuxer_with_hardware(demuxer, index, decoder_budget, true)
    }
    fn from_demuxer_with_hardware(
        demuxer: Mp4Reader<R>,
        index: usize,
        decoder_budget: usize,
        allow_hardware: bool,
    ) -> Result<Self> {
        let _ = allow_hardware;
        let track = demuxer
            .tracks()
            .get(index)
            .ok_or_else(|| invalid("MP4 track index out of range"))?;
        if track.handler != *b"vide"
            || !matches!(
                &track.codec,
                b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"vp09" | b"av01"
            )
        {
            return Err(invalid(
                "selected MP4 track is not AVC, HEVC, VP9 or AV1 video",
            ));
        }
        // Reserve half the supplied budget for output reordering and its index.
        // Reference storage and reconstruction use the other half.
        let queue_budget = decoder_budget / 2;
        // A video track indexes one record per sample; the compact form belongs
        // to uncompressed audio, which this source never decodes.
        let Some(samples) = track.samples.expanded() else {
            return Err(invalid(
                "selected MP4 track is not AVC, HEVC, VP9 or AV1 video",
            ));
        };
        let index_bytes = samples
            .len()
            .checked_mul(std::mem::size_of::<i64>())
            .ok_or_else(|| invalid("MP4 reorder index overflow"))?;
        let queue_budget = queue_budget
            .checked_sub(index_bytes)
            .ok_or_else(|| invalid("MP4 reorder index exceeds budget"))?;
        let mut future_pts = Vec::new();
        future_pts
            .try_reserve_exact(samples.len())
            .map_err(|_| invalid("cannot allocate MP4 reorder index"))?;
        let mut minimum = i64::MAX;
        for sample in samples.iter().rev() {
            minimum = minimum.min(sample.pts);
            future_pts.push(minimum);
        }
        future_pts.reverse();
        let work_budget = decoder_budget - decoder_budget / 2;
        let decoder = match &track.codec {
            b"hvc1" | b"hev1" => Decoder::Hevc(Box::new(HevcDecoder::from_configuration(
                &track.configuration,
                work_budget,
            )?)),
            b"avc1" | b"avc3" => Decoder::Avc(Box::new(AvcDecoder::new(&track.configuration, work_budget)?)),
            b"vp09" => Decoder::Vp9(Box::new(vp9::Decoder::new(work_budget))),
            b"av01" => Decoder::Av1(Box::new(av1::Decoder::new(work_budget))),
            _ => unreachable!(),
        };
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        let hardware = if !allow_hardware || std::env::var_os("FVID_SOFTWARE_DECODE").is_some() {
            None
        } else {
            open_hardware(
                &track.codec,
                &track.configuration,
                track.width as u64,
                track.height as u64,
                work_budget,
                false,
            )
        };
        let mut source = Self {
            demuxer,
            decoder,
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            hardware,
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            decoded_surface: None,
            track_index: index,
            sample_index: 0,
            packet: Vec::new(),
            open_signal: None,
            failed: false,
            future_pts,
            pending: Vec::new(),
            pending_bytes: 0,
            queue_budget,
        };
        source.read_open_signal();
        Ok(source)
    }
    /// Ask an AV1 track what its own bytes state before any picture is decoded,
    /// keeping the answer for [`Self::bitstream_colour`] and
    /// [`Self::bitstream_hdr`] to hand on. The `av1C` configuration OBUs and the
    /// first sample are the two places an encoder puts them; bytes that do not
    /// parse state nothing, since the picture route meets the same bytes later
    /// and reports what it makes of them there.
    fn read_open_signal(&mut self) {
        if self.track().codec != *b"av01" {
            return;
        }
        let mut seed = av1_metadata::signal_from_bytes(&self.track().configuration);
        let mut packet = Vec::new();
        if self
            .demuxer
            .read_packet(self.track_index, 0, &mut packet)
            .is_ok()
        {
            // The sample is the later statement of the two, so it wins and the
            // configuration record fills whatever it left out.
            let (stated, light) = av1_metadata::signal_from_bytes(&packet);
            seed = (stated.filled_with(seed.0), light.filled_with(seed.1));
        }
        self.open_signal = Some(seed);
    }
    pub fn active_vui(&self) -> Option<&crate::codec::avc::Vui> {
        match &self.decoder {
            Decoder::Avc(d) => d.active_vui(),
            Decoder::Hevc(_) | Decoder::Vp9(_) | Decoder::Av1(_) => None,
        }
    }
    /// The VUI this reader's pictures are read with: the parameter set a decoded
    /// picture named, or, while none has, the first one the `avcC` record lists.
    /// A coded picture selects its own set, so the active answer wins as soon as
    /// there is one; a stream that states its signal only in the recorded set is
    /// thereby described the moment the file is open, which is when a caller that
    /// grades the first picture asks.
    fn avc_vui(&self) -> Option<&crate::codec::avc::Vui> {
        match &self.decoder {
            Decoder::Avc(d) => d.active_vui().or_else(|| d.recorded_vui()),
            Decoder::Hevc(_) | Decoder::Vp9(_) | Decoder::Av1(_) => None,
        }
    }
    pub fn active_colour(&self) -> Result<crate::playback_native::AvcColour> {
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        if let Some(hardware) = &self.hardware {
            return Ok(hardware.colour);
        }
        match &self.decoder {
            Decoder::Hevc(d) => {
                crate::playback_native::AvcColour::from_hevc_vui(d.parameters().0.vui.as_ref())
            }
            Decoder::Avc(_) => crate::playback_native::AvcColour::from_vui(self.avc_vui()),
            Decoder::Vp9(_) | Decoder::Av1(_) => Ok(crate::playback_native::AvcColour::default()),
        }
    }
    /// The signal the coding itself states, which is what a picture is graded
    /// by when the container's own `colr` atom says nothing: an HEVC or AVC
    /// VUI's three H.273 codes with the range its `video_signal_type` names, or
    /// the colour an AV1 sequence header carries. VP9 states nothing of its own
    /// and is left to the container.
    ///
    /// An AV1 and an AVC answer are both there the moment the file is open: the
    /// first from the `av1C` configuration or the sample behind it, the second
    /// from the parameter set the `avcC` record lists. A decoded picture that
    /// names a set of its own then wins, so a stream that changes its signal
    /// mid-file is graded by what it is coding then, not by what its first
    /// picture said.
    pub fn bitstream_colour(&self) -> ColourDescription {
        match &self.decoder {
            Decoder::Hevc(d) => d
                .parameters()
                .0
                .vui
                .as_ref()
                .and_then(|vui| vui.signal.as_ref())
                .map(|signal| {
                    let [primaries, transfer, matrix] = signal.colour.unwrap_or([0; 3]);
                    ColourDescription {
                        primaries,
                        transfer,
                        matrix,
                        full_range: signal.full_range,
                    }
                })
                .unwrap_or_default(),
            Decoder::Avc(_) => self
                .avc_vui()
                .and_then(|vui| vui.video_signal)
                .map(|(_, full_range, colour)| {
                    let [primaries, transfer, matrix] = colour.unwrap_or([0; 3]);
                    ColourDescription {
                        primaries,
                        transfer,
                        matrix,
                        full_range,
                    }
                })
                .unwrap_or_default(),
            Decoder::Av1(d) => {
                let seed = self.open_signal.map(|(seed, _)| seed).unwrap_or_default();
                d.color()
                    .map(crate::codec::av1_sequence::Color::signal)
                    .unwrap_or_default()
                    .filled_with(seed)
            }
            Decoder::Vp9(_) => ColourDescription::default(),
        }
    }
    /// The light the coding itself names for its pictures: an HEVC stream's
    /// mastering display and content light level SEI messages, or the AV1
    /// metadata OBUs of the same volume. An encoder that writes them in-band
    /// often writes them nowhere else, so a container with no `mdcv`/`ccll` box
    /// still has a tone-mappable answer.
    ///
    /// Both codecs give it the moment the file is open: HEVC's messages are read
    /// from the `hvcC`'s own NAL unit array as the decoder is built, and an AV1
    /// stream's OBUs are read from its `av1C` configuration and its first sample
    /// before a picture is decoded. That is what a caller grading the first
    /// picture needs; a stream that first states its light deeper in still
    /// states it there, once that packet has been decoded.
    pub fn bitstream_hdr(&self) -> HdrMetadata {
        match &self.decoder {
            Decoder::Hevc(d) => d.hdr(),
            Decoder::Av1(d) => {
                let seed = self.open_signal.map(|(_, seed)| seed).unwrap_or_default();
                let mut hdr = d.hdr();
                hdr.merge(seed);
                hdr
            }
            Decoder::Avc(_) | Decoder::Vp9(_) => HdrMetadata::default(),
        }
    }
    /// Whether frames come from the platform's hardware decoder.
    pub fn hardware_accelerated(&self) -> bool {
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        {
            self.hardware.is_some()
        }
        #[cfg(not(all(target_os = "macos", feature = "videotoolbox")))]
        {
            false
        }
    }
    /// Enable shared decoder surfaces before reading the first frame. Software
    /// readers remain software; unsupported codecs keep their existing output.
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    pub fn enable_shared_surfaces(&mut self) -> Result<bool> {
        if self.sample_index != 0 || !self.pending.is_empty() {
            return Err(invalid(
                "shared surfaces must be configured before decoding",
            ));
        }
        if self.hardware.is_none()
            || !matches!(&self.track().codec, b"avc1" | b"avc3" | b"hvc1" | b"hev1")
        {
            return Ok(false);
        }
        let track = self.track();
        let hardware = open_hardware(
            &track.codec,
            &track.configuration,
            u64::from(track.width),
            u64::from(track.height),
            self.queue_budget,
            true,
        );
        let Some(hardware) = hardware else {
            return Ok(false);
        };
        self.hardware = Some(hardware);
        Ok(true)
    }
    /// Decode the packet in `self.packet` with whichever decoder is active.
    /// Hardware frames come back as 8-bit planes plus a geometry-only picture.
    #[allow(clippy::type_complexity)]
    fn decode_packet(
        &mut self,
    ) -> Result<
        Option<(
            Arc<IntraPicture>,
            Option<Arc<crate::playback_native::Planar8>>,
        )>,
    > {
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        if let Some(hardware) = &mut self.hardware {
            if hardware.shared {
                let surface = hardware
                    .session
                    .decode_surface(&self.packet)
                    .map_err(|error| invalid(&error.to_string()))?;
                return Ok(surface.map(|surface| {
                    let (w, h) = (surface.width(), surface.height());
                    let picture = IntraPicture {
                        coded_width: w + w % 2,
                        coded_height: h + h % 2,
                        crop: [0, w % 2, 0, h % 2],
                        bit_depth: surface.depth(),
                        y: Vec::new(),
                        cb: Vec::new(),
                        cr: Vec::new(),
                    };
                    self.decoded_surface = Some(surface);
                    (Arc::new(picture), None)
                }));
            }
            let planes = hardware
                .session
                .decode(&self.packet)
                .map_err(|error| invalid(&error.to_string()))?;
            return Ok(planes.map(|planes| {
                let (picture, planes8) = hardware_frame(planes, hardware.colour);
                (Arc::new(picture), planes8.map(Arc::new))
            }));
        }
        match &mut self.decoder {
            Decoder::Avc(d) => Ok(d.decode_order(&self.packet)?.map(|picture| (picture, None))),
            Decoder::Hevc(d) => {
                let Some(frame) = d.decode_packet(&self.packet)?.filter(|f| f.output) else {
                    return Ok(None);
                };
                let p = &frame.picture;
                let colour = crate::playback_native::AvcColour::from_hevc_vui(
                    d.parameters().0.vui.as_ref(),
                )?;
                let coded_width = p.dimensions[0] as usize;
                let coded_height = p.dimensions[1] as usize;
                let crop = p.crop.map(|v| v as usize);
                let picture = IntraPicture {
                    coded_width,
                    coded_height,
                    crop,
                    bit_depth: p.depth[0],
                    y: p.planes[0].samples().to_vec(),
                    cb: p.planes[1].samples().to_vec(),
                    cr: p.planes[2].samples().to_vec(),
                };
                let planes = crate::playback_native::coded_planes_to_planar8(
                    &picture.y,
                    &picture.cb,
                    &picture.cr,
                    coded_width,
                    coded_height,
                    crop,
                    p.depth[0],
                    colour,
                );
                Ok(Some((Arc::new(picture), Some(Arc::new(planes)))))
            }
            Decoder::Vp9(d) => {
                for frame in crate::codec::vp9::frames(&self.packet)? {
                    let decoded = d.decode(frame)?;
                    if decoded.header.show_frame {
                        let p = &decoded.picture;
                        let colour = crate::playback_native::AvcColour {
                            kr: if decoded.header.picture.format.color_space == 1 {
                                0.2126
                            } else {
                                0.299
                            },
                            kb: if decoded.header.picture.format.color_space == 1 {
                                0.0722
                            } else {
                                0.114
                            },
                            full: decoded.header.picture.format.full_range,
                        };
                        let coded_width = p.size[0] as usize;
                        let coded_height = p.size[1] as usize;
                        let picture = IntraPicture {
                            coded_width,
                            coded_height,
                            crop: [0; 4],
                            bit_depth: p.depth,
                            y: p.planes[0].samples.clone(),
                            cb: p.planes[1].samples.clone(),
                            cr: p.planes[2].samples.clone(),
                        };
                        let planes = crate::playback_native::coded_planes_to_planar8(
                            &picture.y,
                            &picture.cb,
                            &picture.cr,
                            coded_width,
                            coded_height,
                            [0; 4],
                            p.depth,
                            colour,
                        );
                        return Ok(Some((Arc::new(picture), Some(Arc::new(planes)))));
                    }
                }
                Ok(None)
            }
            Decoder::Av1(d) => {
                for decoded in d.decode_packet(&self.packet)? {
                    if decoded.show {
                        let p = &decoded.picture;
                        let colour = crate::playback_native::AvcColour {
                            kr: match decoded.color.matrix {
                                1 => 0.2126,
                                2 | 5 | 6 => 0.299,
                                9 => 0.2627,
                                _ => {
                                    return Err(invalid("unsupported AV1 RGB matrix coefficients"));
                                }
                            },
                            kb: match decoded.color.matrix {
                                1 => 0.0722,
                                2 | 5 | 6 => 0.114,
                                9 => 0.0593,
                                _ => {
                                    return Err(invalid("unsupported AV1 RGB matrix coefficients"));
                                }
                            },
                            full: decoded.color.full_range,
                        };
                        let coded_width = p.size[0] as usize;
                        let coded_height = p.size[1] as usize;
                        let crop = [0; 4];
                        let picture = IntraPicture {
                            coded_width,
                            coded_height,
                            crop,
                            bit_depth: p.depth,
                            y: p.planes[0].samples.clone(),
                            cb: p.planes[1].samples.clone(),
                            cr: p.planes[2].samples.clone(),
                        };
                        let planes = crate::playback_native::coded_planes_to_planar8(
                            &picture.y,
                            &picture.cb,
                            &picture.cr,
                            coded_width,
                            coded_height,
                            crop,
                            p.depth,
                            colour,
                        );
                        return Ok(Some((Arc::new(picture), Some(Arc::new(planes)))));
                    }
                }
                Ok(None)
            }
        }
    }
    pub fn track(&self) -> &Track {
        &self.demuxer.tracks()[self.track_index]
    }
    pub fn movie_timescale(&self) -> u32 {
        self.demuxer.movie_timescale()
    }
    /// Restarts decoding from the first sample, dropping all reference pictures.
    /// Packet reads seek to indexed offsets, so no eager I/O is needed here.
    pub fn rewind(&mut self) {
        self.decoder.reset();
        self.demuxer.invalidate_position();
        self.sample_index = 0;
        self.failed = false;
        self.packet.clear();
        self.pending.clear();
        self.pending_bytes = 0;
    }
    /// Restart decoding at the sync sample with the greatest presentation time
    /// at or before `pts` (the first sample when none qualifies), dropping all
    /// reference pictures. Returns the presentation time decoding resumes at.
    pub fn seek_to_sync(&mut self, pts: i64) -> i64 {
        // A video track always indexes one record per sample; with no such index
        // there is nothing to restart from but the first sample.
        let samples = self.track().samples.expanded().unwrap_or(&[]);
        let index = samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.sync && s.pts <= pts)
            // Start before the entire equal-PTS group so seek agrees with
            // sequential playback, including its accumulated final interval.
            .max_by_key(|(index, s)| (s.pts, std::cmp::Reverse(*index)))
            .map_or(0, |(i, _)| i);
        self.rewind();
        self.sample_index = index;
        self.track().samples.get(index).map_or(0, |s| s.pts)
    }
    pub fn read_frame(&mut self) -> Result<Option<VideoFrame>> {
        if self.failed {
            return Err(invalid("MP4 frame source requires rewind after an error"));
        }
        let result = self.read_frame_inner();
        self.failed = result.is_err();
        result
    }
    fn read_frame_inner(&mut self) -> Result<Option<VideoFrame>> {
        loop {
            if let Some((index, frame)) = self
                .pending
                .iter()
                .enumerate()
                .min_by_key(|(_, frame)| (frame.presentation_time.ticks, frame.sample_index))
            {
                let future = self.future_pts.get(self.sample_index).copied();
                // Decode every equal-PTS picture before publishing the last
                // one at that time. Earlier duplicates still serve as references.
                if future.is_none_or(|pts| frame.presentation_time.ticks < pts) {
                    let mut frame = self.pending.swap_remove(index);
                    self.pending_bytes -= frame_storage(&frame)?;
                    let next = self
                        .pending
                        .iter()
                        .map(|f| f.presentation_time.ticks)
                        .chain(future)
                        .min();
                    frame.duration.ticks = presentation_duration(
                        frame.presentation_time.ticks,
                        next,
                        frame.duration.ticks,
                    )?;
                    return Ok(Some(frame));
                }
            }
            let index = self.sample_index;
            let Some(sample) = self.track().samples.get(index) else {
                return Ok(None);
            };
            self.demuxer
                .read_packet(self.track_index, index, &mut self.packet)?;
            let picture = self.decode_packet()?;
            self.sample_index += 1;
            if let Some((picture, planes8)) = picture {
                let mut frame = VideoFrame {
                    picture,
                    planes8,
                    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
                    surface: self.decoded_surface.take(),
                    presentation_time: MediaTime {
                        ticks: sample.pts,
                        timescale: self.track().timescale,
                    },
                    duration: MediaTime {
                        ticks: i64::from(sample.duration),
                        timescale: self.track().timescale,
                    },
                    sample_index: index,
                };
                while let Some(duplicate) = self
                    .pending
                    .iter()
                    .position(|old| old.presentation_time.ticks == frame.presentation_time.ticks)
                {
                    let old = self.pending.swap_remove(duplicate);
                    self.pending_bytes -= frame_storage(&old)?;
                    // If this group is at EOF there is no following PTS to
                    // establish its endpoint. Retain its nominal total span.
                    frame.duration.ticks = frame
                        .duration
                        .ticks
                        .checked_add(old.duration.ticks)
                        .ok_or_else(|| invalid("duplicate-PTS duration overflow"))?;
                }
                let bytes = frame_storage(&frame)?;
                let total = self
                    .pending_bytes
                    .checked_add(bytes)
                    .ok_or_else(|| invalid("MP4 reorder storage overflow"))?;
                if total > self.queue_budget {
                    return Err(invalid("MP4 reordered frames exceed memory budget"));
                }
                self.pending
                    .try_reserve(1)
                    .map_err(|_| invalid("cannot allocate reorder queue"))?;
                self.pending.push(frame);
                self.pending_bytes = total;
            }
        }
    }
}

/// Open a VideoToolbox session for the stream's parameter sets; `None` (with a
/// note on stderr) leaves decoding to the software decoder.
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn open_hardware(
    codec: &[u8],
    configuration: &[u8],
    width: u64,
    height: u64,
    budget: usize,
    shared: bool,
) -> Option<Hardware> {
    match codec {
        b"hvc1" | b"hev1" => open_hardware_hevc(configuration, budget, shared),
        b"avc1" | b"avc3" => open_hardware_avc(configuration, shared),
        b"vp09" => open_hardware_vp9(configuration, width, height),
        b"av01" => open_hardware_av1(configuration, width, height),
        _ => None,
    }
}
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn open_hardware_avc(configuration: &[u8], shared: bool) -> Option<Hardware> {
    let config = crate::codec::config::AvcConfig::parse(configuration).ok()?;
    let sps = crate::codec::avc::Sps::parse(config.sps.first()?).ok()?;
    let colour = crate::playback_native::AvcColour::from_vui(sps.vui.as_ref()).ok()?;
    let session = if shared {
        fvid_vt::Session::new_avc_surface(&config.sps, &config.pps, config.length_size, colour.full)
    } else {
        fvid_vt::Session::new(&config.sps, &config.pps, config.length_size)
    };
    match session {
        Ok(session) => Some(Hardware {
            session,
            colour,
            shared,
        }),
        Err(error) => {
            eprintln!("{error}; using the software decoder");
            None
        }
    }
}
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn open_hardware_hevc(configuration: &[u8], budget: usize, shared: bool) -> Option<Hardware> {
    use crate::codec::config::HevcConfig;
    let config = HevcConfig::parse(configuration).ok()?;
    let mut vps = Vec::new();
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    for array in &config.arrays {
        match array.nal_type {
            32 => vps.extend(array.units.iter().copied()),
            33 => sps.extend(array.units.iter().copied()),
            34 => pps.extend(array.units.iter().copied()),
            _ => {}
        }
    }
    if vps.is_empty() || sps.is_empty() || pps.is_empty() {
        return None;
    }
    let sps_parsed = crate::codec::hevc_sps::Sps::parse(sps.first()?, budget).ok()?;
    let colour = crate::playback_native::AvcColour::from_hevc_vui(sps_parsed.vui.as_ref()).ok()?;
    let constructor = if shared {
        fvid_vt::Session::new_hevc_surface
    } else {
        fvid_vt::Session::new_hevc_with_depth
    };
    match constructor(
        &vps,
        &sps,
        &pps,
        config.length_size,
        sps_parsed.depth[0],
        colour.full,
    ) {
        Ok(session) => Some(Hardware {
            session,
            colour,
            shared,
        }),
        Err(error) => {
            eprintln!("{error}; using the software HEVC decoder");
            None
        }
    }
}
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn open_hardware_vp9(configuration: &[u8], width: u64, height: u64) -> Option<Hardware> {
    let colour = crate::playback_native::AvcColour::default();
    match fvid_vt::Session::new_vp9(configuration, width as u32, height as u32) {
        Ok(session) => Some(Hardware {
            session,
            colour,
            shared: false,
        }),
        Err(error) => {
            eprintln!("{error}; using the software VP9 decoder");
            None
        }
    }
}
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn open_hardware_av1(configuration: &[u8], width: u64, height: u64) -> Option<Hardware> {
    let colour = crate::playback_native::AvcColour::default();
    match fvid_vt::Session::new_av1(configuration, width as u32, height as u32) {
        Ok(session) => Some(Hardware {
            session,
            colour,
            shared: false,
        }),
        Err(error) => {
            eprintln!("{error}; using the software AV1 decoder");
            None
        }
    }
}
/// Keep hardware output at its decoded precision: 8-bit bytes move into
/// `Planar8`; Main10 words retain low bits in a coded picture that the player's
/// source-depth path packs for GPU upload. A geometry-only picture accompanies
/// 8-bit planes, while Main10 owns samples and crops any padded odd border.
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn hardware_frame(
    planes: fvid_vt::Planes,
    colour: crate::playback_native::AvcColour,
) -> (IntraPicture, Option<crate::playback_native::Planar8>) {
    let (w, h) = (planes.width, planes.height);
    let mut colour = colour;
    colour.full = planes.full_range;
    if planes.depth > 8 {
        let words = |bytes: Vec<u8>| {
            bytes
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect()
        };
        let mut y: Vec<u16> = words(planes.y);
        // IntraPicture uses even coded geometry. Pad the last luma row/column
        // for odd visible sizes; the crop removes that padding before upload.
        if w % 2 != 0 {
            y = y
                .chunks_exact(w)
                .flat_map(|row| {
                    row.iter()
                        .copied()
                        .chain(std::iter::once(*row.last().unwrap()))
                })
                .collect();
        }
        if h % 2 != 0 {
            let last = y[y.len() - (w + w % 2)..].to_vec();
            y.extend(last);
        }
        return (
            IntraPicture {
                coded_width: w + w % 2,
                coded_height: h + h % 2,
                crop: [0, w % 2, 0, h % 2],
                bit_depth: planes.depth,
                y,
                cb: words(planes.cb),
                cr: words(planes.cr),
            },
            None,
        );
    }

    let picture = IntraPicture {
        coded_width: w + w % 2,
        coded_height: h + h % 2,
        crop: [0, w % 2, 0, h % 2],
        bit_depth: 8,
        y: Vec::new(),
        cb: Vec::new(),
        cr: Vec::new(),
    };
    let planar = crate::playback_native::Planar8 {
        width: w,
        height: h,
        chroma_width: w.div_ceil(2),
        chroma_height: h.div_ceil(2),
        y: planes.y,
        cb: planes.cb,
        cr: planes.cr,
        colour,
    };
    (picture, Some(planar))
}

fn presentation_duration(pts: i64, next: Option<i64>, final_duration: i64) -> Result<i64> {
    let duration = match next {
        Some(next) => next
            .checked_sub(pts)
            .ok_or_else(|| invalid("presentation duration overflow"))?,
        None => final_duration,
    };
    if duration <= 0 {
        return Err(invalid("non-increasing MP4 presentation timestamps"));
    }
    Ok(duration)
}

fn frame_storage(frame: &VideoFrame) -> Result<usize> {
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    let surface_bytes = frame
        .surface
        .as_ref()
        .map_or(0, fvid_vt::Surface::storage_bytes);
    #[cfg(not(all(target_os = "macos", feature = "videotoolbox")))]
    let surface_bytes = 0usize;
    let planes8 = frame
        .planes8
        .as_ref()
        .map_or(0, |p| p.y.len() + p.cb.len() + p.cr.len());
    frame
        .picture
        .y
        .len()
        .checked_add(frame.picture.cb.len())
        .and_then(|n| n.checked_add(frame.picture.cr.len()))
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(planes8))
        .and_then(|n| n.checked_add(surface_bytes))
        .and_then(|n| n.checked_add(std::mem::size_of::<VideoFrame>()))
        .ok_or_else(|| invalid("MP4 output frame size overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_intervals_follow_composition_order() {
        assert_eq!(presentation_duration(1024, Some(1536), 1024).unwrap(), 512);
        assert_eq!(presentation_duration(1536, Some(2560), 512).unwrap(), 1024);
        assert_eq!(presentation_duration(2560, None, 512).unwrap(), 512);
        assert_eq!(presentation_duration(-512, Some(0), 256).unwrap(), 512);
        assert!(presentation_duration(0, Some(0), 512).is_err());
        assert!(presentation_duration(i64::MIN, Some(i64::MAX), 512).is_err());
        assert!(presentation_duration(0, None, 0).is_err());
    }
    #[test]
    fn media_time_preserves_fractional_and_negative_timestamps() {
        assert_eq!(
            MediaTime {
                ticks: 1001,
                timescale: 30000
            }
            .nanoseconds()
            .unwrap(),
            33_366_666
        );
        assert_eq!(
            MediaTime {
                ticks: -1001,
                timescale: 30000
            }
            .nanoseconds()
            .unwrap(),
            -33_366_666
        );
        assert_eq!(
            MediaTime {
                ticks: i64::MAX,
                timescale: 1
            }
            .nanoseconds()
            .unwrap(),
            i128::from(i64::MAX) * 1_000_000_000
        );
        assert!(
            MediaTime {
                ticks: 0,
                timescale: 0
            }
            .nanoseconds()
            .is_err()
        );
    }

    /// The coding's half of a file's signal, read out of the stream itself: an
    /// HEVC clip whose parameter sets name BT.2020 primaries, a PQ curve and the
    /// BT.2020-NCL matrix states that triple through the reader, which is what
    /// fills in a container that wrote no `colr` atom.
    #[test]
    fn an_hevc_parameter_set_states_its_own_signal() {
        let data = include_bytes!("../../../tests/fixtures/hevc/hdr10.mp4").to_vec();
        let mut source =
            Mp4VideoReader::open(std::io::Cursor::new(data), Limits::default(), 16 << 20).unwrap();
        let stated = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        // The `hvcC`'s own parameter sets answer before a picture has been read.
        assert_eq!(source.bitstream_colour(), stated);
        let mut frames = 0;
        while source.read_frame().unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 5);
        assert_eq!(source.bitstream_colour(), stated);
    }

    /// An AV1 clip's light, in the only place its file writes it: SVT-AV1 4.2.0's
    /// own metadata OBUs for a BT.2020/1 000 cd/m² volume and a 1 234/567 content
    /// light level, spliced into the first packet of the 32x32 ramp and muxed
    /// into an MP4 with no `ccll`/`mdcv` box beside it. The reader states them
    /// the moment the file is open, because the grade a first picture gets is
    /// decided then — it asks the packet for its OBUs before it asks a decoder to
    /// turn them into a picture.
    #[test]
    fn an_av1_metadata_obu_states_its_own_light() {
        let data = include_bytes!("../../../tests/fixtures/av1/hdr-metadata.mp4").to_vec();
        let mut source =
            Mp4VideoReader::open(std::io::Cursor::new(data), Limits::default(), 16 << 20).unwrap();
        let opened = source.bitstream_hdr();
        assert!(opened.mastering.unwrap().is_hdr10());
        assert_eq!(
            (opened.light.max_cll, opened.light.max_fall),
            (1_234.0, 567.0)
        );
        // Decoding the packet that carries them states the same light again,
        // by whichever route the samples went.
        assert!(source.read_frame().unwrap().is_some());
        assert_eq!(source.bitstream_hdr(), opened);
    }

    /// The same light as an HEVC clip states it, which the file writes in no box
    /// at all: SEI 137 states the BT.2020 mastering volume and SEI 144 the
    /// content light levels. x265 wrote both messages into the `hvcC`'s NAL unit
    /// array as well as into the first access unit, so a reader states them the
    /// moment the file is open — which is when a caller that grades the first
    /// picture asks — and this holds whether samples go to the software walk or a
    /// hardware session.
    #[test]
    fn an_hevc_sei_states_its_own_light() {
        let data = include_bytes!("../../../tests/fixtures/hevc/hdr10.mp4").to_vec();
        let mut source =
            Mp4VideoReader::open(std::io::Cursor::new(data), Limits::default(), 16 << 20).unwrap();
        let opened = source.bitstream_hdr();
        assert!(opened.mastering.unwrap().is_hdr10());
        assert_eq!(
            (opened.light.max_cll, opened.light.max_fall),
            (1_000.0, 400.0)
        );
        // Decoding the packets that repeat the messages states the same light,
        // by whichever route the samples went.
        assert!(source.read_frame().unwrap().is_some());
        assert_eq!(source.bitstream_hdr(), opened);
    }

    /// An H.264 clip whose whole signal lives in the VUI of the sequence
    /// parameter set inside its `avcC`: libx264 wrote BT.2020 primaries, the
    /// ARIB STD-B67 curve and the BT.2020 NCL weights over a studio range into
    /// that SPS, and the muxer wrote no `colr` atom at all — measured from this
    /// file's own bytes, which hold neither `colr` nor `nclx`. A caller that
    /// grades the first picture asks when the file is open, so the recorded
    /// parameter set answers then rather than waiting for a picture to select
    /// one.
    #[test]
    fn an_avc_parameter_set_in_the_record_states_its_signal_at_open() {
        let data = include_bytes!("../../../tests/fixtures/avc/hlg-vui-only.mp4").to_vec();
        let mut source =
            Mp4VideoReader::open(std::io::Cursor::new(data), Limits::default(), 16 << 20).unwrap();
        // The container says nothing about the signal; the coding says HLG.
        assert_eq!(source.track().colour, ColourDescription::default());
        let stated = ColourDescription {
            primaries: 9,
            transfer: 18,
            matrix: 9,
            full_range: false,
        };
        assert_eq!(source.bitstream_colour(), stated);
        // A decoded picture that selects the very same SPS changes nothing.
        assert!(source.read_frame().unwrap().is_some());
        assert_eq!(source.bitstream_colour(), stated);
    }
}
