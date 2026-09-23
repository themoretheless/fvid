//! Indexed MP4 frame source for AVC, HEVC, VP9 and AV1.
//! Frame timestamps remain in the track media timeline; `track().edits` describes
//! presentation edits separately. This source does not silently discard edits.
use crate::codec::{
    av1_decoder as av1, avc_decoder::AvcDecoder, avc_picture::IntraPicture,
    hevc_decoder::HevcDecoder, vp9_decoder as vp9,
};
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
    pub presentation_time: MediaTime,
    pub duration: MediaTime,
    pub sample_index: usize,
}

/// Indexed non-fragmented MP4 source for the currently supported AVC I/P/B tools.
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
}

enum Decoder {
    Avc(AvcDecoder),
    Hevc(HevcDecoder),
    Vp9(vp9::Decoder),
    Av1(av1::Decoder),
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
    track_index: usize,
    sample_index: usize,
    packet: Vec<u8>,
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
        Self::from_demuxer(demuxer, index, decoder_budget)
    }
    pub fn from_demuxer(
        demuxer: Mp4Reader<R>,
        index: usize,
        decoder_budget: usize,
    ) -> Result<Self> {
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
        let index_bytes = track
            .samples
            .len()
            .checked_mul(std::mem::size_of::<i64>())
            .ok_or_else(|| invalid("MP4 reorder index overflow"))?;
        let queue_budget = queue_budget
            .checked_sub(index_bytes)
            .ok_or_else(|| invalid("MP4 reorder index exceeds budget"))?;
        let mut future_pts = Vec::new();
        future_pts
            .try_reserve_exact(track.samples.len())
            .map_err(|_| invalid("cannot allocate MP4 reorder index"))?;
        let mut minimum = i64::MAX;
        for sample in track.samples.iter().rev() {
            minimum = minimum.min(sample.pts);
            future_pts.push(minimum);
        }
        future_pts.reverse();
        let work_budget = decoder_budget - decoder_budget / 2;
        let decoder = match &track.codec {
            b"hvc1" | b"hev1" => Decoder::Hevc(HevcDecoder::from_configuration(
                &track.configuration,
                work_budget,
            )?),
            b"avc1" | b"avc3" => Decoder::Avc(AvcDecoder::new(&track.configuration, work_budget)?),
            b"vp09" => Decoder::Vp9(vp9::Decoder::new(work_budget)),
            b"av01" => Decoder::Av1(av1::Decoder::new(work_budget)),
            _ => unreachable!(),
        };
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        let hardware = if std::env::var_os("FVID_SOFTWARE_DECODE").is_some() {
            None
        } else {
            open_hardware(
                &track.codec,
                &track.configuration,
                track.width as u64,
                track.height as u64,
                work_budget,
            )
        };
        Ok(Self {
            demuxer,
            decoder,
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            hardware,
            track_index: index,
            sample_index: 0,
            packet: Vec::new(),
            failed: false,
            future_pts,
            pending: Vec::new(),
            pending_bytes: 0,
            queue_budget,
        })
    }
    pub fn active_vui(&self) -> Option<&crate::codec::avc::Vui> {
        match &self.decoder {
            Decoder::Avc(d) => d.active_vui(),
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
            Decoder::Avc(_) => crate::playback_native::AvcColour::from_vui(self.active_vui()),
            Decoder::Vp9(_) | Decoder::Av1(_) => Ok(crate::playback_native::AvcColour::default()),
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
            let planes = hardware
                .session
                .decode(&self.packet)
                .map_err(|error| invalid(&error.to_string()))?;
            return Ok(planes.map(|planes| {
                let (picture, planes8) = hardware_frame(planes, hardware.colour);
                (Arc::new(picture), Some(Arc::new(planes8)))
            }));
        }
        match &mut self.decoder {
            Decoder::Avc(d) => Ok(d.decode_order(&self.packet)?.map(|picture| (picture, None))),
            Decoder::Hevc(d) => {
                let Some(frame) = d.decode_packet(&self.packet)?.filter(|f| f.output) else {
                    return Ok(None);
                };
                let p = &frame.picture;
                let colour =
                    crate::playback_native::AvcColour::from_hevc_vui(d.parameters().0.vui.as_ref())?;
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
                                    return Err(invalid(
                                        "unsupported AV1 RGB matrix coefficients",
                                    ))
                                }
                            },
                            kb: match decoded.color.matrix {
                                1 => 0.0722,
                                2 | 5 | 6 => 0.114,
                                9 => 0.0593,
                                _ => {
                                    return Err(invalid(
                                        "unsupported AV1 RGB matrix coefficients",
                                    ))
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
        let samples = &self.track().samples;
        let index = samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.sync && s.pts <= pts)
            .max_by_key(|(_, s)| s.pts)
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
                if future.is_none_or(|pts| frame.presentation_time.ticks <= pts) {
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
            if self.sample_index == self.track().samples.len() {
                return Ok(None);
            }

            let index = self.sample_index;
            let sample = self.track().samples[index].clone();
            self.demuxer
                .read_packet(self.track_index, index, &mut self.packet)?;
            let picture = self.decode_packet()?;
            self.sample_index += 1;
            if let Some((picture, planes8)) = picture {
                let frame = VideoFrame {
                    picture,
                    planes8,
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
) -> Option<Hardware> {
    match codec {
        b"hvc1" | b"hev1" => open_hardware_hevc(configuration, budget),
        b"avc1" | b"avc3" => open_hardware_avc(configuration),
        b"vp09" => open_hardware_vp9(configuration, width, height),
        b"av01" => open_hardware_av1(configuration, width, height),
        _ => None,
    }
}
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn open_hardware_avc(configuration: &[u8]) -> Option<Hardware> {
    let config = crate::codec::config::AvcConfig::parse(configuration).ok()?;
    let sps = crate::codec::avc::Sps::parse(config.sps.first()?).ok()?;
    let colour = crate::playback_native::AvcColour::from_vui(sps.vui.as_ref()).ok()?;
    match fvid_vt::Session::new(&config.sps, &config.pps, config.length_size) {
        Ok(session) => Some(Hardware { session, colour }),
        Err(error) => {
            eprintln!("{error}; using the software decoder");
            None
        }
    }
}
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn open_hardware_hevc(configuration: &[u8], budget: usize) -> Option<Hardware> {
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
    let colour =
        crate::playback_native::AvcColour::from_hevc_vui(sps_parsed.vui.as_ref()).ok()?;
    match fvid_vt::Session::new_hevc(&vps, &sps, &pps, config.length_size) {
        Ok(session) => Some(Hardware { session, colour }),
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
        Ok(session) => Some(Hardware { session, colour }),
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
        Ok(session) => Some(Hardware { session, colour }),
        Err(error) => {
            eprintln!("{error}; using the software AV1 decoder");
            None
        }
    }
}
/// Wrap the hardware decoder's packed 8-bit planes without copying them: the
/// planes move into a `Planar8`, and a geometry-only picture (empty sample
/// vectors, even coded size, odd edges cropped) carries the dimensions.
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn hardware_frame(
    planes: fvid_vt::Planes,
    colour: crate::playback_native::AvcColour,
) -> (IntraPicture, crate::playback_native::Planar8) {
    let (w, h) = (planes.width, planes.height);
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
    (picture, planar)
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
}
