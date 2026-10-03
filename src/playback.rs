//! FVid-owned raw-video reader. No external container or codec implementation.
use crate::playback_native::yuv_to_rgb_range;
use crate::{Header, Result, buffer, invalid, line};
use std::io::{BufRead, Seek, SeekFrom};
use std::time::Duration;

/// Streaming 8/9/10/12/14/16-bit planar Y4M playback, using BT.601 limited or full-range colour.
/// Frame storage is reused; the budget covers the YUV and RGB buffers only.
pub struct Y4mReader<R> {
    reader: R,
    header: Header,
    marker: Vec<u8>,
    yuv: Vec<u8>,
    rgb: Vec<u8>,
    period: Duration,
    rate: (u32, u32),
    pixel_aspect: (u32, u32),
    full_range: bool,
    first_frame: u64,
    frames_read: u64,
}

impl<R: BufRead + Seek> Y4mReader<R> {
    pub fn new(mut reader: R, memory_limit: usize) -> Result<Self> {
        let mut marker = Vec::new();
        if !line(&mut reader, &mut marker)? {
            return Err(invalid("empty video input"));
        }
        if !marker.starts_with(b"YUV4MPEG2 ") {
            return Err(invalid(
                "native playback supports Y4M only; compressed codecs are not implemented",
            ));
        }
        let header = Header::parse(&marker)?;
        let mut rate = None;
        let mut pixel_aspect = None;
        let mut full_range = None;
        for token in &header.tokens {
            if let Some(value) = token.strip_prefix('A') {
                if pixel_aspect.is_some() {
                    return Err(invalid("duplicate pixel aspect"));
                }
                let (num, den) = value
                    .split_once(':')
                    .ok_or_else(|| invalid("invalid pixel aspect"))?;
                let num: u32 = num.parse().map_err(|_| invalid("invalid pixel aspect"))?;
                let den: u32 = den.parse().map_err(|_| invalid("invalid pixel aspect"))?;
                pixel_aspect = Some(match (num, den) {
                    (0, 0) => (1, 1),
                    (0, _) | (_, 0) => return Err(invalid("invalid pixel aspect")),
                    pair => pair,
                });
            }
            if let Some(value) = token.strip_prefix('F') {
                if rate.is_some() {
                    return Err(invalid("duplicate frame rate"));
                }
                let (num, den) = value
                    .split_once(':')
                    .ok_or_else(|| invalid("invalid frame rate"))?;
                let num: u32 = num.parse().map_err(|_| invalid("invalid frame rate"))?;
                let den: u32 = den.parse().map_err(|_| invalid("invalid frame rate"))?;
                if num == 0 || den == 0 {
                    return Err(invalid("playback requires a positive frame rate"));
                }
                rate = Some((num, den));
            }
            if let Some(value) = token.strip_prefix("XCOLORRANGE=") {
                if full_range.is_some() {
                    return Err(invalid("duplicate Y4M colour range"));
                }
                full_range = Some(match value {
                    "LIMITED" => false,
                    "FULL" => true,
                    _ => return Err(invalid("invalid Y4M colour range")),
                });
            }
            if matches!(token.as_str(), "C420mpeg2" | "C420paldv") {
                return Err(invalid(
                    "playback currently supports C420jpeg, C422 and C444",
                ));
            }
        }
        let rate = rate.ok_or_else(|| invalid("playback requires a Y4M frame rate"))?;
        let period = Duration::from_secs_f64(f64::from(rate.1) / f64::from(rate.0));
        if period.is_zero() {
            return Err(invalid("frame rate exceeds clock precision"));
        }
        let yuv_len = header.frame_len()?;
        let rgb_len = header
            .width
            .checked_mul(header.height)
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(|| invalid("RGB dimensions overflow"))?;
        if yuv_len
            .checked_add(rgb_len)
            .is_none_or(|n| n > memory_limit)
        {
            return Err(invalid("video frame exceeds playback buffer budget"));
        }
        let first_frame = reader.stream_position()?;
        Ok(Self {
            reader,
            header,
            marker,
            yuv: buffer(yuv_len)?,
            rgb: buffer(rgb_len)?,
            period,
            rate,
            pixel_aspect: pixel_aspect.unwrap_or((1, 1)),
            full_range: full_range.unwrap_or(false),
            first_frame,
            frames_read: 0,
        })
    }

    pub fn full_range(&self) -> bool {
        self.full_range
    }
    pub fn pixel_aspect(&self) -> (u32, u32) {
        self.pixel_aspect
    }
    pub fn dimensions(&self) -> [usize; 2] {
        [self.header.width, self.header.height]
    }
    pub fn frame_rate(&self) -> (u32, u32) {
        self.rate
    }
    pub fn frame_period(&self) -> Duration {
        self.period
    }
    pub fn frames_read(&self) -> u64 {
        self.frames_read
    }
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    pub fn rewind(&mut self) -> Result<()> {
        self.reader.seek(SeekFrom::Start(self.first_frame))?;
        self.frames_read = 0;
        Ok(())
    }

