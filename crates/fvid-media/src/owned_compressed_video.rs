//! Owned packet decoding through the shared bitstream library.
//! Plain WebM decoding is the initial dispatch; transform/export visitors follow
//! the existing frame pipeline rather than silently ignoring requested options.
use fvid_codecs::codec::{av1_decoder, vp9, vp9_decoder};
use fvid_media_info::{DecodeStats, DecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn try_decode(
    source: &Path,
    transform: &DecodeTransform,
) -> Result<Option<DecodeStats>> {
    let mut request = transform.clone();
    request.input_format = None;
    if !transform
        .input_format
        .as_deref()
        .is_none_or(|v| matches!(v, "webm" | "matroska" | "mp4" | "mov"))
    {
        return Ok(None);
    }
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut magic = [0; 8];
    let count = file.read(&mut magic).map_err(|e| e.to_string())?;
    if count == 8 && crate::owned_mp4::recognizes_prefix(&magic) {
        if request == DecodeTransform::default() {
            return crate::owned_mp4_video_decode::try_decode(source, transform);
        }
        if transform
            .input_format
            .as_deref()
            .is_some_and(|v| !matches!(v, "mp4" | "mov"))
        {
            return Ok(None);
        }
        let Some(temporary) = crate::owned_mp4_video_bridge::prepare_video(source)? else {
            return Ok(None);
        };
        let mut request = transform.clone();
        request.input_format = Some("matroska".into());
        let mut result = crate::owned_video_decode::try_webm(&temporary.path, &request)?;
        if let Some(stats) = result.as_mut() {
            stats.backend = "owned MP4 compressed video pipeline";
        }
        return Ok(result);
    }
    if request != DecodeTransform::default() {
        return crate::owned_video_decode::try_webm(source, transform);
    }
    if count < 4 || magic[..4] != [0x1a, 0x45, 0xdf, 0xa3] {
        return Ok(None);
    }
    if transform
        .input_format
        .as_deref()
        .is_some_and(|v| !matches!(v, "webm" | "matroska"))
    {
        return Ok(None);
    }
    let mut reader = crate::owned_webm::WebmReader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    reader.scan_all().map_err(|e| e.to_string())?;
    let track = reader
        .tracks
        .iter()
        .find(|v| v.kind == 1)
        .ok_or("input has no video stream")?;
    if !matches!(track.codec.as_str(), "V_VP9" | "V_AV1")
        || track.crop != [0; 4]
        || track.rotation != 0
    {
        return Ok(None);
    }
    let codec = track.codec.clone();
    let number = track.number;
    let private = track.codec_private.clone();
    let mut stats = DecodeStats {
        backend: "owned WebM compressed video decode",
        video_frames: 0,
        width: u32::try_from(track.width).map_err(|_| "video width exceeds API range")?,
        height: u32::try_from(track.height).map_err(|_| "video height exceeds API range")?,
        pixel_format: String::new(),
        decode_errors: 0,
    };
    // No arbitrary frame-memory ceiling: each decoder validates allocation sizes.
    let mut vp9 = vp9_decoder::Decoder::new(usize::MAX);
    let mut av1 = match av1_decoder::Decoder::from_configuration(
        if codec == "V_AV1" { &private } else { &[] },
        usize::MAX,
    ) {
        Ok(value) => value,
        Err(fvid_codecs::Error::Unsupported(_)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };

    for index in 0..reader.packets.len() {
        let metadata = &reader.packets[index];
        if metadata.track != number {
            continue;
        }
        let visible = !metadata.invisible;
        let packet = reader.read_packet(index).map_err(|e| e.to_string())?;
        if codec == "V_VP9" {
            for frame in vp9::frames(&packet).map_err(|e| e.to_string())? {
                let decoded = match vp9.decode(frame) {
                    Ok(value) => value,
                    Err(fvid_codecs::Error::Unsupported(_)) => return Ok(None),
                    Err(error) => return Err(error.to_string()),
                };
                if visible && decoded.header.show_frame {
                    let format = decoded.header.picture.format;
                    account(
                        &mut stats,
                        decoded.picture.size,
                        decoded.picture.depth,
                        format.subsampling,
                        false,
                        format.color_space == 7,
                    )?;
                }
            }
        } else {
            let decoded_frames = match av1.decode_packet(&packet) {
                Ok(value) => value,
                Err(fvid_codecs::Error::Unsupported(_)) => return Ok(None),
                Err(error) => return Err(error.to_string()),
            };
            for decoded in decoded_frames {
                if visible && decoded.show {
                    account(
                        &mut stats,
                        decoded.picture.size,
                        decoded.picture.depth,
                        decoded.color.subsampling,
                        decoded.color.monochrome,
                        decoded.color.matrix == 0,
                    )?;
                }
            }
        }
    }
    if stats.pixel_format.is_empty() {
        return Err("input has no visible decoded video frames".into());
    }
    Ok(Some(stats))
}
pub(crate) fn account(
    stats: &mut DecodeStats,
    size: [u32; 2],
    depth: u8,
    sub: [bool; 2],
    mono: bool,
    rgb: bool,
) -> Result<()> {
    stats.video_frames = stats
        .video_frames
        .checked_add(1)
        .ok_or("video frame count overflow")?;
    [stats.width, stats.height] = size;
    let base = if mono {
        "gray"
    } else if rgb {
        "gbrp"
    } else {
        match sub {
            [true, true] => "yuv420p",
            [true, false] => "yuv422p",
            [false, false] => "yuv444p",
            _ => return Err("unsupported decoded chroma geometry".into()),
        }
    };
    stats.pixel_format = if depth == 8 {
        base.into()
    } else {
        format!("{base}{depth}le")
    };
    Ok(())
}
