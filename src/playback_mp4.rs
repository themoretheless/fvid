//! FVid MP4/AVC frame source. No external demultiplexer or decoder.
//! Frame timestamps remain in the track media timeline; `track().edits` describes
//! presentation edits separately. This source does not silently discard edits.
use crate::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture};
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
    pub picture: Arc<IntraPicture>,
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
    sps: crate::codec::avc::Sps,
}

pub struct Mp4AvcReader<R> {
    demuxer: Mp4Reader<R>,
    decoder: AvcDecoder,
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
impl<R: Read + Seek> Mp4AvcReader<R> {
    /// Selects the first AVC video track. Use `from_demuxer` to choose explicitly.
    pub fn open(reader: R, limits: Limits, decoder_budget: usize) -> Result<Self> {
        let demuxer = Mp4Reader::open(reader, limits)?;
        let index = demuxer
            .tracks()
            .iter()
            .position(|t| t.handler == *b"vide" && matches!(&t.codec, b"avc1" | b"avc3"))
            .ok_or_else(|| invalid("MP4 has no AVC video track"))?;
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
        if track.handler != *b"vide" || !matches!(&track.codec, b"avc1" | b"avc3") {
            return Err(invalid("selected MP4 track is not AVC video"));
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
        let decoder = AvcDecoder::new(&track.configuration, decoder_budget - decoder_budget / 2)?;
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        let hardware = if std::env::var_os("FVID_SOFTWARE_DECODE").is_some() {
            None
        } else {
            open_hardware(&track.configuration)
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
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        if let Some(hardware) = &self.hardware {
            return hardware.sps.vui.as_ref();
        }
        self.decoder.active_vui()
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
    fn decode_packet(&mut self) -> Result<Option<Arc<IntraPicture>>> {
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        if let Some(hardware) = &mut self.hardware {
            let planes = hardware
                .session
                .decode(&self.packet)
                .map_err(|error| invalid(&error.to_string()))?;
            return Ok(planes.map(|planes| Arc::new(planes_to_picture(planes))));
        }
        self.decoder.decode_order(&self.packet)
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
            if let Some(picture) = picture {
                let frame = VideoFrame {
                    picture,
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
fn open_hardware(configuration: &[u8]) -> Option<Hardware> {
    let config = crate::codec::config::AvcConfig::parse(configuration).ok()?;
    let sps = crate::codec::avc::Sps::parse(config.sps.first()?).ok()?;
    match fvid_vt::Session::new(&config.sps, &config.pps, config.length_size) {
        Ok(session) => Some(Hardware { session, sps }),
        Err(error) => {
            eprintln!("{error}; using the software decoder");
            None
        }
    }
}
/// Widen the hardware decoder's packed 8-bit planes into the decoder's picture
/// layout (even coded size, odd edges cropped).
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
fn planes_to_picture(planes: fvid_vt::Planes) -> IntraPicture {
    let (w, h) = (planes.width, planes.height);
    let coded_width = w + w % 2;
    let coded_height = h + h % 2;
    let widen = |src: &[u8], width: usize, height: usize, stride: usize, rows: usize| {
        let mut out = vec![0u16; stride * rows];
        for (row, line) in src.chunks_exact(width).take(height).enumerate() {
            for (dst, &s) in out[row * stride..][..width].iter_mut().zip(line) {
                *dst = u16::from(s);
            }
        }
        out
    };
    IntraPicture {
        coded_width,
        coded_height,
        crop: [0, w % 2, 0, h % 2],
        bit_depth: 8,
        y: widen(&planes.y, w, h, coded_width, coded_height),
        cb: widen(
            &planes.cb,
            w.div_ceil(2),
            h.div_ceil(2),
            coded_width / 2,
            coded_height / 2,
        ),
        cr: widen(
            &planes.cr,
            w.div_ceil(2),
            h.div_ceil(2),
            coded_width / 2,
            coded_height / 2,
        ),
    }
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
    frame
        .picture
        .y
        .len()
        .checked_add(frame.picture.cb.len())
        .and_then(|n| n.checked_add(frame.picture.cr.len()))
        .and_then(|n| n.checked_mul(2))
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
