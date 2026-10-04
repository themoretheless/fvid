//! Plain owned AVC/HEVC packet decoding. Edit-list workflows are admitted later.
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
    if !matches!(&track.codec, b"avc1" | b"avc3" | b"hvc1" | b"hev1")
        || track.rotation != 0
        || !track.edits.is_empty()
    {
        return Ok(None);
    }
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
    let mut packet = Vec::new();
    if avc {
        let mut decoder = match AvcDecoder::new(&configuration, usize::MAX) {
            Ok(value) => value,
            Err(Error::Unsupported(_)) => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        for sample in 0..samples {
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
                crate::owned_compressed_video::account(
                    &mut stats,
                    [
                        u32::try_from(width).map_err(|_| "AVC width overflow")?,
                        u32::try_from(height).map_err(|_| "AVC height overflow")?,
                    ],
                    picture.bit_depth,
                    [true, true],
                    false,
                    false,
                )?;
            }
        }
    } else {
        let mut decoder = match HevcDecoder::from_configuration(&configuration, usize::MAX) {
            Ok(value) => value,
            Err(Error::Unsupported(_)) => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        for sample in 0..samples {
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
                crate::owned_compressed_video::account(
                    &mut stats,
                    size,
                    picture.depth[0],
                    sub,
                    chroma == 0,
                    false,
                )?;
            }
        }
    }
    if stats.video_frames == 0 {
        return Err("input has no visible decoded video frames".into());
    }
    Ok(Some(stats))
}