    /// Returns false only at a clean frame boundary; truncated frames are errors.
    pub fn read_frame(&mut self) -> Result<bool> {
        if !self.read_frame_raw()? {
            return Ok(false);
        }
        if self.depth()!=8 {
            let packed=self.packed()?;let budget=self.rgb.len();packed.to_rgb(&mut self.rgb,budget)?;return Ok(true);
        }
        let (sx, sy) = self.header.format.subsampling();
        let luma_len = self.header.width * self.header.height;
        let chroma_len = self.width().div_ceil(sx) * self.height().div_ceil(sy);
        yuv_to_rgb_range(
            &self.yuv,
            luma_len,
            chroma_len,
            self.header.width,
            self.header.height,
            sx,
            sy,
            self.full_range,
            &mut self.rgb,
        );
        Ok(true)
    }
    /// Reads one YUV frame without converting to RGB; `frames_read` advances.
    pub fn read_frame_raw(&mut self) -> Result<bool> {
        if !line(&mut self.reader, &mut self.marker)? {
            return Ok(false);
        }
        if self.marker != b"FRAME\n" && !self.marker.starts_with(b"FRAME ") {
            return Err(invalid("expected FRAME marker"));
        }
        self.reader.read_exact(&mut self.yuv)?;
        self.frames_read += 1;
        Ok(true)
    }
    pub fn depth(&self)->u8 {self.header.depth()}
    pub fn packed(&self)->Result<crate::playback_native::PackedPlanar> {
        let (sx,sy)=self.subsampling();
        crate::playback_native::PackedPlanar::new(crate::native_geometry::GeometryFrame {
            width:self.width(),height:self.height(),subsampling:Some([sx,sy]),data:self.yuv.clone(),
        },self.depth(),crate::playback_native::AvcColour {kr:0.299,kb:0.114,full:self.full_range()})
    }
    pub fn subsampling(&self) -> (usize, usize) {
        self.header.format.subsampling()
    }
    pub fn width(&self) -> usize {
        self.header.width
    }
    pub fn height(&self) -> usize {
        self.header.height
    }
    pub fn yuv(&self) -> &[u8] {
        &self.yuv
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn converts_chroma_planes_in_all_supported_layouts() {
        for (format, chroma_len) in [("420jpeg", 1), ("422", 2), ("444", 4)] {
            let mut data =
                format!("YUV4MPEG2 W2 H2 F30000:1001 Ip C{format}\nFRAME tag\n").into_bytes();
            data.extend_from_slice(&[81; 4]);
            data.extend(std::iter::repeat_n(90, chroma_len));
            data.extend(std::iter::repeat_n(240, chroma_len));
            let mut reader = Y4mReader::new(Cursor::new(data), 100).unwrap();
            assert!(reader.read_frame().unwrap());
            for pixel in reader.rgb().chunks_exact(3) {
                assert!(pixel[0] >= 254 && pixel[1] <= 1 && pixel[2] <= 1);
            }
            assert!(!reader.read_frame().unwrap());
        }
    }

    #[test]
    fn decode_black_white_and_rewind() {
        let mut data = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\nFRAME\n".to_vec();
        data.extend_from_slice(&[16, 235, 16, 235, 128, 128]);
        let mut reader = Y4mReader::new(Cursor::new(data), 18).unwrap();
        assert_eq!(reader.frame_period(), Duration::from_millis(40));
        assert!(reader.read_frame().unwrap());
        assert_eq!(
            reader.rgb(),
            &[0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255]
        );
        assert!(!reader.read_frame().unwrap());
        reader.rewind().unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.frames_read(), 1);
    }

    #[test]
    fn rejects_invalid_rate_budget_and_truncation() {
        for header in [
            "YUV4MPEG2 W2 H2 F0:1\n",
            "YUV4MPEG2 W2 H2 F25:0\n",
            "YUV4MPEG2 W2 H2\n",
            "YUV4MPEG2 W2 H2 F25:1 F30:1\n",
        ] {
            assert!(Y4mReader::new(Cursor::new(header), 100).is_err());
        }
        let data = b"YUV4MPEG2 W2 H2 F25:1\nFRAME\n\x10";
        assert!(Y4mReader::new(Cursor::new(data), 17).is_err());
        let mut reader = Y4mReader::new(Cursor::new(data), 18).unwrap();
        assert!(reader.read_frame().is_err());
    }
}


/// Video-container readers and the common native reader.
pub mod video {
    pub use crate::{playback_mp4 as mp4, playback_native as native, playback_webm as webm};
}

/// Audio readers by source/container. The old `crate::playback_*` names remain supported.
#[cfg(feature = "player")]
pub mod audio {
    pub use crate::{
        playback_aac as aac, playback_ac3 as ac3, playback_aiff as aiff, playback_au as au,
        playback_avi_audio as avi, playback_flac as flac, playback_mp3 as mp3,
        playback_mp4_audio as mp4, playback_ogg_audio as ogg, playback_smf as smf,
        playback_wav as wav, playback_webm_audio as webm, playback_xm as xm,
    };
}

/// Subtitle readers attached to supported video containers.
#[cfg(feature = "player")]
pub mod subtitle_tracks {
    pub use crate::{playback_mp4_subtitles as mp4, playback_webm_subtitles as webm};
}

/// Playback worker and bounded-spool implementation details.
pub mod runtime {
    pub use crate::{playback_spool as spool, playback_thread as thread};
}
