//! RGB playback adapter for FVid's own Y4M, MP4/AVC and WebM/VP9/AV1 readers.
use crate::{
    Result, container::mp4::Limits, invalid, playback::Y4mReader, playback_mp4::Mp4AvcReader,
};
use std::{
    io::{BufRead, Seek, SeekFrom},
    time::Duration,
};

pub enum NativeReader<R> {
    Webm(crate::playback_webm::WebmVideoReader<R>),
    Y4m(Y4mReader<R>),
    Avc {
        source: Mp4AvcReader<R>,
        rgb: Vec<u8>,
        dimensions: [usize; 2],
        period: Duration,
        next_pts: i64,
        frame_start: i64,
        frames: u64,
        rgb_budget: usize,
        media_start: i64,
        media_end: Option<i64>,
    },
}
impl<R: BufRead + Seek> NativeReader<R> {
    /// Playback without an application-imposed memory cap. Format bounds and
    /// checked size arithmetic still apply; storage is allocated as needed.
    pub fn without_memory_limit(reader: R) -> Result<Self> {
        Self::new(reader, usize::MAX)
    }

    pub fn new(mut reader: R, budget: usize) -> Result<Self> {
        let start = reader.stream_position()?;
        let mut prefix = [0u8; 9];
        let mut length = 0;
        while length < prefix.len() {
            let count = reader.read(&mut prefix[length..])?;
            if count == 0 {
                break;
            }
            length += count;
        }
        reader.seek(SeekFrom::Start(start))?;
        if &prefix == b"YUV4MPEG2" {
            return Ok(Self::Y4m(Y4mReader::new(reader, budget)?));
        }
        if length >= 4 && prefix[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
            return Ok(Self::Webm(crate::playback_webm::WebmVideoReader::open(
                reader, budget,
            )?));
        }
        let rgb_budget = budget / 4;
        let source = Mp4AvcReader::open(reader, Limits::default(), budget - rgb_budget)?;
        let track = source.track();
        let (media_start, media_end) = playback_window(track, source.movie_timescale())?;
        Ok(Self::Avc {
            source,
            rgb: Vec::new(),
            dimensions: [0; 2],
            period: Duration::ZERO,
            next_pts: 0,
            frame_start: 0,
            frames: 0,
            rgb_budget,
            media_start,
            media_end,
        })
    }
    pub fn dimensions(&self) -> [usize; 2] {
        match self {
            Self::Y4m(r) => r.dimensions(),
            Self::Webm(r) => r.dimensions(),
            Self::Avc { dimensions, .. } => *dimensions,
        }
    }
    pub fn rgb(&self) -> &[u8] {
        match self {
            Self::Y4m(r) => r.rgb(),
            Self::Webm(r) => r.rgb(),
            Self::Avc { rgb, .. } => rgb,
        }
    }
    pub fn frame_period(&self) -> Duration {
        match self {
            Self::Y4m(r) => r.frame_period(),
            Self::Webm(r) => r.frame_period(),
            Self::Avc { period, .. } => *period,
        }
    }
    /// Exact half-open interval of the current frame, in source clock ticks.
    /// No accumulated floating-point or nanosecond rounding is involved.
    pub fn frame_interval(&self) -> Option<(u128, u128, u32)> {
        match self {
            Self::Webm(r) => r.frame_interval(),
            Self::Y4m(reader) => {
                let count = reader.frames_read();
                if count == 0 {
                    return None;
                }
                let (num, den) = reader.frame_rate();
                Some((
                    u128::from(count - 1) * u128::from(den),
                    u128::from(count) * u128::from(den),
                    num,
                ))
            }
            Self::Avc {
                source,
                frame_start,
                next_pts,
                frames,
                ..
            } => {
                if *frames == 0 {
                    return None;
                }
                Some((
                    *frame_start as u128,
                    *next_pts as u128,
                    source.track().timescale,
                ))
            }
        }
    }
    pub fn rewind(&mut self) -> Result<()> {
        match self {
            Self::Y4m(r) => r.rewind(),
            Self::Webm(r) => {
                r.rewind();
                Ok(())
            }
            Self::Avc {
                source,
                next_pts,
                frame_start,
                frames,
                ..
            } => {
                source.rewind();
                *next_pts = 0;
                *frame_start = 0;
                *frames = 0;
                Ok(())
            }
        }
    }
    pub fn read_frame(&mut self) -> Result<bool> {
        match self {
            Self::Y4m(r) => r.read_frame(),
            Self::Webm(r) => r.read_frame(),
            Self::Avc {
                source,
                rgb,
                dimensions,
                period,
                next_pts,
                frame_start,
                frames,
                rgb_budget,
                media_start,
                media_end,
            } => {
                if media_end.is_some_and(|end| {
                    i128::from(*next_pts) + i128::from(*media_start) >= i128::from(end)
                }) {
                    return Ok(false);
                }
                let frame = loop {
                    let Some(mut frame) = source.read_frame()? else {
                        return Ok(false);
                    };
                    let begin = frame.presentation_time.ticks;
                    let end = begin
                        .checked_add(frame.duration.ticks)
                        .ok_or_else(|| invalid("video timestamp overflow"))?;
                    if end <= *media_start {
                        continue;
                    }
                    if media_end.is_some_and(|limit| begin >= limit) {
                        return Ok(false);
                    }
                    let clipped_begin = begin.max(*media_start);
                    let clipped_end = media_end.map_or(end, |limit| end.min(limit));
                    frame.presentation_time.ticks = clipped_begin
                        .checked_sub(*media_start)
                        .ok_or_else(|| invalid("video edit timestamp overflow"))?;
                    frame.duration.ticks = clipped_end
                        .checked_sub(clipped_begin)
                        .ok_or_else(|| invalid("video edit duration overflow"))?;
                    break frame;
                };
                if frame.presentation_time.ticks != *next_pts || frame.duration.ticks <= 0 {
                    return Err(invalid(
                        "non-contiguous AVC presentation timestamps are not implemented",
                    ));
                }
                let nanos = frame.duration.nanoseconds()?;
                let nanos = u64::try_from(nanos)
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| invalid("invalid video frame duration"))?;
                let signal = source.active_vui().and_then(|v| v.video_signal);
                let full = signal.is_some_and(|(_, full, _)| full);
                let matrix = signal
                    .and_then(|(_, _, colour)| colour)
                    .map(|c| c[2])
                    .unwrap_or(2);
                let (kr, kb) = match matrix {
                    1 => (0.2126, 0.0722),
                    2 | 5 | 6 => (0.299, 0.114),
                    _ => return Err(invalid("AVC colour matrix is not implemented for playback")),
                };
                let p = &frame.picture;
                let (w, h) = p.dimensions();
                let len = w
                    .checked_mul(h)
                    .and_then(|n| n.checked_mul(3))
                    .filter(|n| *n <= *rgb_budget)
                    .ok_or_else(|| invalid("RGB frame exceeds playback budget"))?;
                if rgb.len() != len {
                    *rgb = crate::buffer(len)?;
                }
                let scale = f64::from(1u32 << (p.bit_depth - 8));
                let (y_offset, y_range, c_range) = if full {
                    (
                        0.0,
                        f64::from((1u32 << p.bit_depth) - 1),
                        f64::from((1u32 << p.bit_depth) - 1),
                    )
                } else {
                    (16.0 * scale, 219.0 * scale, 224.0 * scale)
                };
                for (i, pixel) in rgb.chunks_exact_mut(3).enumerate() {
                    let x = i % w + p.crop[0];
                    let y = i / w + p.crop[2];
                    let at = (y / 2) * (p.coded_width / 2) + x / 2;
                    let luma = (f64::from(p.y[y * p.coded_width + x]) - y_offset) / y_range;
                    let cb = (f64::from(p.cb[at]) - 128.0 * scale) / c_range;
                    let cr = (f64::from(p.cr[at]) - 128.0 * scale) / c_range;
                    let red = luma + 2.0 * (1.0 - kr) * cr;
                    let blue = luma + 2.0 * (1.0 - kb) * cb;
                    let green = (luma - kr * red - kb * blue) / (1.0 - kr - kb);
                    for (out, value) in pixel.iter_mut().zip([red, green, blue]) {
                        *out = (value * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                }
                *dimensions = [w, h];
                *period = Duration::from_nanos(nanos);
                *frame_start = frame.presentation_time.ticks;
                *next_pts = frame
                    .presentation_time
                    .ticks
                    .checked_add(frame.duration.ticks)
                    .ok_or_else(|| invalid("video timestamp overflow"))?;
                *frames += 1;
                Ok(true)
            }
        }
    }
}

/// A single rate-one edit can trim/offset the media timeline without changing
/// the decode sequence. Empty edits and repeated ranges need a richer scheduler.
fn playback_window(
    track: &crate::container::mp4::Track,
    movie_scale: u32,
) -> Result<(i64, Option<i64>)> {
    if track.edits.is_empty() {
        return Ok((0, None));
    }
    if track.edits.len() != 1 || track.edits[0].media_time < 0 || movie_scale == 0 {
        return Err(invalid(
            "multiple or empty MP4 playback edits are not implemented",
        ));
    }
    let edit = &track.edits[0];
    let numerator = u128::from(edit.duration) * u128::from(track.timescale);
    if numerator % u128::from(movie_scale) != 0 {
        return Err(invalid(
            "fractional media-tick edit endpoint is not implemented",
        ));
    }
    let duration = i64::try_from(numerator / u128::from(movie_scale))
        .map_err(|_| invalid("MP4 edit duration overflow"))?;
    if duration <= 0 {
        return Err(invalid("empty MP4 playback edit"));
    }
    let end = edit
        .media_time
        .checked_add(duration)
        .ok_or_else(|| invalid("MP4 edit endpoint overflow"))?;
    Ok((edit.media_time, Some(end)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};
    #[test]
    fn webm_signature_never_enters_mp4_parser() {
        let bytes = [0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 0x01];
        let error = match NativeReader::new(Cursor::new(bytes), 1 << 20) {
            Ok(_) => panic!("truncated WebM accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("EBML"));
        assert!(!error.contains("box"));
    }
    #[test]
    fn edit_window_preserves_track_units_and_rejects_unrepresentable_endpoints() {
        use crate::container::mp4::{Edit, Track};
        let mut track = Track {
            id: 1,
            handler: *b"vide",
            codec: *b"avc1",
            timescale: 12800,
            duration: 6144,
            width: 64,
            height: 64,
            channels: 0,
            sample_rate: 0,
            configuration: vec![],
            edits: vec![],
            samples: vec![],
        };
        assert_eq!(playback_window(&track, 1000).unwrap(), (0, None));
        track.edits.push(Edit {
            duration: 480,
            media_time: 1024,
        });
        assert_eq!(playback_window(&track, 1000).unwrap(), (1024, Some(7168)));
        track.edits[0].duration = 1;
        assert!(playback_window(&track, 1000).is_err());
        track.edits[0].duration = 480;
        track.edits[0].media_time = -1;
        assert!(playback_window(&track, 1000).is_err());
        track.edits[0].media_time = i64::MAX;
        assert!(playback_window(&track, 1000).is_err());
        track.edits[0].media_time = 0;
        track.edits.push(track.edits[0].clone());
        assert!(playback_window(&track, 1000).is_err());
    }
    #[test]
    fn format_detection_handles_small_buffers_and_y4m_rewind() {
        let mut bytes = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\nFRAME\n".to_vec();
        bytes.extend_from_slice(&[16, 235, 16, 235, 128, 128]);
        let mut reader =
            NativeReader::new(BufReader::with_capacity(1, Cursor::new(bytes)), 18).unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.dimensions(), [2, 2]);
        assert_eq!(
            reader.rgb(),
            &[0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255]
        );
        assert_eq!(reader.frame_period(), Duration::from_millis(40));
        assert!(!reader.read_frame().unwrap());
        reader.rewind().unwrap();
        assert!(reader.read_frame().unwrap());
    }
}
