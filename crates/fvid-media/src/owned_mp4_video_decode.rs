//! Owned AVC/HEVC packet decoding with shared MP4 presentation edits.
use fvid_codecs::{
    Error,
    codec::{avc_decoder::AvcDecoder, hevc_decoder::HevcDecoder},
};
use fvid_media_info::{DecodeStats, DecodeTransform};
use std::{fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn try_decode(
    source: &Path,
    transform: &DecodeTransform,
) -> Result<Option<DecodeStats>> {
    if transform
        .input_format
        .as_deref()
        .is_some_and(|v| !matches!(v, "mp4" | "mov"))
    {
        return Ok(None);
    }
    let mut reader = match crate::owned_mp4::Mp4Reader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        Default::default(),
    ) {
        Ok(value) => value,
        Err(error) if error.is_unsupported() => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let Some(index) = reader
        .tracks()
        .iter()
        .position(|track| track.handler == *b"vide")
    else {
        return Ok(None);
    };
    let track = &reader.tracks()[index];
    if !matches!(&track.codec, b"avc1" | b"avc3" | b"hvc1" | b"hev1") || track.rotation != 0 {
        return Ok(None);
    }
    let edits = match crate::owned_video_timeline::map_edits(
        track
            .edits
            .iter()
            .map(|edit| (edit.duration, edit.media_time)),
        track.timescale,
        reader.movie_timescale(),
    ) {
        Ok(value) => value,
        Err(Error::Unsupported(_)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let configuration = track.configuration.clone();
    let samples = track.samples.len();
    let avc = matches!(&track.codec, b"avc1" | b"avc3");
    let mut stats = DecodeStats {
        backend: "owned MP4 compressed video decode",
        video_frames: 0,
        width: 0,
        height: 0,
        pixel_format: String::new(),
        decode_errors: 0,
    };
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(samples)
        .map_err(|_| "MP4 presentation metadata allocation failed")?;
    let mut packet = Vec::new();
    if avc {
        let mut decoder = match AvcDecoder::new(&configuration, usize::MAX) {
            Ok(value) => value,
            Err(Error::Unsupported(_)) => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        for sample in 0..samples {
            let timing = reader.tracks()[index]
                .samples
                .get(sample)
                .ok_or("missing MP4 sample timing")?;
            reader
                .read_packet(index, sample, &mut packet)
                .map_err(|e| e.to_string())?;
            let decoded = match decoder.decode_order(&packet) {
                Ok(value) => value,
                Err(Error::Unsupported(_)) => return Ok(None),
                Err(error) => return Err(error.to_string()),
            };
            if let Some(picture) = decoded {
                let (width, height) = picture.dimensions();
                frames.push(FrameMetadata {
                    pts: timing.pts,
                    duration: i64::from(timing.duration),
                    sample,
                    size: [
                        u32::try_from(width).map_err(|_| "AVC width overflow")?,
                        u32::try_from(height).map_err(|_| "AVC height overflow")?,
                    ],
                    depth: picture.bit_depth,
                    sub: [true, true],
                    mono: false,
                });
            }
        }
    } else {
        let mut decoder = match HevcDecoder::from_configuration(&configuration, usize::MAX) {
            Ok(value) => value,
            Err(Error::Unsupported(_)) => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        for sample in 0..samples {
            let timing = reader.tracks()[index]
                .samples
                .get(sample)
                .ok_or("missing MP4 sample timing")?;
            reader
                .read_packet(index, sample, &mut packet)
                .map_err(|e| e.to_string())?;
            let decoded = match decoder.decode_packet(&packet) {
                Ok(value) => value,
                Err(Error::Unsupported(_)) => return Ok(None),
                Err(error) => return Err(error.to_string()),
            };
            if let Some(decoded) = decoded.filter(|value| value.output) {
                let picture = &decoded.picture;
                let size = [
                    picture.dimensions[0]
                        .checked_sub(picture.crop[0])
                        .and_then(|v| v.checked_sub(picture.crop[1]))
                        .ok_or("HEVC crop exceeds width")?,
                    picture.dimensions[1]
                        .checked_sub(picture.crop[2])
                        .and_then(|v| v.checked_sub(picture.crop[3]))
                        .ok_or("HEVC crop exceeds height")?,
                ];
                let chroma = decoder.parameters().0.chroma_format;
                let sub = match chroma {
                    0 | 3 => [false, false],
                    1 => [true, true],
                    2 => [true, false],
                    _ => return Err("invalid HEVC chroma format".into()),
                };
                frames.push(FrameMetadata {
                    pts: timing.pts,
                    duration: i64::from(timing.duration),
                    sample,
                    size,
                    depth: picture.depth[0],
                    sub,
                    mono: chroma == 0,
                });
            }
        }
    }
    normalize_presentations(&mut frames)?;
    if edits.is_empty() {
        for frame in &frames {
            if frame
                .pts
                .checked_add(frame.duration)
                .ok_or("video timestamp overflow")?
                > 0
            {
                frame.account(&mut stats)?;
            }
        }
    } else {
        for edit in &edits {
            // Normalization validated every endpoint and made ends monotone.
            let first =
                frames.partition_point(|frame| frame.pts + frame.duration <= edit.media_start);
            for frame in &frames[first..] {
                if frame.pts >= edit.media_end {
                    break;
                }
                if crate::owned_video_timeline::appearances(
                    std::slice::from_ref(edit),
                    frame.pts,
                    frame.duration,
                )
                .map_err(|e| e.to_string())?
                    != 0
                {
                    frame.account(&mut stats)?;
                }
            }
        }
    }
    if stats.video_frames == 0 {
        return Err("input has no visible decoded video frames".into());
    }
    Ok(Some(stats))
}

#[derive(Clone, Copy)]
struct FrameMetadata {
    pts: i64,
    duration: i64,
    sample: usize,
    size: [u32; 2],
    depth: u8,
    sub: [bool; 2],
    mono: bool,
}
impl FrameMetadata {
    fn account(&self, stats: &mut DecodeStats) -> Result<()> {
        crate::owned_compressed_video::account(
            stats, self.size, self.depth, self.sub, self.mono, false,
        )
    }
}
// Decode order determines the last equal-PTS picture. Display order determines
// intervals; the terminal equal-PTS group retains its accumulated nominal span.
fn normalize_presentations(frames: &mut Vec<FrameMetadata>) -> Result<()> {
    frames.sort_unstable_by_key(|frame| (frame.pts, frame.sample));
    let mut write = 0;
    for read in 0..frames.len() {
        let mut frame = frames[read];
        if write > 0 && frames[write - 1].pts == frame.pts {
            frame.duration = frame
                .duration
                .checked_add(frames[write - 1].duration)
                .ok_or("duplicate-PTS duration overflow")?;
            frames[write - 1] = frame;
        } else {
            frames[write] = frame;
            write += 1;
        }
    }
    frames.truncate(write);
    for index in 0..frames.len() {
        if index + 1 < frames.len() {
            frames[index].duration = frames[index + 1]
                .pts
                .checked_sub(frames[index].pts)
                .ok_or("presentation duration overflow")?;
        }
        if frames[index].duration <= 0 {
            return Err("invalid video duration".into());
        }
        frames[index]
            .pts
            .checked_add(frames[index].duration)
            .ok_or("video timestamp overflow")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(pts: i64, duration: i64, sample: usize) -> FrameMetadata {
        FrameMetadata {
            pts,
            duration,
            sample,
            size: [sample as u32 + 1, 1],
            depth: 8,
            sub: [true, true],
            mono: false,
        }
    }
    #[test]
    fn duplicate_pts_replace_metadata_and_keep_terminal_nominal_span() {
        let mut frames = vec![frame(2, 1, 0), frame(0, 0, 1), frame(2, 2, 2)];
        normalize_presentations(&mut frames).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!((frames[0].pts, frames[0].duration), (0, 2));
        assert_eq!(
            (frames[1].pts, frames[1].duration, frames[1].size),
            (2, 3, [3, 1])
        );
        assert!(normalize_presentations(&mut vec![frame(0, 0, 0)]).is_err());
        assert!(normalize_presentations(&mut vec![frame(i64::MAX, 1, 0)]).is_err());
    }
}
