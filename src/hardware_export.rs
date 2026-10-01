//! Streaming hardware video encode into the native Matroska muxer.
use crate::container::matroska_write::{Encoding, PacketWriter, TrackSpec};
pub use crate::container::matroska_write::{TrackOptions, VideoMetadata};
use crate::{Error, Result, invalid};
pub use fvid_vt::{EncodedTime, EncoderCodec, Surface};
use std::io::{Seek, Write};

pub struct HardwareFrame {
    pub surface: Surface,
    pub pts: EncodedTime,
    pub duration: EncodedTime,
    pub force_keyframe: bool,
}
#[derive(Debug, Default)]
pub struct ExportStats {
    pub frames: u64,
    pub compressed_bytes: u64,
}

/// Video track streaming API; the caller supplies all timestamps and handles
/// destination publication. Audio/subtitles are separate tracks, not implicitly
/// copied. Pixel surfaces remain native; only compressed packets reach the CPU.
pub fn write_video<W: Write + Seek>(
    output: &mut W,
    frames: impl IntoIterator<Item = Result<HardwareFrame>>,
    codec: EncoderCodec,
    bits_per_second: u32,
) -> Result<ExportStats> {
    write_video_with_options(
        output,
        frames,
        codec,
        bits_per_second,
        TrackOptions::default(),
    )
}
/// Describe the output signal, HDR data and display geometry explicitly.
/// These options belong to the encoded result, including any applied shader.
pub fn write_video_with_options<W: Write + Seek>(
    output: &mut W,
    frames: impl IntoIterator<Item = Result<HardwareFrame>>,
    codec: EncoderCodec,
    bits_per_second: u32,
    options: TrackOptions,
) -> Result<ExportStats> {
    if options.codec_delay_ns != 0 {
        return Err(invalid(
            "video hardware export does not take audio codec delay",
        ));
    }
    let mut frames = frames.into_iter();
    let first = frames
        .next()
        .ok_or_else(|| invalid("hardware export requires a frame"))??;
    let width =
        u32::try_from(first.surface.width()).map_err(|_| invalid("export width overflow"))?;
    let height =
        u32::try_from(first.surface.height()).map_err(|_| invalid("export height overflow"))?;
    let full_range = first.surface.full_range();
    let mut encoder = fvid_vt::Encoder::new_with_bitrate(
        codec,
        width as usize,
        height as usize,
        first.surface.depth(),
        bits_per_second,
    )
    .map_err(gpu)?;
    let first = encode(&mut encoder, first)?;
    let configuration = &first.decoder_configuration;
    let encoding = match codec {
        EncoderCodec::H264 => Encoding::Avc {
            configuration,
            width,
            height,
        },
        EncoderCodec::Hevc => Encoding::Hevc {
            configuration,
            width,
            height,
        },
    };
    let mut writer = PacketWriter::new_with_options(
        output,
        &[TrackSpec {
            encoding,
            name: "hardware video",
            language: "und",
        }],
        &[options],
    )?;
    let mut stats = ExportStats::default();
    let mut previous = None;
    let mut write = |frame: &fvid_vt::EncodedFrame| -> Result<()> {
        if &frame.decoder_configuration != configuration {
            return Err(invalid(
                "hardware encoder configuration changed during export",
            ));
        }
        let pts = u64::try_from(frame.pts.nanoseconds().map_err(gpu)?).map_err(|_| {
            invalid("export needs a nonnegative timeline; offset negative PTS explicitly")
        })?;
        let duration = u64::try_from(frame.duration.nanoseconds().map_err(gpu)?)
            .map_err(|_| invalid("negative encoded duration"))?;
        if duration == 0 || previous.is_some_and(|value| pts <= value) {
            return Err(invalid(
                "export requires increasing PTS and positive nanosecond duration",
            ));
        }
        writer.write_packet(0, pts, duration, frame.key_frame, &frame.data)?;
        previous = Some(pts);
        stats.frames += 1;
        stats.compressed_bytes = stats
            .compressed_bytes
            .checked_add(frame.data.len() as u64)
            .ok_or_else(|| invalid("export byte count overflow"))?;
        Ok(())
    };
    write(&first)?;
    for frame in frames {
        let frame = frame?;
        if frame.surface.full_range() != full_range {
            return Err(invalid("hardware surface range changed during export"));
        }
        write(&encode(&mut encoder, frame)?)?;
    }
    drop(write);
    writer.finish()?;
    Ok(stats)
}
fn gpu(error: fvid_vt::Error) -> Error {
    Error::Gpu(error.to_string())
}
fn encode(encoder: &mut fvid_vt::Encoder, frame: HardwareFrame) -> Result<fvid_vt::EncodedFrame> {
    let scale = i32::try_from(frame.pts.timescale)
        .map_err(|_| invalid("encoder timescale exceeds signed 32-bit range"))?;
    if scale == 0 || frame.duration.timescale == 0 {
        return Err(invalid("zero hardware frame timescale"));
    }
    let scaled = i128::from(frame.duration.value) * i128::from(scale);
    if scaled % i128::from(frame.duration.timescale) != 0 {
        return Err(invalid("duration cannot be represented in PTS timescale"));
    }
    let duration = i64::try_from(scaled / i128::from(frame.duration.timescale))
        .map_err(|_| invalid("hardware duration overflow"))?;
    encoder
        .encode_with_keyframe(
            &frame.surface,
            frame.pts.value,
            duration,
            scale,
            frame.force_keyframe,
        )
        .map_err(gpu)
}
